use std::collections::BTreeMap;
use std::io::Cursor;

use image::{ImageFormat, RgbaImage};
use serde_json::{json, Value};
use tessel_core::voxel::greedy_mesh_volume;
use tessel_core::{Palette, PaletteIndex, SceneState};

use crate::error::IoError;
use crate::ExportPreset;

/// Collects all visible 3D geometry (voxels + low-poly meshes) into flat indexed buffers
/// scaled by `preset.scale_factor()`.
pub(crate) struct CollectedMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

impl CollectedMesh {
    pub fn from_scene(scene: &SceneState, preset: ExportPreset) -> Result<Self, IoError> {
        let scale = preset.scale_factor();
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uvs = Vec::new();
        let mut indices = Vec::new();

        for (node_id, vol) in &scene.voxel_volumes {
            let visible = scene.nodes.get(node_id).map(|n| n.visible).unwrap_or(true);
            if !visible || vol.is_empty() {
                continue;
            }
            let mesh = greedy_mesh_volume(vol, scene.unit_scale * scale);
            let base = positions.len() as u32;
            for v in mesh.vertices {
                positions.push(v.position);
                normals.push(v.normal);
                uvs.push([(f32::from(v.palette_index.get()) + 0.5) / 256.0, 0.5]);
            }
            indices.extend(mesh.indices.into_iter().map(|i| i + base));
        }

        for (node_id, mesh) in &scene.lowpoly_meshes {
            let visible = scene.nodes.get(node_id).map(|n| n.visible).unwrap_or(true);
            if !visible || mesh.faces.is_empty() {
                continue;
            }
            let tri = mesh.to_triangulated_flat();
            let base = positions.len() as u32;
            for v in tri.vertices {
                positions.push([
                    v.position[0] * scale,
                    v.position[1] * scale,
                    v.position[2] * scale,
                ]);
                normals.push(v.normal);
                uvs.push([(f32::from(v.palette_index.get()) + 0.5) / 256.0, 0.5]);
            }
            indices.extend(tri.indices.into_iter().map(|i| i + base));
        }

        if positions.is_empty() || indices.is_empty() {
            return Err(IoError::EmptyCanvas);
        }

        Ok(Self {
            positions,
            normals,
            uvs,
            indices,
        })
    }
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
    let img = RgbaImage::from_raw(256, 1, rgba).ok_or(IoError::InvalidDimensions(256, 1))?;
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, ImageFormat::Png)?;
    Ok(buf.into_inner())
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
    let mesh = CollectedMesh::from_scene(scene, preset)?;
    let palette_png = build_palette_png(&scene.palette)?;
    let scale = preset.scale_factor();

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

    let mut nodes: Vec<Value> = vec![json!({ "name": "TesselAsset", "mesh": 0 })];
    let mut root_nodes: Vec<usize> = vec![0];
    let mut animations: Vec<Value> = Vec::new();

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
        "images": [{
            "bufferView": img_bv,
            "mimeType": "image/png"
        }],
        "textures": [{
            "sampler": 0,
            "source": 0
        }],
        "materials": [{
            "name": "TesselPaletteMaterial",
            "pbrMetallicRoughness": {
                "baseColorTexture": { "index": 0 },
                "metallicFactor": 0.0,
                "roughnessFactor": 0.85
            }
        }],
        "meshes": [{
            "name": "TesselMesh",
            "primitives": [{
                "attributes": {
                    "POSITION": 0,
                    "NORMAL": 1,
                    "TEXCOORD_0": 2
                },
                "indices": 3,
                "material": 0
            }]
        }],
        "buffers": [{ "byteLength": bin.len() }],
        "bufferViews": buffer_views,
        "accessors": accessors
    });

    if !animations.is_empty() {
        gltf_json["animations"] = Value::Array(animations);
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

    Ok((glb, mesh.indices.len() / 3))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tessel_core::{Command, VoxelCoord};

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
        assert!(parsed.nodes().count() >= 8); // Mesh + 7 biped bones
        assert!(parsed.blob.is_some());
    }
}
