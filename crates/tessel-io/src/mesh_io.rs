use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::fs;
use std::path::Path;

use glam::{Mat4, Quat, Vec3, Vec4};
use tessel_core::scene::{validate_node_name, MAX_NODE_NAME_LEN};
use tessel_core::{
    AnimationClip, Bone, BoneChannel, BoneId, CharacterRig, Face, Keyframe, KeyframeInterpolation,
    LowPolyMesh, MeshTexture, MorphTarget, PaletteIndex, SceneState, Transform, VertexSkinWeights,
};

use crate::error::IoError;
use crate::gltf::CollectedMesh;
use crate::ExportPreset;

/// Exports visible scene 3D geometry to Wavefront `.obj` text.
pub fn export_obj_string(
    scene: &SceneState,
    preset: ExportPreset,
) -> Result<(String, usize), IoError> {
    let mesh = CollectedMesh::from_scene(scene, preset)?;
    let mut out = String::from("# Exported by Tessel\no TesselAsset\n");

    for p in &mesh.positions {
        let _ = writeln!(out, "v {:.6} {:.6} {:.6}", p[0], p[1], p[2]);
    }
    for n in &mesh.normals {
        let _ = writeln!(out, "vn {:.6} {:.6} {:.6}", n[0], n[1], n[2]);
    }
    for uv in &mesh.uvs {
        let _ = writeln!(out, "vt {:.6} {:.6}", uv[0], uv[1]);
    }
    for tri in mesh.indices.as_chunks::<3>().0 {
        let a = tri[0] + 1;
        let b = tri[1] + 1;
        let c = tri[2] + 1;
        let _ = writeln!(out, "f {a}/{a}/{a} {b}/{b}/{b} {c}/{c}/{c}");
    }

    Ok((out, mesh.indices.len() / 3))
}

/// Exports visible scene 3D geometry to binary `.stl` (suitable for 3D printing).
pub fn export_stl_bytes(
    scene: &SceneState,
    preset: ExportPreset,
) -> Result<(Vec<u8>, usize), IoError> {
    let mesh = CollectedMesh::from_scene(scene, preset)?;
    let tri_count = mesh.indices.len() / 3;
    let mut out = Vec::with_capacity(84 + tri_count * 50);

    let mut header = [0u8; 80];
    let label = b"Exported by Tessel";
    header[..label.len()].copy_from_slice(label);
    out.extend_from_slice(&header);
    out.extend_from_slice(&(tri_count as u32).to_le_bytes());

    for tri in mesh.indices.as_chunks::<3>().0 {
        let p0 = mesh.positions[tri[0] as usize];
        let p1 = mesh.positions[tri[1] as usize];
        let p2 = mesh.positions[tri[2] as usize];
        let n = mesh.normals[tri[0] as usize];

        for v in [n, p0, p1, p2] {
            for f in v {
                out.extend_from_slice(&f.to_le_bytes());
            }
        }
        out.extend_from_slice(&0u16.to_le_bytes());
    }

    Ok((out, tri_count))
}

const MAX_IMPORT_BYTES: usize = 64 * 1024 * 1024;
const MAX_IMPORT_VERTICES: usize = 500_000;
const MAX_IMPORT_FACES: usize = 500_000;
const MAX_IMPORT_ACCESSOR_ITEMS: usize = 2_000_000;
const MAX_IMPORT_MORPH_TARGETS: usize = 128;
const MAX_OBJ_FACE_VERTICES: usize = 256;
const MAX_IMPORT_RIG_BONES: usize = 1024;
const MAX_IMPORT_RIG_CLIPS: usize = 256;
const MAX_IMPORT_KEYFRAMES_PER_CHANNEL: usize = 16_384;

/// Imports a Wavefront `.obj` text document into a `LowPolyMesh`, preserving `vt` UVs when present.
pub fn import_obj_str(obj_text: &str, material: PaletteIndex) -> Result<LowPolyMesh, IoError> {
    if obj_text.len() > MAX_IMPORT_BYTES {
        return Err(IoError::PayloadTooLarge(
            "OBJ text exceeds 64 MiB safety limit".into(),
        ));
    }
    let mut vertices = Vec::new();
    let mut tex_coords: Vec<[f32; 2]> = Vec::new();
    let mut faces = Vec::new();

    for raw_line in obj_text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(tag) = parts.next() else {
            continue;
        };
        match tag {
            "v" => {
                let x: f32 = parts
                    .next()
                    .and_then(|s| s.parse().ok())
                    .ok_or_else(|| IoError::InvalidFormat("invalid OBJ vertex x".into()))?;
                let y: f32 = parts
                    .next()
                    .and_then(|s| s.parse().ok())
                    .ok_or_else(|| IoError::InvalidFormat("invalid OBJ vertex y".into()))?;
                let z: f32 = parts
                    .next()
                    .and_then(|s| s.parse().ok())
                    .ok_or_else(|| IoError::InvalidFormat("invalid OBJ vertex z".into()))?;
                let v = Vec3::new(x, y, z);
                if !v.is_finite() {
                    return Err(IoError::InvalidFormat(
                        "non-finite OBJ vertex coordinate".into(),
                    ));
                }
                if vertices.len() >= MAX_IMPORT_VERTICES {
                    return Err(IoError::PayloadTooLarge(format!(
                        "OBJ vertex count exceeds limit {MAX_IMPORT_VERTICES}"
                    )));
                }
                vertices.push(v);
            }
            "vt" => {
                if tex_coords.len() >= MAX_IMPORT_ACCESSOR_ITEMS {
                    return Err(IoError::PayloadTooLarge(format!(
                        "OBJ texcoord count exceeds limit {MAX_IMPORT_ACCESSOR_ITEMS}"
                    )));
                }
                if let (Some(u), Some(v)) = (
                    parts.next().and_then(|s| s.parse::<f32>().ok()),
                    parts.next().and_then(|s| s.parse::<f32>().ok()),
                ) {
                    if u.is_finite() && v.is_finite() {
                        tex_coords.push([u, 1.0 - v]);
                    }
                }
            }
            "f" => {
                let mut indices = Vec::new();
                let mut face_uvs = Vec::new();
                for token in parts {
                    let mut segs = token.split('/');
                    let v_str = segs.next().unwrap_or("");
                    let raw_idx: isize = v_str
                        .parse()
                        .map_err(|_| IoError::InvalidFormat("invalid OBJ face index".into()))?;
                    let resolved = if raw_idx > 0 {
                        (raw_idx - 1) as usize
                    } else if raw_idx < 0 {
                        let back = raw_idx.unsigned_abs();
                        vertices
                            .len()
                            .checked_sub(back)
                            .ok_or_else(|| IoError::InvalidFormat("OBJ index underflow".into()))?
                    } else {
                        return Err(IoError::InvalidFormat("OBJ index cannot be 0".into()));
                    };
                    if resolved >= vertices.len() {
                        return Err(IoError::InvalidFormat(
                            "OBJ face index out of bounds".into(),
                        ));
                    }
                    if indices.len() >= MAX_OBJ_FACE_VERTICES {
                        return Err(IoError::PayloadTooLarge(format!(
                            "OBJ face vertex count exceeds limit {MAX_OBJ_FACE_VERTICES}"
                        )));
                    }
                    indices.push(resolved);
                    if let Some(vt_str) = segs.next().filter(|s| !s.is_empty()) {
                        if let Ok(vt_raw) = vt_str.parse::<isize>() {
                            let vt_idx = if vt_raw > 0 {
                                Some((vt_raw - 1) as usize)
                            } else if vt_raw < 0 {
                                tex_coords.len().checked_sub(vt_raw.unsigned_abs())
                            } else {
                                None
                            };
                            if let Some(uv) = vt_idx.and_then(|i| tex_coords.get(i).copied()) {
                                face_uvs.push(uv);
                            }
                        }
                    }
                }
                if indices.len() >= 3 {
                    if faces.len() >= MAX_IMPORT_FACES {
                        return Err(IoError::PayloadTooLarge(format!(
                            "OBJ face count exceeds limit {MAX_IMPORT_FACES}"
                        )));
                    }
                    if face_uvs.len() == indices.len() {
                        faces.push(Face::with_uvs(indices, material, face_uvs));
                    } else {
                        faces.push(Face::new(indices, material));
                    }
                }
            }
            _ => {}
        }
    }

    if vertices.is_empty() || faces.is_empty() {
        return Err(IoError::InvalidFormat(
            "OBJ file contains no valid mesh faces".into(),
        ));
    }
    Ok(LowPolyMesh {
        vertices,
        faces,
        texture: None,
        normal_map: None,
        ao_map: None,
        matcap_map: None,
        face_sdf_map: None,
        hatching_ramp_map: None,
        paint_layers: Vec::new(),
        active_paint_layer: None,
        vertex_colors: BTreeMap::new(),
        crease_edges: Vec::new(),
        uv_seams: Vec::new(),
        proxy_normals: BTreeMap::new(),
        modifiers: Vec::new(),
        smooth_shading: false,
        material: Default::default(),
        skin_weights: Default::default(),
        morphs: Default::default(),
    })
}

/// Imports a binary glTF 2.0 (`.glb`) buffer into a `LowPolyMesh`, preserving `TEXCOORD_0` UVs.
pub fn import_glb_bytes(bytes: &[u8], material: PaletteIndex) -> Result<LowPolyMesh, IoError> {
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(IoError::PayloadTooLarge(
            "GLB buffer exceeds 64 MiB safety limit".into(),
        ));
    }
    let gltf = gltf::Gltf::from_slice(bytes)
        .map_err(|e| IoError::InvalidFormat(format!("invalid GLB: {e}")))?;
    let blob = gltf.blob.as_deref();

    let mut vertices = Vec::new();
    let mut faces = Vec::new();
    let mut skin_weights = BTreeMap::new();
    let mut morphs: Vec<MorphTarget> = Vec::new();

    for mesh in gltf.meshes() {
        for prim in mesh.primitives() {
            if prim.mode() != gltf::mesh::Mode::Triangles {
                continue;
            }
            if let Some(pos_acc) = prim.get(&gltf::Semantic::Positions) {
                if pos_acc.count() > MAX_IMPORT_ACCESSOR_ITEMS
                    || vertices.len().saturating_add(pos_acc.count()) > MAX_IMPORT_VERTICES
                {
                    return Err(IoError::PayloadTooLarge(format!(
                        "GLB vertex count exceeds limit {MAX_IMPORT_VERTICES}"
                    )));
                }
            }
            if let Some(idx_acc) = prim.indices() {
                if idx_acc.count() > MAX_IMPORT_ACCESSOR_ITEMS
                    || faces.len().saturating_add(idx_acc.count() / 3) > MAX_IMPORT_FACES
                {
                    return Err(IoError::PayloadTooLarge(format!(
                        "GLB index/face count exceeds safety limit ({MAX_IMPORT_FACES} faces)"
                    )));
                }
            }
            for semantic in [
                gltf::Semantic::Joints(0),
                gltf::Semantic::Weights(0),
                gltf::Semantic::TexCoords(0),
            ] {
                if let Some(acc) = prim.get(&semantic) {
                    if acc.count() > MAX_IMPORT_ACCESSOR_ITEMS {
                        return Err(IoError::PayloadTooLarge(format!(
                            "GLB accessor count {} exceeds limit {MAX_IMPORT_ACCESSOR_ITEMS}",
                            acc.count()
                        )));
                    }
                }
            }
            if morphs.len().saturating_add(prim.morph_targets().len()) > MAX_IMPORT_MORPH_TARGETS {
                return Err(IoError::PayloadTooLarge(format!(
                    "GLB morph target count exceeds limit {MAX_IMPORT_MORPH_TARGETS}"
                )));
            }
            let reader = prim.reader(|_| blob);
            let Some(positions) = reader.read_positions() else {
                continue;
            };
            let base = vertices.len();
            for p in positions {
                if vertices.len() >= MAX_IMPORT_VERTICES {
                    return Err(IoError::PayloadTooLarge(format!(
                        "GLB vertex count exceeds limit {MAX_IMPORT_VERTICES}"
                    )));
                }
                let v = Vec3::from_array(p);
                if !v.is_finite() {
                    return Err(IoError::InvalidFormat(
                        "non-finite GLB vertex position".into(),
                    ));
                }
                vertices.push(v);
            }
            let added = vertices.len() - base;
            if added < 3 {
                continue;
            }
            // Round-trip skinning: JOINTS_0/WEIGHTS_0 become manual overrides.
            // All-zero rows are skipped so the auto solver still covers them.
            if let (Some(joints), Some(weights)) = (reader.read_joints(0), reader.read_weights(0)) {
                let joints: Vec<[u16; 4]> = joints.into_u16().collect();
                let weights: Vec<[f32; 4]> = weights.into_f32().collect();
                for (i, (j, w)) in joints.iter().zip(weights.iter()).enumerate() {
                    if i >= added || !w.iter().all(|x| x.is_finite()) {
                        continue;
                    }
                    if w.iter().any(|x| *x > 0.0) {
                        skin_weights.insert(
                            base + i,
                            VertexSkinWeights {
                                joints: *j,
                                weights: *w,
                            },
                        );
                    }
                }
            }
            // Round-trip blend shapes: target POSITION + NORMAL deltas and mesh default weights.
            let mesh_weights = mesh.weights().unwrap_or(&[]);
            for (t_idx, (positions, normals, _)) in reader.read_morph_targets().enumerate() {
                let Some(positions) = positions else {
                    continue;
                };
                let name = format!("Morph{}", morphs.len());
                let mut map = BTreeMap::new();
                for (i, d) in positions.enumerate() {
                    if i >= added {
                        break;
                    }
                    let v = Vec3::from_array(d);
                    if v.is_finite() && v.length_squared() > 1e-12 {
                        map.insert(base + i, v);
                    }
                }
                let mut normal_map = BTreeMap::new();
                if let Some(normals) = normals {
                    for (i, nd) in normals.enumerate() {
                        if i >= added {
                            break;
                        }
                        let v = Vec3::from_array(nd);
                        if v.is_finite() && v.length_squared() > 1e-12 {
                            normal_map.insert(base + i, v);
                        }
                    }
                }
                let default_weight = mesh_weights
                    .get(t_idx)
                    .copied()
                    .filter(|w| w.is_finite())
                    .unwrap_or(0.0)
                    .clamp(0.0, 1.0);
                if !map.is_empty() || !normal_map.is_empty() {
                    morphs.push(MorphTarget {
                        name,
                        deltas: map,
                        normal_deltas: normal_map,
                        default_weight,
                    });
                }
            }
            let prim_uvs: Vec<[f32; 2]> = reader
                .read_tex_coords(0)
                .map(|tc| {
                    tc.into_f32()
                        .map(|uv| {
                            if uv[0].is_finite() && uv[1].is_finite() {
                                uv
                            } else {
                                [0.0, 0.0]
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            let indices: Vec<usize> = match reader.read_indices() {
                Some(iter) => iter.into_u32().map(|i| base + i as usize).collect(),
                None => (base..base + added).collect(),
            };
            for tri in indices.as_chunks::<3>().0 {
                if faces.len() >= MAX_IMPORT_FACES {
                    return Err(IoError::PayloadTooLarge(format!(
                        "GLB face count exceeds limit {MAX_IMPORT_FACES}"
                    )));
                }
                if tri[0] < vertices.len() && tri[1] < vertices.len() && tri[2] < vertices.len() {
                    let (l0, l1, l2) = (tri[0] - base, tri[1] - base, tri[2] - base);
                    if l0 < prim_uvs.len() && l1 < prim_uvs.len() && l2 < prim_uvs.len() {
                        faces.push(Face::with_uvs(
                            vec![tri[0], tri[1], tri[2]],
                            material,
                            vec![prim_uvs[l0], prim_uvs[l1], prim_uvs[l2]],
                        ));
                    } else {
                        faces.push(Face::new(vec![tri[0], tri[1], tri[2]], material));
                    }
                }
            }
        }
    }

    if vertices.is_empty() || faces.is_empty() {
        return Err(IoError::InvalidFormat(
            "GLB file contains no triangle geometry".into(),
        ));
    }
    Ok(LowPolyMesh {
        vertices,
        faces,
        texture: None,
        normal_map: None,
        ao_map: None,
        matcap_map: None,
        face_sdf_map: None,
        hatching_ramp_map: None,
        paint_layers: Vec::new(),
        active_paint_layer: None,
        vertex_colors: BTreeMap::new(),
        crease_edges: Vec::new(),
        uv_seams: Vec::new(),
        proxy_normals: BTreeMap::new(),
        modifiers: Vec::new(),
        smooth_shading: false,
        material: Default::default(),
        skin_weights,
        morphs,
    })
}

#[derive(Debug, Default)]
struct GlbBoneTracks {
    translations: Vec<(f32, Vec3, KeyframeInterpolation)>,
    rotations: Vec<(f32, Quat, KeyframeInterpolation)>,
    scales: Vec<(f32, Vec3, KeyframeInterpolation)>,
}

fn decompose_positive_trs(mat: Mat4) -> Option<Transform> {
    if !mat.to_cols_array().iter().all(|v| v.is_finite()) {
        return None;
    }
    if (mat.row(3) - Vec4::W).length_squared() > 1e-5 {
        return None;
    }
    let sx = mat.x_axis.truncate().length();
    let sy = mat.y_axis.truncate().length();
    let sz = mat.z_axis.truncate().length();
    if !sx.is_finite()
        || !sy.is_finite()
        || !sz.is_finite()
        || sx <= 1e-6
        || sy <= 1e-6
        || sz <= 1e-6
    {
        return None;
    }
    let det = mat.determinant();
    let scale_vol = sx * sy * sz;
    if !det.is_finite() || det <= 0.0 || !scale_vol.is_finite() || scale_vol <= 0.0 {
        return None;
    }
    let norm_det = det / scale_vol;
    if !norm_det.is_finite() || norm_det <= 1e-3 {
        return None;
    }
    let (cs, cr, ct) = mat.to_scale_rotation_translation();
    if !ct.is_finite()
        || !cr.is_finite()
        || cr.length_squared() <= 1e-8
        || !cs.is_finite()
        || cs.x <= 1e-6
        || cs.y <= 1e-6
        || cs.z <= 1e-6
    {
        return None;
    }
    Some(Transform {
        translation: ct,
        rotation: cr.normalize(),
        scale: cs,
    })
}

fn time_bucket(t: f32) -> i64 {
    (t * 10_000.0).round() as i64
}

fn normalize_track<T: Copy>(track: &mut Vec<(f32, T, KeyframeInterpolation)>) {
    track.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut deduped: Vec<(f32, T, KeyframeInterpolation)> = Vec::with_capacity(track.len());
    for item in track.drain(..) {
        if let Some(last) = deduped.last_mut() {
            if time_bucket(last.0) == time_bucket(item.0) {
                *last = item;
                continue;
            }
        }
        deduped.push(item);
    }
    *track = deduped;
}

fn sample_vec3_track(track: &[(f32, Vec3, KeyframeInterpolation)], t: f32) -> Option<Vec3> {
    let first = track.first()?;
    let last = track.last()?;
    if t <= first.0 {
        return Some(first.1);
    }
    if t >= last.0 {
        return Some(last.1);
    }
    let idx = track.partition_point(|k| k.0 <= t);
    let prev = track[idx.saturating_sub(1)];
    let next = track[idx.min(track.len() - 1)];
    if (prev.0 - t).abs() <= 1e-5 || prev.2 == KeyframeInterpolation::Step {
        return Some(prev.1);
    }
    let span = next.0 - prev.0;
    if span <= 1e-6 {
        return Some(next.1);
    }
    let alpha = ((t - prev.0) / span).clamp(0.0, 1.0);
    Some(prev.1.lerp(next.1, alpha))
}

fn sample_quat_track(track: &[(f32, Quat, KeyframeInterpolation)], t: f32) -> Option<Quat> {
    let first = track.first()?;
    let last = track.last()?;
    if t <= first.0 {
        return Some(first.1);
    }
    if t >= last.0 {
        return Some(last.1);
    }
    let idx = track.partition_point(|k| k.0 <= t);
    let prev = track[idx.saturating_sub(1)];
    let next = track[idx.min(track.len() - 1)];
    if (prev.0 - t).abs() <= 1e-5 || prev.2 == KeyframeInterpolation::Step {
        return Some(prev.1);
    }
    let span = next.0 - prev.0;
    if span <= 1e-6 {
        return Some(next.1);
    }
    let alpha = ((t - prev.0) / span).clamp(0.0, 1.0);
    Some(prev.1.slerp(next.1, alpha).normalize())
}

fn sanitize_glb_bone_name(raw: Option<&str>, fallback_idx: usize) -> String {
    if let Some(s) = raw {
        let cleaned: String = s
            .trim()
            .chars()
            .filter(|c| !c.is_control())
            .take(MAX_NODE_NAME_LEN)
            .collect();
        if validate_node_name(&cleaned).is_ok() {
            return cleaned;
        }
    }
    format!("Bone_{fallback_idx}")
}

/// Imports a skeletal hierarchy and bone animations from a binary glTF 2.0 (`.glb`) buffer
/// into a standalone `CharacterRig`. Validates bounds, finiteness, and acyclicity, and
/// topologically sorts bones parent-before-child.
pub fn import_glb_rig_bytes(bytes: &[u8]) -> Result<CharacterRig, IoError> {
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(IoError::PayloadTooLarge(
            "GLB buffer exceeds 64 MiB safety limit".into(),
        ));
    }
    let gltf = gltf::Gltf::from_slice(bytes)
        .map_err(|e| IoError::InvalidFormat(format!("invalid GLB: {e}")))?;
    let blob = gltf.blob.as_deref();

    let mut joint_node_indices: Vec<usize> = Vec::new();
    let mut seen_joints: BTreeSet<usize> = BTreeSet::new();
    for skin in gltf.skins() {
        for joint in skin.joints() {
            if seen_joints.insert(joint.index()) {
                if joint_node_indices.len() >= MAX_IMPORT_RIG_BONES {
                    return Err(IoError::PayloadTooLarge(format!(
                        "GLB skeleton exceeds safety limit of {MAX_IMPORT_RIG_BONES} bones"
                    )));
                }
                joint_node_indices.push(joint.index());
            }
        }
    }
    if joint_node_indices.is_empty() {
        return Err(IoError::InvalidFormat(
            "GLB file contains no skeleton skin".into(),
        ));
    }

    let mut gltf_parent: BTreeMap<usize, usize> = BTreeMap::new();
    let mut nodes_by_idx: BTreeMap<usize, gltf::Node<'_>> = BTreeMap::new();
    for node in gltf.nodes() {
        for child in node.children() {
            gltf_parent.entry(child.index()).or_insert(node.index());
        }
        nodes_by_idx.insert(node.index(), node);
    }

    let mut gltf_node_to_bone: BTreeMap<usize, BoneId> = BTreeMap::new();
    for (idx, &node_idx) in joint_node_indices.iter().enumerate() {
        let bone_id = BoneId(
            u32::try_from(idx)
                .map_err(|_| IoError::PayloadTooLarge("GLB bone index overflow".into()))?,
        );
        gltf_node_to_bone.insert(node_idx, bone_id);
    }

    let mut bones = Vec::with_capacity(joint_node_indices.len());
    let mut rest_by_bone: BTreeMap<BoneId, Transform> = BTreeMap::new();
    // Track folded non-joint intermediate ancestor matrices between a joint and its nearest
    // joint parent (or scene root) so both rest pose AND animation translations align.
    let mut ancestor_by_bone: BTreeMap<BoneId, Mat4> = BTreeMap::new();
    for (idx, &node_idx) in joint_node_indices.iter().enumerate() {
        let Some(node) = nodes_by_idx.get(&node_idx) else {
            return Err(IoError::InvalidFormat(format!(
                "GLB skin references missing node index {node_idx}"
            )));
        };
        let bone_id = gltf_node_to_bone[&node_idx];

        let (t, r, s) = node.transform().decomposed();
        let translation = Vec3::from_array(t);
        let raw_rot = Quat::from_array(r);
        let scale = Vec3::from_array(s);
        if !translation.is_finite()
            || !raw_rot.is_finite()
            || raw_rot.length_squared() <= 1e-8
            || !scale.is_finite()
            || scale.x <= 1e-6
            || scale.y <= 1e-6
            || scale.z <= 1e-6
        {
            return Err(IoError::InvalidFormat(format!(
                "GLB joint node {node_idx} has non-finite or non-positive scale transform"
            )));
        }
        let local_tr = Transform {
            translation,
            rotation: raw_rot.normalize(),
            scale,
        };

        // Walk up `gltf_parent` to find the nearest joint ancestor, collecting any intermediate
        // non-joint wrapper nodes (e.g. Armature / offset empties) strictly between them.
        let mut ancestor_chain = Vec::new();
        let mut parent: Option<BoneId> = None;
        let mut cur = gltf_parent.get(&node_idx).copied();
        let mut hops = 0usize;
        while let Some(pid) = cur {
            if hops >= nodes_by_idx.len() {
                return Err(IoError::InvalidFormat(
                    "GLB node hierarchy contains a cycle".into(),
                ));
            }
            if let Some(&parent_bone_id) = gltf_node_to_bone.get(&pid) {
                parent = Some(parent_bone_id);
                break;
            }
            if let Some(pnode) = nodes_by_idx.get(&pid) {
                ancestor_chain.push(Mat4::from_cols_array_2d(&pnode.transform().matrix()));
            }
            cur = gltf_parent.get(&pid).copied();
            hops += 1;
        }

        let mut anc_mat = Mat4::IDENTITY;
        for m in ancestor_chain.into_iter().rev() {
            anc_mat *= m;
        }
        let rest_local = if anc_mat != Mat4::IDENTITY {
            let local_mat = Mat4::from_scale_rotation_translation(
                local_tr.scale,
                local_tr.rotation,
                local_tr.translation,
            );
            let combined = anc_mat * local_mat;
            let Some(composed) = decompose_positive_trs(combined) else {
                return Err(IoError::InvalidFormat(format!(
                    "GLB joint {node_idx} has degenerate or singular ancestor transform"
                )));
            };
            ancestor_by_bone.insert(bone_id, anc_mat);
            composed
        } else {
            local_tr
        };
        let name = sanitize_glb_bone_name(node.name(), idx);
        // Keep raw glTF node local rest transform for converting animation channels cleanly.
        rest_by_bone.insert(bone_id, local_tr);
        bones.push(Bone::new(bone_id, name, parent, rest_local));
    }

    let mut animations: Vec<AnimationClip> = Vec::new();
    for (clip_idx, anim) in gltf.animations().enumerate() {
        if animations.len() >= MAX_IMPORT_RIG_CLIPS {
            break;
        }
        let mut per_bone: BTreeMap<BoneId, GlbBoneTracks> = BTreeMap::new();
        let mut max_time = 0.0f32;

        for channel in anim.channels() {
            let target = channel.target();
            let Some(&bone_id) = gltf_node_to_bone.get(&target.node().index()) else {
                continue;
            };
            let sampler = channel.sampler();
            let is_cubic = matches!(
                sampler.interpolation(),
                gltf::animation::Interpolation::CubicSpline
            );
            let stride = if is_cubic { 3 } else { 1 };
            let offset = if is_cubic { 1 } else { 0 };
            let input_count = sampler.input().count();
            let output_count = sampler.output().count();
            if input_count > MAX_IMPORT_KEYFRAMES_PER_CHANNEL
                || output_count > MAX_IMPORT_KEYFRAMES_PER_CHANNEL.saturating_mul(stride)
                || input_count > MAX_IMPORT_ACCESSOR_ITEMS
                || output_count > MAX_IMPORT_ACCESSOR_ITEMS
            {
                return Err(IoError::PayloadTooLarge(format!(
                    "GLB animation channel exceeds limit of {MAX_IMPORT_KEYFRAMES_PER_CHANNEL} keyframes"
                )));
            }
            let interp = match sampler.interpolation() {
                gltf::animation::Interpolation::Step => KeyframeInterpolation::Step,
                gltf::animation::Interpolation::Linear => KeyframeInterpolation::Linear,
                gltf::animation::Interpolation::CubicSpline => KeyframeInterpolation::EaseInOut,
            };
            let reader = channel.reader(|_| blob);
            let Some(inputs_iter) = reader.read_inputs() else {
                continue;
            };
            let times: Vec<f32> = inputs_iter.collect();
            let Some(outputs) = reader.read_outputs() else {
                continue;
            };

            let tracks = per_bone.entry(bone_id).or_default();
            let required_outputs = times.len().saturating_mul(stride);

            match outputs {
                gltf::animation::util::ReadOutputs::Translations(iter) => {
                    let vals: Vec<[f32; 3]> = iter.collect();
                    if vals.len() < required_outputs {
                        return Err(IoError::InvalidFormat(
                            "GLB translation animation output length mismatch".into(),
                        ));
                    }
                    let mut track = Vec::with_capacity(times.len());
                    for (i, &t) in times.iter().enumerate() {
                        if !t.is_finite() || !(0.0..=600.0).contains(&t) {
                            continue;
                        }
                        let Some(&raw) = vals.get(i * stride + offset) else {
                            break;
                        };
                        let v = Vec3::from_array(raw);
                        if !v.is_finite() {
                            continue;
                        }
                        max_time = max_time.max(t);
                        track.push((t, v, interp));
                    }
                    normalize_track(&mut track);
                    if !track.is_empty() {
                        tracks.translations = track;
                    }
                }
                gltf::animation::util::ReadOutputs::Rotations(iter) => {
                    let vals: Vec<[f32; 4]> = iter.into_f32().collect();
                    if vals.len() < required_outputs {
                        return Err(IoError::InvalidFormat(
                            "GLB rotation animation output length mismatch".into(),
                        ));
                    }
                    let mut track = Vec::with_capacity(times.len());
                    for (i, &t) in times.iter().enumerate() {
                        if !t.is_finite() || !(0.0..=600.0).contains(&t) {
                            continue;
                        }
                        let Some(&raw) = vals.get(i * stride + offset) else {
                            break;
                        };
                        let q = Quat::from_array(raw);
                        if !q.is_finite() || q.length_squared() <= 1e-8 {
                            continue;
                        }
                        max_time = max_time.max(t);
                        track.push((t, q.normalize(), interp));
                    }
                    normalize_track(&mut track);
                    if !track.is_empty() {
                        tracks.rotations = track;
                    }
                }
                gltf::animation::util::ReadOutputs::Scales(iter) => {
                    let vals: Vec<[f32; 3]> = iter.collect();
                    if vals.len() < required_outputs {
                        return Err(IoError::InvalidFormat(
                            "GLB scale animation output length mismatch".into(),
                        ));
                    }
                    let mut track = Vec::with_capacity(times.len());
                    for (i, &t) in times.iter().enumerate() {
                        if !t.is_finite() || !(0.0..=600.0).contains(&t) {
                            continue;
                        }
                        let Some(&raw) = vals.get(i * stride + offset) else {
                            break;
                        };
                        let s = Vec3::from_array(raw);
                        if !s.is_finite() || s.x <= 1e-6 || s.y <= 1e-6 || s.z <= 1e-6 {
                            continue;
                        }
                        max_time = max_time.max(t);
                        track.push((t, s, interp));
                    }
                    normalize_track(&mut track);
                    if !track.is_empty() {
                        tracks.scales = track;
                    }
                }
                gltf::animation::util::ReadOutputs::MorphTargetWeights(_) => {}
            }
        }

        let mut channels = Vec::new();
        for (bone_id, tracks) in per_bone {
            let Some(&rest) = rest_by_bone.get(&bone_id) else {
                continue;
            };
            let mut union_times: BTreeMap<i64, (f32, KeyframeInterpolation)> = BTreeMap::new();
            for &(t, _, interp) in &tracks.translations {
                union_times.entry(time_bucket(t)).or_insert((t, interp));
            }
            for &(t, _, interp) in &tracks.rotations {
                union_times.entry(time_bucket(t)).or_insert((t, interp));
            }
            for &(t, _, interp) in &tracks.scales {
                union_times.entry(time_bucket(t)).or_insert((t, interp));
            }
            if union_times.len() > MAX_IMPORT_KEYFRAMES_PER_CHANNEL {
                return Err(IoError::PayloadTooLarge(format!(
                    "GLB merged animation channel exceeds limit of {MAX_IMPORT_KEYFRAMES_PER_CHANNEL} keyframes"
                )));
            }

            let rest_rot_inv = rest.rotation.inverse();
            let anc_mat = ancestor_by_bone.get(&bone_id).copied();
            let safe_div = |num: f32, den: f32| -> f32 {
                if den > 1e-6 {
                    num / den
                } else {
                    1.0
                }
            };
            let keyframes: Vec<Keyframe> = union_times
                .into_values()
                .map(|(t, interp)| {
                    let raw_delta_t = sample_vec3_track(&tracks.translations, t)
                        .map(|abs_t| abs_t - rest.translation)
                        .unwrap_or(Vec3::ZERO);
                    let translation = match anc_mat {
                        Some(m) => m.transform_vector3(raw_delta_t),
                        None => raw_delta_t,
                    };
                    let rotation = sample_quat_track(&tracks.rotations, t)
                        .map(|abs_r| (rest_rot_inv * abs_r).normalize())
                        .unwrap_or(Quat::IDENTITY);
                    let scale = sample_vec3_track(&tracks.scales, t)
                        .map(|abs_s| {
                            Vec3::new(
                                safe_div(abs_s.x, rest.scale.x),
                                safe_div(abs_s.y, rest.scale.y),
                                safe_div(abs_s.z, rest.scale.z),
                            )
                        })
                        .unwrap_or(Vec3::ONE);
                    Keyframe {
                        time_secs: t,
                        translation,
                        rotation,
                        scale,
                        interpolation: interp,
                    }
                })
                .collect();
            if !keyframes.is_empty() {
                channels.push(BoneChannel { bone_id, keyframes });
            }
        }

        if !channels.is_empty() {
            let clip_name = anim
                .name()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.chars().take(MAX_NODE_NAME_LEN).collect::<String>())
                .unwrap_or_else(|| format!("Clip_{clip_idx}"));
            let duration_secs = if max_time > 0.0 && max_time.is_finite() {
                max_time.clamp(0.1, 600.0)
            } else {
                1.0
            };
            let mut clip = AnimationClip::new_empty(clip_name, duration_secs);
            clip.channels = channels;
            animations.push(clip);
        }
    }

    let mut rig = CharacterRig {
        bones,
        animations,
        ..CharacterRig::default()
    };
    rig.validate_and_sanitize().map_err(|_| {
        IoError::InvalidFormat("GLB skeleton hierarchy or animation data is invalid".into())
    })?;
    Ok(rig)
}

/// Imports a `.obj` or `.glb` 3D mesh from disk, returning `(asset_name, mesh)`.
pub fn import_mesh_from_path(
    path: &Path,
    material: PaletteIndex,
) -> Result<(String, LowPolyMesh), IoError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("Imported Mesh")
        .to_string();

    let meta = fs::metadata(path)?;
    if meta.len() > 64 * 1024 * 1024 {
        return Err(IoError::InvalidFormat(
            "mesh file exceeds 64 MiB safety limit".into(),
        ));
    }
    let mesh = match ext.as_str() {
        "obj" => {
            let text = fs::read_to_string(path)?;
            import_obj_str(&text, material)?
        }
        "glb" => {
            let bytes = fs::read(path)?;
            import_glb_bytes(&bytes, material)?
        }
        other => {
            return Err(IoError::InvalidFormat(format!(
                "unsupported mesh format '.{other}' (expected .obj or .glb)"
            )));
        }
    };
    Ok((name, mesh))
}

/// Loads a custom `.png` or `.jpg`/`.jpeg` texture image (e.g. from Blender, Poly Haven, AmbientCG)
/// into a `MeshTexture`, downscaling safely if larger than 1024x1024.
const MAX_IMPORT_IMAGE_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_IMPORT_IMAGE_DIM: u32 = 4096;
const MAX_IMPORT_IMAGE_ALLOC_BYTES: u64 = 64 * 1024 * 1024;

pub fn import_mesh_texture_from_path(path: &Path) -> Result<MeshTexture, IoError> {
    let meta = fs::metadata(path)?;
    if meta.len() > MAX_IMPORT_IMAGE_FILE_BYTES {
        return Err(IoError::ExportPathDenied(
            "image file exceeds 32 MiB import limit".into(),
        ));
    }
    let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMPORT_IMAGE_DIM);
    limits.max_image_height = Some(MAX_IMPORT_IMAGE_DIM);
    limits.max_alloc = Some(MAX_IMPORT_IMAGE_ALLOC_BYTES);
    reader.limits(limits);
    let mut img = reader.decode()?;
    if img.width() > 1024 || img.height() > 1024 {
        img = img.thumbnail(1024, 1024);
    }
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    if width == 0 || height == 0 {
        return Err(IoError::InvalidDimensions(width, height));
    }
    Ok(MeshTexture {
        width,
        height,
        rgba: rgba.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gltf::export_glb_bytes;
    use tessel_core::{Command, PaletteIndex, PrimitiveKind};

    #[test]
    fn obj_and_stl_export_valid_triangle_counts() {
        let mut scene = SceneState::new();
        Command::CreatePrimitiveMesh {
            name: "Cube".into(),
            kind: PrimitiveKind::Cube,
            size: 1.0,
            material: PaletteIndex::new(4).unwrap(),
        }
        .apply(&mut scene)
        .unwrap();

        let (obj, obj_tris) = export_obj_string(&scene, ExportPreset::Godot4).unwrap();
        assert_eq!(obj_tris, 12);
        assert!(obj.contains("v "));
        assert!(obj.contains("f "));

        let (stl, stl_tris) = export_stl_bytes(&scene, ExportPreset::Print3D).unwrap();
        assert_eq!(stl_tris, 12);
        assert_eq!(stl.len(), 84 + 12 * 50);
    }

    #[test]
    fn obj_and_glb_export_import_roundtrip() {
        let mut scene = SceneState::new();
        let mat = PaletteIndex::new(4).unwrap();
        Command::CreatePrimitiveMesh {
            name: "Cube".into(),
            kind: PrimitiveKind::Cube,
            size: 1.0,
            material: mat,
        }
        .apply(&mut scene)
        .unwrap();

        let (obj, _) = export_obj_string(&scene, ExportPreset::Godot4).unwrap();
        let imported_obj = import_obj_str(&obj, mat).unwrap();
        assert_eq!(imported_obj.faces.len(), 12);

        let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
        let imported_glb = import_glb_bytes(&glb, mat).unwrap();
        assert_eq!(imported_glb.faces.len(), 12);
    }

    #[test]
    fn glb_roundtrip_preserves_weights_and_morphs() {
        let mut scene = SceneState::new();
        let mat = PaletteIndex::new(4).unwrap();
        Command::CreatePrimitiveMesh {
            name: "Cube".into(),
            kind: PrimitiveKind::Cube,
            size: 1.0,
            material: mat,
        }
        .apply(&mut scene)
        .unwrap();
        let node_id = scene.active_lowpoly_node().unwrap();
        Command::SetMeshVertexWeights {
            node_id,
            vertex_ids: vec![0, 1],
            joints: [2, 0, 0, 0],
            weights: [1.0, 0.0, 0.0, 0.0],
        }
        .apply(&mut scene)
        .unwrap();
        Command::AddMorphDelta {
            node_id,
            name: "Smile".into(),
            vertex_ids: vec![0],
            delta: glam::Vec3::new(0.0, 0.5, 0.0),
        }
        .apply(&mut scene)
        .unwrap();
        Command::ApplyBipedRig.apply(&mut scene).unwrap();

        let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
        let imported = import_glb_bytes(&glb, mat).unwrap();
        // Every exported corner round-trips (manual where painted, auto
        // solver values elsewhere); painted corners keep exact values.
        let mesh = scene.lowpoly_meshes.get(&node_id).unwrap();
        let tri = mesh.to_triangulated_flat();
        assert_eq!(imported.skin_weights.len(), tri.vertices.len());
        let painted = imported
            .skin_weights
            .values()
            .filter(|w| w.joints == [2, 0, 0, 0] && w.weights == [1.0, 0.0, 0.0, 0.0])
            .count();
        let corners01: usize = tri
            .source_indices
            .iter()
            .filter(|s| **s == 0 || **s == 1)
            .count();
        assert_eq!(painted, corners01);
        for w in imported.skin_weights.values() {
            assert!((w.weights.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        }
        // Morph deltas survive the round trip (renamed Morph0 on import).
        assert_eq!(imported.morphs.len(), 1);
        let corners0: usize = tri.source_indices.iter().filter(|s| **s == 0).count();
        assert_eq!(imported.morphs[0].deltas.len(), corners0);
        for d in imported.morphs[0].deltas.values() {
            assert!((d.y - 0.5).abs() < 1e-5);
        }
    }

    #[test]
    fn custom_png_texture_import_loads_rgba_dimensions() {
        let dir = std::env::temp_dir().join("tessel_custom_tex_test");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("bark.png");
        let img = image::RgbaImage::from_pixel(32, 16, image::Rgba([120, 80, 45, 255]));
        img.save(&path).unwrap();

        let tex = import_mesh_texture_from_path(&path).unwrap();
        assert_eq!(tex.width, 32);
        assert_eq!(tex.height, 16);
        assert_eq!(tex.rgba.len(), 32 * 16 * 4);
        assert_eq!(&tex.rgba[0..4], &[120, 80, 45, 255]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_mesh_texture_supports_tga_tiff_and_exr() {
        let dir = std::env::temp_dir().join("tessel_v47_tex_interop_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let rgba_img = image::RgbaImage::from_pixel(16, 12, image::Rgba([90, 140, 210, 255]));
        for ext in ["tga", "tiff"] {
            let p = dir.join(format!("albedo.{ext}"));
            rgba_img.save(&p).unwrap();
            let tex = import_mesh_texture_from_path(&p).unwrap();
            assert_eq!(tex.width, 16);
            assert_eq!(tex.height, 12);
            assert_eq!(&tex.rgba[0..4], &[90, 140, 210, 255]);
        }

        // HDR 32-bit float EXR round-trip (converts cleanly to RGBA8).
        let exr_path = dir.join("hdr_bake.exr");
        let exr_img =
            image::Rgba32FImage::from_pixel(8, 8, image::Rgba([0.5_f32, 0.25_f32, 0.75_f32, 1.0]));
        exr_img.save(&exr_path).unwrap();
        let exr_tex = import_mesh_texture_from_path(&exr_path).unwrap();
        assert_eq!(exr_tex.width, 8);
        assert_eq!(exr_tex.height, 8);
        assert_eq!(exr_tex.rgba.len(), 8 * 8 * 4);
        assert_eq!(exr_tex.rgba[3], 255);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn g3_glb_rig_roundtrip_preserves_skeleton_and_animations() {
        let mut scene = SceneState::new();
        let mat = PaletteIndex::new(4).unwrap();
        Command::CreatePrimitiveMesh {
            name: "Hero".into(),
            kind: PrimitiveKind::Cube,
            size: 1.0,
            material: mat,
        }
        .apply(&mut scene)
        .unwrap();

        // GLB without skin must fail cleanly
        let (no_rig_glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
        assert!(matches!(
            import_glb_rig_bytes(&no_rig_glb),
            Err(IoError::InvalidFormat(_))
        ));

        // Apply humanoid 19-bone rig + starter Idle/Walk clips and export GLB
        Command::ApplyHumanoidRig.apply(&mut scene).unwrap();
        let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
        let imported = import_glb_rig_bytes(&glb).unwrap();

        let orig = scene.character_rig.as_ref().unwrap();
        assert_eq!(imported.bones.len(), orig.bones.len());
        assert_eq!(imported.animations.len(), orig.animations.len());
        for (a, b) in orig.bones.iter().zip(imported.bones.iter()) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.parent, b.parent);
            assert!((a.rest_local.translation - b.rest_local.translation).length() < 1e-4);
        }
        assert_eq!(imported.animations[0].name, "Idle");
        assert_eq!(imported.animations[1].name, "Walk");
        assert!(!imported.animations[1].channels.is_empty());
    }

    fn build_minimal_glb(json_val: serde_json::Value, bin: &[u8]) -> Vec<u8> {
        let mut json_bytes = serde_json::to_vec(&json_val).unwrap();
        while !json_bytes.len().is_multiple_of(4) {
            json_bytes.push(b' ');
        }
        let mut bin_padded = bin.to_vec();
        while !bin_padded.len().is_multiple_of(4) {
            bin_padded.push(0);
        }
        let has_bin = !bin_padded.is_empty();
        let total_len = 12 + 8 + json_bytes.len() + if has_bin { 8 + bin_padded.len() } else { 0 };
        let mut out = Vec::with_capacity(total_len);
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(total_len as u32).to_le_bytes());
        out.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(b"JSON");
        out.extend_from_slice(&json_bytes);
        if has_bin {
            out.extend_from_slice(&(bin_padded.len() as u32).to_le_bytes());
            out.extend_from_slice(b"BIN\0");
            out.extend_from_slice(&bin_padded);
        }
        out
    }

    #[test]
    fn g3_glb_rig_import_handles_intermediate_wrapper_async_channels_and_singular_matrix() {
        // 1. Intermediate non-joint wrapper between JointA (0) and JointB (2) + async T/R channels.
        let mut bin = Vec::new();
        let push_floats = |buf: &mut Vec<u8>, vals: &[f32]| -> (usize, usize) {
            let offset = buf.len();
            for &v in vals {
                buf.extend_from_slice(&v.to_le_bytes());
            }
            (offset, buf.len() - offset)
        };
        // Translation times [0.0, 1.0], values [0,0,0] -> [0,2,0]
        let (t_in_off, t_in_len) = push_floats(&mut bin, &[0.0, 1.0]);
        let (t_out_off, t_out_len) = push_floats(&mut bin, &[0.0, 0.0, 0.0, 0.0, 2.0, 0.0]);
        // Rotation times [0.0, 0.5, 1.0], values identity -> 90deg Y -> 180deg Y
        let q0 = Quat::IDENTITY.to_array();
        let q1 = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2).to_array();
        let q2 = Quat::from_rotation_y(std::f32::consts::PI).to_array();
        let (r_in_off, r_in_len) = push_floats(&mut bin, &[0.0, 0.5, 1.0]);
        let mut r_vals = Vec::new();
        r_vals.extend_from_slice(&q0);
        r_vals.extend_from_slice(&q1);
        r_vals.extend_from_slice(&q2);
        let (r_out_off, r_out_len) = push_floats(&mut bin, &r_vals);

        let gltf_json = serde_json::json!({
            "asset": { "version": "2.0" },
            "buffers": [{ "byteLength": bin.len() }],
            "bufferViews": [
                { "buffer": 0, "byteOffset": t_in_off, "byteLength": t_in_len },
                { "buffer": 0, "byteOffset": t_out_off, "byteLength": t_out_len },
                { "buffer": 0, "byteOffset": r_in_off, "byteLength": r_in_len },
                { "buffer": 0, "byteOffset": r_out_off, "byteLength": r_out_len }
            ],
            "accessors": [
                { "bufferView": 0, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0.0], "max": [1.0] },
                { "bufferView": 1, "componentType": 5126, "count": 2, "type": "VEC3" },
                { "bufferView": 2, "componentType": 5126, "count": 3, "type": "SCALAR", "min": [0.0], "max": [1.0] },
                { "bufferView": 3, "componentType": 5126, "count": 3, "type": "VEC4" }
            ],
            "nodes": [
                { "name": "RootJoint", "children": [1] },
                { "name": "NonJointWrapper", "translation": [0.0, 1.0, 0.0], "children": [2] },
                { "name": "ChildJoint", "translation": [0.0, 0.5, 0.0] }
            ],
            "skins": [{ "joints": [0, 2] }],
            "animations": [{
                "name": "AsyncClip",
                "samplers": [
                    { "input": 0, "output": 1, "interpolation": "LINEAR" },
                    { "input": 2, "output": 3, "interpolation": "LINEAR" }
                ],
                "channels": [
                    { "sampler": 0, "target": { "node": 0, "path": "translation" } },
                    { "sampler": 1, "target": { "node": 0, "path": "rotation" } }
                ]
            }]
        });
        let glb = build_minimal_glb(gltf_json, &bin);
        let rig = import_glb_rig_bytes(&glb).expect("valid wrapper + async GLB rig");
        assert_eq!(rig.bones.len(), 2);
        let root = rig.bones.iter().find(|b| b.name == "RootJoint").unwrap();
        let child = rig.bones.iter().find(|b| b.name == "ChildJoint").unwrap();
        assert_eq!(child.parent, Some(root.id));
        assert!((child.rest_local.translation - Vec3::new(0.0, 1.5, 0.0)).length() < 1e-4);

        let clip = &rig.animations[0];
        let ch = clip.channels.iter().find(|c| c.bone_id == root.id).unwrap();
        assert_eq!(ch.keyframes.len(), 3);
        let mid = ch
            .keyframes
            .iter()
            .find(|k| (k.time_secs - 0.5).abs() < 1e-3)
            .unwrap();
        // At t=0.5, translation must be interpolated to [0, 1, 0], NOT snapped to rest [0, 0, 0].
        assert!(
            (mid.translation - Vec3::new(0.0, 1.0, 0.0)).length() < 1e-4,
            "async T/R channel must interpolate translation at rotation-only keyframe, got {:?}",
            mid.translation
        );

        // 2. Singular (zero-scale) ancestor matrix must fail cleanly without panicking.
        let singular_json = serde_json::json!({
            "asset": { "version": "2.0" },
            "nodes": [
                { "name": "BadWrapper", "scale": [0.0, 1.0, 1.0], "children": [1] },
                { "name": "RootJoint" }
            ],
            "skins": [{ "joints": [1] }]
        });
        let bad_glb = build_minimal_glb(singular_json, &[]);
        assert!(matches!(
            import_glb_rig_bytes(&bad_glb),
            Err(IoError::InvalidFormat(_))
        ));
    }
}
