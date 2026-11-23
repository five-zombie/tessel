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

        for (node_id, mesh) in &scene.lowpoly_meshes {
            let visible = scene.nodes.get(node_id).map(|n| n.visible).unwrap_or(true);
            let is_custom = mesh.texture.is_some()
                || mesh.normal_map.is_some()
                || mesh.material != tessel_core::MeshMaterial::default();
            if !visible || mesh.faces.is_empty() || is_custom {
                continue;
            }
            let (node_mat, normal_mat) = node_world_matrices(scene, *node_id);
            let tri = mesh.to_triangulated_flat();
            let base = positions.len() as u32;
            for v in tri.vertices {
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

                let sampler_idx = samplers.len();
                samplers.push(json!({
                    "input": time_acc,
                    "interpolation": "LINEAR",
                    "output": rot_acc
                }));
                channels.push(json!({
                    "sampler": sampler_idx,
                    "target": { "node": target_node, "path": "rotation" }
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

    // Export any Low-Poly meshes with custom RGBA texture or PBR material as dedicated primitives.
    let mut palette_tex_index: Option<usize> = if has_palette_prim { Some(0) } else { None };
    for (node_id, lp_mesh) in &scene.lowpoly_meshes {
        let visible = scene.nodes.get(node_id).map(|n| n.visible).unwrap_or(true);
        let has_custom = lp_mesh.texture.is_some()
            || lp_mesh.normal_map.is_some()
            || lp_mesh.material != tessel_core::MeshMaterial::default();
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

        let mat_idx = materials.len();
        let mut mat_json = json!({
            "name": format!("TesselMeshMaterial_{}", node_id.0),
            "pbrMetallicRoughness": {
                "baseColorTexture": { "index": tex_idx },
                "metallicFactor": lp_mesh.material.metallic,
                "roughnessFactor": lp_mesh.material.roughness
            }
        });
        if lp_mesh.material.emissive.iter().any(|&c| c > 0.0) {
            mat_json["emissiveFactor"] = json!(lp_mesh.material.emissive);
        }
        if let Some(nmap) = &lp_mesh.normal_map {
            let norm_png = encode_rgba_png(nmap.width, nmap.height, &nmap.rgba)?;
            let n_img_bv = push_view(&mut bin, &mut buffer_views, &norm_png, None);
            let n_img_idx = images.len();
            images.push(json!({ "bufferView": n_img_bv, "mimeType": "image/png" }));
            let n_tex_idx = textures.len();
            textures.push(json!({ "sampler": 0, "source": n_img_idx }));
            mat_json["normalTexture"] = json!({ "index": n_tex_idx });
        }
        materials.push(mat_json);
        let mut custom_attrs = json!({
            "POSITION": p_acc,
            "NORMAL": n_acc,
            "TEXCOORD_0": u_acc
        });
        if let Some(solver) = &skin_solver {
            let mut t_joints = Vec::with_capacity(t_pos.len());
            let mut t_weights = Vec::with_capacity(t_pos.len());
            for &p in &t_pos {
                let sw = solver.weights_for_vertex(Some(*node_id), Vec3::from_array(p));
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
        primitives.push(json!({
            "attributes": custom_attrs,
            "indices": i_acc,
            "material": mat_idx
        }));
        total_triangles += tri.indices.len() / 3;
    }

    if primitives.is_empty() {
        return Err(IoError::EmptyCanvas);
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
        "meshes": [{
            "name": "TesselMesh",
            "primitives": primitives
        }],
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
}
