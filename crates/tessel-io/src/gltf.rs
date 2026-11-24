use std::collections::BTreeMap;
use std::io::Cursor;

use glam::{Mat4, Vec3};
use image::{ImageFormat, RgbaImage};
use serde_json::{json, Value};
use tessel_core::voxel::greedy_mesh_volume;
use tessel_core::{AutoSkinSolver, Palette, PaletteIndex, SceneState};

use crate::error::IoError;
use crate::ExportPreset;

fn node_world_matrices(scene: &SceneState, node_id: tessel_core::NodeId) -> (Mat4, Mat4) {
    let mat = scene.node_world_matrix(node_id);
    let normal_mat = if mat.determinant().abs() > 1e-8 {
        mat.inverse().transpose()
    } else {
        mat
    };
    (mat, normal_mat)
}

/// Collects all visible 3D geometry (voxels + low-poly meshes) into flat indexed buffers
/// scaled by `preset.scale_factor()`.
pub(crate) struct CollectedMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
}

impl CollectedMesh {
    pub fn from_scene_untextured(
        scene: &SceneState,
        preset: ExportPreset,
        skin: Option<&AutoSkinSolver>,
    ) -> Self {
        let scale = preset.scale_factor();
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uvs = Vec::new();
        let mut joints = Vec::new();
        let mut weights = Vec::new();
        let mut indices = Vec::new();

        for (node_id, vol) in &scene.voxel_volumes {
            let visible = scene.nodes.get(node_id).map(|n| n.visible).unwrap_or(true);
            if !visible || vol.is_empty() {
                continue;
            }
            let (node_mat, normal_mat) = node_world_matrices(scene, *node_id);
            let mesh = greedy_mesh_volume(vol, scene.unit_scale);
            let base = positions.len() as u32;
            for v in mesh.vertices {
                let wp = node_mat.transform_point3(Vec3::from_array(v.position)) * scale;
                let wn = normal_mat
                    .transform_vector3(Vec3::from_array(v.normal))
                    .normalize_or_zero();
                positions.push(wp.to_array());
                normals.push(wn.to_array());
                uvs.push([(f32::from(v.palette_index.get()) + 0.5) / 256.0, 0.5]);
                if let Some(solver) = skin {
                    let sw = solver.weights_for_vertex(Some(*node_id), wp);
                    joints.push(sw.joints);
                    weights.push(sw.weights);
                }
            }
            indices.extend(mesh.indices.into_iter().map(|i| i + base));
        }

        let vis_animated: std::collections::BTreeSet<tessel_core::NodeId> = scene
            .character_rig
            .as_ref()
            .map(|rig| {
                rig.animations
                    .iter()
                    .flat_map(|c| {
                        c.visibility_tracks
                            .iter()
                            .map(|t| t.node)
                            .chain(c.parent_switch_tracks.iter().map(|t| t.node))
                    })
                    .collect()
            })
            .unwrap_or_default();

        for (node_id, mesh) in &scene.lowpoly_meshes {
            let visible = scene.nodes.get(node_id).map(|n| n.visible).unwrap_or(true);
            let is_custom = mesh.texture.is_some()
                || mesh.normal_map.is_some()
                || mesh.material != tessel_core::MeshMaterial::default()
                || !mesh.morphs.is_empty()
                || vis_animated.contains(node_id);
            if !visible || mesh.faces.is_empty() || is_custom {
                continue;
            }
            let (node_mat, normal_mat) = node_world_matrices(scene, *node_id);
            let tri = mesh.to_triangulated_flat();
            let base = positions.len() as u32;
            for (corner, v) in tri.vertices.iter().enumerate() {
                let wp = node_mat.transform_point3(Vec3::from_array(v.position)) * scale;
                let wn = normal_mat
                    .transform_vector3(Vec3::from_array(v.normal))
                    .normalize_or_zero();
                positions.push(wp.to_array());
                normals.push(wn.to_array());
                uvs.push([(f32::from(v.palette_index.get()) + 0.5) / 256.0, 0.5]);
                if let Some(solver) = skin {
                    // Manual weight-paint overrides win over the auto solver.
                    let sw = tri
                        .source_indices
                        .get(corner)
                        .and_then(|src| mesh.skin_weights.get(src))
                        .copied()
                        .unwrap_or_else(|| solver.weights_for_vertex(Some(*node_id), wp));
                    joints.push(sw.joints);
                    weights.push(sw.weights);
                }
            }
            indices.extend(tri.indices.into_iter().map(|i| i + base));
        }

        Self {
            positions,
            normals,
            uvs,
            joints,
            weights,
            indices,
        }
    }

    pub fn from_scene(scene: &SceneState, preset: ExportPreset) -> Result<Self, IoError> {
        let scale = preset.scale_factor();
        let mut out = Self::from_scene_untextured(scene, preset, None);
        for (node_id, mesh) in &scene.lowpoly_meshes {
            let visible = scene.nodes.get(node_id).map(|n| n.visible).unwrap_or(true);
            let is_custom = mesh.texture.is_some()
                || mesh.normal_map.is_some()
                || mesh.material != tessel_core::MeshMaterial::default();
            if !visible || mesh.faces.is_empty() || !is_custom {
                continue;
            }
            let (node_mat, normal_mat) = node_world_matrices(scene, *node_id);
            let tri = mesh.to_triangulated_flat();
            let base = out.positions.len() as u32;
            for v in tri.vertices {
                let wp = node_mat.transform_point3(Vec3::from_array(v.position)) * scale;
                let wn = normal_mat
                    .transform_vector3(Vec3::from_array(v.normal))
                    .normalize_or_zero();
                out.positions.push(wp.to_array());
                out.normals.push(wn.to_array());
                out.uvs.push(v.uv);
            }
            out.indices
                .extend(tri.indices.into_iter().map(|i| i + base));
        }
        if out.positions.is_empty() || out.indices.is_empty() {
            return Err(IoError::EmptyCanvas);
        }
        Ok(out)
    }
}

fn encode_rgba_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, IoError> {
    let img = RgbaImage::from_raw(width, height, rgba.to_vec())
        .ok_or(IoError::InvalidDimensions(width, height))?;
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, ImageFormat::Png)?;
    Ok(buf.into_inner())
}

fn build_palette_png(palette: &Palette) -> Result<Vec<u8>, IoError> {
    let mut rgba = vec![255u8; 256 * 4];
    for raw in 1u8..=255 {
        if let Ok(idx) = PaletteIndex::new(raw) {
            if let Some(entry) = palette.get(idx) {
                let base = usize::from(raw) * 4;
                rgba[base] = entry.color.r;
                rgba[base + 1] = entry.color.g;
                rgba[base + 2] = entry.color.b;
                rgba[base + 3] = entry.color.a;
            }
        }
    }
    encode_rgba_png(256, 1, &rgba)
}

fn pad4(buf: &mut Vec<u8>, pad_byte: u8) {
    while !buf.len().is_multiple_of(4) {
        buf.push(pad_byte);
    }
}

/// Linear-interpolates morph weight keys at `time_secs` (clamped ends),
/// mirroring `AnimationClip::sample_morph_weights` for export baking.
fn sample_morph_keys(keys: &[tessel_core::MorphWeightKey], time_secs: f32) -> f32 {
    if keys.is_empty() {
        return 0.0;
    }
    if keys.len() == 1 || time_secs <= keys[0].time_secs {
        return keys[0].weight;
    }
    if time_secs >= keys.last().map(|k| k.time_secs).unwrap_or(0.0) {
        return keys.last().map(|k| k.weight).unwrap_or(0.0);
    }
    for pair in keys.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if time_secs >= a.time_secs && time_secs <= b.time_secs {
            let span = (b.time_secs - a.time_secs).max(1e-5);
            let alpha = ((time_secs - a.time_secs) / span).clamp(0.0, 1.0);
            return a.weight + (b.weight - a.weight) * alpha;
        }
    }
    keys.last().map(|k| k.weight).unwrap_or(0.0)
}

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
    let mesh = CollectedMesh::from_scene_untextured(scene, preset, skin_solver.as_ref());
    let has_palette_prim = !mesh.positions.is_empty() && !mesh.indices.is_empty();
    let palette_png = build_palette_png(&scene.palette)?;

    let mut min_pos = [f32::INFINITY; 3];
    let mut max_pos = [f32::NEG_INFINITY; 3];
    for p in &mesh.positions {
        for axis in 0..3 {
            min_pos[axis] = min_pos[axis].min(p[axis]);
            max_pos[axis] = max_pos[axis].max(p[axis]);
        }
    }

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
    let mut total_triangles = 0usize;

    if has_palette_prim {
        let pos_bv = push_view(
            &mut bin,
            &mut buffer_views,
            bytemuck::cast_slice(&mesh.positions),
            Some(34962),
        );
        let norm_bv = push_view(
            &mut bin,
            &mut buffer_views,
            bytemuck::cast_slice(&mesh.normals),
            Some(34962),
        );
        let uv_bv = push_view(
            &mut bin,
            &mut buffer_views,
            bytemuck::cast_slice(&mesh.uvs),
            Some(34962),
        );
        let idx_bv = push_view(
            &mut bin,
            &mut buffer_views,
            bytemuck::cast_slice(&mesh.indices),
            Some(34963),
        );
        let img_bv = push_view(&mut bin, &mut buffer_views, &palette_png, None);

        accessors.push(json!({
            "bufferView": pos_bv,
            "componentType": 5126,
            "count": mesh.positions.len(),
            "type": "VEC3",
            "min": min_pos,
            "max": max_pos
        }));
        accessors.push(json!({
            "bufferView": norm_bv,
            "componentType": 5126,
            "count": mesh.normals.len(),
            "type": "VEC3"
        }));
        accessors.push(json!({
            "bufferView": uv_bv,
            "componentType": 5126,
            "count": mesh.uvs.len(),
            "type": "VEC2"
        }));
        accessors.push(json!({
            "bufferView": idx_bv,
            "componentType": 5125,
            "count": mesh.indices.len(),
            "type": "SCALAR"
        }));

        images.push(json!({
            "bufferView": img_bv,
            "mimeType": "image/png"
        }));
        textures.push(json!({
            "sampler": 0,
            "source": 0
        }));
        materials.push(json!({
            "name": "TesselPaletteMaterial",
            "pbrMetallicRoughness": {
                "baseColorTexture": { "index": 0 },
                "metallicFactor": 0.0,
                "roughnessFactor": 0.85
            }
        }));
        let mut prim_attrs = json!({
            "POSITION": 0,
            "NORMAL": 1,
            "TEXCOORD_0": 2
        });
        if skin_solver.is_some() && mesh.joints.len() == mesh.positions.len() {
            let j_bv = push_view(
                &mut bin,
                &mut buffer_views,
                bytemuck::cast_slice(&mesh.joints),
                Some(34962),
            );
            let w_bv = push_view(
                &mut bin,
                &mut buffer_views,
                bytemuck::cast_slice(&mesh.weights),
                Some(34962),
            );
            let j_acc = accessors.len();
            accessors.push(json!({
                "bufferView": j_bv,
                "componentType": 5123,
                "count": mesh.joints.len(),
                "type": "VEC4"
            }));
            let w_acc = accessors.len();
            accessors.push(json!({
                "bufferView": w_bv,
                "componentType": 5126,
                "count": mesh.weights.len(),
                "type": "VEC4"
            }));
            prim_attrs["JOINTS_0"] = json!(j_acc);
            prim_attrs["WEIGHTS_0"] = json!(w_acc);
        }
        primitives.push(json!({
            "attributes": prim_attrs,
            "indices": 3,
            "material": 0
        }));
        total_triangles += mesh.indices.len() / 3;
    }

    let mut nodes: Vec<Value> = vec![json!({ "name": "TesselAsset", "mesh": 0 })];
    let mut root_nodes: Vec<usize> = vec![0];
    let mut animations: Vec<Value> = Vec::new();
    let mut skins: Vec<Value> = Vec::new();

    // Morphed and visibility-animated meshes get dedicated export nodes appended
    // after the bone nodes by the custom-primitive pass below. Precompute their
    // order here (same BTreeMap order and same skip filters) so animation channels
    // and node indices agree.
    let vis_animated_nodes: std::collections::BTreeSet<tessel_core::NodeId> = scene
        .character_rig
        .as_ref()
        .map(|rig| {
            rig.animations
                .iter()
                .flat_map(|c| {
                    c.visibility_tracks
                        .iter()
                        .map(|t| t.node)
                        .chain(c.parent_switch_tracks.iter().map(|t| t.node))
                })
                .collect()
        })
        .unwrap_or_default();
    let morphed_order: Vec<tessel_core::NodeId> = scene
        .lowpoly_meshes
        .iter()
        .filter(|(id, m)| {
            scene.nodes.get(id).map(|n| n.visible).unwrap_or(true)
                && !m.faces.is_empty()
                && (!m.morphs.is_empty() || vis_animated_nodes.contains(id))
                && !m.to_triangulated_flat().vertices.is_empty()
        })
        .map(|(id, _)| *id)
        .collect();
    let morph_base_node = 1 + scene
        .character_rig
        .as_ref()
        .map(|r| r.bones.len())
        .unwrap_or(0);

    if let Some(rig) = &scene.character_rig {
        let mut bone_to_node: BTreeMap<tessel_core::BoneId, usize> = BTreeMap::new();
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

        if let Some(solver) = &skin_solver {
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
                nodes[0]["skin"] = json!(0);
            }
        }

        for clip in &rig.animations {
            let mut samplers: Vec<Value> = Vec::new();
            let mut channels: Vec<Value> = Vec::new();

            for ch in &clip.channels {
                let Some(&target_node) = bone_to_node.get(&ch.bone_id) else {
                    continue;
                };
                if ch.keyframes.is_empty() {
                    continue;
                }
                let times: Vec<f32> = ch.keyframes.iter().map(|k| k.time_secs).collect();
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

                let rots: Vec<[f32; 4]> =
                    ch.keyframes.iter().map(|k| k.rotation.to_array()).collect();
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

                let interp_mode = if ch
                    .keyframes
                    .iter()
                    .all(|k| k.interpolation == tessel_core::KeyframeInterpolation::Step)
                {
                    "STEP"
                } else {
                    "LINEAR"
                };

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
                    let trans: Vec<[f32; 3]> = ch
                        .keyframes
                        .iter()
                        .map(|k| ((rest_t + k.translation) * scale).to_array())
                        .collect();
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
                        "interpolation": "LINEAR",
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
                    let scales: Vec<[f32; 3]> = ch
                        .keyframes
                        .iter()
                        .map(|k| (rest_s * k.scale).to_array())
                        .collect();
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
                        "interpolation": "LINEAR",
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
                let duration = clip.duration_secs.max(0.05);
                let steps = ((duration * 24.0).round() as usize).max(1);
                let clip_idx = rig
                    .animations
                    .iter()
                    .position(|c| c.name == clip.name)
                    .unwrap_or(0);
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
                    let base_mat = scene.node_world_matrix(ps_track.node);
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

            if !channels.is_empty() {
                animations.push(json!({
                    "name": clip.name,
                    "samplers": samplers,
                    "channels": channels
                }));
            }
        }
    }

    // Export any Low-Poly meshes with custom RGBA texture, PBR material, or
    // morph targets as dedicated primitives (morphs need per-mesh targets,
    // which the merged palette mesh cannot carry).
    let mut palette_tex_index: Option<usize> = if has_palette_prim { Some(0) } else { None };
    // Deduplicate identical PBR materials across nodes so multi-part assets avoid redundant draw calls.
    type MaterialKey = (usize, Option<usize>, u32, u32, [u32; 3]);
    let mut material_cache: BTreeMap<MaterialKey, usize> = BTreeMap::new();
    if has_palette_prim {
        material_cache.insert(
            (
                0,
                None,
                0.0_f32.to_bits(),
                0.85_f32.to_bits(),
                [0.0_f32.to_bits(); 3],
            ),
            0,
        );
    }
    let mut morphed: Vec<(tessel_core::NodeId, Value, Vec<f64>)> = Vec::new();
    for (node_id, lp_mesh) in &scene.lowpoly_meshes {
        let visible = scene.nodes.get(node_id).map(|n| n.visible).unwrap_or(true);
        let has_custom = lp_mesh.texture.is_some()
            || lp_mesh.normal_map.is_some()
            || lp_mesh.material != tessel_core::MeshMaterial::default()
            || !lp_mesh.morphs.is_empty()
            || vis_animated_nodes.contains(node_id);
        if !visible || lp_mesh.faces.is_empty() || !has_custom {
            continue;
        }
        let (node_mat, normal_mat) = node_world_matrices(scene, *node_id);
        let tri = lp_mesh.to_triangulated_flat();
        if tri.vertices.is_empty() || tri.indices.is_empty() {
            continue;
        }
        let mut t_pos = Vec::with_capacity(tri.vertices.len());
        let mut t_norm = Vec::with_capacity(tri.vertices.len());
        let mut t_uv = Vec::with_capacity(tri.vertices.len());
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
            let tex_png = encode_rgba_png(tex.width, tex.height, &tex.rgba)?;
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
        let mat_key: MaterialKey = (
            tex_idx,
            norm_tex_idx,
            lp_mesh.material.metallic.to_bits(),
            lp_mesh.material.roughness.to_bits(),
            [
                lp_mesh.material.emissive[0].to_bits(),
                lp_mesh.material.emissive[1].to_bits(),
                lp_mesh.material.emissive[2].to_bits(),
            ],
        );
        let mat_idx = if let Some(&existing) = material_cache.get(&mat_key) {
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
            if lp_mesh.material.emissive.iter().any(|&c| c > 0.0) {
                mat_json["emissiveFactor"] = json!(lp_mesh.material.emissive);
            }
            if let Some(n_tex_idx) = norm_tex_idx {
                mat_json["normalTexture"] = json!({ "index": n_tex_idx });
            }
            materials.push(mat_json);
            material_cache.insert(mat_key, idx);
            idx
        };
        let mut custom_attrs = json!({
            "POSITION": p_acc,
            "NORMAL": n_acc,
            "TEXCOORD_0": u_acc
        });
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
                targets.push(json!({ "POSITION": d_acc }));
                morph_weights.push(0.0);
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
        } else if vis_animated_nodes.contains(node_id) {
            let prim_json = json!({
                "attributes": custom_attrs,
                "indices": i_acc,
                "material": mat_idx
            });
            morphed.push((*node_id, prim_json, Vec::new()));
        } else {
            primitives.push(json!({
                "attributes": custom_attrs,
                "indices": i_acc,
                "material": mat_idx
            }));
        }
        total_triangles += tri.indices.len() / 3;
    }

    if primitives.is_empty() && morphed.is_empty() {
        return Err(IoError::EmptyCanvas);
    }

    // Morphed meshes get dedicated mesh objects (with exact default
    // weights) plus dedicated identity nodes, since morph targets cannot
    // ride the merged palette mesh. Vertex data is already baked, so the
    // nodes carry no transform.
    let mut meshes = vec![json!({
        "name": "TesselMesh",
        "primitives": primitives
    })];
    let has_skin = !skins.is_empty();
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
        if has_skin {
            mnode_json["skin"] = json!(0);
        }
        nodes.push(mnode_json);
        root_nodes.push(node_idx);
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

#[cfg(test)]
mod tests {
    use super::*;
    use tessel_core::{Command, PrimitiveKind, Rgba8, VoxelCoord};

    #[test]
    fn glb_export_embeds_painted_mesh_rgba_texture() {
        let mut scene = SceneState::new();
        Command::CreatePrimitiveMesh {
            name: "PaintedCube".into(),
            kind: PrimitiveKind::Cube,
            size: 1.0,
            material: PaletteIndex::new(1).unwrap(),
        }
        .apply(&mut scene)
        .unwrap();
        let node_id = scene.active_lowpoly_node().unwrap();
        Command::PaintMeshTexture {
            node_id,
            uv: [0.5, 0.75],
            radius_px: 12.0,
            color: Rgba8::rgb(255, 255, 255),
        }
        .apply(&mut scene)
        .unwrap();

        let (glb, tris) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
        assert_eq!(tris, 12);
        let parsed = gltf::Gltf::from_slice(&glb).unwrap();
        assert_eq!(parsed.images().count(), 1);
        assert_eq!(parsed.materials().count(), 1);

        // Verify PBR metallic, roughness, and emissive factors export cleanly into glTF 2.0
        Command::SetMeshMaterial {
            node_id,
            material: tessel_core::MeshMaterial {
                metallic: 0.9,
                roughness: 0.2,
                emissive: [0.1, 0.5, 0.8],
            },
        }
        .apply(&mut scene)
        .unwrap();
        let (glb_pbr, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
        let parsed_pbr = gltf::Gltf::from_slice(&glb_pbr).unwrap();
        let mat = parsed_pbr.materials().next().unwrap();
        assert!((mat.pbr_metallic_roughness().metallic_factor() - 0.9).abs() < 1e-4);
        assert!((mat.pbr_metallic_roughness().roughness_factor() - 0.2).abs() < 1e-4);
        assert!((mat.emissive_factor()[2] - 0.8).abs() < 1e-4);

        // Verify Normal Map embeds as glTF normalTexture
        Command::SetMeshNormalMap {
            node_id,
            normal_map: Some(tessel_core::MeshTexture::solid(
                16,
                16,
                Rgba8::rgb(128, 128, 255),
            )),
        }
        .apply(&mut scene)
        .unwrap();
        let (glb_norm, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
        let parsed_norm = gltf::Gltf::from_slice(&glb_norm).unwrap();
        assert!(parsed_norm
            .materials()
            .next()
            .unwrap()
            .normal_texture()
            .is_some());
    }

    #[test]
    fn glb_export_passes_gltf_parser_validation_with_animations() {
        let mut scene = SceneState::new();
        Command::CreateVoxelVolume {
            name: "Hero".into(),
        }
        .apply(&mut scene)
        .unwrap();
        let id = scene.active_voxel_node().unwrap();
        Command::FillVoxelBox {
            node_id: id,
            min: VoxelCoord::new(0, 0, 0),
            max: VoxelCoord::new(1, 2, 1),
            color: Some(PaletteIndex::new(4).unwrap()),
            hollow: false,
        }
        .apply(&mut scene)
        .unwrap();
        Command::ApplyBipedRig.apply(&mut scene).unwrap();

        let (glb, tris) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
        assert_eq!(tris, 12);

        let parsed = gltf::Gltf::from_slice(&glb).unwrap();
        assert_eq!(parsed.meshes().count(), 1);
        assert_eq!(parsed.materials().count(), 1);
        assert_eq!(parsed.images().count(), 1);
        assert_eq!(parsed.animations().count(), 2); // Idle + Walk
        assert_eq!(parsed.skins().count(), 1);
        let skin = parsed.skins().next().unwrap();
        assert_eq!(skin.joints().count(), 7);
        assert!(skin.inverse_bind_matrices().is_some());
        assert!(parsed.nodes().next().unwrap().skin().is_some());
        assert!(parsed.nodes().count() >= 8); // Mesh + 7 biped bones
        let blob = parsed.blob.as_ref().unwrap();
        let prim = parsed.meshes().next().unwrap().primitives().next().unwrap();
        let reader = prim.reader(|_| Some(blob.as_slice()));
        let joints: Vec<[u16; 4]> = reader.read_joints(0).unwrap().into_u16().collect();
        let weights: Vec<[f32; 4]> = reader.read_weights(0).unwrap().into_f32().collect();
        assert!(!joints.is_empty());
        assert_eq!(joints.len(), weights.len());
        for w in &weights {
            let sum: f32 = w.iter().sum();
            assert!((sum - 1.0).abs() < 1e-4, "weights must sum to 1.0: {w:?}");
        }
    }

    #[test]
    fn glb_export_writes_translation_and_scale_channels_for_trs_keyframes() {
        use tessel_core::{BoneId, Keyframe};

        let mut scene = SceneState::new();
        Command::CreateVoxelVolume {
            name: "Hero".into(),
        }
        .apply(&mut scene)
        .unwrap();
        let id = scene.active_voxel_node().unwrap();
        Command::FillVoxelBox {
            node_id: id,
            min: tessel_core::VoxelCoord::new(0, 0, 0),
            max: tessel_core::VoxelCoord::new(0, 0, 0),
            color: Some(PaletteIndex::new(4).unwrap()),
            hollow: false,
        }
        .apply(&mut scene)
        .unwrap();
        Command::ApplyBipedRig.apply(&mut scene).unwrap();
        Command::AddAnimationClip {
            name: "Bounce".into(),
            duration_secs: 1.0,
        }
        .apply(&mut scene)
        .unwrap();
        Command::SetAnimationKeyframe {
            clip_index: 2, // Idle + Walk ship with the rig
            bone_id: BoneId(1),
            keyframe: Keyframe {
                time_secs: 0.5,
                translation: Vec3::new(0.0, 0.1, 0.0),
                rotation: glam::Quat::IDENTITY,
                scale: Vec3::new(1.0, 1.0, 2.0),
                interpolation: Default::default(),
            },
        }
        .apply(&mut scene)
        .unwrap();

        let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let json: serde_json::Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();
        let animations = json["animations"].as_array().unwrap();
        let bounce = animations
            .iter()
            .find(|a| a["name"] == "Bounce")
            .expect("Bounce clip must export");
        let mut paths: Vec<&str> = bounce["channels"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["target"]["path"].as_str().unwrap())
            .collect();
        paths.sort_unstable();
        assert_eq!(paths, vec!["rotation", "scale", "translation"]);

        // The built-in Idle clip breathes via translation offsets: those were
        // silently dropped before and must now export as absolute values.
        let idle = animations
            .iter()
            .find(|a| a["name"] == "Idle")
            .expect("Idle clip must export");
        let idle_paths: Vec<&str> = idle["channels"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["target"]["path"].as_str().unwrap())
            .collect();
        assert!(idle_paths.contains(&"translation"));
        // No scale keyframes exist anywhere in the starter clips.
        assert!(!idle_paths.contains(&"scale"));

        // Rotation-only Walk stays rotation-only (skip rule).
        let walk = animations
            .iter()
            .find(|a| a["name"] == "Walk")
            .expect("Walk clip must export");
        let walk_paths: Vec<&str> = walk["channels"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["target"]["path"].as_str().unwrap())
            .collect();
        assert!(walk_paths.iter().all(|p| *p == "rotation"));
    }

    #[test]
    fn glb_export_prefers_manual_weights_over_auto_solver() {
        let mut scene = SceneState::new();
        Command::CreatePrimitiveMesh {
            name: "PaintedCube".into(),
            kind: PrimitiveKind::Cube,
            size: 1.0,
            material: PaletteIndex::new(1).unwrap(),
        }
        .apply(&mut scene)
        .unwrap();
        let node_id = scene.active_lowpoly_node().unwrap();
        Command::SetMeshVertexWeights {
            node_id,
            vertex_ids: (0..8).collect(),
            joints: [3, 0, 0, 0],
            weights: [1.0, 0.0, 0.0, 0.0],
        }
        .apply(&mut scene)
        .unwrap();
        Command::ApplyBipedRig.apply(&mut scene).unwrap();

        let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
        let parsed = gltf::Gltf::from_slice(&glb).unwrap();
        let blob = parsed.blob.as_ref().unwrap();
        let prim = parsed.meshes().next().unwrap().primitives().next().unwrap();
        let reader = prim.reader(|_| Some(blob.as_slice()));
        let joints: Vec<[u16; 4]> = reader.read_joints(0).unwrap().into_u16().collect();
        let weights: Vec<[f32; 4]> = reader.read_weights(0).unwrap().into_f32().collect();
        assert!(!joints.is_empty());
        assert_eq!(joints.len(), weights.len());
        for (j, w) in joints.iter().zip(weights.iter()) {
            assert_eq!(*j, [3, 0, 0, 0]);
            assert_eq!(*w, [1.0, 0.0, 0.0, 0.0]);
        }
    }

    #[test]
    fn glb_export_writes_morph_targets_for_morphed_mesh() {
        let mut scene = SceneState::new();
        Command::CreatePrimitiveMesh {
            name: "Smiler".into(),
            kind: PrimitiveKind::Cube,
            size: 1.0,
            material: PaletteIndex::new(1).unwrap(),
        }
        .apply(&mut scene)
        .unwrap();
        let node_id = scene.active_lowpoly_node().unwrap();
        Command::AddMorphDelta {
            node_id,
            name: "Smile".into(),
            vertex_ids: vec![0],
            delta: Vec3::new(0.0, 0.5, 0.0),
        }
        .apply(&mut scene)
        .unwrap();

        let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
        // The binary must stay parseable with morphs present.
        gltf::Gltf::from_slice(&glb).unwrap();
        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let json: serde_json::Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();
        let mesh_json = json["meshes"].as_array().unwrap().last().unwrap();
        let prim = mesh_json["primitives"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p.get("targets").is_some())
            .expect("a primitive with morph targets must export");
        let targets = prim["targets"].as_array().unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(mesh_json["weights"], serde_json::json!([0.0]));

        // Target deltas land on the triangulated corners of source vertex 0.
        let mesh = scene.lowpoly_meshes.get(&node_id).unwrap();
        let tri = mesh.to_triangulated_flat();
        let expected: usize = tri.source_indices.iter().filter(|s| **s == 0).count();
        assert!(expected > 0);
        let acc_idx = targets[0]["POSITION"].as_u64().unwrap() as usize;
        let acc = json["accessors"].as_array().unwrap()[acc_idx].clone();
        assert_eq!(acc["count"], tri.vertices.len());
        assert_eq!(acc["type"], "VEC3");
        // Sum of absolute y deltas equals |delta.y| * corners sharing vertex 0.
        // BIN data starts right after the 12-byte header + 8-byte JSON chunk
        // header + padded JSON (BIN chunk header is 8 bytes).
        let bv = acc["bufferView"].as_u64().unwrap() as usize;
        let bvs = json["bufferViews"].as_array().unwrap();
        let byte_offset: usize = bvs[bv]["byteOffset"].as_u64().unwrap_or(0) as usize;
        let bin_start = 12 + 8 + json_len + 8;
        let mut sum = 0.0f32;
        for i in 0..tri.vertices.len() {
            let off = bin_start + byte_offset + i * 12 + 4; // y component
            let y = f32::from_le_bytes(glb[off..off + 4].try_into().unwrap()).abs();
            sum += y;
        }
        assert!((sum - 0.5 * expected as f32).abs() < 1e-3, "sum {sum}");
    }

    #[test]
    fn glb_export_writes_morph_weight_channels() {
        let mut scene = SceneState::new();
        Command::CreatePrimitiveMesh {
            name: "Talker".into(),
            kind: PrimitiveKind::Cube,
            size: 1.0,
            material: PaletteIndex::new(1).unwrap(),
        }
        .apply(&mut scene)
        .unwrap();
        let node_id = scene.active_lowpoly_node().unwrap();
        Command::AddMorphDelta {
            node_id,
            name: "Smile".into(),
            vertex_ids: vec![0],
            delta: Vec3::new(0.0, 0.5, 0.0),
        }
        .apply(&mut scene)
        .unwrap();
        Command::ApplyBipedRig.apply(&mut scene).unwrap();
        Command::AddAnimationClip {
            name: "Talk".into(),
            duration_secs: 1.0,
        }
        .apply(&mut scene)
        .unwrap();
        Command::SetMorphWeightKeyframe {
            clip_index: 2,
            node_id,
            target: "Smile".into(),
            time_secs: 0.0,
            weight: 0.0,
        }
        .apply(&mut scene)
        .unwrap();
        Command::SetMorphWeightKeyframe {
            clip_index: 2,
            node_id,
            target: "Smile".into(),
            time_secs: 1.0,
            weight: 1.0,
        }
        .apply(&mut scene)
        .unwrap();

        let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
        let parsed = gltf::Gltf::from_slice(&glb).unwrap();
        // Clips export in order: Idle, Walk, Talk.
        let anim = parsed.animations().nth(2).expect("Talk clip must export");
        let mut found_weights = false;
        for ch in anim.channels() {
            use gltf::animation::{util::ReadOutputs, Property};
            if ch.target().property() == Property::MorphTargetWeights {
                found_weights = true;
                let reader = ch.reader(|_| parsed.blob.as_deref());
                let inputs: Vec<f32> = reader.read_inputs().unwrap().collect();
                assert_eq!(inputs, vec![0.0, 1.0]);
                let Some(ReadOutputs::MorphTargetWeights(out)) = reader.read_outputs() else {
                    panic!("weights output must be morph weights");
                };
                let outputs: Vec<f32> = out.into_f32().collect();
                assert_eq!(outputs, vec![0.0, 1.0]);
            }
        }
        assert!(found_weights, "weights channel must export");
    }

    #[test]
    fn glb_export_survives_bone_and_clip_removal() {
        let mut scene = SceneState::new();
        Command::CreatePrimitiveMesh {
            name: "Hero".into(),
            kind: PrimitiveKind::Cube,
            size: 1.0,
            material: PaletteIndex::new(1).unwrap(),
        }
        .apply(&mut scene)
        .unwrap();
        Command::ApplyBipedRig.apply(&mut scene).unwrap();
        // Drop RightArm (leaf) and the Walk clip, then export must parse.
        Command::RemoveBone {
            bone_id: tessel_core::BoneId(4),
        }
        .apply(&mut scene)
        .unwrap();
        let rig = scene.character_rig.as_ref().unwrap();
        assert_eq!(rig.bones.len(), 6);
        Command::RemoveAnimationClip { clip_index: 1 }
            .apply(&mut scene)
            .unwrap();
        assert_eq!(scene.character_rig.as_ref().unwrap().animations.len(), 1);

        let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
        let parsed = gltf::Gltf::from_slice(&glb).unwrap();
        assert_eq!(parsed.animations().count(), 1);
        assert_eq!(parsed.skins().count(), 1);
    }

    #[test]
    fn glb_export_applies_node_transform_to_mesh_positions() {
        let mut scene = SceneState::new();
        Command::CreatePrimitiveMesh {
            name: "ShiftedCube".into(),
            kind: PrimitiveKind::Cube,
            size: 1.0,
            material: PaletteIndex::new(1).unwrap(),
        }
        .apply(&mut scene)
        .unwrap();
        let node_id = scene.active_lowpoly_node().unwrap();
        Command::SetNodeTransform {
            node_id,
            transform: tessel_core::Transform {
                translation: Vec3::new(5.0, 2.0, -3.0),
                rotation: glam::Quat::IDENTITY,
                scale: Vec3::new(2.0, 1.0, 1.0),
            },
        }
        .apply(&mut scene)
        .unwrap();

        let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
        let parsed = gltf::Gltf::from_slice(&glb).unwrap();
        let blob = parsed.blob.as_ref().unwrap();
        let prim = parsed.meshes().next().unwrap().primitives().next().unwrap();
        let reader = prim.reader(|_| Some(blob.as_slice()));
        let positions: Vec<[f32; 3]> = reader.read_positions().unwrap().collect();
        let min_x = positions.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min);
        let max_x = positions
            .iter()
            .map(|p| p[0])
            .fold(f32::NEG_INFINITY, f32::max);
        let min_y = positions.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min);
        assert!((min_x - 4.0).abs() < 1e-4);
        assert!((max_x - 6.0).abs() < 1e-4);
        assert!((min_y - 2.0).abs() < 1e-4);
    }

    #[test]
    fn glb_export_deduplicates_identical_pbr_materials() {
        let mut scene = SceneState::new();
        let mat = tessel_core::MeshMaterial {
            metallic: 0.45,
            roughness: 0.5,
            emissive: [0.0; 3],
        };
        for i in 0..4 {
            Command::CreatePrimitiveMesh {
                name: format!("Part_{i}"),
                kind: PrimitiveKind::Cube,
                size: 0.5,
                material: PaletteIndex::new((i + 1) as u8).unwrap(),
            }
            .apply(&mut scene)
            .unwrap();
            let id = scene.active_lowpoly_node().unwrap();
            Command::SetMeshMaterial {
                node_id: id,
                material: mat,
            }
            .apply(&mut scene)
            .unwrap();
        }
        let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
        let parsed = gltf::Gltf::from_slice(&glb).unwrap();
        // All 4 parts share identical PBR parameters + palette texture -> deduplicated to 1 material!
        assert_eq!(parsed.materials().count(), 1);
    }

    #[test]
    fn glb_export_writes_step_scale_channels_for_visibility_keyframes() {
        let mut scene = SceneState::new();
        Command::CreatePrimitiveMesh {
            name: "MuzzleFlash".into(),
            kind: PrimitiveKind::Sphere,
            size: 0.1,
            material: PaletteIndex::new(6).unwrap(),
        }
        .apply(&mut scene)
        .unwrap();
        let flash_id = scene.active_lowpoly_node().unwrap();
        Command::ApplyHumanoidRig.apply(&mut scene).unwrap();
        Command::SetVisibilityKeyframe {
            clip_index: 0,
            node_id: flash_id,
            time_secs: 0.0,
            visible: false,
        }
        .apply(&mut scene)
        .unwrap();
        Command::SetVisibilityKeyframe {
            clip_index: 0,
            node_id: flash_id,
            time_secs: 0.25,
            visible: true,
        }
        .apply(&mut scene)
        .unwrap();

        let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
        let parsed = gltf::Gltf::from_slice(&glb).unwrap();
        let idle = parsed.animations().next().expect("Idle clip must export");
        let vis_chan = idle
            .channels()
            .find(|c| {
                c.target().property() == gltf::animation::Property::Scale
                    && c.sampler().interpolation() == gltf::animation::Interpolation::Step
            })
            .expect("visibility track should export as STEP scale channel");
        // Node 0 = merged mesh, nodes 1..=19 = 19 humanoid bones, node 20 = dedicated MuzzleFlash
        assert_eq!(vis_chan.target().node().index(), 20);
    }
}
