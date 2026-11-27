use std::collections::BTreeMap;

use glam::{Mat4, Vec3};
use serde_json::{json, Value};
use tessel_core::{AutoSkinSolver, SceneState};

use crate::error::IoError;
use crate::ExportPreset;

use super::collect::{CollectedMesh, PaletteMaterialGroup};

#[derive(Default)]
struct GltfExtensionFlags {
    unlit: bool,
    emissive_strength: bool,
    transmission: bool,
    ior: bool,
}

impl GltfExtensionFlags {
    fn to_extensions_used(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.unlit {
            out.push("KHR_materials_unlit");
        }
        if self.emissive_strength {
            out.push("KHR_materials_emissive_strength");
        }
        if self.transmission {
            out.push("KHR_materials_transmission");
        }
        if self.ior {
            out.push("KHR_materials_ior");
        }
        out
    }
}

fn build_material_extensions(
    unlit: bool,
    has_emissive: bool,
    emissive_intensity: f32,
    transmission: f32,
    ior: f32,
    flags: &mut GltfExtensionFlags,
) -> serde_json::Map<String, Value> {
    let mut exts = serde_json::Map::new();
    if unlit {
        flags.unlit = true;
        exts.insert("KHR_materials_unlit".into(), json!({}));
    }
    if has_emissive && (emissive_intensity - 1.0).abs() > 1e-4 {
        flags.emissive_strength = true;
        exts.insert(
            "KHR_materials_emissive_strength".into(),
            json!({ "emissiveStrength": emissive_intensity }),
        );
    }
    if transmission > 0.0 {
        flags.transmission = true;
        exts.insert(
            "KHR_materials_transmission".into(),
            json!({ "transmissionFactor": transmission }),
        );
    }
    if transmission > 0.0 || (ior - 1.5).abs() > 1e-4 {
        flags.ior = true;
        exts.insert("KHR_materials_ior".into(), json!({ "ior": ior }));
    }
    exts
}

fn build_palette_group_material_json(
    name: String,
    tex_idx: usize,
    group: PaletteMaterialGroup,
    flags: &mut GltfExtensionFlags,
) -> Value {
    let mut mat_json = json!({
        "name": name,
        "pbrMetallicRoughness": {
            "baseColorTexture": { "index": tex_idx },
            "metallicFactor": group.metallic(),
            "roughnessFactor": group.roughness()
        }
    });
    let emissive_strength = group.emissive();
    let has_emissive = emissive_strength > 0.0;
    let ext_emissive_strength = if has_emissive {
        let base_rgb = group.emissive_rgb();
        if emissive_strength <= 1.0 {
            mat_json["emissiveFactor"] = json!([
                base_rgb[0] * emissive_strength,
                base_rgb[1] * emissive_strength,
                base_rgb[2] * emissive_strength
            ]);
            1.0
        } else {
            mat_json["emissiveFactor"] = json!(base_rgb);
            emissive_strength
        }
    } else {
        1.0
    };
    let exts = build_material_extensions(
        false,
        has_emissive,
        ext_emissive_strength,
        group.transmission(),
        group.ior(),
        flags,
    );
    if !exts.is_empty() {
        mat_json["extensions"] = Value::Object(exts);
    }
    mat_json
}
use super::helpers::{
    align_quat_hemispheres, build_palette_png, collect_node_transform_animated,
    collect_visibility_animated_nodes, encode_rgba_png, expand_mixed_step_keyframes,
    has_group_ancestor, has_hidden_group_ancestor, is_cubic_interpolation, is_morphed_export_node,
    node_world_matrices, pack_cubicspline, pad4, sample_morph_keys,
};

/// Builds a valid glTF 2.0 Binary (`.glb`) with an embedded 256x1 nearest-neighbor palette PNG,
/// plus optional `CharacterRig` bone hierarchy nodes and `AnimationClip` channels.
pub fn export_glb_bytes(
    scene: &SceneState,
    preset: ExportPreset,
) -> Result<(Vec<u8>, usize), IoError> {
    let scale = preset.scale_factor();
    let skin_solver = scene
        .character_rig
        .as_ref()
        .and_then(|rig| AutoSkinSolver::from_rig(rig, scale));
    let node_is_skinned = |node_id: tessel_core::NodeId| -> bool {
        skin_solver
            .as_ref()
            .is_some_and(|s| s.node_has_skin_influence(node_id))
            && scene
                .character_rig
                .as_ref()
                .is_some_and(|rig| rig.should_skin_node(node_id))
    };
    let (mesh, static_mesh) = if skin_solver.is_some() {
        (
            CollectedMesh::from_scene_untextured_filtered(
                scene,
                preset,
                skin_solver.as_ref(),
                &node_is_skinned,
            ),
            CollectedMesh::from_scene_untextured_filtered(scene, preset, None, |nid| {
                !node_is_skinned(nid)
            }),
        )
    } else {
        (
            CollectedMesh::from_scene_untextured(scene, preset, None),
            CollectedMesh::empty(),
        )
    };
    let has_palette_prim = !mesh.positions.is_empty() && !mesh.indices.is_empty();
    let has_static_palette_prim =
        !static_mesh.positions.is_empty() && !static_mesh.indices.is_empty();
    let palette_png = build_palette_png(&scene.palette)?;

    let mut bin = Vec::new();
    let mut buffer_views: Vec<Value> = Vec::new();
    let mut accessors: Vec<Value> = Vec::new();

    let push_view =
        |bin: &mut Vec<u8>, views: &mut Vec<Value>, bytes: &[u8], target: Option<u32>| -> usize {
            let offset = bin.len();
            bin.extend_from_slice(bytes);
            let len = bytes.len();
            pad4(bin, 0);
            let idx = views.len();
            let mut entry = json!({
                "buffer": 0,
                "byteOffset": offset,
                "byteLength": len,
            });
            if let Some(t) = target {
                entry["target"] = json!(t);
            }
            views.push(entry);
            idx
        };

    let mut images: Vec<Value> = Vec::new();
    let mut textures: Vec<Value> = Vec::new();
    let mut materials: Vec<Value> = Vec::new();
    let mut primitives: Vec<Value> = Vec::new();
    let mut static_primitives: Vec<Value> = Vec::new();
    let mut palette_tex_index: Option<usize> = None;
    let mut palette_mat_index: Option<usize> = None;
    let mut palette_group_materials: BTreeMap<PaletteMaterialGroup, usize> = BTreeMap::new();
    let mut ext_flags = GltfExtensionFlags::default();
    let mut total_triangles = 0usize;

    let ensure_palette_tex = |palette_tex_index: &mut Option<usize>,
                              bin: &mut Vec<u8>,
                              buffer_views: &mut Vec<Value>,
                              images: &mut Vec<Value>,
                              textures: &mut Vec<Value>|
     -> usize {
        if let Some(existing) = *palette_tex_index {
            return existing;
        }
        let img_bv = push_view(bin, buffer_views, &palette_png, None);
        let img_idx = images.len();
        images.push(json!({
            "bufferView": img_bv,
            "mimeType": "image/png"
        }));
        let tex_idx = textures.len();
        textures.push(json!({
            "sampler": 0,
            "source": img_idx
        }));
        *palette_tex_index = Some(tex_idx);
        tex_idx
    };

    if has_palette_prim {
        let groups = mesh.split_by_palette_material(&scene.palette);
        for (group, sub) in groups {
            if sub.positions.is_empty() || sub.indices.is_empty() {
                continue;
            }
            let mut g_min = [f32::INFINITY; 3];
            let mut g_max = [f32::NEG_INFINITY; 3];
            for p in &sub.positions {
                for axis in 0..3 {
                    g_min[axis] = g_min[axis].min(p[axis]);
                    g_max[axis] = g_max[axis].max(p[axis]);
                }
            }
            let pos_bv = push_view(
                &mut bin,
                &mut buffer_views,
                bytemuck::cast_slice(&sub.positions),
                Some(34962),
            );
            let norm_bv = push_view(
                &mut bin,
                &mut buffer_views,
                bytemuck::cast_slice(&sub.normals),
                Some(34962),
            );
            let uv_bv = push_view(
                &mut bin,
                &mut buffer_views,
                bytemuck::cast_slice(&sub.uvs),
                Some(34962),
            );
            let idx_bv = push_view(
                &mut bin,
                &mut buffer_views,
                bytemuck::cast_slice(&sub.indices),
                Some(34963),
            );
            let tex_idx = ensure_palette_tex(
                &mut palette_tex_index,
                &mut bin,
                &mut buffer_views,
                &mut images,
                &mut textures,
            );
            let mat_idx = if let Some(&existing) = palette_group_materials.get(&group) {
                existing
            } else {
                let m_idx = materials.len();
                let mat_name = if group.is_default {
                    "TesselPaletteMaterial".to_string()
                } else {
                    format!("TesselPaletteMaterial_{m_idx}")
                };
                materials.push(build_palette_group_material_json(
                    mat_name,
                    tex_idx,
                    group,
                    &mut ext_flags,
                ));
                palette_group_materials.insert(group, m_idx);
                if group.is_default {
                    palette_mat_index = Some(m_idx);
                }
                m_idx
            };

            let p_acc = accessors.len();
            accessors.push(json!({
                "bufferView": pos_bv,
                "componentType": 5126,
                "count": sub.positions.len(),
                "type": "VEC3",
                "min": g_min,
                "max": g_max
            }));
            let n_acc = accessors.len();
            accessors.push(json!({
                "bufferView": norm_bv,
                "componentType": 5126,
                "count": sub.normals.len(),
                "type": "VEC3"
            }));
            let u_acc = accessors.len();
            accessors.push(json!({
                "bufferView": uv_bv,
                "componentType": 5126,
                "count": sub.uvs.len(),
                "type": "VEC2"
            }));
            let i_acc = accessors.len();
            accessors.push(json!({
                "bufferView": idx_bv,
                "componentType": 5125,
                "count": sub.indices.len(),
                "type": "SCALAR"
            }));
            let mut prim_attrs = json!({
                "POSITION": p_acc,
                "NORMAL": n_acc,
                "TEXCOORD_0": u_acc
            });
            if skin_solver.is_some() && sub.joints.len() == sub.positions.len() {
                let j_bv = push_view(
                    &mut bin,
                    &mut buffer_views,
                    bytemuck::cast_slice(&sub.joints),
                    Some(34962),
                );
                let w_bv = push_view(
                    &mut bin,
                    &mut buffer_views,
                    bytemuck::cast_slice(&sub.weights),
                    Some(34962),
                );
                let j_acc = accessors.len();
                accessors.push(json!({
                    "bufferView": j_bv,
                    "componentType": 5123,
                    "count": sub.joints.len(),
                    "type": "VEC4"
                }));
                let w_acc = accessors.len();
                accessors.push(json!({
                    "bufferView": w_bv,
                    "componentType": 5126,
                    "count": sub.weights.len(),
                    "type": "VEC4"
                }));
                prim_attrs["JOINTS_0"] = json!(j_acc);
                prim_attrs["WEIGHTS_0"] = json!(w_acc);
            }
            primitives.push(json!({
                "attributes": prim_attrs,
                "indices": i_acc,
                "material": mat_idx
            }));
            total_triangles += sub.indices.len() / 3;
        }
    }

    if has_static_palette_prim {
        let groups = static_mesh.split_by_palette_material(&scene.palette);
        for (group, sub) in groups {
            if sub.positions.is_empty() || sub.indices.is_empty() {
                continue;
            }
            let mut s_min = [f32::INFINITY; 3];
            let mut s_max = [f32::NEG_INFINITY; 3];
            for p in &sub.positions {
                for axis in 0..3 {
                    s_min[axis] = s_min[axis].min(p[axis]);
                    s_max[axis] = s_max[axis].max(p[axis]);
                }
            }
            let pos_bv = push_view(
                &mut bin,
                &mut buffer_views,
                bytemuck::cast_slice(&sub.positions),
                Some(34962),
            );
            let norm_bv = push_view(
                &mut bin,
                &mut buffer_views,
                bytemuck::cast_slice(&sub.normals),
                Some(34962),
            );
            let uv_bv = push_view(
                &mut bin,
                &mut buffer_views,
                bytemuck::cast_slice(&sub.uvs),
                Some(34962),
            );
            let idx_bv = push_view(
                &mut bin,
                &mut buffer_views,
                bytemuck::cast_slice(&sub.indices),
                Some(34963),
            );
            let p_acc = accessors.len();
            accessors.push(json!({
                "bufferView": pos_bv,
                "componentType": 5126,
                "count": sub.positions.len(),
                "type": "VEC3",
                "min": s_min,
                "max": s_max
            }));
            let n_acc = accessors.len();
            accessors.push(json!({
                "bufferView": norm_bv,
                "componentType": 5126,
                "count": sub.normals.len(),
                "type": "VEC3"
            }));
            let u_acc = accessors.len();
            accessors.push(json!({
                "bufferView": uv_bv,
                "componentType": 5126,
                "count": sub.uvs.len(),
                "type": "VEC2"
            }));
            let i_acc = accessors.len();
            accessors.push(json!({
                "bufferView": idx_bv,
                "componentType": 5125,
                "count": sub.indices.len(),
                "type": "SCALAR"
            }));
            let tex_idx = ensure_palette_tex(
                &mut palette_tex_index,
                &mut bin,
                &mut buffer_views,
                &mut images,
                &mut textures,
            );
            let mat_idx = if let Some(&existing) = palette_group_materials.get(&group) {
                existing
            } else {
                let m_idx = materials.len();
                let mat_name = if group.is_default {
                    "TesselPaletteMaterial".to_string()
                } else {
                    format!("TesselPaletteMaterial_{m_idx}")
                };
                materials.push(build_palette_group_material_json(
                    mat_name,
                    tex_idx,
                    group,
                    &mut ext_flags,
                ));
                palette_group_materials.insert(group, m_idx);
                if group.is_default {
                    palette_mat_index = Some(m_idx);
                }
                m_idx
            };
            static_primitives.push(json!({
                "attributes": {
                    "POSITION": p_acc,
                    "NORMAL": n_acc,
                    "TEXCOORD_0": u_acc
                },
                "indices": i_acc,
                "material": mat_idx
            }));
            total_triangles += sub.indices.len() / 3;
        }
    }

    let mut nodes: Vec<Value> = vec![json!({ "name": "TesselAsset" })];
    let mut root_nodes: Vec<usize> = vec![0];
    let mut animations: Vec<Value> = Vec::new();
    let mut skins: Vec<Value> = Vec::new();

    // Morphed and visibility-animated meshes get dedicated export nodes appended
    // after the bone nodes by the custom-primitive pass below. Precompute their
    // order here (same BTreeMap order and same skip filters) so animation channels
    // and node indices agree.
    let vis_animated_nodes = collect_visibility_animated_nodes(scene);
    let node_trs_animated = collect_node_transform_animated(scene);
    let group_order: Vec<tessel_core::NodeId> = scene
        .nodes
        .iter()
        .filter(|(id, n)| {
            n.visible
                && n.kind == tessel_core::NodeKind::Group
                && !has_hidden_group_ancestor(scene, **id)
        })
        .map(|(id, _)| *id)
        .collect();
    let morphed_order: Vec<tessel_core::NodeId> = scene
        .lowpoly_meshes
        .iter()
        .filter(|(id, m)| {
            is_morphed_export_node(scene, **id, m, &vis_animated_nodes, &node_trs_animated)
        })
        .map(|(id, _)| *id)
        .collect();
    let group_base_node = 1 + scene
        .character_rig
        .as_ref()
        .map(|r| r.bones.len())
        .unwrap_or(0);
    let morph_base_node = group_base_node + group_order.len();
    let mut group_to_gltf: BTreeMap<tessel_core::NodeId, usize> = BTreeMap::new();
    let mut scene_node_to_gltf: BTreeMap<tessel_core::NodeId, usize> = BTreeMap::new();
    for (gi, &gid) in group_order.iter().enumerate() {
        let gidx = group_base_node + gi;
        group_to_gltf.insert(gid, gidx);
        scene_node_to_gltf.insert(gid, gidx);
    }
    for (mi, &mid) in morphed_order.iter().enumerate() {
        scene_node_to_gltf.insert(mid, morph_base_node + mi);
    }

    let mut bone_to_node: BTreeMap<tessel_core::BoneId, usize> = BTreeMap::new();
    if let Some(rig) = &scene.character_rig {
        for bone in &rig.bones {
            let node_idx = nodes.len();
            bone_to_node.insert(bone.id, node_idx);
            let t = bone.rest_local.translation * scale;
            let r = bone.rest_local.rotation.to_array();
            let s = bone.rest_local.scale.to_array();
            nodes.push(json!({
                "name": bone.name,
                "translation": [t.x, t.y, t.z],
                "rotation": r,
                "scale": s
            }));
        }

        for bone in &rig.bones {
            let child_idx = bone_to_node[&bone.id];
            match bone.parent.and_then(|pid| bone_to_node.get(&pid).copied()) {
                Some(parent_idx) => {
                    let children = nodes[parent_idx].as_object_mut().and_then(|obj| {
                        obj.entry("children")
                            .or_insert_with(|| json!([]))
                            .as_array_mut()
                    });
                    if let Some(arr) = children {
                        arr.push(json!(child_idx));
                    }
                }
                None => root_nodes.push(child_idx),
            }
        }

        for (clip_idx, raw_clip) in rig.animations.iter().enumerate() {
            let baked_clip;
            let clip = if let Some(fps) = raw_clip.stepped_fps {
                baked_clip = raw_clip.bake_stepped(fps);
                &baked_clip
            } else {
                raw_clip
            };
            let mut samplers: Vec<Value> = Vec::new();
            let mut channels: Vec<Value> = Vec::new();

            for ch in &clip.channels {
                let Some(&target_node) = bone_to_node.get(&ch.bone_id) else {
                    continue;
                };
                if ch.keyframes.is_empty() {
                    continue;
                }
                let raw_times: Vec<f32> = ch.keyframes.iter().map(|k| k.time_secs).collect();
                let raw_interps: Vec<tessel_core::KeyframeInterpolation> =
                    ch.keyframes.iter().map(|k| k.interpolation).collect();
                let is_cubic = ch.keyframes.len() >= 2
                    && raw_interps.iter().copied().any(is_cubic_interpolation);
                let has_mixed_step =
                    is_cubic && raw_interps.contains(&tessel_core::KeyframeInterpolation::Step);
                let (times, interps, kf_indices): (
                    Vec<f32>,
                    Vec<tessel_core::KeyframeInterpolation>,
                    Vec<usize>,
                ) = if has_mixed_step {
                    let idx_list: Vec<usize> = (0..ch.keyframes.len()).collect();
                    expand_mixed_step_keyframes(&raw_times, &raw_interps, &idx_list)
                } else {
                    (raw_times, raw_interps, (0..ch.keyframes.len()).collect())
                };
                let min_t = times.iter().copied().fold(f32::INFINITY, f32::min);
                let max_t = times.iter().copied().fold(f32::NEG_INFINITY, f32::max);

                let time_bv = push_view(
                    &mut bin,
                    &mut buffer_views,
                    bytemuck::cast_slice(&times),
                    None,
                );
                let time_acc = accessors.len();
                accessors.push(json!({
                    "bufferView": time_bv,
                    "componentType": 5126,
                    "count": times.len(),
                    "type": "SCALAR",
                    "min": [min_t],
                    "max": [max_t]
                }));

                let interp_mode = if is_cubic {
                    "CUBICSPLINE"
                } else if interps
                    .iter()
                    .all(|&i| i == tessel_core::KeyframeInterpolation::Step)
                {
                    "STEP"
                } else {
                    "LINEAR"
                };

                let rest = rig.bones.iter().find(|b| b.id == ch.bone_id);
                let rest_r = rest
                    .map(|b| b.rest_local.rotation)
                    .unwrap_or(glam::Quat::IDENTITY);
                let raw_rots: Vec<[f32; 4]> = kf_indices
                    .iter()
                    .map(|&idx| (rest_r * ch.keyframes[idx].rotation).normalize().to_array())
                    .collect();
                let rots: Vec<[f32; 4]> = if is_cubic {
                    let aligned = align_quat_hemispheres(raw_rots);
                    pack_cubicspline(&times, &interps, &aligned)
                } else {
                    raw_rots
                };
                let rot_bv = push_view(
                    &mut bin,
                    &mut buffer_views,
                    bytemuck::cast_slice(&rots),
                    None,
                );
                let rot_acc = accessors.len();
                accessors.push(json!({
                    "bufferView": rot_bv,
                    "componentType": 5126,
                    "count": rots.len(),
                    "type": "VEC4"
                }));

                let sampler_idx = samplers.len();
                samplers.push(json!({
                    "input": time_acc,
                    "interpolation": interp_mode,
                    "output": rot_acc
                }));
                channels.push(json!({
                    "sampler": sampler_idx,
                    "target": { "node": target_node, "path": "rotation" }
                }));

                // Keyframe translation is an offset added to the rest pose
                // (rig.rs pose composition); export absolute local values so
                // external players match the viewport. Skipped when all zero
                // to keep rotation-only clips byte-identical.
                let rest = rig.bones.iter().find(|b| b.id == ch.bone_id);
                let rest_t = rest.map(|b| b.rest_local.translation).unwrap_or(Vec3::ZERO);
                if ch.keyframes.iter().any(|k| k.translation != Vec3::ZERO) {
                    let raw_trans: Vec<[f32; 3]> = kf_indices
                        .iter()
                        .map(|&idx| ((rest_t + ch.keyframes[idx].translation) * scale).to_array())
                        .collect();
                    let trans: Vec<[f32; 3]> = if is_cubic {
                        pack_cubicspline(&times, &interps, &raw_trans)
                    } else {
                        raw_trans
                    };
                    let trans_bv = push_view(
                        &mut bin,
                        &mut buffer_views,
                        bytemuck::cast_slice(&trans),
                        None,
                    );
                    let trans_acc = accessors.len();
                    accessors.push(json!({
                        "bufferView": trans_bv,
                        "componentType": 5126,
                        "count": trans.len(),
                        "type": "VEC3"
                    }));
                    let sampler_idx = samplers.len();
                    samplers.push(json!({
                        "input": time_acc,
                        "interpolation": interp_mode,
                        "output": trans_acc
                    }));
                    channels.push(json!({
                        "sampler": sampler_idx,
                        "target": { "node": target_node, "path": "translation" }
                    }));
                }

                // Keyframe scale multiplies the rest scale; same skip rule.
                let rest_s = rest.map(|b| b.rest_local.scale).unwrap_or(Vec3::ONE);
                if ch.keyframes.iter().any(|k| k.scale != Vec3::ONE) {
                    let raw_scales: Vec<[f32; 3]> = kf_indices
                        .iter()
                        .map(|&idx| (rest_s * ch.keyframes[idx].scale).to_array())
                        .collect();
                    let scales: Vec<[f32; 3]> = if is_cubic {
                        pack_cubicspline(&times, &interps, &raw_scales)
                    } else {
                        raw_scales
                    };
                    let scale_bv = push_view(
                        &mut bin,
                        &mut buffer_views,
                        bytemuck::cast_slice(&scales),
                        None,
                    );
                    let scale_acc = accessors.len();
                    accessors.push(json!({
                        "bufferView": scale_bv,
                        "componentType": 5126,
                        "count": scales.len(),
                        "type": "VEC3"
                    }));
                    let sampler_idx = samplers.len();
                    samplers.push(json!({
                        "input": time_acc,
                        "interpolation": interp_mode,
                        "output": scale_acc
                    }));
                    channels.push(json!({
                        "sampler": sampler_idx,
                        "target": { "node": target_node, "path": "scale" }
                    }));
                }
            }

            // Morph weight tracks: one channel per animated mesh node. The
            // output holds the FULL weight array per keyframe (all targets,
            // missing tracks read as their sampled value or 0.0), matching
            // the glTF "weights" path layout.
            let mut tracks_by_node: BTreeMap<
                tessel_core::NodeId,
                Vec<&tessel_core::MorphWeightTrack>,
            > = BTreeMap::new();
            for track in &clip.morph_tracks {
                if !track.keys.is_empty() {
                    tracks_by_node.entry(track.node).or_default().push(track);
                }
            }
            for (mnode, tracks) in &tracks_by_node {
                let Some(pos) = morphed_order.iter().position(|id| id == mnode) else {
                    continue;
                };
                let Some(mesh) = scene.lowpoly_meshes.get(mnode) else {
                    continue;
                };
                if mesh.morphs.is_empty() {
                    continue;
                }
                let mut times: Vec<f32> = tracks
                    .iter()
                    .flat_map(|t| t.keys.iter().map(|k| k.time_secs))
                    .collect();
                times.sort_by(f32::total_cmp);
                times.dedup_by(|a, b| (*a - *b).abs() <= 1e-3);
                let mut out: Vec<f32> = Vec::with_capacity(times.len() * mesh.morphs.len());
                for t in &times {
                    for morph in &mesh.morphs {
                        let w = tracks
                            .iter()
                            .find(|tr| tr.target == morph.name)
                            .map(|tr| sample_morph_keys(&tr.keys, *t))
                            .unwrap_or(0.0);
                        out.push(if w.is_finite() { w } else { 0.0 });
                    }
                }
                let min_t = times.iter().copied().fold(f32::INFINITY, f32::min);
                let max_t = times.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                let time_bv = push_view(
                    &mut bin,
                    &mut buffer_views,
                    bytemuck::cast_slice(&times),
                    None,
                );
                let time_acc = accessors.len();
                accessors.push(json!({
                    "bufferView": time_bv,
                    "componentType": 5126,
                    "count": times.len(),
                    "type": "SCALAR",
                    "min": [min_t],
                    "max": [max_t]
                }));
                let out_bv = push_view(
                    &mut bin,
                    &mut buffer_views,
                    bytemuck::cast_slice(&out),
                    None,
                );
                let out_acc = accessors.len();
                accessors.push(json!({
                    "bufferView": out_bv,
                    "componentType": 5126,
                    "count": out.len(),
                    "type": "SCALAR"
                }));
                let sampler_idx = samplers.len();
                samplers.push(json!({
                    "input": time_acc,
                    "interpolation": "LINEAR",
                    "output": out_acc
                }));
                channels.push(json!({
                    "sampler": sampler_idx,
                    "target": { "node": morph_base_node + pos, "path": "weights" }
                }));
            }

            // Export per-node visibility tracks as STEP-interpolated scale [0,0,0] / [1,1,1].
            for vis_track in &clip.visibility_tracks {
                if vis_track.keys.is_empty() {
                    continue;
                }
                let Some(pos) = morphed_order.iter().position(|id| *id == vis_track.node) else {
                    continue;
                };
                let times: Vec<f32> = vis_track.keys.iter().map(|k| k.time_secs).collect();
                let scales: Vec<[f32; 3]> = vis_track
                    .keys
                    .iter()
                    .map(|k| {
                        if k.visible {
                            [1.0, 1.0, 1.0]
                        } else {
                            [0.0, 0.0, 0.0]
                        }
                    })
                    .collect();
                let min_t = times.iter().copied().fold(f32::INFINITY, f32::min);
                let max_t = times.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                let time_bv = push_view(
                    &mut bin,
                    &mut buffer_views,
                    bytemuck::cast_slice(&times),
                    None,
                );
                let time_acc = accessors.len();
                accessors.push(json!({
                    "bufferView": time_bv,
                    "componentType": 5126,
                    "count": times.len(),
                    "type": "SCALAR",
                    "min": [min_t],
                    "max": [max_t]
                }));
                let scale_bv = push_view(
                    &mut bin,
                    &mut buffer_views,
                    bytemuck::cast_slice(&scales),
                    None,
                );
                let scale_acc = accessors.len();
                accessors.push(json!({
                    "bufferView": scale_bv,
                    "componentType": 5126,
                    "count": scales.len(),
                    "type": "VEC3"
                }));
                let sampler_idx = samplers.len();
                samplers.push(json!({
                    "input": time_acc,
                    "interpolation": "STEP",
                    "output": scale_acc
                }));
                channels.push(json!({
                    "sampler": sampler_idx,
                    "target": { "node": morph_base_node + pos, "path": "scale" }
                }));
            }

            // Export per-node dynamic parent-switch tracks as standard glTF 2.0 TRS channels.
            if !clip.parent_switch_tracks.is_empty() {
                let rest_mats = rig.world_rest_matrices();
                let duration = if clip.duration_secs.is_finite() {
                    clip.duration_secs.clamp(0.05, 600.0)
                } else {
                    1.0
                };
                let steps = ((duration * 24.0).round() as usize).clamp(1, 2048);
                let mut sim = rig.clone();
                sim.clip_layers.clear();
                sim.active_animation = clip_idx;
                for ps_track in &clip.parent_switch_tracks {
                    if ps_track.keys.is_empty() {
                        continue;
                    }
                    let Some(pos) = morphed_order.iter().position(|id| *id == ps_track.node) else {
                        continue;
                    };
                    let base_mat = scene.node_rest_world_matrix(ps_track.node);
                    let base_inv = base_mat.inverse();
                    let mut times = Vec::with_capacity(steps + 1);
                    let mut trans_out: Vec<[f32; 3]> = Vec::with_capacity(steps + 1);
                    let mut rot_out: Vec<[f32; 4]> = Vec::with_capacity(steps + 1);
                    for step in 0..=steps {
                        let t = if step == steps {
                            duration
                        } else {
                            (step as f32 / steps as f32) * duration
                        };
                        sim.playback_time = if step == steps {
                            (duration - 1e-4).max(0.0)
                        } else {
                            t
                        };
                        sim.apply_active_animation();
                        let world_mats = sim.world_pose_matrices();
                        let sampled = sim
                            .node_parent_overrides
                            .get(&ps_track.node)
                            .copied()
                            .unwrap_or(ps_track.keys[0]);
                        let world_override =
                            tessel_core::CharacterRig::compute_node_override_matrix(
                                sampled,
                                base_mat,
                                &world_mats,
                                &rest_mats,
                            );
                        let delta_mat = world_override * base_inv;
                        let (_, r, tr) = delta_mat.to_scale_rotation_translation();
                        times.push(t);
                        trans_out.push((tr * scale).to_array());
                        rot_out.push(r.normalize().to_array());
                    }
                    let min_t = times.iter().copied().fold(f32::INFINITY, f32::min);
                    let max_t = times.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                    let time_bv = push_view(
                        &mut bin,
                        &mut buffer_views,
                        bytemuck::cast_slice(&times),
                        None,
                    );
                    let time_acc = accessors.len();
                    accessors.push(json!({
                        "bufferView": time_bv,
                        "componentType": 5126,
                        "count": times.len(),
                        "type": "SCALAR",
                        "min": [min_t],
                        "max": [max_t]
                    }));
                    let t_bv = push_view(
                        &mut bin,
                        &mut buffer_views,
                        bytemuck::cast_slice(&trans_out),
                        None,
                    );
                    let t_acc = accessors.len();
                    accessors.push(json!({
                        "bufferView": t_bv,
                        "componentType": 5126,
                        "count": trans_out.len(),
                        "type": "VEC3"
                    }));
                    let t_sampler = samplers.len();
                    samplers.push(json!({
                        "input": time_acc,
                        "interpolation": "LINEAR",
                        "output": t_acc
                    }));
                    channels.push(json!({
                        "sampler": t_sampler,
                        "target": { "node": morph_base_node + pos, "path": "translation" }
                    }));
                    let r_bv = push_view(
                        &mut bin,
                        &mut buffer_views,
                        bytemuck::cast_slice(&rot_out),
                        None,
                    );
                    let r_acc = accessors.len();
                    accessors.push(json!({
                        "bufferView": r_bv,
                        "componentType": 5126,
                        "count": rot_out.len(),
                        "type": "VEC4"
                    }));
                    let r_sampler = samplers.len();
                    samplers.push(json!({
                        "input": time_acc,
                        "interpolation": "LINEAR",
                        "output": r_acc
                    }));
                    channels.push(json!({
                        "sampler": r_sampler,
                        "target": { "node": morph_base_node + pos, "path": "rotation" }
                    }));
                }
            }

            // Export per-node local TRS tracks (`node_transform_tracks`) as standard glTF 2.0 TRS channels.
            for nt_track in &clip.node_transform_tracks {
                if nt_track.keyframes.is_empty() {
                    continue;
                }
                let Some(&target_gltf_node) = scene_node_to_gltf.get(&nt_track.node) else {
                    continue;
                };
                let base_tr = scene
                    .nodes
                    .get(&nt_track.node)
                    .map(|n| n.transform)
                    .unwrap_or_default();
                let raw_nt_times: Vec<f32> =
                    nt_track.keyframes.iter().map(|k| k.time_secs).collect();
                let raw_nt_interps: Vec<tessel_core::KeyframeInterpolation> =
                    nt_track.keyframes.iter().map(|k| k.interpolation).collect();
                let is_nt_cubic = nt_track.keyframes.len() >= 2
                    && raw_nt_interps.iter().copied().any(is_cubic_interpolation);
                let has_nt_mixed_step = is_nt_cubic
                    && raw_nt_interps.contains(&tessel_core::KeyframeInterpolation::Step);
                let (times, nt_interps, nt_indices): (
                    Vec<f32>,
                    Vec<tessel_core::KeyframeInterpolation>,
                    Vec<usize>,
                ) = if has_nt_mixed_step {
                    let idx_list: Vec<usize> = (0..nt_track.keyframes.len()).collect();
                    expand_mixed_step_keyframes(&raw_nt_times, &raw_nt_interps, &idx_list)
                } else {
                    (
                        raw_nt_times,
                        raw_nt_interps,
                        (0..nt_track.keyframes.len()).collect(),
                    )
                };
                let min_t = times.iter().copied().fold(f32::INFINITY, f32::min);
                let max_t = times.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                let time_bv = push_view(
                    &mut bin,
                    &mut buffer_views,
                    bytemuck::cast_slice(&times),
                    None,
                );
                let time_acc = accessors.len();
                accessors.push(json!({
                    "bufferView": time_bv,
                    "componentType": 5126,
                    "count": times.len(),
                    "type": "SCALAR",
                    "min": [min_t],
                    "max": [max_t]
                }));
                let interp_mode = if is_nt_cubic {
                    "CUBICSPLINE"
                } else if nt_interps
                    .iter()
                    .all(|&i| i == tessel_core::KeyframeInterpolation::Step)
                {
                    "STEP"
                } else {
                    "LINEAR"
                };

                let raw_trans: Vec<[f32; 3]> = nt_indices
                    .iter()
                    .map(|&idx| {
                        ((base_tr.translation + nt_track.keyframes[idx].translation) * scale)
                            .to_array()
                    })
                    .collect();
                let trans_out: Vec<[f32; 3]> = if is_nt_cubic {
                    pack_cubicspline(&times, &nt_interps, &raw_trans)
                } else {
                    raw_trans
                };
                let t_bv = push_view(
                    &mut bin,
                    &mut buffer_views,
                    bytemuck::cast_slice(&trans_out),
                    None,
                );
                let t_acc = accessors.len();
                accessors.push(json!({
                    "bufferView": t_bv,
                    "componentType": 5126,
                    "count": trans_out.len(),
                    "type": "VEC3"
                }));
                let t_sampler = samplers.len();
                samplers.push(json!({
                    "input": time_acc,
                    "interpolation": interp_mode,
                    "output": t_acc
                }));
                channels.push(json!({
                    "sampler": t_sampler,
                    "target": { "node": target_gltf_node, "path": "translation" }
                }));

                let raw_rots: Vec<[f32; 4]> = nt_indices
                    .iter()
                    .map(|&idx| {
                        (base_tr.rotation * nt_track.keyframes[idx].rotation)
                            .normalize()
                            .to_array()
                    })
                    .collect();
                let rot_out: Vec<[f32; 4]> = if is_nt_cubic {
                    let aligned = align_quat_hemispheres(raw_rots);
                    pack_cubicspline(&times, &nt_interps, &aligned)
                } else {
                    raw_rots
                };
                let r_bv = push_view(
                    &mut bin,
                    &mut buffer_views,
                    bytemuck::cast_slice(&rot_out),
                    None,
                );
                let r_acc = accessors.len();
                accessors.push(json!({
                    "bufferView": r_bv,
                    "componentType": 5126,
                    "count": rot_out.len(),
                    "type": "VEC4"
                }));
                let r_sampler = samplers.len();
                samplers.push(json!({
                    "input": time_acc,
                    "interpolation": interp_mode,
                    "output": r_acc
                }));
                channels.push(json!({
                    "sampler": r_sampler,
                    "target": { "node": target_gltf_node, "path": "rotation" }
                }));

                let raw_scales: Vec<[f32; 3]> = nt_indices
                    .iter()
                    .map(|&idx| (base_tr.scale * nt_track.keyframes[idx].scale).to_array())
                    .collect();
                let scale_out: Vec<[f32; 3]> = if is_nt_cubic {
                    pack_cubicspline(&times, &nt_interps, &raw_scales)
                } else {
                    raw_scales
                };
                let s_bv = push_view(
                    &mut bin,
                    &mut buffer_views,
                    bytemuck::cast_slice(&scale_out),
                    None,
                );
                let s_acc = accessors.len();
                accessors.push(json!({
                    "bufferView": s_bv,
                    "componentType": 5126,
                    "count": scale_out.len(),
                    "type": "VEC3"
                }));
                let s_sampler = samplers.len();
                samplers.push(json!({
                    "input": time_acc,
                    "interpolation": interp_mode,
                    "output": s_acc
                }));
                channels.push(json!({
                    "sampler": s_sampler,
                    "target": { "node": target_gltf_node, "path": "scale" }
                }));
            }

            if !channels.is_empty() {
                animations.push(json!({
                    "name": clip.name,
                    "samplers": samplers,
                    "channels": channels
                }));
            }
        }
    }

    // Emit dedicated glTF empty nodes for `NodeKind::Group` folders right after bone nodes.
    for &gid in &group_order {
        let Some(gnode) = scene.nodes.get(&gid) else {
            continue;
        };
        let mut children: Vec<usize> = Vec::new();
        for &child_gid in &group_order {
            if scene.nodes.get(&child_gid).and_then(|n| n.parent) == Some(gid) {
                if let Some(&cidx) = group_to_gltf.get(&child_gid) {
                    children.push(cidx);
                }
            }
        }
        for &child_mid in &morphed_order {
            if scene.nodes.get(&child_mid).and_then(|n| n.parent) == Some(gid) {
                if let Some(&cidx) = scene_node_to_gltf.get(&child_mid) {
                    children.push(cidx);
                }
            }
        }
        let mut gjson = json!({
            "name": gnode.name,
            "translation": (gnode.transform.translation * scale).to_array(),
            "rotation": gnode.transform.rotation.normalize().to_array(),
            "scale": gnode.transform.scale.to_array()
        });
        if !children.is_empty() {
            gjson["children"] = json!(children);
        }
        let gidx = nodes.len();
        nodes.push(gjson);
        let parent_is_group = gnode
            .parent
            .is_some_and(|pid| group_to_gltf.contains_key(&pid));
        if !parent_is_group {
            root_nodes.push(gidx);
        }
    }

    // Export any Low-Poly meshes with custom RGBA texture, PBR material, or
    // morph targets as dedicated primitives (morphs need per-mesh targets,
    // which the merged palette mesh cannot carry).
    // Deduplicate identical PBR materials across nodes so multi-part assets avoid redundant draw calls.
    #[allow(clippy::type_complexity)]
    type MaterialKey = (
        usize,
        Option<usize>,
        Option<(usize, u32)>,
        u32,
        u32,
        [u32; 3],
        u32,
        u32,
        u32,
        bool,
        (u8, u32),
        bool,
    );
    let mut material_cache: BTreeMap<MaterialKey, usize> = BTreeMap::new();
    if let (Some(ptex), Some(pmat)) = (palette_tex_index, palette_mat_index) {
        material_cache.insert(
            (
                ptex,
                None,
                None,
                0.0_f32.to_bits(),
                0.85_f32.to_bits(),
                [0.0_f32.to_bits(); 3],
                1.0_f32.to_bits(),
                0.0_f32.to_bits(),
                1.5_f32.to_bits(),
                false,
                (0, 0),
                false,
            ),
            pmat,
        );
    }
    let mut morphed: Vec<(tessel_core::NodeId, Value, Vec<f64>)> = Vec::new();
    for (node_id, lp_mesh) in &scene.lowpoly_meshes {
        let is_local_trs_node =
            node_trs_animated.contains(node_id) || has_group_ancestor(scene, *node_id);
        let is_morphed = is_morphed_export_node(
            scene,
            *node_id,
            lp_mesh,
            &vis_animated_nodes,
            &node_trs_animated,
        );
        let has_custom_material = lp_mesh.texture.is_some()
            || lp_mesh.normal_map.is_some()
            || lp_mesh.ao_map.is_some()
            || !lp_mesh.vertex_colors.is_empty()
            || lp_mesh.material != tessel_core::MeshMaterial::default();
        // Same gate as `morphed_order` above: hidden nodes still export when a
        // visibility track animates them, via `is_morphed`.
        if !(scene.nodes.get(node_id).map(|n| n.visible).unwrap_or(true)
            || vis_animated_nodes.contains(node_id))
            || lp_mesh.faces.is_empty()
            || (!has_custom_material && !is_morphed)
        {
            continue;
        }
        let (node_mat, normal_mat) = if is_local_trs_node {
            (Mat4::IDENTITY, Mat4::IDENTITY)
        } else {
            node_world_matrices(scene, *node_id)
        };
        let tri = lp_mesh.to_triangulated_flat();
        if tri.vertices.is_empty() || tri.indices.is_empty() {
            continue;
        }
        let mut t_pos = Vec::with_capacity(tri.vertices.len());
        let mut t_norm = Vec::with_capacity(tri.vertices.len());
        let mut t_uv = Vec::with_capacity(tri.vertices.len());
        let need_uv1 = lp_mesh.ao_map.is_some() && lp_mesh.texture.is_none();
        let mut t_uv1 = if need_uv1 {
            Vec::with_capacity(tri.vertices.len())
        } else {
            Vec::new()
        };
        let mut t_min = [f32::INFINITY; 3];
        let mut t_max = [f32::NEG_INFINITY; 3];
        for v in &tri.vertices {
            let wp = node_mat.transform_point3(Vec3::from_array(v.position)) * scale;
            let wn = normal_mat
                .transform_vector3(Vec3::from_array(v.normal))
                .normalize_or_zero();
            let p = wp.to_array();
            for axis in 0..3 {
                t_min[axis] = t_min[axis].min(p[axis]);
                t_max[axis] = t_max[axis].max(p[axis]);
            }
            t_pos.push(p);
            t_norm.push(wn.to_array());
            if lp_mesh.texture.is_some() {
                t_uv.push(v.uv);
            } else {
                t_uv.push([(f32::from(v.palette_index.get()) + 0.5) / 256.0, 0.5]);
                if need_uv1 {
                    t_uv1.push(v.uv);
                }
            }
        }
        let p_bv = push_view(
            &mut bin,
            &mut buffer_views,
            bytemuck::cast_slice(&t_pos),
            Some(34962),
        );
        let n_bv = push_view(
            &mut bin,
            &mut buffer_views,
            bytemuck::cast_slice(&t_norm),
            Some(34962),
        );
        let u_bv = push_view(
            &mut bin,
            &mut buffer_views,
            bytemuck::cast_slice(&t_uv),
            Some(34962),
        );
        let i_bv = push_view(
            &mut bin,
            &mut buffer_views,
            bytemuck::cast_slice(&tri.indices),
            Some(34963),
        );

        let tex_idx = if let Some(tex) = &lp_mesh.texture {
            let exported_tex = if lp_mesh.material.alpha_mode == tessel_core::AlphaMode::Opaque {
                tessel_core::dilate_rgba_texture(tex, 2)
            } else {
                tex.clone()
            };
            let tex_png =
                encode_rgba_png(exported_tex.width, exported_tex.height, &exported_tex.rgba)?;
            let t_img_bv = push_view(&mut bin, &mut buffer_views, &tex_png, None);
            let img_idx = images.len();
            images.push(json!({ "bufferView": t_img_bv, "mimeType": "image/png" }));
            let idx = textures.len();
            textures.push(json!({ "sampler": 0, "source": img_idx }));
            idx
        } else if let Some(idx) = palette_tex_index {
            idx
        } else {
            let pal_bv = push_view(&mut bin, &mut buffer_views, &palette_png, None);
            let img_idx = images.len();
            images.push(json!({ "bufferView": pal_bv, "mimeType": "image/png" }));
            let idx = textures.len();
            textures.push(json!({ "sampler": 0, "source": img_idx }));
            palette_tex_index = Some(idx);
            idx
        };

        let p_acc = accessors.len();
        accessors.push(json!({
            "bufferView": p_bv, "componentType": 5126, "count": t_pos.len(),
            "type": "VEC3", "min": t_min, "max": t_max
        }));
        let n_acc = accessors.len();
        accessors.push(json!({
            "bufferView": n_bv, "componentType": 5126, "count": t_norm.len(), "type": "VEC3"
        }));
        let u_acc = accessors.len();
        accessors.push(json!({
            "bufferView": u_bv, "componentType": 5126, "count": t_uv.len(), "type": "VEC2"
        }));
        let i_acc = accessors.len();
        accessors.push(json!({
            "bufferView": i_bv, "componentType": 5125, "count": tri.indices.len(), "type": "SCALAR"
        }));

        let norm_tex_idx = if let Some(nmap) = &lp_mesh.normal_map {
            let norm_png = encode_rgba_png(nmap.width, nmap.height, &nmap.rgba)?;
            let n_img_bv = push_view(&mut bin, &mut buffer_views, &norm_png, None);
            let n_img_idx = images.len();
            images.push(json!({ "bufferView": n_img_bv, "mimeType": "image/png" }));
            let n_tex_idx = textures.len();
            textures.push(json!({ "sampler": 0, "source": n_img_idx }));
            Some(n_tex_idx)
        } else {
            None
        };
        let u1_acc = if need_uv1 {
            let u1_bv = push_view(
                &mut bin,
                &mut buffer_views,
                bytemuck::cast_slice(&t_uv1),
                Some(34962),
            );
            let idx = accessors.len();
            accessors.push(json!({
                "bufferView": u1_bv, "componentType": 5126, "count": t_uv1.len(), "type": "VEC2"
            }));
            Some(idx)
        } else {
            None
        };
        let ao_tex_info = if let Some(ao_map) = &lp_mesh.ao_map {
            let ao_png = encode_rgba_png(ao_map.width, ao_map.height, &ao_map.rgba)?;
            let a_img_bv = push_view(&mut bin, &mut buffer_views, &ao_png, None);
            let a_img_idx = images.len();
            images.push(json!({ "bufferView": a_img_bv, "mimeType": "image/png" }));
            let a_tex_idx = textures.len();
            textures.push(json!({ "sampler": 0, "source": a_img_idx }));
            let tex_coord = if need_uv1 { 1u32 } else { 0u32 };
            Some((a_tex_idx, tex_coord))
        } else {
            None
        };
        let alpha_key = match lp_mesh.material.alpha_mode {
            tessel_core::AlphaMode::Opaque => (0u8, 0u32),
            tessel_core::AlphaMode::Mask { cutoff } => (1u8, cutoff.to_bits()),
            tessel_core::AlphaMode::Blend => (2u8, 0u32),
        };
        let mat_key: MaterialKey = (
            tex_idx,
            norm_tex_idx,
            ao_tex_info,
            lp_mesh.material.metallic.to_bits(),
            lp_mesh.material.roughness.to_bits(),
            [
                lp_mesh.material.emissive[0].to_bits(),
                lp_mesh.material.emissive[1].to_bits(),
                lp_mesh.material.emissive[2].to_bits(),
            ],
            lp_mesh.material.emissive_intensity.to_bits(),
            lp_mesh.material.transmission.to_bits(),
            lp_mesh.material.ior.to_bits(),
            lp_mesh.material.double_sided,
            alpha_key,
            lp_mesh.material.unlit,
        );
        let has_npr_extras = lp_mesh.matcap_map.is_some()
            || lp_mesh.face_sdf_map.is_some()
            || lp_mesh.hatching_ramp_map.is_some()
            || lp_mesh.material.matcap_strength > 0.0
            || lp_mesh.material.face_sdf_light_override.is_some()
            || (lp_mesh.material.hatching_scale - 1.0).abs() > 1e-4;
        let cached_idx = if !has_npr_extras {
            material_cache.get(&mat_key).copied()
        } else {
            None
        };
        let mat_idx = if let Some(existing) = cached_idx {
            existing
        } else {
            let idx = materials.len();
            let mut mat_json = json!({
                "name": format!("TesselMeshMaterial_{}", idx),
                "pbrMetallicRoughness": {
                    "baseColorTexture": { "index": tex_idx },
                    "metallicFactor": lp_mesh.material.metallic,
                    "roughnessFactor": lp_mesh.material.roughness
                }
            });
            let has_emissive = lp_mesh.material.emissive.iter().any(|&c| c > 0.0);
            if has_emissive {
                mat_json["emissiveFactor"] = json!(lp_mesh.material.emissive);
            }
            if lp_mesh.material.double_sided {
                mat_json["doubleSided"] = json!(true);
            }
            match lp_mesh.material.alpha_mode {
                tessel_core::AlphaMode::Opaque => {}
                tessel_core::AlphaMode::Mask { cutoff } => {
                    mat_json["alphaMode"] = json!("MASK");
                    mat_json["alphaCutoff"] = json!(cutoff);
                }
                tessel_core::AlphaMode::Blend => {
                    mat_json["alphaMode"] = json!("BLEND");
                }
            }
            let exts = build_material_extensions(
                lp_mesh.material.unlit,
                has_emissive,
                lp_mesh.material.emissive_intensity,
                lp_mesh.material.transmission,
                lp_mesh.material.ior,
                &mut ext_flags,
            );
            if !exts.is_empty() {
                mat_json["extensions"] = Value::Object(exts);
            }
            if let Some(n_tex_idx) = norm_tex_idx {
                mat_json["normalTexture"] = json!({ "index": n_tex_idx });
            }
            if let Some((a_tex_idx, tex_coord)) = ao_tex_info {
                mat_json["occlusionTexture"] = if tex_coord > 0 {
                    json!({ "index": a_tex_idx, "texCoord": tex_coord })
                } else {
                    json!({ "index": a_tex_idx })
                };
            }
            if has_npr_extras {
                let mut extras = serde_json::Map::new();
                extras.insert(
                    "tessel_has_matcap".into(),
                    json!(lp_mesh.matcap_map.is_some()),
                );
                extras.insert(
                    "tessel_has_face_sdf".into(),
                    json!(lp_mesh.face_sdf_map.is_some()),
                );
                extras.insert(
                    "tessel_has_hatching_ramp".into(),
                    json!(lp_mesh.hatching_ramp_map.is_some()),
                );
                extras.insert(
                    "tessel_matcap_strength".into(),
                    json!(lp_mesh.material.matcap_strength),
                );
                extras.insert(
                    "tessel_hatching_scale".into(),
                    json!(lp_mesh.material.hatching_scale),
                );
                if let Some(dir) = lp_mesh.material.face_sdf_light_override {
                    extras.insert("tessel_face_sdf_light_override".into(), json!(dir));
                }
                mat_json["extras"] = serde_json::Value::Object(extras);
            } else {
                material_cache.insert(mat_key, idx);
            }
            materials.push(mat_json);
            idx
        };
        let mut custom_attrs = json!({
            "POSITION": p_acc,
            "NORMAL": n_acc,
            "TEXCOORD_0": u_acc
        });
        if let Some(u1_idx) = u1_acc {
            custom_attrs["TEXCOORD_1"] = json!(u1_idx);
        }
        if !lp_mesh.vertex_colors.is_empty() {
            let mut t_colors = Vec::with_capacity(t_pos.len());
            for corner in 0..t_pos.len() {
                let c = tri
                    .source_indices
                    .get(corner)
                    .and_then(|src| lp_mesh.vertex_colors.get(src))
                    .copied()
                    .unwrap_or([1.0, 1.0, 1.0, 1.0]);
                t_colors.push(c);
            }
            let c_bv = push_view(
                &mut bin,
                &mut buffer_views,
                bytemuck::cast_slice(&t_colors),
                Some(34962),
            );
            let c_acc = accessors.len();
            accessors.push(json!({
                "bufferView": c_bv, "componentType": 5126, "count": t_colors.len(), "type": "VEC4"
            }));
            custom_attrs["COLOR_0"] = json!(c_acc);
        }
        let is_skinned_node = node_is_skinned(*node_id) && !is_local_trs_node;
        if is_skinned_node {
            if let Some(solver) = &skin_solver {
                let mut t_joints = Vec::with_capacity(t_pos.len());
                let mut t_weights = Vec::with_capacity(t_pos.len());
                for (corner, &p) in t_pos.iter().enumerate() {
                    // Manual weight-paint overrides win over the auto solver.
                    let sw = tri
                        .source_indices
                        .get(corner)
                        .and_then(|src| lp_mesh.skin_weights.get(src))
                        .copied()
                        .unwrap_or_else(|| {
                            solver.weights_for_vertex(Some(*node_id), Vec3::from_array(p))
                        });
                    t_joints.push(sw.joints);
                    t_weights.push(sw.weights);
                }
                let j_bv = push_view(
                    &mut bin,
                    &mut buffer_views,
                    bytemuck::cast_slice(&t_joints),
                    Some(34962),
                );
                let w_bv = push_view(
                    &mut bin,
                    &mut buffer_views,
                    bytemuck::cast_slice(&t_weights),
                    Some(34962),
                );
                let j_acc = accessors.len();
                accessors.push(json!({
                    "bufferView": j_bv,
                    "componentType": 5123,
                    "count": t_joints.len(),
                    "type": "VEC4"
                }));
                let w_acc = accessors.len();
                accessors.push(json!({
                    "bufferView": w_bv,
                    "componentType": 5126,
                    "count": t_weights.len(),
                    "type": "VEC4"
                }));
                custom_attrs["JOINTS_0"] = json!(j_acc);
                custom_attrs["WEIGHTS_0"] = json!(w_acc);
            }
        }
        if !lp_mesh.morphs.is_empty() {
            let mut targets = Vec::with_capacity(lp_mesh.morphs.len());
            let mut morph_weights = Vec::with_capacity(lp_mesh.morphs.len());
            for morph in &lp_mesh.morphs {
                let mut deltas = vec![[0.0f32; 3]; tri.vertices.len()];
                for (corner, src) in tri.source_indices.iter().enumerate() {
                    if let Some(d) = morph.deltas.get(src) {
                        deltas[corner] = (d * scale).to_array();
                    }
                }
                let d_bv = push_view(
                    &mut bin,
                    &mut buffer_views,
                    bytemuck::cast_slice(&deltas),
                    Some(34962),
                );
                let d_acc = accessors.len();
                accessors.push(json!({
                    "bufferView": d_bv, "componentType": 5126, "count": deltas.len(),
                    "type": "VEC3"
                }));
                let mut target_obj = json!({ "POSITION": d_acc });
                if !morph.normal_deltas.is_empty() {
                    let mut n_deltas = vec![[0.0f32; 3]; tri.vertices.len()];
                    for (corner, src) in tri.source_indices.iter().enumerate() {
                        if let Some(nd) = morph.normal_deltas.get(src) {
                            n_deltas[corner] = nd.to_array();
                        }
                    }
                    let nd_bv = push_view(
                        &mut bin,
                        &mut buffer_views,
                        bytemuck::cast_slice(&n_deltas),
                        Some(34962),
                    );
                    let nd_acc = accessors.len();
                    accessors.push(json!({
                        "bufferView": nd_bv, "componentType": 5126, "count": n_deltas.len(),
                        "type": "VEC3"
                    }));
                    target_obj["NORMAL"] = json!(nd_acc);
                }
                targets.push(target_obj);
                morph_weights.push(morph.default_weight.clamp(0.0, 1.0) as f64);
            }
            // Targets are siblings of "attributes", not entries: morph
            // target dictionaries map attribute -> accessor index.
            let mut prim_json = json!({
                "attributes": custom_attrs,
                "indices": i_acc,
                "material": mat_idx
            });
            prim_json["targets"] = Value::Array(targets);
            morphed.push((*node_id, prim_json, morph_weights));
        } else if vis_animated_nodes.contains(node_id) || is_local_trs_node {
            let prim_json = json!({
                "attributes": custom_attrs,
                "indices": i_acc,
                "material": mat_idx
            });
            morphed.push((*node_id, prim_json, Vec::new()));
        } else {
            let prim_val = json!({
                "attributes": custom_attrs,
                "indices": i_acc,
                "material": mat_idx
            });
            if is_skinned_node || skin_solver.is_none() {
                primitives.push(prim_val);
            } else {
                static_primitives.push(prim_val);
            }
        }
        total_triangles += tri.indices.len() / 3;
    }

    if primitives.is_empty() && static_primitives.is_empty() && morphed.is_empty() {
        return Err(IoError::EmptyCanvas);
    }

    // Only emit glTF `skins` and its IBM accessor/bufferView when at least one exported
    // node actually references skinning, avoiding orphaned `UNUSED_OBJECT` skins on
    // all-unrigged exports.
    let any_skinned_export = (!primitives.is_empty() && skin_solver.is_some())
        || morphed.iter().any(|(mnode, _, _)| {
            let is_local_trs_node =
                node_trs_animated.contains(mnode) || has_group_ancestor(scene, *mnode);
            node_is_skinned(*mnode) && !is_local_trs_node
        });
    if any_skinned_export {
        if let (Some(rig), Some(solver)) = (&scene.character_rig, &skin_solver) {
            let joint_nodes: Vec<usize> = rig
                .bones
                .iter()
                .filter_map(|b| bone_to_node.get(&b.id).copied())
                .collect();
            let ibm = solver.inverse_bind_matrices();
            if !joint_nodes.is_empty() && joint_nodes.len() == ibm.len() {
                let ibm_bv =
                    push_view(&mut bin, &mut buffer_views, bytemuck::cast_slice(ibm), None);
                let ibm_acc = accessors.len();
                accessors.push(json!({
                    "bufferView": ibm_bv,
                    "componentType": 5126,
                    "count": ibm.len(),
                    "type": "MAT4"
                }));
                let mut skin_json = json!({
                    "name": "TesselCharacterSkin",
                    "inverseBindMatrices": ibm_acc,
                    "joints": joint_nodes
                });
                if let Some(skel) = rig
                    .bones
                    .iter()
                    .find(|b| b.parent.is_none())
                    .and_then(|b| bone_to_node.get(&b.id).copied())
                {
                    skin_json["skeleton"] = json!(skel);
                }
                skins.push(skin_json);
            }
        }
    }

    // Morphed meshes get dedicated mesh objects (with exact default
    // weights) plus dedicated identity nodes, since morph targets cannot
    // ride the merged palette mesh. Vertex data is already baked, so the
    // nodes carry no transform.
    let has_skin = !skins.is_empty();
    let mut static_mesh_assigned_to_root = false;
    let mut meshes: Vec<Value> = if !primitives.is_empty() {
        nodes[0]["mesh"] = json!(0);
        if has_skin {
            nodes[0]["skin"] = json!(0);
        }
        vec![json!({
            "name": "TesselMesh",
            "primitives": primitives
        })]
    } else if !static_primitives.is_empty() {
        nodes[0]["mesh"] = json!(0);
        static_mesh_assigned_to_root = true;
        vec![json!({
            "name": "TesselMesh",
            "primitives": static_primitives
        })]
    } else {
        Vec::new()
    };
    // Node indices must match the precomputed `morphed_order` used by the
    // morph weight channels above (same BTreeMap order, same filters).
    debug_assert!(morphed
        .iter()
        .map(|(id, _, _)| *id)
        .eq(morphed_order.iter().copied()));
    for (mnode, prim, weights) in morphed {
        let mesh_idx = meshes.len();
        let mut mesh_json = json!({
            "name": format!("TesselMesh_{}", mnode.0),
            "primitives": [prim]
        });
        if !weights.is_empty() {
            mesh_json["weights"] = json!(weights);
        }
        meshes.push(mesh_json);
        let node_idx = nodes.len();
        let node_name = scene
            .nodes
            .get(&mnode)
            .map(|n| n.name.clone())
            .unwrap_or_else(|| format!("TesselAsset_{}", mnode.0));
        let mut mnode_json = json!({
            "name": node_name,
            "mesh": mesh_idx
        });
        let is_local_trs_node =
            node_trs_animated.contains(&mnode) || has_group_ancestor(scene, mnode);
        if is_local_trs_node {
            if let Some(snode) = scene.nodes.get(&mnode) {
                mnode_json["translation"] = json!((snode.transform.translation * scale).to_array());
                mnode_json["rotation"] = json!(snode.transform.rotation.normalize().to_array());
                mnode_json["scale"] = json!(snode.transform.scale.to_array());
            }
        }
        if has_skin && node_is_skinned(mnode) && !is_local_trs_node {
            mnode_json["skin"] = json!(0);
        }
        nodes.push(mnode_json);
        let parent_is_group = scene
            .nodes
            .get(&mnode)
            .and_then(|n| n.parent)
            .is_some_and(|pid| group_to_gltf.contains_key(&pid));
        if !parent_is_group {
            root_nodes.push(node_idx);
        }
    }

    if !static_primitives.is_empty() && !static_mesh_assigned_to_root {
        let static_mesh_idx = meshes.len();
        meshes.push(json!({
            "name": "TesselStaticMesh",
            "primitives": static_primitives
        }));
        let static_node_idx = nodes.len();
        nodes.push(json!({
            "name": "TesselStaticAsset",
            "mesh": static_mesh_idx
        }));
        root_nodes.push(static_node_idx);
    }

    let mut gltf_json = json!({
        "asset": {
            "version": "2.0",
            "generator": "Tessel"
        },
        "scene": 0,
        "scenes": [{ "nodes": root_nodes }],
        "nodes": nodes,
        "samplers": [{
            "magFilter": 9728,
            "minFilter": 9728,
            "wrapS": 33071,
            "wrapT": 33071
        }],
        "images": images,
        "textures": textures,
        "materials": materials,
        "meshes": meshes,
        "buffers": [{ "byteLength": bin.len() }],
        "bufferViews": buffer_views,
        "accessors": accessors
    });

    if !animations.is_empty() {
        gltf_json["animations"] = Value::Array(animations);
    }
    let extensions_used = ext_flags.to_extensions_used();
    if !extensions_used.is_empty() {
        gltf_json["extensionsUsed"] = json!(extensions_used);
    }
    if !skins.is_empty() {
        gltf_json["skins"] = Value::Array(skins);
    }

    let mut json_bytes = serde_json::to_vec(&gltf_json)?;
    pad4(&mut json_bytes, 0x20);

    let total_len = 12 + 8 + json_bytes.len() + 8 + bin.len();
    let mut glb = Vec::with_capacity(total_len);
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(total_len as u32).to_le_bytes());

    glb.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    glb.extend_from_slice(&0x4E4F_534Au32.to_le_bytes()); // JSON
    glb.extend_from_slice(&json_bytes);

    glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(&0x004E_4942u32.to_le_bytes()); // BIN
    glb.extend_from_slice(&bin);

    Ok((glb, total_triangles))
}
