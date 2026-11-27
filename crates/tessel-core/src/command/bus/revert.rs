#![allow(unused_imports)]
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

use super::UndoDelta;

impl UndoDelta {
    pub fn revert(&self, scene: &mut SceneState) -> Result<(), CoreError> {
        match self {
            Self::RestoreMode(mode) => {
                scene.active_mode = *mode;
            }
            Self::RemoveNode {
                id,
                previous_selection,
            } => {
                scene.nodes.remove(id);
                scene.pixel_canvases.remove(id);
                scene.voxel_volumes.remove(id);
                scene.lowpoly_meshes.remove(id);
                scene.node_extras.remove(id);
                scene.collision_hulls.remove(id);
                scene.mesh_instances.remove(id);
                if let Some(rig) = scene.character_rig.as_mut() {
                    rig.bindings.retain(|b| b.node_id != *id);
                    rig.unrigged_nodes.remove(id);
                }
                scene.rollback_node_id(*id);
                scene.selected_node = *previous_selection;
            }
            Self::RestoreNode {
                node,
                pixel_canvas,
                voxel_volume,
                lowpoly_mesh,
                previous_selection,
                previous_selected_face,
                orphaned_children,
                previous_rig,
                previous_extras,
                previous_collision_hull,
                previous_mesh_instances,
            } => {
                if let Some(canvas) = pixel_canvas {
                    scene.pixel_canvases.insert(node.id, (**canvas).clone());
                }
                if let Some(volume) = voxel_volume {
                    scene.voxel_volumes.insert(node.id, (**volume).clone());
                }
                if let Some(mesh) = lowpoly_mesh {
                    scene.lowpoly_meshes.insert(node.id, (**mesh).clone());
                }
                let restored_id = node.id;
                scene.nodes.insert(restored_id, (**node).clone());
                if let Some(ext) = previous_extras {
                    scene.node_extras.insert(restored_id, ext.clone());
                }
                if let Some(hull) = previous_collision_hull {
                    scene.collision_hulls.insert(restored_id, hull.clone());
                }
                if let Some(inst) = previous_mesh_instances {
                    scene.mesh_instances.insert(restored_id, inst.clone());
                }
                for child_id in orphaned_children {
                    if let Some(child) = scene.nodes.get_mut(child_id) {
                        child.parent = Some(restored_id);
                    }
                }
                if let Some(rig) = previous_rig {
                    scene.character_rig = Some((**rig).clone());
                }
                scene.selected_node = *previous_selection;
                scene.selected_face = *previous_selected_face;
            }
            Self::RestoreSelection(prev) => {
                scene.selected_node = *prev;
            }
            Self::RestoreNodeTransform { node_id, previous } => {
                let node = scene
                    .nodes
                    .get_mut(node_id)
                    .ok_or(CoreError::NodeNotFound(*node_id))?;
                node.transform = *previous;
            }
            Self::RestoreNodeParent { node_id, previous } => {
                let node = scene
                    .nodes
                    .get_mut(node_id)
                    .ok_or(CoreError::NodeNotFound(*node_id))?;
                node.parent = *previous;
            }
            Self::RestoreLightDirection { azimuth, elevation } => {
                scene.light_azimuth = *azimuth;
                scene.light_elevation = *elevation;
            }
            Self::RestoreShadingMode(prev) => {
                scene.shading_mode = *prev;
            }
            Self::RestorePaletteEntry { index, previous } => match previous {
                Some(entry) => {
                    scene.palette.set(*index, entry.clone());
                }
                None => {
                    scene.palette.remove(*index);
                }
            },
            Self::RestorePixels {
                node_id,
                frame,
                layer_id,
                previous_pixels,
            } => {
                let canvas = scene
                    .pixel_canvases
                    .get_mut(node_id)
                    .ok_or(CoreError::PixelCanvasNotFound(*node_id))?;
                let frame_ref = canvas
                    .frame_mut(*frame)
                    .ok_or(CoreError::PixelFrameNotFound(*frame))?;
                let layer = frame_ref
                    .layer_mut(*layer_id)
                    .ok_or(CoreError::PixelLayerNotFound(*layer_id))?;
                for prev in previous_pixels.iter().rev() {
                    layer.set(prev.coord, prev.color);
                }
            }
            Self::RemovePixelLayer {
                node_id,
                frame,
                layer_id,
                previous_active,
            } => {
                let canvas = scene
                    .pixel_canvases
                    .get_mut(node_id)
                    .ok_or(CoreError::PixelCanvasNotFound(*node_id))?;
                let frame_ref = canvas
                    .frame_mut(*frame)
                    .ok_or(CoreError::PixelFrameNotFound(*frame))?;
                frame_ref.layers.retain(|l| l.id != *layer_id);
                frame_ref.active_layer =
                    (*previous_active).min(frame_ref.layers.len().saturating_sub(1));
                canvas.rollback_layer_id(*layer_id);
            }
            Self::RestorePixelLayerVisible {
                node_id,
                frame,
                layer_id,
                previous,
            } => {
                let canvas = scene
                    .pixel_canvases
                    .get_mut(node_id)
                    .ok_or(CoreError::PixelCanvasNotFound(*node_id))?;
                let frame_ref = canvas
                    .frame_mut(*frame)
                    .ok_or(CoreError::PixelFrameNotFound(*frame))?;
                let layer = frame_ref
                    .layer_mut(*layer_id)
                    .ok_or(CoreError::PixelLayerNotFound(*layer_id))?;
                layer.visible = *previous;
            }
            Self::PopPixelFrame {
                node_id,
                previous_active,
            } => {
                let canvas = scene
                    .pixel_canvases
                    .get_mut(node_id)
                    .ok_or(CoreError::PixelCanvasNotFound(*node_id))?;
                if let Some(popped) = canvas.frames.pop() {
                    if let Some(first) = popped.layers.first() {
                        canvas.rollback_layer_id(first.id);
                    }
                }
                canvas.active_frame = (*previous_active).min(canvas.frames.len().saturating_sub(1));
            }
            Self::RestoreActivePixelFrame {
                node_id,
                previous_active,
            } => {
                let canvas = scene
                    .pixel_canvases
                    .get_mut(node_id)
                    .ok_or(CoreError::PixelCanvasNotFound(*node_id))?;
                canvas.active_frame = *previous_active;
            }
            Self::RestorePixelFps { node_id, previous } => {
                let canvas = scene
                    .pixel_canvases
                    .get_mut(node_id)
                    .ok_or(CoreError::PixelCanvasNotFound(*node_id))?;
                canvas.fps = *previous;
            }
            Self::RestoreVoxels {
                node_id,
                previous_voxels,
            } => {
                let volume = scene
                    .voxel_volumes
                    .get_mut(node_id)
                    .ok_or(CoreError::VoxelVolumeNotFound(*node_id))?;
                for prev in previous_voxels.iter().rev() {
                    volume.set(prev.coord, prev.color);
                }
            }
            Self::RestoreLowPolyMesh {
                node_id,
                previous_mesh,
            } => {
                scene.lowpoly_meshes.insert(*node_id, previous_mesh.clone());
            }
            Self::RestoreMeshTexturePixels {
                node_id,
                was_none,
                previous_pixels,
            } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                if *was_none {
                    mesh.texture = None;
                } else if let Some(tex) = &mut mesh.texture {
                    tex.restore_pixels(previous_pixels);
                }
            }
            Self::RestoreMeshTextureFull { node_id, previous } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                mesh.texture = previous.clone();
            }
            Self::RestoreMeshNormalMap { node_id, previous } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                mesh.normal_map = previous.clone();
            }
            Self::RestoreMeshAoMap { node_id, previous } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                mesh.ao_map = previous.clone();
            }
            Self::RestoreMeshNprSlot {
                node_id,
                slot,
                previous,
            } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                match slot {
                    crate::mesh::NprTextureSlot::MatCap => mesh.matcap_map = previous.clone(),
                    crate::mesh::NprTextureSlot::FaceSdf => mesh.face_sdf_map = previous.clone(),
                    crate::mesh::NprTextureSlot::HatchingRamp => {
                        mesh.hatching_ramp_map = previous.clone()
                    }
                }
            }
            Self::RestoreMeshVertexPositions { node_id, previous } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                for &(vi, pos) in previous {
                    if let Some(v) = mesh.vertices.get_mut(vi) {
                        *v = pos;
                    }
                }
            }
            Self::RestoreMeshPaintLayers {
                node_id,
                previous_layers,
                previous_active,
                previous_texture,
            } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                mesh.paint_layers = previous_layers.clone();
                mesh.active_paint_layer = *previous_active;
                mesh.texture = previous_texture.clone();
            }
            Self::RestoreMeshPaintLayerPixels {
                node_id,
                layer_id,
                previous_pixels,
            } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                if let Some(layer) = mesh.paint_layers.iter_mut().find(|l| l.id == *layer_id) {
                    layer.texture.restore_pixels(previous_pixels);
                }
                mesh.recomposite_paint_layers();
            }
            Self::RestoreMeshVertexColors { node_id, previous } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                mesh.vertex_colors = previous.clone();
            }
            Self::RestoreMeshCreaseEdges { node_id, previous } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                mesh.crease_edges = previous.clone();
            }
            Self::RestoreMeshSmoothShading { node_id, previous } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                mesh.smooth_shading = *previous;
            }
            Self::RestoreMeshMaterial { node_id, previous } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                mesh.material = *previous;
            }
            Self::RestoreCharacterRig(prev) => {
                scene.character_rig = prev.clone();
                if let Some(sel_bone) = scene.selected_bone {
                    let still_exists = scene
                        .character_rig
                        .as_ref()
                        .is_some_and(|r| r.bone(sel_bone).is_some());
                    if !still_exists {
                        scene.selected_bone = None;
                    }
                }
            }
            Self::RestoreBoneName { bone_id, previous } => {
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                let bone = rig.bone_mut(*bone_id).ok_or(CoreError::OutOfBounds)?;
                bone.name = previous.clone();
            }
            Self::RestoreBoneRotation { bone_id, previous } => {
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                let bone = rig.bone_mut(*bone_id).ok_or(CoreError::OutOfBounds)?;
                bone.pose_local.rotation = *previous;
            }
            Self::RestoreBoneTranslation { bone_id, previous } => {
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                let bone = rig.bone_mut(*bone_id).ok_or(CoreError::OutOfBounds)?;
                bone.pose_local.translation = *previous;
            }
            Self::RestoreBoneRestPosition {
                bone_id,
                previous_rest,
                previous_pose,
            } => {
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                let bone = rig.bone_mut(*bone_id).ok_or(CoreError::OutOfBounds)?;
                bone.rest_local.translation = *previous_rest;
                bone.pose_local.translation = *previous_pose;
            }
            Self::RestoreBoneRestRotation {
                bone_id,
                previous_rest,
                previous_pose,
            } => {
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                let bone = rig.bone_mut(*bone_id).ok_or(CoreError::OutOfBounds)?;
                bone.rest_local.rotation = *previous_rest;
                bone.pose_local.rotation = *previous_pose;
            }
            Self::RestoreClipLayers(previous) => {
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                rig.clip_layers = previous.clone();
                rig.apply_active_animation();
            }
            Self::RestoreNodeBoneBinding { node_id, previous } => {
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                rig.bindings.retain(|b| b.node_id != *node_id);
                if let Some(bid) = previous {
                    rig.bindings.push(SkinBinding {
                        node_id: *node_id,
                        bone_id: *bid,
                    });
                }
            }
            Self::RestoreNodeRigSkinned {
                node_id,
                previous_unrigged,
            } => {
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                if *previous_unrigged {
                    rig.unrigged_nodes.insert(*node_id);
                } else {
                    rig.unrigged_nodes.remove(node_id);
                }
            }
            Self::PopAnimationClip {
                previous_active,
                previous_time,
            } => {
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                rig.animations.pop();
                rig.active_animation =
                    (*previous_active).min(rig.animations.len().saturating_sub(1));
                rig.playback_time = *previous_time;
                rig.apply_active_animation();
            }
            Self::RestorePixelLayerOpacity {
                node_id,
                frame,
                layer_id,
                previous,
            } => {
                if let Some(canvas) = scene.pixel_canvases.get_mut(node_id) {
                    if let Some(frame_ref) = canvas.frame_mut(*frame) {
                        if let Some(layer) = frame_ref.layer_mut(*layer_id) {
                            layer.opacity = *previous;
                        }
                    }
                }
            }
            Self::RestorePixelOnionSkin { node_id, previous } => {
                if let Some(canvas) = scene.pixel_canvases.get_mut(node_id) {
                    canvas.onion_skin = *previous;
                }
            }
            Self::RestoreMeshProxyNormals { node_id, previous } => {
                if let Some(mesh) = scene.lowpoly_meshes.get_mut(node_id) {
                    mesh.proxy_normals = previous.clone();
                }
            }
            Self::RestoreAnimationClip {
                clip_index,
                previous,
            } => {
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                let clip = rig
                    .animations
                    .get_mut(*clip_index)
                    .ok_or(CoreError::OutOfBounds)?;
                *clip = previous.clone();
                if rig.active_animation == *clip_index {
                    rig.apply_active_animation();
                }
            }
            Self::RestoreNodeVisibility { node_id, previous } => {
                let node = scene
                    .nodes
                    .get_mut(node_id)
                    .ok_or(CoreError::NodeNotFound(*node_id))?;
                node.visible = *previous;
            }
            Self::RestoreNodeName { node_id, previous } => {
                let node = scene
                    .nodes
                    .get_mut(node_id)
                    .ok_or(CoreError::NodeNotFound(*node_id))?;
                node.name = previous.clone();
            }
            Self::RestoreBoneRotations(entries) => {
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                for &(bone_id, prev_rot) in entries {
                    if let Some(bone) = rig.bone_mut(bone_id) {
                        bone.pose_local.rotation = prev_rot;
                    }
                }
            }
            Self::RestoreMorphDefaultWeight {
                node_id,
                name,
                previous_mesh,
                previous_rig_weight,
            } => {
                scene.lowpoly_meshes.insert(*node_id, previous_mesh.clone());
                if let (Some(rig), Some(prev_w)) =
                    (scene.character_rig.as_mut(), previous_rig_weight)
                {
                    match prev_w {
                        Some(w) => {
                            rig.morph_weights.insert((*node_id, name.clone()), *w);
                        }
                        None => {
                            rig.morph_weights.remove(&(*node_id, name.clone()));
                        }
                    }
                }
            }
            Self::RestoreNodeExtras { node_id, previous } => match previous {
                Some(m) => {
                    scene.node_extras.insert(*node_id, m.clone());
                }
                None => {
                    scene.node_extras.remove(node_id);
                }
            },
            Self::RestoreCollisionHull { node_id, previous } => match previous {
                Some(h) => {
                    scene.collision_hulls.insert(*node_id, h.clone());
                }
                None => {
                    scene.collision_hulls.remove(node_id);
                }
            },
            Self::RestoreMeshInstances { node_id, previous } => match previous {
                Some(t) => {
                    scene.mesh_instances.insert(*node_id, t.clone());
                }
                None => {
                    scene.mesh_instances.remove(node_id);
                }
            },
            Self::RestoreLookDevPreset(prev) => {
                scene.lookdev_preset = *prev;
            }
            Self::RestoreCameraBookmarks(prev) => {
                scene.camera_bookmarks = prev.clone();
            }
            Self::RestoreReferencePlanes(prev) => {
                scene.reference_planes = prev.clone();
            }
            Self::RestoreNprLookParams(prev) => {
                scene.npr_look = *prev;
            }
            Self::Transaction(deltas) => {
                for delta in deltas.iter().rev() {
                    delta.revert(scene)?;
                }
            }
        }
        scene.mark_modified();
        Ok(())
    }
}
