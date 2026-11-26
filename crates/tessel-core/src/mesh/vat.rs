use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::character::{AutoSkinSolver, CharacterRig};
use crate::error::CoreError;
use crate::scene::{NodeId, SceneState};

/// Parameters for baking a Vertex Animation Texture (VAT) from a mesh node & animation clip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VatBakeParams {
    pub node_id: NodeId,
    #[serde(default)]
    pub clip_index: usize,
    #[serde(default = "default_vat_fps")]
    pub fps: f32,
    #[serde(default = "default_vat_max_frames")]
    pub max_frames: usize,
}

fn default_vat_fps() -> f32 {
    15.0
}

fn default_vat_max_frames() -> usize {
    120
}

/// Exportable VAT metadata describing the encoded bounding box and texture dimensions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VatMetadata {
    pub node_name: String,
    pub vertex_count: usize,
    pub frame_count: usize,
    pub fps: f32,
    pub duration_secs: f32,
    pub pos_min: [f32; 3],
    pub pos_max: [f32; 3],
    pub width: u32,
    pub height: u32,
}

/// Deterministic baked Vertex Animation Texture (RGBA8 position + RGBA8 normal + metadata).
/// Layout: `width = vertex_count`, `height = frame_count`, row `f` stores frame `f`, column `v` stores vertex `v`.
#[derive(Debug, Clone, PartialEq)]
pub struct VatBakeResult {
    pub metadata: VatMetadata,
    pub position_rgba: Vec<u8>,
    pub normal_rgba: Vec<u8>,
}

/// Hard upper bound on total VAT pixels (`vertex_count * frame_count`, ~16 MB per RGBA8 texture)
/// to prevent unbounded memory allocation on dense meshes.
pub const MAX_VAT_PIXELS: usize = 4_194_304;
/// Hard upper bound on VAT texture width (`vertex_count`) matching common GPU/PNG texture limits.
pub const MAX_VAT_VERTICES: usize = 16_384;

/// Deterministically bakes per-frame deformed positions & normals of `params.node_id` across
/// `params.clip_index` into a Vertex Animation Texture (VAT).
pub fn bake_vertex_animation_texture(
    scene: &SceneState,
    params: &VatBakeParams,
) -> Result<VatBakeResult, CoreError> {
    let mesh = scene
        .lowpoly_meshes
        .get(&params.node_id)
        .ok_or(CoreError::NodeNotFound(params.node_id))?;
    if mesh.vertices.is_empty() {
        return Err(CoreError::OutOfBounds);
    }
    let node_name = scene
        .nodes
        .get(&params.node_id)
        .map(|n| n.name.clone())
        .unwrap_or_else(|| format!("node_{}", params.node_id.0));

    let fps = if params.fps.is_finite() {
        params.fps.clamp(1.0, 60.0)
    } else {
        15.0
    };
    let max_frames = params.max_frames.clamp(2, 512);

    let duration = scene
        .character_rig
        .as_ref()
        .and_then(|r| r.animations.get(params.clip_index))
        .map(|c| c.duration_secs)
        .unwrap_or(1.0)
        .clamp(0.05, 600.0);

    let frame_count = ((duration * fps).round() as usize + 1).clamp(2, max_frames);
    let vertex_count = mesh.vertices.len();
    if vertex_count > MAX_VAT_VERTICES || vertex_count.saturating_mul(frame_count) > MAX_VAT_PIXELS
    {
        return Err(CoreError::OutOfBounds);
    }

    let mut frame_positions: Vec<Vec<Vec3>> = Vec::with_capacity(frame_count);
    let mut frame_normals: Vec<Vec<Vec3>> = Vec::with_capacity(frame_count);
    let mut pos_min = Vec3::splat(f32::INFINITY);
    let mut pos_max = Vec3::splat(f32::NEG_INFINITY);

    let mut eval_scene = scene.clone();
    let skin_solver = eval_scene
        .character_rig
        .as_ref()
        .and_then(|r| AutoSkinSolver::from_rig(r, 1.0));

    for frame in 0..frame_count {
        let t = if frame + 1 == frame_count {
            duration
        } else {
            (frame as f32 / (frame_count - 1) as f32) * duration
        };

        if let Some(rig) = eval_scene.character_rig.as_mut() {
            if params.clip_index < rig.animations.len() {
                rig.active_animation = params.clip_index;
                rig.playback_time = t;
                rig.apply_active_animation();
            }
        }

        let base_node_mat = eval_scene.node_world_matrix(params.node_id);
        let parent_override = eval_scene
            .character_rig
            .as_ref()
            .and_then(|r| r.node_parent_overrides.get(&params.node_id).copied());
        let node_mat = match (parent_override, eval_scene.character_rig.as_ref()) {
            (Some(ps_key), Some(rig)) => CharacterRig::compute_node_override_matrix(
                ps_key,
                base_node_mat,
                &rig.world_pose_matrices(),
                &rig.world_rest_matrices(),
            ),
            _ => base_node_mat,
        };
        let skip_skin = parent_override.is_some();
        let skin_mats = skin_solver
            .as_ref()
            .zip(eval_scene.character_rig.as_ref())
            .map(|(solver, rig)| solver.skin_matrices(rig));

        let mut verts = Vec::with_capacity(vertex_count);
        for (vi, &v) in mesh.vertices.iter().enumerate() {
            let mut lv = v;
            for morph in &mesh.morphs {
                let w = eval_scene
                    .character_rig
                    .as_ref()
                    .and_then(|r| {
                        r.morph_weights
                            .get(&(params.node_id, morph.name.clone()))
                            .copied()
                    })
                    .unwrap_or(morph.default_weight);
                if w.abs() > 1e-5 {
                    if let Some(&delta) = morph.deltas.get(&vi) {
                        lv += delta * w;
                    }
                }
            }
            let mut wp = node_mat.transform_point3(lv);
            if !skip_skin {
                if let (Some(solver), Some(mats)) = (&skin_solver, &skin_mats) {
                    let w = mesh
                        .skin_weights
                        .get(&vi)
                        .copied()
                        .unwrap_or_else(|| solver.weights_for_vertex(Some(params.node_id), wp));
                    let (sp, _) = solver.deform_point_normal(mats, w, wp, Vec3::Y);
                    wp = sp;
                }
            }
            pos_min = pos_min.min(wp);
            pos_max = pos_max.max(wp);
            verts.push(wp);
        }

        let normals = compute_deformed_normals(&verts, &mesh.faces);
        frame_positions.push(verts);
        frame_normals.push(normals);
    }

    if !pos_min.is_finite() || !pos_max.is_finite() {
        pos_min = Vec3::ZERO;
        pos_max = Vec3::ONE;
    }
    let extent = (pos_max - pos_min).max(Vec3::splat(1e-5));

    let pixel_count = vertex_count * frame_count;
    let mut position_rgba = vec![0u8; pixel_count * 4];
    let mut normal_rgba = vec![0u8; pixel_count * 4];

    for f in 0..frame_count {
        for v in 0..vertex_count {
            let idx = (f * vertex_count + v) * 4;
            let p = frame_positions[f][v];
            let np = ((p - pos_min) / extent).clamp(Vec3::ZERO, Vec3::ONE);
            position_rgba[idx] = (np.x * 255.0).round() as u8;
            position_rgba[idx + 1] = (np.y * 255.0).round() as u8;
            position_rgba[idx + 2] = (np.z * 255.0).round() as u8;
            position_rgba[idx + 3] = 255;

            let n = frame_normals[f][v].normalize_or_zero();
            let nn = (n * 0.5 + Vec3::splat(0.5)).clamp(Vec3::ZERO, Vec3::ONE);
            normal_rgba[idx] = (nn.x * 255.0).round() as u8;
            normal_rgba[idx + 1] = (nn.y * 255.0).round() as u8;
            normal_rgba[idx + 2] = (nn.z * 255.0).round() as u8;
            normal_rgba[idx + 3] = 255;
        }
    }

    Ok(VatBakeResult {
        metadata: VatMetadata {
            node_name,
            vertex_count,
            frame_count,
            fps,
            duration_secs: duration,
            pos_min: pos_min.to_array(),
            pos_max: pos_max.to_array(),
            width: vertex_count as u32,
            height: frame_count as u32,
        },
        position_rgba,
        normal_rgba,
    })
}

/// Decodes a world-space vertex position from a baked `VatBakeResult` at fractional frame `frame_f`
/// (linearly interpolating between adjacent frames; mirrors shader VAT sampling).
pub fn sample_vat_position(vat: &VatBakeResult, vertex_idx: usize, frame_f: f32) -> Option<Vec3> {
    let vc = vat.metadata.vertex_count;
    let fc = vat.metadata.frame_count;
    if vertex_idx >= vc || fc == 0 {
        return None;
    }
    let clamped = frame_f.clamp(0.0, (fc - 1) as f32);
    let f0 = clamped.floor() as usize;
    let f1 = (f0 + 1).min(fc - 1);
    let alpha = clamped - f0 as f32;

    let min = Vec3::from_array(vat.metadata.pos_min);
    let max = Vec3::from_array(vat.metadata.pos_max);
    let extent = max - min;

    let read = |f: usize| -> Vec3 {
        let idx = (f * vc + vertex_idx) * 4;
        Vec3::new(
            vat.position_rgba[idx] as f32 / 255.0,
            vat.position_rgba[idx + 1] as f32 / 255.0,
            vat.position_rgba[idx + 2] as f32 / 255.0,
        ) * extent
            + min
    };

    Some(read(f0).lerp(read(f1), alpha))
}

fn compute_deformed_normals(vertices: &[Vec3], faces: &[super::face::Face]) -> Vec<Vec3> {
    let mut acc = vec![Vec3::ZERO; vertices.len()];
    for face in faces {
        if face.vertices.len() < 3 {
            continue;
        }
        let Some(&i0) = face.vertices.first() else {
            continue;
        };
        let Some(&p0) = vertices.get(i0) else {
            continue;
        };
        for fan in 1..(face.vertices.len() - 1) {
            let (Some(&i1), Some(&i2)) = (face.vertices.get(fan), face.vertices.get(fan + 1))
            else {
                continue;
            };
            let (Some(&p1), Some(&p2)) = (vertices.get(i1), vertices.get(i2)) else {
                continue;
            };
            let cross = (p1 - p0).cross(p2 - p0);
            if cross.length_squared() > 1e-10 {
                acc[i0] += cross;
                acc[i1] += cross;
                acc[i2] += cross;
            }
        }
    }
    for n in &mut acc {
        *n = n.normalize_or_zero();
        if n.length_squared() < 1e-6 {
            *n = Vec3::Y;
        }
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Command;
    use crate::mesh::PrimitiveKind;
    use crate::palette::PaletteIndex;

    #[test]
    fn vat_bake_and_sample_roundtrip_matches_bounds() {
        let mut scene = SceneState::new();
        Command::CreatePrimitiveMesh {
            name: "Flag".to_string(),
            kind: PrimitiveKind::Cube,
            size: 1.0,
            material: PaletteIndex::new(1).unwrap(),
        }
        .apply(&mut scene)
        .unwrap();
        let node_id = scene.active_lowpoly_node().unwrap();

        let vat = bake_vertex_animation_texture(
            &scene,
            &VatBakeParams {
                node_id,
                clip_index: 0,
                fps: 10.0,
                max_frames: 16,
            },
        )
        .unwrap();

        assert_eq!(vat.metadata.vertex_count, 8);
        assert!(vat.metadata.frame_count >= 2);
        let sampled = sample_vat_position(&vat, 0, 0.0).unwrap();
        let orig = scene.lowpoly_meshes[&node_id].vertices[0];
        assert!((sampled - orig).length() < 0.02);
    }
}
