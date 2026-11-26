use glam::Vec3;
use tessel_core::voxel::greedy_mesh_volume;
use tessel_core::{AutoSkinSolver, SceneState};

use crate::error::IoError;
use crate::ExportPreset;

use super::helpers::{
    collect_node_transform_animated, collect_visibility_animated_nodes, is_custom_export_mesh,
    node_world_matrices,
};

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

        let vis_animated = collect_visibility_animated_nodes(scene);
        let node_trs_animated = collect_node_transform_animated(scene);

        for (node_id, mesh) in &scene.lowpoly_meshes {
            let visible = scene.nodes.get(node_id).map(|n| n.visible).unwrap_or(true);
            let is_custom =
                is_custom_export_mesh(scene, *node_id, mesh, &vis_animated, &node_trs_animated);
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
        let vis_animated = collect_visibility_animated_nodes(scene);
        let node_trs_animated = collect_node_transform_animated(scene);
        for (node_id, mesh) in &scene.lowpoly_meshes {
            let visible = scene.nodes.get(node_id).map(|n| n.visible).unwrap_or(true);
            let is_custom =
                is_custom_export_mesh(scene, *node_id, mesh, &vis_animated, &node_trs_animated);
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
