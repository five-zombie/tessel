use std::collections::BTreeSet;
use std::io::Cursor;

use glam::Mat4;
use image::{ImageFormat, RgbaImage};
use tessel_core::{LowPolyMesh, MeshMaterial, NodeId, Palette, PaletteIndex, SceneState};

use crate::error::IoError;

pub(super) fn is_cubic_interpolation(interp: tessel_core::KeyframeInterpolation) -> bool {
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
pub(super) fn easing_segment_slopes(interp: tessel_core::KeyframeInterpolation) -> (f32, f32) {
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

pub(super) fn align_quat_hemispheres(mut quats: Vec<[f32; 4]>) -> Vec<[f32; 4]> {
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

pub(super) fn pack_cubicspline<const N: usize>(
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
pub(super) fn expand_mixed_step_keyframes<T: Copy>(
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

pub(super) fn node_world_matrices(
    scene: &SceneState,
    node_id: tessel_core::NodeId,
) -> (Mat4, Mat4) {
    let mat = scene.node_rest_world_matrix(node_id);
    let normal_mat = if mat.determinant().abs() > 1e-8 {
        mat.inverse().transpose()
    } else {
        mat
    };
    (mat, normal_mat)
}

/// Returns `true` if any `NodeKind::Group` ancestor within 16 hops is hidden (`!visible`).
pub(super) fn has_hidden_group_ancestor(scene: &SceneState, node_id: tessel_core::NodeId) -> bool {
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
pub(super) fn has_group_ancestor(scene: &SceneState, node_id: tessel_core::NodeId) -> bool {
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

pub(super) fn collect_node_transform_animated(
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
pub(super) fn is_morphed_export_node(
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

/// Collects all `NodeId`s targeted by visibility or parent-switch animation tracks.
pub(super) fn collect_visibility_animated_nodes(scene: &SceneState) -> BTreeSet<NodeId> {
    scene
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
        .unwrap_or_default()
}

/// Returns `true` when a `LowPolyMesh` requires a dedicated glTF primitive/node
/// rather than being merged into the shared palette-indexed static mesh.
pub(super) fn is_custom_export_mesh(
    scene: &SceneState,
    node_id: NodeId,
    mesh: &LowPolyMesh,
    vis_animated: &BTreeSet<NodeId>,
    node_trs_animated: &BTreeSet<NodeId>,
) -> bool {
    mesh.texture.is_some()
        || mesh.normal_map.is_some()
        || mesh.ao_map.is_some()
        || !mesh.vertex_colors.is_empty()
        || mesh.material != MeshMaterial::default()
        || !mesh.morphs.is_empty()
        || vis_animated.contains(&node_id)
        || node_trs_animated.contains(&node_id)
        || has_group_ancestor(scene, node_id)
}

pub(super) fn encode_rgba_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, IoError> {
    let img = RgbaImage::from_raw(width, height, rgba.to_vec())
        .ok_or(IoError::InvalidDimensions(width, height))?;
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, ImageFormat::Png)?;
    Ok(buf.into_inner())
}

pub(super) fn build_palette_png(palette: &Palette) -> Result<Vec<u8>, IoError> {
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

pub(super) fn pad4(buf: &mut Vec<u8>, pad_byte: u8) {
    while !buf.len().is_multiple_of(4) {
        buf.push(pad_byte);
    }
}

/// Linear-interpolates morph weight keys at `time_secs` (clamped ends),
/// mirroring `AnimationClip::sample_morph_weights` for export baking.
pub(super) fn sample_morph_keys(keys: &[tessel_core::MorphWeightKey], time_secs: f32) -> f32 {
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
