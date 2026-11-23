use crate::error::CoreError;
use crate::scene::SceneState;

use super::bus::{Command, UndoDelta};

const DEFAULT_MAX_HISTORY: usize = 256;

#[derive(Debug, Clone)]
struct HistoryRecord {
    command: Command,
    delta: UndoDelta,
}

/// Deterministic Undo/Redo stack bound to the single Command Bus.
#[derive(Debug, Clone)]
pub struct HistoryStack {
    undo_stack: Vec<HistoryRecord>,
    redo_stack: Vec<Command>,
    max_depth: usize,
}

impl HistoryStack {
    pub fn new() -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            max_depth: DEFAULT_MAX_HISTORY,
        }
    }

    pub fn execute(&mut self, scene: &mut SceneState, command: Command) -> Result<(), CoreError> {
        let delta = command.apply(scene)?;
        self.redo_stack.clear();
        if self.undo_stack.len() >= self.max_depth {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(HistoryRecord { command, delta });
        Ok(())
    }

    pub fn undo(&mut self, scene: &mut SceneState) -> Result<(), CoreError> {
        let record = self.undo_stack.pop().ok_or(CoreError::EmptyHistory)?;
        record.delta.revert(scene)?;
        self.redo_stack.push(record.command);
        Ok(())
    }

    pub fn redo(&mut self, scene: &mut SceneState) -> Result<(), CoreError> {
        let command = self.redo_stack.pop().ok_or(CoreError::EmptyHistory)?;
        let delta = command.apply(scene)?;
        self.undo_stack.push(HistoryRecord { command, delta });
        Ok(())
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn undo_len(&self) -> usize {
        self.undo_stack.len()
    }

    pub fn redo_len(&self) -> usize {
        self.redo_stack.len()
    }
}

impl Default for HistoryStack {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::{PaletteEntry, PaletteIndex, Rgba8};
    use crate::scene::{AppMode, NodeId, NodeKind, Transform};

    #[test]
    fn mode_switch_undo_redo_roundtrip() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();

        assert_eq!(scene.active_mode, AppMode::Voxel);
        history
            .execute(&mut scene, Command::SetMode(AppMode::Pixel))
            .unwrap();
        assert_eq!(scene.active_mode, AppMode::Pixel);

        history.undo(&mut scene).unwrap();
        assert_eq!(scene.active_mode, AppMode::Voxel);

        history.redo(&mut scene).unwrap();
        assert_eq!(scene.active_mode, AppMode::Pixel);
    }

    #[test]
    fn create_and_delete_node_roundtrip() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();

        history
            .execute(
                &mut scene,
                Command::CreateNode {
                    name: "Knight".into(),
                    kind: NodeKind::VoxelVolume,
                    transform: Transform::default(),
                },
            )
            .unwrap();
        assert_eq!(scene.nodes.len(), 1);
        let id = scene.selected_node.unwrap();

        history
            .execute(&mut scene, Command::DeleteNode(id))
            .unwrap();
        assert!(scene.nodes.is_empty());
        assert_eq!(scene.selected_node, None);

        history.undo(&mut scene).unwrap();
        assert_eq!(scene.nodes.len(), 1);
        assert_eq!(scene.selected_node, Some(id));

        history.undo(&mut scene).unwrap();
        assert!(scene.nodes.is_empty());
        assert_eq!(scene.selected_node, None);
    }

    #[test]
    fn transaction_rolls_back_atomically_on_failure() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let idx = PaletteIndex::new(9).unwrap();

        let tx = Command::Transaction(vec![
            Command::SetMode(AppMode::Character),
            Command::SetPaletteEntry {
                index: idx,
                entry: PaletteEntry::matte("Gold", Rgba8::rgb(255, 200, 50)),
            },
            Command::DeleteNode(NodeId(999)),
        ]);

        let err = history.execute(&mut scene, tx).unwrap_err();
        assert_eq!(err, CoreError::NodeNotFound(NodeId(999)));
        assert_eq!(scene.active_mode, AppMode::Voxel);
        assert!(scene.palette.get(idx).is_none());
        assert!(!history.can_undo());
    }

    #[test]
    fn palette_index_zero_is_rejected() {
        assert_eq!(PaletteIndex::new(0), Err(CoreError::InvalidPaletteIndex));
    }

    #[test]
    fn pixel_draw_layer_and_frame_undo_redo_roundtrip() {
        use crate::pixel::{FrameIndex, PixelCoord, PixelEdit};

        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let amber = PaletteIndex::new(4).unwrap();

        history
            .execute(
                &mut scene,
                Command::CreatePixelCanvas {
                    name: "Sprite".into(),
                    width: 16,
                    height: 16,
                },
            )
            .unwrap();
        let node_id = scene.selected_node.unwrap();
        let layer_id = scene.pixel_canvases[&node_id].frames[0].layers[0].id;

        history
            .execute(
                &mut scene,
                Command::DrawPixels {
                    node_id,
                    frame: FrameIndex(0),
                    layer_id,
                    edits: vec![PixelEdit {
                        coord: PixelCoord::new(3, 4),
                        color: Some(amber),
                    }],
                },
            )
            .unwrap();
        assert_eq!(
            scene.pixel_canvases[&node_id].sample_active_pixel(PixelCoord::new(3, 4)),
            Some(amber)
        );

        history.undo(&mut scene).unwrap();
        assert_eq!(
            scene.pixel_canvases[&node_id].sample_active_pixel(PixelCoord::new(3, 4)),
            None
        );

        history.redo(&mut scene).unwrap();
        assert_eq!(
            scene.pixel_canvases[&node_id].sample_active_pixel(PixelCoord::new(3, 4)),
            Some(amber)
        );

        history
            .execute(&mut scene, Command::AddPixelFrame { node_id })
            .unwrap();
        assert_eq!(scene.pixel_canvases[&node_id].frames.len(), 2);
        history.undo(&mut scene).unwrap();
        assert_eq!(scene.pixel_canvases[&node_id].frames.len(), 1);
    }

    #[test]
    fn voxel_fill_and_undo_redo_roundtrip() {
        use crate::voxel::VoxelCoord;

        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let amber = PaletteIndex::new(4).unwrap();

        history
            .execute(
                &mut scene,
                Command::CreateVoxelVolume {
                    name: "Block".into(),
                },
            )
            .unwrap();
        let id = scene.selected_node.unwrap();

        history
            .execute(
                &mut scene,
                Command::FillVoxelBox {
                    node_id: id,
                    min: VoxelCoord::new(0, 0, 0),
                    max: VoxelCoord::new(2, 2, 2),
                    color: Some(amber),
                    hollow: false,
                },
            )
            .unwrap();
        assert_eq!(scene.voxel_volumes[&id].voxel_count(), 27);

        history.undo(&mut scene).unwrap();
        assert_eq!(scene.voxel_volumes[&id].voxel_count(), 0);

        history.redo(&mut scene).unwrap();
        assert_eq!(scene.voxel_volumes[&id].voxel_count(), 27);
    }

    #[test]
    fn lowpoly_and_character_rig_undo_redo_roundtrip() {
        use crate::character::BoneId;
        use crate::mesh::{FaceId, PrimitiveKind};
        use glam::Quat;

        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let mat = PaletteIndex::new(4).unwrap();

        history
            .execute(
                &mut scene,
                Command::CreatePrimitiveMesh {
                    name: "Cube".into(),
                    kind: PrimitiveKind::Cube,
                    size: 1.0,
                    material: mat,
                },
            )
            .unwrap();
        let mesh_id = scene.selected_node.unwrap();
        assert_eq!(scene.lowpoly_meshes[&mesh_id].faces.len(), 6);

        history
            .execute(
                &mut scene,
                Command::ExtrudeMeshFace {
                    node_id: mesh_id,
                    face_id: FaceId(4),
                    distance: 0.4,
                },
            )
            .unwrap();
        assert_eq!(scene.lowpoly_meshes[&mesh_id].faces.len(), 10);

        history.undo(&mut scene).unwrap();
        assert_eq!(scene.lowpoly_meshes[&mesh_id].faces.len(), 6);

        history
            .execute(
                &mut scene,
                Command::InsetMeshFace {
                    node_id: mesh_id,
                    face_id: FaceId(4),
                    factor: 0.3,
                },
            )
            .unwrap();
        assert_eq!(scene.lowpoly_meshes[&mesh_id].faces.len(), 10);
        history.undo(&mut scene).unwrap();
        assert_eq!(scene.lowpoly_meshes[&mesh_id].faces.len(), 6);

        history
            .execute(
                &mut scene,
                Command::SmoothSubdivideMesh { node_id: mesh_id },
            )
            .unwrap();
        assert_eq!(scene.lowpoly_meshes[&mesh_id].faces.len(), 24);
        history.undo(&mut scene).unwrap();
        assert_eq!(scene.lowpoly_meshes[&mesh_id].faces.len(), 6);

        history.execute(&mut scene, Command::ApplyBipedRig).unwrap();
        assert!(scene.character_rig.is_some());
        history
            .execute(
                &mut scene,
                Command::SetBoneRotation {
                    bone_id: BoneId(2),
                    rotation: Quat::from_rotation_y(0.5),
                },
            )
            .unwrap();
        history.undo(&mut scene).unwrap();
        assert_eq!(
            scene
                .character_rig
                .as_ref()
                .unwrap()
                .bone(BoneId(2))
                .unwrap()
                .pose_local
                .rotation,
            Quat::IDENTITY
        );

        let moved = Transform {
            translation: glam::Vec3::new(1.5, 0.0, -0.5),
            ..Transform::default()
        };
        history
            .execute(
                &mut scene,
                Command::SetNodeTransform {
                    node_id: mesh_id,
                    transform: moved,
                },
            )
            .unwrap();
        assert_eq!(scene.nodes[&mesh_id].transform.translation.x, 1.5);
        history.undo(&mut scene).unwrap();
        assert_eq!(scene.nodes[&mesh_id].transform.translation.x, 0.0);

        history
            .execute(
                &mut scene,
                Command::BindNodeToBone {
                    node_id: mesh_id,
                    bone_id: Some(BoneId(2)),
                },
            )
            .unwrap();
        assert_eq!(scene.character_rig.as_ref().unwrap().bindings.len(), 1);
        history.undo(&mut scene).unwrap();
        assert!(scene.character_rig.as_ref().unwrap().bindings.is_empty());

        // Custom animation clip + keyframe authoring undo/redo
        history
            .execute(
                &mut scene,
                Command::AddAnimationClip {
                    name: "Attack".into(),
                    duration_secs: 1.5,
                },
            )
            .unwrap();
        let clip_idx = scene.character_rig.as_ref().unwrap().active_animation;
        assert_eq!(clip_idx, 2);

        history
            .execute(
                &mut scene,
                Command::SetAnimationKeyframe {
                    clip_index: clip_idx,
                    bone_id: BoneId(4),
                    keyframe: crate::character::Keyframe::rotation_only(
                        0.5,
                        Quat::from_rotation_x(0.8),
                    ),
                },
            )
            .unwrap();
        assert_eq!(
            scene.character_rig.as_ref().unwrap().animations[clip_idx]
                .keyframe_times(Some(BoneId(4)))
                .len(),
            1
        );

        history
            .execute(
                &mut scene,
                Command::RemoveAnimationKeyframe {
                    clip_index: clip_idx,
                    bone_id: BoneId(4),
                    time_secs: 0.5,
                },
            )
            .unwrap();
        assert!(scene.character_rig.as_ref().unwrap().animations[clip_idx]
            .keyframe_times(Some(BoneId(4)))
            .is_empty());

        history.undo(&mut scene).unwrap();
        assert_eq!(
            scene.character_rig.as_ref().unwrap().animations[clip_idx]
                .keyframe_times(Some(BoneId(4)))
                .len(),
            1
        );
        history.undo(&mut scene).unwrap();
        history.undo(&mut scene).unwrap();
        assert_eq!(scene.character_rig.as_ref().unwrap().animations.len(), 2);
    }
}
