use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::character::{BoneId, CharacterRig};
use crate::mesh::{FaceId, LowPolyMesh};
use crate::palette::{Palette, PaletteIndex};
use crate::pixel::{PixelCanvas, PixelTool, Symmetry2D};
use crate::voxel::{Symmetry3D, VoxelTool, VoxelVolume};

use super::nano::{AppMode, NodeId, NodeKind, Transform};

/// Sub-Unit: Single scene graph node metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneNode {
    pub id: NodeId,
    pub name: String,
    pub kind: NodeKind,
    pub visible: bool,
    pub transform: Transform,
}

/// Root Anchor: Single source of truth for the entire Tessel workspace state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneState {
    pub active_mode: AppMode,
    pub unit_scale: f32,
    #[serde(default = "default_light_azimuth")]
    pub light_azimuth: f32,
    #[serde(default = "default_light_elevation")]
    pub light_elevation: f32,
    pub palette: Palette,
    pub active_color: PaletteIndex,
    pub pixel_tool: PixelTool,
    pub pixel_symmetry: Symmetry2D,
    pub voxel_tool: VoxelTool,
    pub voxel_symmetry: Symmetry3D,
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

impl SceneState {
    pub fn new() -> Self {
        let active_color = PaletteIndex::new(4).unwrap_or_else(|_| {
            PaletteIndex::new(1)
                .ok()
                .into_iter()
                .next()
                .unwrap_or_else(|| unreachable!())
        });
        Self {
            active_mode: AppMode::Voxel,
            unit_scale: 0.1,
            light_azimuth: default_light_azimuth(),
            light_elevation: default_light_elevation(),
            palette: Palette::studio_default(),
            active_color,
            pixel_tool: PixelTool::Pencil,
            pixel_symmetry: Symmetry2D::default(),
            voxel_tool: VoxelTool::Place,
            voxel_symmetry: Symmetry3D::default(),
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
}

impl Default for SceneState {
    fn default() -> Self {
        Self::new()
    }
}
