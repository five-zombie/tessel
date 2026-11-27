use glam::Vec3;
use tessel_core::voxel::greedy_mesh_volume;
use tessel_core::{AutoSkinSolver, SceneState};

use crate::error::IoError;
use crate::ExportPreset;

use super::helpers::{
    collect_node_transform_animated, collect_visibility_animated_nodes, is_custom_export_mesh,
    node_world_matrices,
};

/// Material grouping key for palette-driven (voxel / default low-poly) triangles in glTF 2.0 export (`H1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct PaletteMaterialGroup {
    pub is_default: bool,
    pub metallic_bits: u32,
    pub roughness_bits: u32,
    pub emissive_bits: u32,
    pub emissive_rgb_bits: [u32; 3],
    pub transmission_bits: u32,
    pub ior_bits: u32,
}

impl PaletteMaterialGroup {
    pub fn default_matte() -> Self {
        Self {
            is_default: true,
            metallic_bits: 0.0_f32.to_bits(),
            roughness_bits: 0.85_f32.to_bits(),
            emissive_bits: 0.0_f32.to_bits(),
            emissive_rgb_bits: [0.0_f32.to_bits(); 3],
            transmission_bits: 0.0_f32.to_bits(),
            ior_bits: 1.5_f32.to_bits(),
        }
    }

    pub fn from_palette_entry(entry: Option<&tessel_core::PaletteEntry>) -> Self {
        let Some(e) = entry else {
            return Self::default_matte();
        };
        let metallic = e.metallic.clamp(0.0, 1.0);
        let roughness = e.roughness.clamp(0.05, 1.0);
        let emissive = e.emissive.clamp(0.0, 20.0);
        let transmission = e.transmission.clamp(0.0, 1.0);
        let ior = e.ior.clamp(1.0, 2.5);
        let is_default = metallic == 0.0
            && (roughness - 0.85).abs() < 1e-4
            && emissive == 0.0
            && transmission == 0.0
            && (ior - 1.5).abs() < 1e-4;
        if is_default {
            return Self::default_matte();
        }
        let srgb_to_lin = |c: u8| -> f32 {
            let s = f32::from(c) / 255.0;
            if s <= 0.04045 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        let emissive_rgb = if emissive > 0.0 {
            [
                srgb_to_lin(e.color.r),
                srgb_to_lin(e.color.g),
                srgb_to_lin(e.color.b),
            ]
        } else {
            [0.0, 0.0, 0.0]
        };
        Self {
            is_default: false,
            metallic_bits: metallic.to_bits(),
            roughness_bits: roughness.to_bits(),
            emissive_bits: emissive.to_bits(),
            emissive_rgb_bits: [
                emissive_rgb[0].to_bits(),
                emissive_rgb[1].to_bits(),
                emissive_rgb[2].to_bits(),
            ],
            transmission_bits: transmission.to_bits(),
            ior_bits: ior.to_bits(),
        }
    }

    pub fn metallic(&self) -> f32 {
        f32::from_bits(self.metallic_bits)
    }

    pub fn roughness(&self) -> f32 {
        f32::from_bits(self.roughness_bits)
    }

    pub fn emissive(&self) -> f32 {
        f32::from_bits(self.emissive_bits)
    }

    pub fn emissive_rgb(&self) -> [f32; 3] {
        [
            f32::from_bits(self.emissive_rgb_bits[0]),
            f32::from_bits(self.emissive_rgb_bits[1]),
            f32::from_bits(self.emissive_rgb_bits[2]),
        ]
    }

    pub fn transmission(&self) -> f32 {
        f32::from_bits(self.transmission_bits)
    }

    pub fn ior(&self) -> f32 {
        f32::from_bits(self.ior_bits)
    }
}

/// Collects all visible 3D geometry (voxels + low-poly meshes) into flat indexed buffers
/// scaled by `preset.scale_factor()`.
pub(crate) struct CollectedMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub palette_indices: Vec<u8>,
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
}

impl CollectedMesh {
    pub fn empty() -> Self {
        Self {
            positions: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            palette_indices: Vec::new(),
            joints: Vec::new(),
            weights: Vec::new(),
            indices: Vec::new(),
        }
    }

    /// Partitions this mesh into deterministic per-material sub-meshes based on `PaletteEntry`
    /// PBR/emissive/transmission/IOR properties. Default matte entries stay merged in a single group.
    pub fn split_by_palette_material(
        &self,
        palette: &tessel_core::Palette,
    ) -> Vec<(PaletteMaterialGroup, Self)> {
        if self.positions.is_empty() || self.indices.is_empty() {
            return Vec::new();
        }
        let has_skin =
            self.joints.len() == self.positions.len() && self.weights.len() == self.positions.len();
        let mut groups: std::collections::BTreeMap<PaletteMaterialGroup, Self> =
            std::collections::BTreeMap::new();
        for tri in self.indices.chunks_exact(3) {
            let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
            if i0 >= self.positions.len()
                || i1 >= self.positions.len()
                || i2 >= self.positions.len()
            {
                continue;
            }
            let raw_pal = self.palette_indices.get(i0).copied().unwrap_or(0);
            let entry = tessel_core::PaletteIndex::new(raw_pal)
                .ok()
                .and_then(|idx| palette.get(idx));
            let key = PaletteMaterialGroup::from_palette_entry(entry);
            let dst = groups.entry(key).or_insert_with(Self::empty);
            let base = dst.positions.len() as u32;
            for &vi in &[i0, i1, i2] {
                dst.positions.push(self.positions[vi]);
                dst.normals.push(self.normals[vi]);
                dst.uvs.push(self.uvs[vi]);
                dst.palette_indices
                    .push(self.palette_indices.get(vi).copied().unwrap_or(0));
                if has_skin {
                    dst.joints.push(self.joints[vi]);
                    dst.weights.push(self.weights[vi]);
                }
            }
            dst.indices.extend_from_slice(&[base, base + 1, base + 2]);
        }
        // Keep default matte group first for deterministic material index 0 compatibility.
        let mut out: Vec<(PaletteMaterialGroup, Self)> = groups.into_iter().collect();
        out.sort_by_key(|(k, _)| (!k.is_default, *k));
        out
    }

    pub fn from_scene_untextured(
        scene: &SceneState,
        preset: ExportPreset,
        skin: Option<&AutoSkinSolver>,
    ) -> Self {
        Self::from_scene_untextured_filtered(scene, preset, skin, |_| true)
    }

    pub fn from_scene_untextured_filtered(
        scene: &SceneState,
        preset: ExportPreset,
        skin: Option<&AutoSkinSolver>,
        mut include_node: impl FnMut(tessel_core::NodeId) -> bool,
    ) -> Self {
        let scale = preset.scale_factor();
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uvs = Vec::new();
        let mut palette_indices = Vec::new();
        let mut joints = Vec::new();
        let mut weights = Vec::new();
        let mut indices = Vec::new();

        for (node_id, vol) in &scene.voxel_volumes {
            let visible = scene.nodes.get(node_id).map(|n| n.visible).unwrap_or(true);
            if !visible || vol.is_empty() || !include_node(*node_id) {
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
                palette_indices.push(v.palette_index.get());
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
            if !visible || mesh.faces.is_empty() || is_custom || !include_node(*node_id) {
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
                palette_indices.push(v.palette_index.get());
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
            palette_indices,
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
