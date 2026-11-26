use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::character::{BoneId, CharacterRig};
use crate::mesh::{FaceId, LowPolyMesh};
use crate::palette::{Palette, PaletteIndex};
use crate::pixel::{PixelCanvas, PixelTool, Symmetry2D};
use crate::voxel::{Symmetry3D, VoxelTool, VoxelVolume};

use super::nano::{AppMode, NodeId, NodeKind, Transform};

/// Sub-Unit: Single scene graph node metadata.
/// Viewport shading & NPR preview mode (shared by live GUI, headless screenshots, and turntables).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewportShadingMode {
    #[default]
    Pbr,
    NprSumie,
    SilhouetteCheck,
    LuminanceValueCheck,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneNode {
    pub id: NodeId,
    pub name: String,
    pub kind: NodeKind,
    pub visible: bool,
    #[serde(default)]
    pub parent: Option<NodeId>,
    pub transform: Transform,
}

/// Maximum character length allowed for a `SceneNode` name.
pub const MAX_NODE_NAME_LEN: usize = 128;

/// Trims and validates a `SceneNode` name against `1..=MAX_NODE_NAME_LEN` characters.
pub fn validate_node_name(raw: &str) -> Result<String, crate::error::CoreError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_NODE_NAME_LEN {
        return Err(crate::error::CoreError::OutOfBounds);
    }
    Ok(trimmed.to_string())
}

/// Root Anchor: Single source of truth for the entire Tessel workspace state.
///
/// # Mutation Discipline
/// External crates (`tessel-app`, `tessel-ai`, `tessel-io`) must mutate scene state exclusively
/// through [`crate::Command`] and [`crate::HistoryStack`] so every edit remains deterministic and
/// undoable. `pub` fields are exposed for read-only rendering, inspection, and serialization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneState {
    pub active_mode: AppMode,
    pub unit_scale: f32,
    #[serde(default = "default_light_azimuth")]
    pub light_azimuth: f32,
    #[serde(default = "default_light_elevation")]
    pub light_elevation: f32,
    #[serde(default)]
    pub shading_mode: ViewportShadingMode,
    pub palette: Palette,
    pub active_color: PaletteIndex,
    pub pixel_tool: PixelTool,
    pub pixel_symmetry: Symmetry2D,
    pub voxel_tool: VoxelTool,
    pub voxel_symmetry: Symmetry3D,
    #[serde(default)]
    pub lowpoly_paint_mode: bool,
    #[serde(default)]
    pub lowpoly_mirror_x: bool,
    #[serde(default = "default_lowpoly_brush_radius")]
    pub lowpoly_brush_radius: f32,
    #[serde(default)]
    pub weight_paint_mode: bool,
    #[serde(default = "default_weight_brush_radius")]
    pub weight_brush_radius: f32,
    #[serde(default = "default_weight_brush_strength")]
    pub weight_brush_strength: f32,
    #[serde(default)]
    pub stick_gizmo: crate::character::StickGizmoSettings,
    pub nodes: BTreeMap<NodeId, SceneNode>,
    pub pixel_canvases: BTreeMap<NodeId, PixelCanvas>,
    pub voxel_volumes: BTreeMap<NodeId, VoxelVolume>,
    pub lowpoly_meshes: BTreeMap<NodeId, LowPolyMesh>,
    pub character_rig: Option<CharacterRig>,
    pub selected_node: Option<NodeId>,
    pub selected_face: Option<FaceId>,
    pub selected_bone: Option<BoneId>,
    next_node_id: u64,
    pub revision: u64,
    pub dirty_viewport: bool,
}

fn default_light_azimuth() -> f32 {
    0.72
}

fn default_light_elevation() -> f32 {
    0.92
}

fn default_lowpoly_brush_radius() -> f32 {
    10.0
}

fn default_weight_brush_radius() -> f32 {
    0.35
}

fn default_weight_brush_strength() -> f32 {
    0.5
}

impl SceneState {
    pub fn new() -> Self {
        let active_color = PaletteIndex::DEFAULT_ACTIVE;
        Self {
            active_mode: AppMode::Voxel,
            unit_scale: 0.1,
            light_azimuth: default_light_azimuth(),
            light_elevation: default_light_elevation(),
            shading_mode: ViewportShadingMode::Pbr,
            palette: Palette::studio_default(),
            active_color,
            pixel_tool: PixelTool::Pencil,
            pixel_symmetry: Symmetry2D::default(),
            voxel_tool: VoxelTool::Place,
            voxel_symmetry: Symmetry3D::default(),
            lowpoly_paint_mode: false,
            lowpoly_mirror_x: false,
            lowpoly_brush_radius: default_lowpoly_brush_radius(),
            weight_paint_mode: false,
            weight_brush_radius: default_weight_brush_radius(),
            weight_brush_strength: default_weight_brush_strength(),
            stick_gizmo: crate::character::StickGizmoSettings::default(),
            nodes: BTreeMap::new(),
            pixel_canvases: BTreeMap::new(),
            voxel_volumes: BTreeMap::new(),
            lowpoly_meshes: BTreeMap::new(),
            character_rig: None,
            selected_node: None,
            selected_face: None,
            selected_bone: None,
            next_node_id: 1,
            revision: 0,
            dirty_viewport: true,
        }
    }

    pub fn allocate_node_id(&mut self) -> NodeId {
        let id = NodeId(self.next_node_id);
        self.next_node_id = self.next_node_id.saturating_add(1);
        id
    }

    /// Rewinds `next_node_id` when undoing the most recently allocated node so Redo reproduces the exact ID.
    pub fn rollback_node_id(&mut self, id: NodeId) {
        if self.next_node_id == id.0.saturating_add(1) {
            self.next_node_id = id.0;
        }
    }

    pub fn mark_modified(&mut self) {
        self.revision = self.revision.saturating_add(1);
        self.dirty_viewport = true;
    }

    pub fn active_pixel_node(&self) -> Option<NodeId> {
        if let Some(id) = self.selected_node {
            if self.pixel_canvases.contains_key(&id) {
                return Some(id);
            }
        }
        self.pixel_canvases.keys().next().copied()
    }

    pub fn active_voxel_node(&self) -> Option<NodeId> {
        if let Some(id) = self.selected_node {
            if self.voxel_volumes.contains_key(&id) {
                return Some(id);
            }
        }
        self.voxel_volumes.keys().next().copied()
    }

    pub fn active_lowpoly_node(&self) -> Option<NodeId> {
        if let Some(id) = self.selected_node {
            if self.lowpoly_meshes.contains_key(&id) {
                return Some(id);
            }
        }
        self.lowpoly_meshes.keys().next().copied()
    }

    /// Computes the live world-space transform matrix for `id` by walking its `parent` hierarchy
    /// and applying any active per-node transform animation overrides (`node_transform_overrides`).
    pub fn node_world_matrix(&self, id: NodeId) -> glam::Mat4 {
        self.compute_node_world_matrix(id, true)
    }

    /// Computes the rest-pose world-space transform matrix for `id` ignoring live animation overrides
    /// (used by exporters and rest-pose skinning).
    pub fn node_rest_world_matrix(&self, id: NodeId) -> glam::Mat4 {
        self.compute_node_world_matrix(id, false)
    }

    fn compute_node_world_matrix(&self, id: NodeId, apply_anim_overrides: bool) -> glam::Mat4 {
        let mut chain = Vec::new();
        let mut curr = Some(id);
        while let Some(nid) = curr {
            if chain.len() >= 16 || chain.contains(&nid) {
                break;
            }
            let Some(node) = self.nodes.get(&nid) else {
                break;
            };
            chain.push(nid);
            curr = node.parent;
        }
        let mut mat = glam::Mat4::IDENTITY;
        for nid in chain.into_iter().rev() {
            if let Some(node) = self.nodes.get(&nid) {
                let mut tr = node.transform;
                if apply_anim_overrides {
                    if let Some(delta) = self
                        .character_rig
                        .as_ref()
                        .and_then(|r| r.node_transform_overrides.get(&nid))
                    {
                        tr.translation += delta.translation;
                        tr.rotation = (tr.rotation * delta.rotation).normalize();
                        tr.scale *= delta.scale;
                    }
                }
                mat *= glam::Mat4::from_scale_rotation_translation(
                    tr.scale,
                    tr.rotation,
                    tr.translation,
                );
            }
        }
        mat
    }
}

impl Default for SceneState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_state_new_uses_default_active_palette_and_validates_node_names() {
        let state = SceneState::new();
        assert_eq!(state.active_color, PaletteIndex::DEFAULT_ACTIVE);
        assert_eq!(state.active_color.get(), 4);

        assert!(validate_node_name("").is_err());
        assert!(validate_node_name("   \t\n ").is_err());
        let max_name = "a".repeat(MAX_NODE_NAME_LEN);
        assert_eq!(
            validate_node_name(&format!("  {max_name}  ")).unwrap(),
            max_name
        );
        let too_long = "a".repeat(MAX_NODE_NAME_LEN + 1);
        assert!(validate_node_name(&too_long).is_err());
    }
}
