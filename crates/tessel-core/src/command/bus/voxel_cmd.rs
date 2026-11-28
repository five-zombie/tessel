#![allow(unused_imports, unused_variables)]
use glam::Quat;
use serde::{Deserialize, Serialize};

use crate::character::{
    AnimationClip, Bone, BoneId, CharacterRig, ClipLayer, Keyframe, SkinBinding,
};
use crate::error::CoreError;
use crate::mesh::{
    ops as mesh_ops, FaceId, LowPolyMesh, MeshMaterial, MeshPaintLayer, MeshTexture, PrimitiveKind,
    UvProjection, DEFAULT_MESH_TEX_SIZE,
};
use crate::palette::{PaletteEntry, PaletteIndex, Rgba8};
use crate::pixel::{FrameIndex, LayerId, PixelCanvas, PixelEdit, PixelFrame, PixelLayer};
use crate::scene::{AppMode, NodeId, NodeKind, SceneNode, SceneState, Transform};
use crate::voxel::{ops as voxel_ops, VoxelCoord, VoxelEdit, VoxelVolume};

use super::{insert_validated_scene_node, Command, UndoDelta};

pub(super) fn apply(cmd: &Command, scene: &mut SceneState) -> Result<UndoDelta, CoreError> {
    let delta = match cmd {
        Command::CreateVoxelVolume { name } => {
            let (id, previous_selection) = insert_validated_scene_node(
                scene,
                name,
                NodeKind::VoxelVolume,
                None,
                Transform::default(),
            )?;
            scene.voxel_volumes.insert(id, VoxelVolume::new());
            UndoDelta::RemoveNode {
                id,
                previous_selection,
            }
        }
        Command::SetVoxels { node_id, edits } => {
            let volume = scene
                .voxel_volumes
                .get_mut(node_id)
                .ok_or(CoreError::VoxelVolumeNotFound(*node_id))?;
            let mut previous_voxels = Vec::with_capacity(edits.len());
            for edit in edits {
                let prev = volume.set(edit.coord, edit.color);
                previous_voxels.push(VoxelEdit {
                    coord: edit.coord,
                    color: prev,
                });
            }
            UndoDelta::RestoreVoxels {
                node_id: *node_id,
                previous_voxels,
            }
        }
        Command::FillVoxelBox {
            node_id,
            min,
            max,
            color,
            hollow,
        } => {
            let edits = voxel_ops::box_fill_edits(*min, *max, *color, *hollow);
            let volume = scene
                .voxel_volumes
                .get_mut(node_id)
                .ok_or(CoreError::VoxelVolumeNotFound(*node_id))?;
            let mut previous_voxels = Vec::with_capacity(edits.len());
            for edit in edits {
                let prev = volume.set(edit.coord, edit.color);
                previous_voxels.push(VoxelEdit {
                    coord: edit.coord,
                    color: prev,
                });
            }
            UndoDelta::RestoreVoxels {
                node_id: *node_id,
                previous_voxels,
            }
        }
        Command::ConvertPixelToVoxel {
            pixel_node_id,
            depth,
            name,
        } => {
            let canvas = scene
                .pixel_canvases
                .get(pixel_node_id)
                .ok_or(CoreError::PixelCanvasNotFound(*pixel_node_id))?;
            if canvas.width > voxel_ops::MAX_PIXEL_TO_VOXEL_DIM
                || canvas.height > voxel_ops::MAX_PIXEL_TO_VOXEL_DIM
            {
                return Err(CoreError::OutOfBounds);
            }
            let vol = voxel_ops::pixel_to_voxel_volume(canvas, *depth);
            let (id, previous_selection) = insert_validated_scene_node(
                scene,
                name,
                NodeKind::VoxelVolume,
                None,
                Transform::default(),
            )?;
            scene.voxel_volumes.insert(id, vol);
            UndoDelta::RemoveNode {
                id,
                previous_selection,
            }
        }
        Command::ConvertVoxelToLowPoly {
            voxel_node_id,
            name,
        } => {
            let vol = scene
                .voxel_volumes
                .get(voxel_node_id)
                .ok_or(CoreError::VoxelVolumeNotFound(*voxel_node_id))?;
            let mesh = mesh_ops::voxel_to_lowpoly(vol, scene.unit_scale);
            let (id, previous_selection) = insert_validated_scene_node(
                scene,
                name,
                NodeKind::LowPolyMesh,
                None,
                Transform::default(),
            )?;
            scene.lowpoly_meshes.insert(id, mesh);
            UndoDelta::RemoveNode {
                id,
                previous_selection,
            }
        }
        _ => return Err(CoreError::OutOfBounds),
    };
    scene.mark_modified();
    Ok(delta)
}
