use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::character::{BoneId, CharacterRig};
use crate::mesh::{FaceId, LowPolyMesh};
use crate::palette::{Palette, PaletteIndex};
use crate::pixel::{PixelCanvas, PixelTool, Symmetry2D};
use crate::voxel::{Symmetry3D, VoxelTool, VoxelVolume};

use super::nano::{AppMode, NodeId, NodeKind, Transform};

/// Look-Dev viewport environment lighting preset (`v5.9`, preview-only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LookDevPreset {
    #[default]
    Daylight,
    Night,
    Indoor,
    Sunset,
}

/// Saved look-dev camera bookmark (`v5.9`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraBookmark {
    pub name: String,
    pub target: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
}

/// 3D viewport reference image plane for 2D->3D authoring (`E7`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReferenceImagePlane {
    pub name: String,
    pub path: String,
    pub position: [f32; 3],
    pub size: [f32; 2],
    pub opacity: f32,
}

/// Extended NPR / look-dev viewport parameters (`E8` / `v5.9`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NprLookParams {
    pub hatching_strength: f32,
    pub bloom_strength: f32,
    pub msaa_samples: u32,
}

impl Default for NprLookParams {
    fn default() -> Self {
        Self {
            hatching_strength: 0.0,
            bloom_strength: 0.0,
            msaa_samples: 1,
        }
    }
}

fn default_pp_true() -> bool {
    true
}
fn default_bloom_threshold() -> f32 {
    1.0
}
fn default_bloom_intensity() -> f32 {
    0.35
}
fn default_exposure() -> f32 {
    1.0
}
fn default_fog_color() -> [f32; 3] {
    [0.08, 0.10, 0.14]
}
fn default_fog_density() -> f32 {
    0.02
}
fn default_fog_start() -> f32 {
    8.0
}
fn default_fog_height_falloff() -> f32 {
    0.25
}

/// Cinematic HDR post-processing & forward atmospheric fog settings (`H1`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PostProcessSettings {
    #[serde(default)]
    pub bloom_enabled: bool,
    #[serde(default = "default_bloom_threshold")]
    pub bloom_threshold: f32,
    #[serde(default = "default_bloom_intensity")]
    pub bloom_intensity: f32,
    #[serde(default = "default_exposure")]
    pub exposure: f32,
    #[serde(default = "default_pp_true")]
    pub aces_enabled: bool,
    #[serde(default)]
    pub fog_enabled: bool,
    #[serde(default = "default_fog_color")]
    pub fog_color: [f32; 3],
    #[serde(default = "default_fog_density")]
    pub fog_density: f32,
    #[serde(default = "default_fog_start")]
    pub fog_start: f32,
    #[serde(default = "default_fog_height_falloff")]
    pub fog_height_falloff: f32,
}

impl Default for PostProcessSettings {
    fn default() -> Self {
        Self {
            bloom_enabled: false,
            bloom_threshold: default_bloom_threshold(),
            bloom_intensity: default_bloom_intensity(),
            exposure: default_exposure(),
            aces_enabled: true,
            fog_enabled: false,
            fog_color: default_fog_color(),
            fog_density: default_fog_density(),
            fog_start: default_fog_start(),
            fog_height_falloff: default_fog_height_falloff(),
        }
    }
}

impl PostProcessSettings {
    /// Validates finite values and clamps all fields to safe rendering bounds.
    pub fn validated(self) -> Result<Self, crate::error::CoreError> {
        if !self.bloom_threshold.is_finite()
            || !self.bloom_intensity.is_finite()
            || !self.exposure.is_finite()
            || self.fog_color.iter().any(|c| !c.is_finite())
            || !self.fog_density.is_finite()
            || !self.fog_start.is_finite()
            || !self.fog_height_falloff.is_finite()
        {
            return Err(crate::error::CoreError::OutOfBounds);
        }
        Ok(Self {
            bloom_enabled: self.bloom_enabled,
            bloom_threshold: self.bloom_threshold.clamp(0.0, 10.0),
            bloom_intensity: self.bloom_intensity.clamp(0.0, 4.0),
            exposure: self.exposure.clamp(0.05, 8.0),
            aces_enabled: self.aces_enabled,
            fog_enabled: self.fog_enabled,
            fog_color: [
                self.fog_color[0].clamp(0.0, 1.0),
                self.fog_color[1].clamp(0.0, 1.0),
                self.fog_color[2].clamp(0.0, 1.0),
            ],
            fog_density: self.fog_density.clamp(0.0, 1.0),
            fog_start: self.fog_start.clamp(0.0, 500.0),
            fog_height_falloff: self.fog_height_falloff.clamp(0.0, 5.0),
        })
    }
}

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
    pub sculpt_mode: bool,
    #[serde(default)]
    pub sculpt_brush: crate::mesh::SculptBrushKind,
    #[serde(default = "default_sculpt_radius")]
    pub sculpt_radius: f32,
    #[serde(default = "default_sculpt_strength")]
    pub sculpt_strength: f32,
    #[serde(default)]
    pub weight_paint_mode: bool,
    #[serde(default = "default_weight_brush_radius")]
    pub weight_brush_radius: f32,
    #[serde(default = "default_weight_brush_strength")]
    pub weight_brush_strength: f32,
    #[serde(default)]
    pub stick_gizmo: crate::character::StickGizmoSettings,
    #[serde(default)]
    pub lookdev_preset: LookDevPreset,
    #[serde(default)]
    pub camera_bookmarks: Vec<CameraBookmark>,
    #[serde(default)]
    pub reference_planes: Vec<ReferenceImagePlane>,
    #[serde(default)]
    pub node_extras: BTreeMap<NodeId, BTreeMap<String, String>>,
    #[serde(default)]
    pub collision_hulls: BTreeMap<NodeId, crate::mesh::CollisionHull>,
    #[serde(default)]
    pub mesh_instances: BTreeMap<NodeId, Vec<Transform>>,
    #[serde(default)]
    pub npr_look: NprLookParams,
    #[serde(default)]
    pub post_process: PostProcessSettings,
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
    #[serde(skip, default = "SceneUid::next")]
    scene_uid: SceneUid,
}

/// Process-unique monotonic scene identity excluded from serialization and logical equality,
/// used by renderer caches to distinguish freshly created or loaded scenes sharing `revision`.
#[derive(Debug)]
struct SceneUid(u64);

impl Clone for SceneUid {
    fn clone(&self) -> Self {
        Self::next()
    }
}

impl SceneUid {
    fn next() -> Self {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self(COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }
}

impl PartialEq for SceneUid {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
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

fn default_sculpt_radius() -> f32 {
    0.35
}

fn default_sculpt_strength() -> f32 {
    0.5
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
            sculpt_mode: false,
            sculpt_brush: crate::mesh::SculptBrushKind::Smooth,
            sculpt_radius: default_sculpt_radius(),
            sculpt_strength: default_sculpt_strength(),
            weight_paint_mode: false,
            weight_brush_radius: default_weight_brush_radius(),
            weight_brush_strength: default_weight_brush_strength(),
            stick_gizmo: crate::character::StickGizmoSettings::default(),
            lookdev_preset: LookDevPreset::Daylight,
            camera_bookmarks: Vec::new(),
            reference_planes: Vec::new(),
            node_extras: BTreeMap::new(),
            collision_hulls: BTreeMap::new(),
            mesh_instances: BTreeMap::new(),
            npr_look: NprLookParams::default(),
            post_process: PostProcessSettings::default(),
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
            scene_uid: SceneUid::next(),
        }
    }

    /// Process-unique monotonic identifier distinguishing independent `SceneState` instances.
    pub const fn scene_uid(&self) -> u64 {
        self.scene_uid.0
    }

    pub fn allocate_node_id(&mut self) -> NodeId {
        let id = NodeId(self.next_node_id);
        self.next_node_id = self.next_node_id.saturating_add(1);
        id
    }

    /// Rewinds `next_node_id` when undoing the most recently allocated node so Redo reproduces the exact ID.
    pub(crate) fn rollback_node_id(&mut self, id: NodeId) {
        if self.next_node_id == id.0.saturating_add(1) {
            self.next_node_id = id.0;
        }
    }

    pub fn mark_modified(&mut self) {
        self.revision = self.revision.saturating_add(1);
        self.dirty_viewport = true;
    }

    // --- Ephemeral UI & Viewport Preview Mutators (non-undoable by design) ---

    pub fn set_pixel_tool(&mut self, tool: PixelTool) {
        self.pixel_tool = tool;
    }

    pub fn toggle_pixel_symmetry_x(&mut self) {
        self.pixel_symmetry.x = !self.pixel_symmetry.x;
    }

    pub fn toggle_pixel_symmetry_y(&mut self) {
        self.pixel_symmetry.y = !self.pixel_symmetry.y;
    }

    pub fn set_active_color(&mut self, color: PaletteIndex) {
        self.active_color = color;
    }

    pub fn select_face_ui(&mut self, face: Option<FaceId>) {
        self.selected_face = face;
        self.dirty_viewport = true;
    }

    pub fn select_bone_ui(&mut self, bone: Option<BoneId>) {
        self.selected_bone = bone;
        self.dirty_viewport = true;
    }

    pub fn set_viewport_dirty(&mut self, dirty: bool) {
        self.dirty_viewport = dirty;
    }

    /// Ephemeral gizmo-drag preview: mutates a node's transform without bumping `revision`
    /// so the caller can restore and commit a single `Command::SetNodeTransform` on drag release.
    pub fn preview_node_transform(&mut self, id: NodeId, transform: Transform) {
        if let Some(node) = self.nodes.get_mut(&id) {
            node.transform = transform;
            self.dirty_viewport = true;
        }
    }

    /// Ephemeral sun-gizmo preview: updates light angles without bumping `revision`
    /// so the caller can restore and commit a single `Command::SetLighting` on drag release.
    pub fn preview_lighting(&mut self, azimuth: f32, elevation: f32) {
        self.light_azimuth = azimuth;
        self.light_elevation = elevation;
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

    /// Returns `true` when the workspace contains any authored objects or painted content
    /// (non-empty low-poly meshes, occupied voxels, painted pixels, or rigged character bones).
    pub fn has_authored_content(&self) -> bool {
        if self.lowpoly_meshes.values().any(|m| !m.vertices.is_empty()) {
            return true;
        }
        if self.voxel_volumes.values().any(|v| !v.is_empty()) {
            return true;
        }
        if self.pixel_canvases.values().any(|canvas| {
            canvas.frames.iter().any(|f| {
                f.layers
                    .iter()
                    .any(|l| l.pixels().iter().any(Option::is_some))
            })
        }) {
            return true;
        }
        if self
            .character_rig
            .as_ref()
            .is_some_and(|rig| !rig.bones.is_empty())
        {
            return true;
        }
        false
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

    #[test]
    fn has_authored_content_detects_pixels_voxels_meshes_and_rig() {
        let mut state = SceneState::new();
        assert!(!state.has_authored_content());

        // Empty pixel canvas and empty voxel volume do not count as authored content.
        let px_id = state.allocate_node_id();
        state
            .pixel_canvases
            .insert(px_id, PixelCanvas::new(8, 8).unwrap());
        let vx_id = state.allocate_node_id();
        state.voxel_volumes.insert(vx_id, VoxelVolume::new());
        assert!(!state.has_authored_content());

        // Painting a single pixel marks the scene as having authored content.
        state.pixel_canvases.get_mut(&px_id).unwrap().frames[0].layers[0].set(
            crate::pixel::PixelCoord::new(0, 0),
            Some(state.active_color),
        );
        assert!(state.has_authored_content());

        // Clearing pixel returns to false, placing a voxel marks true.
        state.pixel_canvases.get_mut(&px_id).unwrap().frames[0].layers[0]
            .set(crate::pixel::PixelCoord::new(0, 0), None);
        assert!(!state.has_authored_content());

        state.voxel_volumes.get_mut(&vx_id).unwrap().set(
            crate::voxel::VoxelCoord::new(0, 0, 0),
            Some(state.active_color),
        );
        assert!(state.has_authored_content());

        // Clearing voxel and adding a low-poly primitive mesh marks true.
        state
            .voxel_volumes
            .get_mut(&vx_id)
            .unwrap()
            .set(crate::voxel::VoxelCoord::new(0, 0, 0), None);
        assert!(!state.has_authored_content());

        let mesh_id = state.allocate_node_id();
        let mut mesh = LowPolyMesh::new();
        state.lowpoly_meshes.insert(mesh_id, mesh.clone());
        assert!(!state.has_authored_content());
        mesh.vertices.push(glam::Vec3::ZERO);
        state.lowpoly_meshes.insert(mesh_id, mesh);
        assert!(state.has_authored_content());
    }
}
