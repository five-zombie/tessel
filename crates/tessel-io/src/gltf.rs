use std::collections::BTreeMap;
use std::io::Cursor;

use glam::{Mat4, Vec3};
use image::{ImageFormat, RgbaImage};
use serde_json::{json, Value};
use tessel_core::voxel::greedy_mesh_volume;
use tessel_core::{AutoSkinSolver, Palette, PaletteIndex, SceneState};

use crate::error::IoError;
use crate::ExportPreset;

fn is_cubic_interpolation(interp: tessel_core::KeyframeInterpolation) -> bool {
    matches!(
        interp,
        tessel_core::KeyframeInterpolation::EaseIn
            | tessel_core::KeyframeInterpolation::EaseOut
            | tessel_core::KeyframeInterpolation::EaseInOut
            | tessel_core::KeyframeInterpolation::CubicBezier { .. }
    )
}

/// Normalized-parameter Hermite endpoint slopes `(out_slope, in_slope)` matching
/// `KeyframeInterpolation::apply(t)` for glTF 2.0 `CUBICSPLINE` tangents.
fn easing_segment_slopes(interp: tessel_core::KeyframeInterpolation) -> (f32, f32) {
    match interp {
        tessel_core::KeyframeInterpolation::Linear => (1.0, 1.0),
        tessel_core::KeyframeInterpolation::EaseIn => (0.0, 2.0),
        tessel_core::KeyframeInterpolation::EaseOut => (2.0, 0.0),
        tessel_core::KeyframeInterpolation::EaseInOut
        | tessel_core::KeyframeInterpolation::Step => (0.0, 0.0),
        tessel_core::KeyframeInterpolation::CubicBezier { p1x, p1y, p2x, p2y } => {
            let x1 = if p1x.is_finite() {
                p1x.clamp(0.0, 1.0)
            } else {
                0.25
            };
            let y1 = if p1y.is_finite() {
                p1y.clamp(-1.0, 2.0)
            } else {
                0.1
            };
            let x2 = if p2x.is_finite() {
                p2x.clamp(0.0, 1.0)
            } else {
                0.75
            };
            let y2 = if p2y.is_finite() {
                p2y.clamp(-1.0, 2.0)
            } else {
                0.9
            };
            let m0 = if x1 > 1e-4 {
                (y1 / x1).clamp(-4.0, 4.0)
            } else {
                0.0
            };
            let m1 = if (1.0 - x2) > 1e-4 {
                ((1.0 - y2) / (1.0 - x2)).clamp(-4.0, 4.0)
            } else {
                0.0
            };
            (m0, m1)
        }
    }
}

fn align_quat_hemispheres(mut quats: Vec<[f32; 4]>) -> Vec<[f32; 4]> {
    for i in 1..quats.len() {
        let dot = quats[i - 1][0] * quats[i][0]
            + quats[i - 1][1] * quats[i][1]
            + quats[i - 1][2] * quats[i][2]
            + quats[i - 1][3] * quats[i][3];
        if dot < 0.0 {
            for c in &mut quats[i] {
                *c = -*c;
            }
        }
    }
    quats
}

fn pack_cubicspline<const N: usize>(
    times: &[f32],
    interps: &[tessel_core::KeyframeInterpolation],
    values: &[[f32; N]],
) -> Vec<[f32; N]> {
    let count = values.len();
    let mut in_tangents = vec![[0.0f32; N]; count];
    let mut out_tangents = vec![[0.0f32; N]; count];
    for i in 0..count.saturating_sub(1) {
        let dt = times[i + 1] - times[i];
        if dt > 1e-6 {
            let (m0, m1) = easing_segment_slopes(interps[i]);
            let inv_dt = 1.0 / dt;
            for c in 0..N {
                let delta = values[i + 1][c] - values[i][c];
                out_tangents[i][c] = m0 * delta * inv_dt;
                in_tangents[i + 1][c] = m1 * delta * inv_dt;
            }
        }
    }
    let mut packed = Vec::with_capacity(count * 3);
    for i in 0..count {
        packed.push(in_tangents[i]);
        packed.push(values[i]);
        packed.push(out_tangents[i]);
    }
    packed
}

/// Expands `Step` segments inside a mixed `CUBICSPLINE` channel by inserting a hold keyframe
/// at `t_next - 1e-4` with the preceding keyframe's value so glTF players hold flat until jump.
fn expand_mixed_step_keyframes<T: Copy>(
    times: &[f32],
    interps: &[tessel_core::KeyframeInterpolation],
    values: &[T],
) -> (Vec<f32>, Vec<tessel_core::KeyframeInterpolation>, Vec<T>) {
    let mut out_t = Vec::with_capacity(times.len() + 2);
    let mut out_i = Vec::with_capacity(interps.len() + 2);
    let mut out_v = Vec::with_capacity(values.len() + 2);
    for idx in 0..times.len() {
        out_t.push(times[idx]);
        out_i.push(interps[idx]);
        out_v.push(values[idx]);
        if idx + 1 < times.len()
            && interps[idx] == tessel_core::KeyframeInterpolation::Step
            && (times[idx + 1] - times[idx]) > 2e-4
        {
            out_t.push(times[idx + 1] - 1e-4);
            out_i.push(tessel_core::KeyframeInterpolation::Linear);
            out_v.push(values[idx]);
        }
    }
    (out_t, out_i, out_v)
}

fn node_world_matrices(scene: &SceneState, node_id: tessel_core::NodeId) -> (Mat4, Mat4) {
    let mat = scene.node_rest_world_matrix(node_id);
    let normal_mat = if mat.determinant().abs() > 1e-8 {
        mat.inverse().transpose()
    } else {
        mat
    };
    (mat, normal_mat)
}

/// Returns `true` if any `NodeKind::Group` ancestor within 16 hops is hidden (`!visible`).
fn has_hidden_group_ancestor(scene: &SceneState, node_id: tessel_core::NodeId) -> bool {
    let mut curr = scene.nodes.get(&node_id).and_then(|n| n.parent);
    let mut hops = 0usize;
    while let Some(pid) = curr {
        if hops >= 16 {
            break;
        }
        hops += 1;
        let Some(pnode) = scene.nodes.get(&pid) else {
            break;
        };
        if pnode.kind == tessel_core::NodeKind::Group && !pnode.visible {
            return true;
        }
        curr = pnode.parent;
    }
    false
}

/// Checks whether `node_id` has an effectively visible `NodeKind::Group` ancestor chain within 16 hops.
/// Returns `false` if any `Group` ancestor in the chain is hidden.
fn has_group_ancestor(scene: &SceneState, node_id: tessel_core::NodeId) -> bool {
    let mut curr = scene.nodes.get(&node_id).and_then(|n| n.parent);
    let mut hops = 0usize;
    let mut found_visible_group = false;
    while let Some(pid) = curr {
        if hops >= 16 {
            break;
        }
        hops += 1;
        let Some(pnode) = scene.nodes.get(&pid) else {
            break;
        };
        if pnode.kind == tessel_core::NodeKind::Group {
            if !pnode.visible {
                return false;
            }
            found_visible_group = true;
        }
        curr = pnode.parent;
    }
    found_visible_group
}

fn collect_node_transform_animated(
    scene: &SceneState,
) -> std::collections::BTreeSet<tessel_core::NodeId> {
    scene
        .character_rig
        .as_ref()
        .map(|rig| {
            rig.animations
                .iter()
                .flat_map(|c| c.node_transform_tracks.iter().map(|t| t.node))
                .collect()
        })
        .unwrap_or_default()
}

/// Single source of truth for "this low-poly node gets a dedicated morphed
/// export node". Used both when precomputing `morphed_order` (node indices for
/// animation channels) and when emitting the dedicated primitives, so the two
/// lists cannot drift apart (checked by the `debug_assert!` after the loop).
fn is_morphed_export_node(
    scene: &SceneState,
    node_id: tessel_core::NodeId,
    mesh: &tessel_core::LowPolyMesh,
    vis_animated: &std::collections::BTreeSet<tessel_core::NodeId>,
    trs_animated: &std::collections::BTreeSet<tessel_core::NodeId>,
) -> bool {
    let visible = scene.nodes.get(&node_id).map(|n| n.visible).unwrap_or(true);
    (visible || vis_animated.contains(&node_id))
        && !mesh.faces.is_empty()
        && (!mesh.morphs.is_empty()
            || vis_animated.contains(&node_id)
            || trs_animated.contains(&node_id)
            || has_group_ancestor(scene, node_id))
        && {
            let tri = mesh.to_triangulated_flat();
            !tri.vertices.is_empty() && !tri.indices.is_empty()
        }
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

        let node_trs_animated = collect_node_transform_animated(scene);

        for (node_id, mesh) in &scene.lowpoly_meshes {
            let visible = scene.nodes.get(node_id).map(|n| n.visible).unwrap_or(true);
            let is_custom = mesh.texture.is_some()
                || mesh.normal_map.is_some()
                || mesh.ao_map.is_some()
                || !mesh.vertex_colors.is_empty()
                || mesh.material != tessel_core::MeshMaterial::default()
                || !mesh.morphs.is_empty()
                || vis_animated.contains(node_id)
                || node_trs_animated.contains(node_id)
                || has_group_ancestor(scene, *node_id);
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
        let vis_animated: std::collections::BTreeSet<tessel_core::NodeId> = scene
            .character_rig
            .as_ref()
            .map(|r| {
                r.animations
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
        let node_trs_animated = collect_node_transform_animated(scene);
        for (node_id, mesh) in &scene.lowpoly_meshes {
            let visible = scene.nodes.get(node_id).map(|n| n.visible).unwrap_or(true);
            let is_custom = mesh.texture.is_some()
                || mesh.normal_map.is_some()
                || mesh.ao_map.is_some()
                || !mesh.vertex_colors.is_empty()
                || mesh.material != tessel_core::MeshMaterial::default()
                || !mesh.morphs.is_empty()
                || vis_animated.contains(node_id)
                || node_trs_animated.contains(node_id)
                || has_group_ancestor(scene, *node_id);
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

    let mut nodes: Vec<Value> = vec![json!({ "name": "TesselAsset" })];
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
    let mut palette_tex_index: Option<usize> = if has_palette_prim { Some(0) } else { None };
    let mut any_unlit = false;
    // Deduplicate identical PBR materials across nodes so multi-part assets avoid redundant draw calls.
    type MaterialKey = (
        usize,
        Option<usize>,
        Option<(usize, u32)>,
        u32,
        u32,
        [u32; 3],
        bool,
        (u8, u32),
        bool,
    );
    let mut material_cache: BTreeMap<MaterialKey, usize> = BTreeMap::new();
    if has_palette_prim {
        material_cache.insert(
            (
                0,
                None,
                None,
                0.0_f32.to_bits(),
                0.85_f32.to_bits(),
                [0.0_f32.to_bits(); 3],
                false,
                (0, 0),
                false,
            ),
            0,
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
            lp_mesh.material.double_sided,
            alpha_key,
            lp_mesh.material.unlit,
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
            if lp_mesh.material.unlit {
                any_unlit = true;
                mat_json["extensions"] = json!({ "KHR_materials_unlit": {} });
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
            materials.push(mat_json);
            material_cache.insert(mat_key, idx);
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
    let has_skin = !skins.is_empty();
    let mut meshes: Vec<Value> = if !primitives.is_empty() {
        nodes[0]["mesh"] = json!(0);
        if has_skin {
            nodes[0]["skin"] = json!(0);
        }
        vec![json!({
            "name": "TesselMesh",
            "primitives": primitives
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
        if has_skin && !is_local_trs_node {
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
    if any_unlit {
        gltf_json["extensionsUsed"] = json!(["KHR_materials_unlit"]);
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

        // Verify PBR metallic, roughness, emissive, and doubleSided factors export cleanly into glTF 2.0
        Command::SetMeshMaterial {
            node_id,
            material: tessel_core::MeshMaterial {
                metallic: 0.9,
                roughness: 0.2,
                emissive: [0.1, 0.5, 0.8],
                double_sided: true,
                ..Default::default()
            },
        }
        .apply(&mut scene)
        .unwrap();
        let (glb_pbr, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
        let parsed_pbr = gltf::Gltf::from_slice(&glb_pbr).unwrap();
        let mat = parsed_pbr.materials().next().unwrap();
        assert!((mat.pbr_metallic_roughness().metallic_factor() - 0.9).abs() < 1e-4);
        assert!(mat.double_sided());
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

        // Verify AO Map embeds as glTF occlusionTexture (shares TEXCOORD_0 when base texture exists)
        Command::SetMeshAoMap {
            node_id,
            ao_map: Some(tessel_core::MeshTexture::solid(
                16,
                16,
                Rgba8::rgb(200, 200, 200),
            )),
        }
        .apply(&mut scene)
        .unwrap();
        let (glb_ao, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
        let parsed_ao = gltf::Gltf::from_slice(&glb_ao).unwrap();
        let occ = parsed_ao
            .materials()
            .next()
            .unwrap()
            .occlusion_texture()
            .expect("expected glTF occlusionTexture");
        assert_eq!(occ.tex_coord(), 0);

        // When base texture is cleared (palette UV on TEXCOORD_0), occlusionTexture uses TEXCOORD_1
        Command::SetMeshTexture {
            node_id,
            texture: None,
        }
        .apply(&mut scene)
        .unwrap();
        let (glb_ao_pal, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
        let parsed_ao_pal = gltf::Gltf::from_slice(&glb_ao_pal).unwrap();
        let occ_pal = parsed_ao_pal
            .materials()
            .next()
            .unwrap()
            .occlusion_texture()
            .expect("expected glTF occlusionTexture with palette base");
        assert_eq!(occ_pal.tex_coord(), 1);
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
            double_sided: false,
            ..Default::default()
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

    #[test]
    fn glb_export_hidden_visibility_animated_node_matches_morphed_order() {
        let mut scene = SceneState::new();
        Command::CreatePrimitiveMesh {
            name: "HiddenFlash".into(),
            kind: PrimitiveKind::Cube,
            size: 1.0,
            material: PaletteIndex::new(1).unwrap(),
        }
        .apply(&mut scene)
        .unwrap();
        let flash_id = scene.active_lowpoly_node().unwrap();
        Command::ApplyHumanoidRig.apply(&mut scene).unwrap();
        // Hide the node *before* export: it must still export a dedicated node
        // because a visibility track animates it (morphed_order includes it).
        Command::SetNodeVisibility {
            node_id: flash_id,
            visible: false,
        }
        .apply(&mut scene)
        .unwrap();
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
        // A second, visible morphed node keeps `morphed` non-empty so the
        // order mismatch surfaces as the assert panic (not EmptyCanvas).
        Command::CreatePrimitiveMesh {
            name: "Smiler".into(),
            kind: PrimitiveKind::Cube,
            size: 1.0,
            material: PaletteIndex::new(1).unwrap(),
        }
        .apply(&mut scene)
        .unwrap();
        let smiler_id = scene.active_lowpoly_node().unwrap();
        Command::AddMorphDelta {
            node_id: smiler_id,
            name: "Smile".into(),
            vertex_ids: vec![0],
            delta: Vec3::new(0.0, 0.5, 0.0),
        }
        .apply(&mut scene)
        .unwrap();

        // Pre-fix this panicked on the morphed/morphed_order debug_assert!.
        let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
        let parsed = gltf::Gltf::from_slice(&glb).unwrap();
        assert!(parsed.meshes().len() >= 2);
        let idle = parsed.animations().next().expect("Idle clip must export");
        assert!(
            idle.channels().any(|c| {
                c.target().property() == gltf::animation::Property::Scale
                    && c.sampler().interpolation() == gltf::animation::Interpolation::Step
            }),
            "hidden visibility-animated node must keep its STEP scale channel"
        );
    }

    #[test]
    fn glb_export_writes_npr_color0_alpha_unlit_and_cubicspline() {
        let mut scene = SceneState::new();
        Command::CreatePrimitiveMesh {
            name: "InkBanner".into(),
            kind: PrimitiveKind::Cube,
            size: 1.0,
            material: PaletteIndex::new(2).unwrap(),
        }
        .apply(&mut scene)
        .unwrap();
        let node_id = scene.active_lowpoly_node().unwrap();
        Command::SetMeshMaterial {
            node_id,
            material: tessel_core::MeshMaterial {
                alpha_mode: tessel_core::AlphaMode::Mask { cutoff: 0.45 },
                unlit: true,
                ..tessel_core::MeshMaterial::default()
            },
        }
        .apply(&mut scene)
        .unwrap();
        Command::SetMeshVertexColors {
            node_id,
            updates: vec![(0, [0.8, 0.5, 0.9, 1.0])],
        }
        .apply(&mut scene)
        .unwrap();
        Command::ApplyBipedRig.apply(&mut scene).unwrap();
        Command::SetAnimationKeyframe {
            clip_index: 0,
            bone_id: tessel_core::BoneId(1),
            keyframe: tessel_core::Keyframe {
                time_secs: 0.0,
                translation: Vec3::ZERO,
                rotation: glam::Quat::IDENTITY,
                scale: Vec3::ONE,
                interpolation: tessel_core::KeyframeInterpolation::CubicBezier {
                    p1x: 0.25,
                    p1y: 0.1,
                    p2x: 0.25,
                    p2y: 1.0,
                },
            },
        }
        .apply(&mut scene)
        .unwrap();

        let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
        let parsed = gltf::Gltf::from_slice(&glb).unwrap();
        // Verify COLOR_0 attribute on primitive
        let has_color0 = parsed.meshes().any(|m| {
            m.primitives()
                .any(|p| p.get(&gltf::Semantic::Colors(0)).is_some())
        });
        assert!(
            has_color0,
            "expected COLOR_0 vertex color attribute in glTF"
        );
        // Verify alphaMode MASK + KHR_materials_unlit
        let has_mask = parsed
            .materials()
            .any(|m| m.alpha_mode() == gltf::material::AlphaMode::Mask);
        assert!(has_mask, "expected alphaMode MASK in glTF material");
        let root_json = parsed.as_json();
        assert!(
            root_json
                .extensions_used
                .iter()
                .any(|e| e == "KHR_materials_unlit"),
            "expected KHR_materials_unlit in extensionsUsed"
        );
        // Verify CUBICSPLINE sampler exported for CubicBezier channel with 3*N packed outputs
        let mut cubic_channels = 0usize;
        for anim in parsed.animations() {
            for ch in anim.channels() {
                let sampler = ch.sampler();
                if sampler.interpolation() == gltf::animation::Interpolation::CubicSpline {
                    assert_eq!(
                        sampler.output().count(),
                        sampler.input().count() * 3,
                        "CUBICSPLINE output accessor must be 3x input count"
                    );
                    assert!(
                        sampler.input().count() >= 2,
                        "CUBICSPLINE requires at least 2 keyframes"
                    );
                    cubic_channels += 1;
                } else {
                    assert_eq!(
                        sampler.output().count(),
                        sampler.input().count(),
                        "LINEAR/STEP output accessor must match input count"
                    );
                }
            }
        }
        assert!(
            cubic_channels >= 2,
            "expected CUBICSPLINE on both rotation and translation channels, got {cubic_channels}"
        );
    }

    #[test]
    fn obj_export_keeps_vertex_color_meshes_and_glb_respects_stepped_fps() {
        let mut scene = SceneState::new();
        Command::CreatePrimitiveMesh {
            name: "ColoredCube".into(),
            kind: PrimitiveKind::Cube,
            size: 1.0,
            material: PaletteIndex::new(3).unwrap(),
        }
        .apply(&mut scene)
        .unwrap();
        let node_id = scene.active_lowpoly_node().unwrap();
        Command::SetMeshVertexColors {
            node_id,
            updates: vec![(0, [1.0, 0.5, 0.5, 1.0])],
        }
        .apply(&mut scene)
        .unwrap();
        // OBJ export must NOT drop meshes that only have vertex_colors
        let (obj, _) = crate::export_obj_string(&scene, ExportPreset::Godot4).unwrap();
        assert!(obj.contains("v ") && obj.contains("f "));

        // Stepped FPS on a clip must automatically export STEP channels to GLB
        Command::ApplyBipedRig.apply(&mut scene).unwrap();
        if let Some(rig) = scene.character_rig.as_mut() {
            rig.animations[1].stepped_fps = Some(8.0);
        }
        let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
        let parsed = gltf::Gltf::from_slice(&glb).unwrap();
        let walk = parsed.animations().nth(1).unwrap();
        assert!(walk
            .channels()
            .all(|c| c.sampler().interpolation() == gltf::animation::Interpolation::Step));
    }

    #[test]
    fn gltf_easing_interpolations_export_as_cubicspline_with_tangents() {
        let mut scene = SceneState::new();
        Command::CreatePrimitiveMesh {
            name: "RigMesh".into(),
            kind: PrimitiveKind::Cube,
            size: 0.5,
            material: PaletteIndex::new(1).unwrap(),
        }
        .apply(&mut scene)
        .unwrap();
        Command::ApplyBipedRig.apply(&mut scene).unwrap();
        let bone = tessel_core::BoneId(1);
        for (clip_idx, interp) in [
            (0usize, tessel_core::KeyframeInterpolation::EaseIn),
            (1usize, tessel_core::KeyframeInterpolation::EaseOut),
        ] {
            Command::SetAnimationKeyframe {
                clip_index: clip_idx,
                bone_id: bone,
                keyframe: tessel_core::Keyframe {
                    time_secs: 0.0,
                    translation: Vec3::ZERO,
                    rotation: glam::Quat::IDENTITY,
                    scale: Vec3::ONE,
                    interpolation: interp,
                },
            }
            .apply(&mut scene)
            .unwrap();
            Command::SetAnimationKeyframe {
                clip_index: clip_idx,
                bone_id: bone,
                keyframe: tessel_core::Keyframe {
                    time_secs: 1.0,
                    translation: Vec3::new(0.0, 1.0, 0.0),
                    rotation: glam::Quat::from_rotation_x(0.5),
                    scale: Vec3::ONE,
                    interpolation: tessel_core::KeyframeInterpolation::EaseInOut,
                },
            }
            .apply(&mut scene)
            .unwrap();
        }

        let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
        let parsed = gltf::Gltf::from_slice(&glb).unwrap();
        let mut cubic_count = 0usize;
        for anim in parsed.animations() {
            for ch in anim.channels() {
                let sampler = ch.sampler();
                if sampler.interpolation() == gltf::animation::Interpolation::CubicSpline {
                    assert_eq!(sampler.output().count(), sampler.input().count() * 3);
                    cubic_count += 1;
                }
            }
        }
        assert!(
            cubic_count >= 4,
            "EaseIn/EaseOut/EaseInOut channels must export as CUBICSPLINE (got {cubic_count})"
        );

        // Verify exact Hermite tangents for EaseIn (0, 2) and EaseOut (2, 0) on [0 -> 1] over dt=1.0
        let packed_in = pack_cubicspline(
            &[0.0, 1.0],
            &[
                tessel_core::KeyframeInterpolation::EaseIn,
                tessel_core::KeyframeInterpolation::EaseIn,
            ],
            &[[0.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        );
        assert!((packed_in[2][1] - 0.0).abs() < 1e-6); // out-tangent at k=0 is 0
        assert!((packed_in[3][1] - 2.0).abs() < 1e-6); // in-tangent at k=1 is 2*delta/dt = 2

        let packed_out = pack_cubicspline(
            &[0.0, 1.0],
            &[
                tessel_core::KeyframeInterpolation::EaseOut,
                tessel_core::KeyframeInterpolation::EaseOut,
            ],
            &[[0.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        );
        assert!((packed_out[2][1] - 2.0).abs() < 1e-6); // out-tangent at k=0 is 2*delta/dt = 2
        assert!((packed_out[3][1] - 0.0).abs() < 1e-6); // in-tangent at k=1 is 0

        // Mixed Step + EaseInOut expands Step segment with a hold keyframe at t_next - 1e-4
        let (exp_t, _, exp_v) = expand_mixed_step_keyframes(
            &[0.0, 0.5, 1.0],
            &[
                tessel_core::KeyframeInterpolation::Step,
                tessel_core::KeyframeInterpolation::EaseInOut,
                tessel_core::KeyframeInterpolation::EaseInOut,
            ],
            &[0.0f32, 1.0, 2.0],
        );
        assert_eq!(exp_t.len(), 4);
        assert!((exp_t[1] - (0.5 - 1e-4)).abs() < 1e-6);
        assert!((exp_v[1] - 0.0).abs() < 1e-6);

        // NodeTransformTrack easing also exports as CUBICSPLINE
        let node_id = scene.active_lowpoly_node().unwrap();
        if let Some(rig) = scene.character_rig.as_mut() {
            rig.animations[0]
                .node_transform_tracks
                .push(tessel_core::NodeTransformTrack {
                    node: node_id,
                    keyframes: vec![
                        tessel_core::Keyframe {
                            time_secs: 0.0,
                            translation: Vec3::ZERO,
                            rotation: glam::Quat::IDENTITY,
                            scale: Vec3::ONE,
                            interpolation: tessel_core::KeyframeInterpolation::Step,
                        },
                        tessel_core::Keyframe {
                            time_secs: 0.5,
                            translation: Vec3::new(0.0, 1.0, 0.0),
                            rotation: glam::Quat::IDENTITY,
                            scale: Vec3::ONE,
                            interpolation: tessel_core::KeyframeInterpolation::EaseInOut,
                        },
                        tessel_core::Keyframe {
                            time_secs: 1.0,
                            translation: Vec3::new(0.0, 2.0, 0.0),
                            rotation: glam::Quat::IDENTITY,
                            scale: Vec3::ONE,
                            interpolation: tessel_core::KeyframeInterpolation::EaseInOut,
                        },
                    ],
                });
        }
        let (glb2, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
        let parsed2 = gltf::Gltf::from_slice(&glb2).unwrap();
        let idle = parsed2.animations().next().unwrap();
        let has_expanded_nt_cubic = idle.channels().any(|c| {
            let s = c.sampler();
            s.interpolation() == gltf::animation::Interpolation::CubicSpline
                && s.input().count() == 4
                && s.output().count() == 12
        });
        assert!(
            has_expanded_nt_cubic,
            "mixed Step+EaseInOut NodeTransformTrack must export as expanded CUBICSPLINE"
        );
    }

    #[test]
    fn v5_4_bone_grid_morph_normals_cage_ribbon_and_stick_gizmo_export_roundtrip() {
        let mut scene = SceneState::new();
        let mut history = tessel_core::HistoryStack::new();

        history
            .execute(
                &mut scene,
                Command::CreatePrimitiveMesh {
                    name: "CapeMesh".to_string(),
                    kind: PrimitiveKind::Cube,
                    size: 1.0,
                    material: PaletteIndex::new(1).unwrap(),
                },
            )
            .unwrap();
        let cape_id = scene.active_lowpoly_node().unwrap();

        // 1. Bone Grid + Cloth Wind Bake
        history
            .execute(
                &mut scene,
                Command::CreateBoneGrid {
                    spec: tessel_core::BoneGridSpec {
                        prefix: "Cape".to_string(),
                        cols: 3,
                        rows: 3,
                        ..Default::default()
                    },
                    bind_node: Some(cape_id),
                },
            )
            .unwrap();
        history
            .execute(
                &mut scene,
                Command::BakeClothGridAnimation {
                    bone_prefix: "Cape".to_string(),
                    params: tessel_core::ClothWindParams {
                        clip_name: "CapeWind".to_string(),
                        duration_secs: 1.0,
                        fps: 6.0,
                        ..Default::default()
                    },
                },
            )
            .unwrap();

        // 2. Proxy Cage baked into MorphTarget (with position + normal deltas) + default weight
        history
            .execute(
                &mut scene,
                Command::ApplyProxyCageDeform {
                    node_id: cape_id,
                    divisions: [2, 2, 2],
                    control_deltas: vec![
                        (0, Vec3::new(0.0, -0.3, 0.2)),
                        (7, Vec3::new(0.2, 0.3, 0.0)),
                    ],
                    floor_y: Some(-0.5),
                    bake_as_morph: Some("CageFlutter".to_string()),
                },
            )
            .unwrap();
        history
            .execute(
                &mut scene,
                Command::SetMorphDefaultWeight {
                    node_id: cape_id,
                    name: "CageFlutter".to_string(),
                    weight: 0.45,
                },
            )
            .unwrap();

        // 3. Procedural Ribbon with STEP visibility track
        history
            .execute(
                &mut scene,
                Command::BakeProceduralRibbon {
                    params: tessel_core::RibbonParams {
                        name: "SwordTrail".to_string(),
                        source_bone: Some(tessel_core::BoneId(2)),
                        source_node: None,
                        explicit_points: Vec::new(),
                        clip_index: 0,
                        start_time: 0.1,
                        end_time: 0.9,
                        samples: 8,
                        width_start: 0.15,
                        width_end: 0.02,
                        material: PaletteIndex::new(2).unwrap(),
                        bake_visibility_track: true,
                    },
                },
            )
            .unwrap();

        // 4. Stick Gizmo Twist + Undo/Redo
        let prev_rot = scene.character_rig.as_ref().unwrap().bones[0]
            .pose_local
            .rotation;
        history
            .execute(
                &mut scene,
                Command::ApplyStickGizmoPose {
                    params: tessel_core::StickGizmoParams {
                        bone_id: tessel_core::BoneId(0),
                        mode: tessel_core::StickGizmoMode::Twist,
                        angle_deg: 45.0,
                        chain_len: 3,
                        snap_deg: 15.0,
                        mirror_lr: false,
                        axis_lock: None,
                    },
                },
            )
            .unwrap();
        assert_ne!(
            scene.character_rig.as_ref().unwrap().bones[0]
                .pose_local
                .rotation,
            prev_rot
        );
        history.undo(&mut scene).unwrap();
        assert_eq!(
            scene.character_rig.as_ref().unwrap().bones[0]
                .pose_local
                .rotation,
            prev_rot
        );
        history.redo(&mut scene).unwrap();

        // 5. Round-trip .tessel archive & glTF 2.0 .glb validation
        let archive = crate::encode_tessel_project(&scene).unwrap();
        let decoded = crate::decode_tessel_project(&archive).unwrap();
        assert_eq!(
            decoded.lowpoly_meshes[&cape_id].morphs[0].name,
            "CageFlutter"
        );
        assert!((decoded.lowpoly_meshes[&cape_id].morphs[0].default_weight - 0.45).abs() < 1e-4);

        let (glb, _) = export_glb_bytes(&decoded, ExportPreset::Godot4).unwrap();
        let parsed = gltf::Gltf::from_slice(&glb).unwrap();
        assert!(parsed.skins().count() >= 1);
        assert!(parsed.animations().count() >= 1);
        let morphed_mesh = parsed
            .meshes()
            .find(|m| m.weights().is_some_and(|w| !w.is_empty()))
            .expect("morphed mesh with weights must export");
        assert!((morphed_mesh.weights().unwrap()[0] - 0.45).abs() < 1e-4);
        let prim = morphed_mesh.primitives().next().unwrap();
        let target = prim.morph_targets().next().unwrap();
        assert!(target.positions().is_some());
        assert!(target.normals().is_some());

        // 6. Verify GLB re-import preserves normal_deltas and default_weight
        let reimported = crate::import_glb_bytes(&glb, PaletteIndex::new(1).unwrap()).unwrap();
        assert!(!reimported.morphs.is_empty());
        assert!(!reimported.morphs[0].normal_deltas.is_empty());
        assert!((reimported.morphs[0].default_weight - 0.45).abs() < 1e-4);

        // 7. Verify SetMorphDefaultWeight undo also clears rig.morph_weights cache
        history
            .execute(
                &mut scene,
                Command::SetMorphDefaultWeight {
                    node_id: cape_id,
                    name: "CageFlutter".to_string(),
                    weight: 0.9,
                },
            )
            .unwrap();
        assert!(
            (scene.character_rig.as_ref().unwrap().morph_weights
                [&(cape_id, "CageFlutter".to_string())]
                - 0.9)
                .abs()
                < 1e-4
        );
        history.undo(&mut scene).unwrap();
        assert!(
            (scene.character_rig.as_ref().unwrap().morph_weights
                [&(cape_id, "CageFlutter".to_string())]
                - 0.45)
                .abs()
                < 1e-4
        );
    }

    #[test]
    fn e5_gltf_spec_invariants_and_deep_group_hierarchy_beyond_16_levels() {
        let mut scene = SceneState::new();
        let mut history = tessel_core::HistoryStack::default();

        // Build an 18-level nested Group hierarchy (g0 -> g1 -> ... -> g17) via CreateGroupNode
        let mut groups: Vec<tessel_core::NodeId> = Vec::with_capacity(18);
        let mut prev_group: Option<tessel_core::NodeId> = None;
        for depth in 0..18 {
            history
                .execute(
                    &mut scene,
                    Command::CreateGroupNode {
                        name: format!("Group_{depth}"),
                        transform: tessel_core::Transform::default(),
                        parent: prev_group,
                    },
                )
                .unwrap();
            let gid = scene.selected_node.expect("group created");
            groups.push(gid);
            prev_group = Some(gid);
        }

        history
            .execute(
                &mut scene,
                Command::CreatePrimitiveMesh {
                    name: "DeepCube".into(),
                    kind: tessel_core::PrimitiveKind::Cube,
                    size: 1.0,
                    material: PaletteIndex::DEFAULT_ACTIVE,
                },
            )
            .unwrap();
        let mesh_id = scene.selected_node.unwrap();

        // SetNodeParent enforces the 16-hop cycle/depth cap (depth 18 fails closed with OutOfBounds).
        assert!(matches!(
            history.execute(
                &mut scene,
                Command::SetNodeParent {
                    node_id: mesh_id,
                    parent: Some(groups[17]),
                },
            ),
            Err(tessel_core::CoreError::OutOfBounds)
        ));

        // Parenting within the 16-hop cap (Group_14, 15 ancestors) succeeds.
        history
            .execute(
                &mut scene,
                Command::SetNodeParent {
                    node_id: mesh_id,
                    parent: Some(groups[14]),
                },
            )
            .unwrap();

        assert!(has_group_ancestor(&scene, mesh_id));

        let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();

        // Full gltf crate document + GLB binary buffer + accessor/node validation
        let parsed = gltf::Gltf::from_slice(&glb).unwrap();
        let doc = &parsed.document;
        assert_eq!(doc.as_json().asset.version, "2.0");
        let bin_len = parsed.blob.as_ref().map_or(0, |b| b.len());
        for bv in doc.views() {
            assert!(bv.offset() + bv.length() <= bin_len);
        }
        for acc in doc.accessors() {
            if let Some(bv) = acc.view() {
                assert!(acc.offset() + acc.count() * acc.size() <= bv.length());
            }
        }

        // Verify all 18 group nodes are present and linked in parent -> child chain via GLB JSON
        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let raw_json: Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();
        let json_nodes = raw_json["nodes"].as_array().unwrap();
        let group_count = json_nodes
            .iter()
            .filter(|n| n["name"].as_str().is_some_and(|s| s.starts_with("Group_")))
            .count();
        assert_eq!(group_count, 18);

        let mut curr_idx = json_nodes
            .iter()
            .position(|n| n["name"].as_str() == Some("Group_0"))
            .expect("Group_0 root node");
        let mut group14_idx = None;
        for expected_depth in 1..18 {
            let expected_name = format!("Group_{expected_depth}");
            let children = json_nodes[curr_idx]["children"]
                .as_array()
                .expect("expected nested group children");
            curr_idx = children
                .iter()
                .filter_map(|v| v.as_u64().map(|u| u as usize))
                .find(|&i| json_nodes[i]["name"].as_str() == Some(expected_name.as_str()))
                .expect("expected nested group child");
            if expected_depth == 14 {
                group14_idx = Some(curr_idx);
            }
        }
        let g14 = group14_idx.expect("Group_14 node index");
        assert!(
            json_nodes[g14]["children"]
                .as_array()
                .is_some_and(|cs| cs.iter().any(|v| {
                    v.as_u64()
                        .is_some_and(|i| json_nodes[i as usize]["mesh"].is_u64())
                })),
            "Group_14 should parent the exported mesh node"
        );
    }

    #[test]
    fn glb_export_omits_empty_primitives_mesh_and_validates_cubicspline_step_scale() {
        let mut scene = SceneState::new();
        Command::CreatePrimitiveMesh {
            name: "MorphedOnly".into(),
            kind: tessel_core::PrimitiveKind::Cube,
            size: 1.0,
            material: PaletteIndex::new(1).unwrap(),
        }
        .apply(&mut scene)
        .unwrap();
        let node_id = scene.active_lowpoly_node().unwrap();
        Command::SetMorphTargetDeltas {
            node_id,
            name: "Blink".into(),
            position_deltas: vec![(0, Vec3::new(0.0, 0.1, 0.0))],
            normal_deltas: vec![],
            replace: true,
        }
        .apply(&mut scene)
        .unwrap();

        Command::AddBone {
            name: "Root".into(),
            parent: None,
            position: Vec3::ZERO,
        }
        .apply(&mut scene)
        .unwrap();
        let bone_id = scene.character_rig.as_ref().unwrap().bones[0].id;
        Command::AddAnimationClip {
            name: "ScaleCubicStep".into(),
            duration_secs: 1.0,
        }
        .apply(&mut scene)
        .unwrap();
        let clip_idx = scene.character_rig.as_ref().unwrap().active_animation;
        for (t, s, interp) in [
            (
                0.0,
                Vec3::ONE,
                tessel_core::KeyframeInterpolation::CubicBezier {
                    p1x: 0.42,
                    p1y: 0.0,
                    p2x: 0.58,
                    p2y: 1.0,
                },
            ),
            (
                0.5,
                Vec3::splat(1.5),
                tessel_core::KeyframeInterpolation::Step,
            ),
            (1.0, Vec3::ONE, tessel_core::KeyframeInterpolation::Linear),
        ] {
            Command::SetAnimationKeyframe {
                clip_index: clip_idx,
                bone_id,
                keyframe: tessel_core::Keyframe {
                    time_secs: t,
                    rotation: glam::Quat::IDENTITY,
                    translation: Vec3::ZERO,
                    scale: s,
                    interpolation: interp,
                },
            }
            .apply(&mut scene)
            .unwrap();
        }

        let (glb, _) = export_glb_bytes(&scene, ExportPreset::default()).unwrap();
        let parsed = gltf::Gltf::from_slice(&glb).expect("GLB must be valid glTF 2.0");
        // Every exported mesh must have at least 1 primitive (no empty primitives: []).
        for mesh in parsed.meshes() {
            assert!(
                mesh.primitives().len() >= 1,
                "mesh {} must have >= 1 primitive",
                mesh.index()
            );
        }
        // For the CUBICSPLINE + mixed Step scale channel, output count must equal input count * 3.
        let scale_ch = parsed
            .animations()
            .flat_map(|a| a.channels())
            .find(|c| c.target().property() == gltf::animation::Property::Scale)
            .expect("scale channel");
        assert_eq!(
            scale_ch.sampler().interpolation(),
            gltf::animation::Interpolation::CubicSpline
        );
        assert_eq!(
            scale_ch.sampler().output().count(),
            scale_ch.sampler().input().count() * 3
        );
    }
}
