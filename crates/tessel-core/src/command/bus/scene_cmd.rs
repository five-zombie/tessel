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
        Command::SetMode(mode) => {
            let prev = scene.active_mode;
            scene.active_mode = *mode;
            UndoDelta::RestoreMode(prev)
        }
        Command::CreateNode {
            name,
            kind,
            transform,
        } => {
            let maybe_canvas = if *kind == NodeKind::PixelCanvas {
                Some(PixelCanvas::new(32, 32)?)
            } else {
                None
            };
            let (id, previous_selection) =
                insert_validated_scene_node(scene, name, *kind, None, *transform)?;
            if let Some(canvas) = maybe_canvas {
                scene.pixel_canvases.insert(id, canvas);
            } else if *kind == NodeKind::VoxelVolume {
                scene.voxel_volumes.insert(id, VoxelVolume::new());
            }
            UndoDelta::RemoveNode {
                id,
                previous_selection,
            }
        }
        Command::DeleteNode(id) => {
            let node = scene.nodes.remove(id).ok_or(CoreError::NodeNotFound(*id))?;
            let pixel_canvas = scene.pixel_canvases.remove(id);
            let voxel_volume = scene.voxel_volumes.remove(id);
            let lowpoly_mesh = scene.lowpoly_meshes.remove(id);
            let previous_extras = scene.node_extras.remove(id);
            let previous_collision_hull = scene.collision_hulls.remove(id);
            let previous_mesh_instances = scene.mesh_instances.remove(id);
            let previous_selection = scene.selected_node;
            let previous_selected_face = scene.selected_face;
            if scene.selected_node == Some(*id) {
                scene.selected_node = None;
                scene.selected_face = None;
            }
            let mut orphaned_children = Vec::new();
            for (child_id, child) in scene.nodes.iter_mut() {
                if child.parent == Some(*id) {
                    child.parent = None;
                    orphaned_children.push(*child_id);
                }
            }
            let mut previous_rig = None;
            if let Some(rig) = scene.character_rig.as_mut() {
                let has_refs = rig.bindings.iter().any(|b| b.node_id == *id)
                    || rig.unrigged_nodes.contains(id)
                    || rig.node_bone_masks.contains_key(id)
                    || rig.node_visibility.contains_key(id)
                    || rig.node_transform_overrides.contains_key(id)
                    || rig.node_parent_overrides.contains_key(id)
                    || rig.morph_weights.keys().any(|(nid, _)| nid == id)
                    || rig.animations.iter().any(|c| {
                        c.visibility_tracks.iter().any(|t| t.node == *id)
                            || c.parent_switch_tracks.iter().any(|t| t.node == *id)
                            || c.node_transform_tracks.iter().any(|t| t.node == *id)
                            || c.morph_tracks.iter().any(|t| t.node == *id)
                    });
                if has_refs {
                    previous_rig = Some(Box::new(rig.clone()));
                    rig.bindings.retain(|b| b.node_id != *id);
                    rig.unrigged_nodes.remove(id);
                    rig.node_bone_masks.remove(id);
                    rig.node_visibility.remove(id);
                    rig.node_transform_overrides.remove(id);
                    rig.node_parent_overrides.remove(id);
                    rig.morph_weights.retain(|(nid, _), _| nid != id);
                    for clip in &mut rig.animations {
                        clip.visibility_tracks.retain(|t| t.node != *id);
                        clip.parent_switch_tracks.retain(|t| t.node != *id);
                        clip.node_transform_tracks.retain(|t| t.node != *id);
                        clip.morph_tracks.retain(|t| t.node != *id);
                    }
                }
            }
            UndoDelta::RestoreNode {
                node: Box::new(node),
                pixel_canvas: pixel_canvas.map(Box::new),
                voxel_volume: voxel_volume.map(Box::new),
                lowpoly_mesh: lowpoly_mesh.map(Box::new),
                previous_selection,
                previous_selected_face,
                orphaned_children,
                previous_rig,
                previous_extras,
                previous_collision_hull,
                previous_mesh_instances,
            }
        }
        Command::SelectNode(target) => {
            if let Some(id) = target {
                if !scene.nodes.contains_key(id) {
                    return Err(CoreError::NodeNotFound(*id));
                }
            }
            let prev = scene.selected_node;
            scene.selected_node = *target;
            UndoDelta::RestoreSelection(prev)
        }
        Command::SetNodeTransform { node_id, transform } => {
            if !transform.translation.is_finite()
                || !transform.rotation.is_finite()
                || !transform.scale.is_finite()
            {
                return Err(CoreError::OutOfBounds);
            }
            let node = scene
                .nodes
                .get_mut(node_id)
                .ok_or(CoreError::NodeNotFound(*node_id))?;
            let previous = node.transform;
            node.transform = *transform;
            UndoDelta::RestoreNodeTransform {
                node_id: *node_id,
                previous,
            }
        }
        Command::SetNodeParent { node_id, parent } => {
            if !scene.nodes.contains_key(node_id) {
                return Err(CoreError::NodeNotFound(*node_id));
            }
            if let Some(pid) = parent {
                if *pid == *node_id || !scene.nodes.contains_key(pid) {
                    return Err(CoreError::OutOfBounds);
                }
                // Prevent cycle: walk ancestors of pid
                let mut curr = Some(*pid);
                let mut hops = 0usize;
                while let Some(anc) = curr {
                    if anc == *node_id || hops >= 16 {
                        return Err(CoreError::OutOfBounds);
                    }
                    curr = scene.nodes.get(&anc).and_then(|n| n.parent);
                    hops += 1;
                }
            }
            let node = scene
                .nodes
                .get_mut(node_id)
                .ok_or(CoreError::NodeNotFound(*node_id))?;
            let previous = node.parent;
            node.parent = *parent;
            UndoDelta::RestoreNodeParent {
                node_id: *node_id,
                previous,
            }
        }
        Command::SetLightDirection { azimuth, elevation } => {
            if !azimuth.is_finite() || !elevation.is_finite() {
                return Err(CoreError::OutOfBounds);
            }
            let prev_az = scene.light_azimuth;
            let prev_el = scene.light_elevation;
            scene.light_azimuth = *azimuth;
            scene.light_elevation = elevation.clamp(0.05, 1.52);
            UndoDelta::RestoreLightDirection {
                azimuth: prev_az,
                elevation: prev_el,
            }
        }
        Command::SetShadingMode(mode) => {
            let prev = scene.shading_mode;
            scene.shading_mode = *mode;
            UndoDelta::RestoreShadingMode(prev)
        }
        Command::SetPaletteEntry { index, entry } => {
            if !entry.roughness.is_finite()
                || !entry.metallic.is_finite()
                || !entry.emissive.is_finite()
                || !entry.transmission.is_finite()
                || !entry.ior.is_finite()
            {
                return Err(CoreError::OutOfBounds);
            }
            let sanitized = PaletteEntry {
                name: entry.name.clone(),
                color: entry.color,
                roughness: entry.roughness.clamp(0.05, 1.0),
                metallic: entry.metallic.clamp(0.0, 1.0),
                emissive: entry.emissive.clamp(0.0, 20.0),
                transmission: entry.transmission.clamp(0.0, 1.0),
                ior: entry.ior.clamp(1.0, 2.5),
            };
            let previous = scene.palette.set(*index, sanitized);
            UndoDelta::RestorePaletteEntry {
                index: *index,
                previous,
            }
        }
        Command::CreateGroupNode {
            name,
            transform,
            parent,
        } => {
            if let Some(pid) = *parent {
                if !scene.nodes.contains_key(&pid) {
                    return Err(CoreError::NodeNotFound(pid));
                }
            }
            let (id, previous_selection) =
                insert_validated_scene_node(scene, name, NodeKind::Group, *parent, *transform)?;
            UndoDelta::RemoveNode {
                id,
                previous_selection,
            }
        }
        Command::DuplicateNode { node_id, name } => {
            let source = scene
                .nodes
                .get(node_id)
                .ok_or(CoreError::NodeNotFound(*node_id))?
                .clone();
            let next_name = match name {
                Some(custom) => crate::scene::validate_node_name(custom)?,
                None => {
                    let base: String = source
                        .name
                        .chars()
                        .take(crate::scene::MAX_NODE_NAME_LEN.saturating_sub(5))
                        .collect();
                    format!("{base} Copy")
                }
            };
            let id = scene.allocate_node_id();
            let previous_selection = scene.selected_node;
            let mut node = source.clone();
            node.id = id;
            node.name = next_name;
            node.transform.translation += glam::Vec3::new(0.5, 0.0, 0.0);
            scene.nodes.insert(id, node);
            if let Some(mesh) = scene.lowpoly_meshes.get(node_id).cloned() {
                scene.lowpoly_meshes.insert(id, mesh);
            }
            if let Some(volume) = scene.voxel_volumes.get(node_id).cloned() {
                scene.voxel_volumes.insert(id, volume);
            }
            if let Some(canvas) = scene.pixel_canvases.get(node_id).cloned() {
                scene.pixel_canvases.insert(id, canvas);
            }
            if let Some(rig) = scene.character_rig.as_mut() {
                let inherited: Vec<BoneId> = rig
                    .bindings
                    .iter()
                    .filter(|b| b.node_id == *node_id)
                    .map(|b| b.bone_id)
                    .collect();
                for bone_id in inherited {
                    rig.bindings.push(SkinBinding {
                        node_id: id,
                        bone_id,
                    });
                }
                if rig.unrigged_nodes.contains(node_id) {
                    rig.unrigged_nodes.insert(id);
                }
            }
            scene.selected_node = Some(id);
            UndoDelta::RemoveNode {
                id,
                previous_selection,
            }
        }
        Command::RenameNode { node_id, name } => {
            let validated = crate::scene::validate_node_name(name)?;
            let node = scene
                .nodes
                .get_mut(node_id)
                .ok_or(CoreError::NodeNotFound(*node_id))?;
            let previous = std::mem::replace(&mut node.name, validated);
            UndoDelta::RestoreNodeName {
                node_id: *node_id,
                previous,
            }
        }
        Command::ClearScene => {
            let node_ids: Vec<NodeId> = scene.nodes.keys().copied().collect();
            let mut deltas = Vec::with_capacity(node_ids.len() + 1);
            for id in node_ids {
                deltas.push(Command::DeleteNode(id).apply(scene)?);
            }
            let prev_rig = scene.character_rig.take();
            if prev_rig.is_some() {
                deltas.push(UndoDelta::RestoreCharacterRig(prev_rig));
            }
            scene.selected_node = None;
            scene.selected_bone = None;
            scene.selected_face = None;
            UndoDelta::Transaction(deltas)
        }
        Command::ResetCharacterPose => {
            let prev = scene.character_rig.clone();
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            for bone in &mut rig.bones {
                bone.pose_local = bone.rest_local;
            }
            rig.morph_weights.clear();
            rig.node_transform_overrides.clear();
            rig.playback_time = 0.0;
            rig.playing = false;
            UndoDelta::RestoreCharacterRig(prev)
        }
        Command::SetNodeExtras { node_id, extras } => {
            if !scene.nodes.contains_key(node_id) {
                return Err(CoreError::NodeNotFound(*node_id));
            }
            let previous = if extras.is_empty() {
                scene.node_extras.remove(node_id)
            } else {
                scene.node_extras.insert(*node_id, extras.clone())
            };
            UndoDelta::RestoreNodeExtras {
                node_id: *node_id,
                previous,
            }
        }
        Command::SetCollisionHull { node_id, hull } => {
            if !scene.nodes.contains_key(node_id) {
                return Err(CoreError::NodeNotFound(*node_id));
            }
            if let Some(h) = hull {
                let valid = match h {
                    crate::mesh::CollisionHull::Box {
                        half_extents,
                        offset,
                    } => {
                        half_extents.iter().all(|v| v.is_finite() && *v > 0.0)
                            && offset.iter().all(|v| v.is_finite())
                    }
                    crate::mesh::CollisionHull::Capsule {
                        radius,
                        height,
                        offset,
                    } => {
                        radius.is_finite()
                            && *radius > 0.0
                            && height.is_finite()
                            && *height > 0.0
                            && offset.iter().all(|v| v.is_finite())
                    }
                    crate::mesh::CollisionHull::ConvexHull { points } => {
                        !points.is_empty()
                            && points.len() <= 4096
                            && points.iter().all(|p| p.iter().all(|v| v.is_finite()))
                    }
                };
                if !valid {
                    return Err(CoreError::OutOfBounds);
                }
            }
            let previous = match hull {
                Some(h) => scene.collision_hulls.insert(*node_id, h.clone()),
                None => scene.collision_hulls.remove(node_id),
            };
            UndoDelta::RestoreCollisionHull {
                node_id: *node_id,
                previous,
            }
        }
        Command::SetMeshInstances {
            node_id,
            transforms,
        } => {
            if !scene.nodes.contains_key(node_id) {
                return Err(CoreError::NodeNotFound(*node_id));
            }
            if transforms.len() > 4096
                || transforms.iter().any(|t| {
                    !t.translation.is_finite() || !t.rotation.is_finite() || !t.scale.is_finite()
                })
            {
                return Err(CoreError::OutOfBounds);
            }
            let previous = if transforms.is_empty() {
                scene.mesh_instances.remove(node_id)
            } else {
                scene.mesh_instances.insert(*node_id, transforms.clone())
            };
            UndoDelta::RestoreMeshInstances {
                node_id: *node_id,
                previous,
            }
        }
        Command::SetLookDevPreset { preset } => {
            let prev = scene.lookdev_preset;
            scene.lookdev_preset = *preset;
            UndoDelta::RestoreLookDevPreset(prev)
        }
        Command::SaveCameraBookmark { bookmark } => {
            if bookmark.name.trim().is_empty()
                || !bookmark.target.iter().all(|v| v.is_finite())
                || !bookmark.yaw.is_finite()
                || !bookmark.pitch.is_finite()
                || !bookmark.distance.is_finite()
                || bookmark.distance <= 0.0
            {
                return Err(CoreError::OutOfBounds);
            }
            let prev = scene.camera_bookmarks.clone();
            if let Some(existing) = scene
                .camera_bookmarks
                .iter_mut()
                .find(|b| b.name == bookmark.name)
            {
                *existing = bookmark.clone();
            } else {
                scene.camera_bookmarks.push(bookmark.clone());
            }
            UndoDelta::RestoreCameraBookmarks(prev)
        }
        Command::AddReferenceImagePlane { plane } => {
            if plane.name.trim().is_empty()
                || !plane.position.iter().all(|v| v.is_finite())
                || !plane.size.iter().all(|v| v.is_finite() && *v > 0.0)
                || !plane.opacity.is_finite()
                || !(0.0..=1.0).contains(&plane.opacity)
            {
                return Err(CoreError::OutOfBounds);
            }
            let prev = scene.reference_planes.clone();
            scene.reference_planes.push(plane.clone());
            UndoDelta::RestoreReferencePlanes(prev)
        }
        Command::SetNprLookParams { params } => {
            if !params.hatching_strength.is_finite()
                || !(0.0..=1.0).contains(&params.hatching_strength)
                || !params.bloom_strength.is_finite()
                || !(0.0..=2.0).contains(&params.bloom_strength)
                || (params.msaa_samples != 1 && params.msaa_samples != 4)
            {
                return Err(CoreError::OutOfBounds);
            }
            let prev = scene.npr_look;
            let prev_pp = scene.post_process;
            scene.npr_look = *params;
            if params.bloom_strength > 0.0 {
                scene.post_process.bloom_enabled = true;
                scene.post_process.bloom_intensity = params.bloom_strength;
            } else if prev.bloom_strength > 0.0 {
                scene.post_process.bloom_enabled = false;
            }
            UndoDelta::RestoreNprLookParams {
                previous: prev,
                previous_post_process: prev_pp,
            }
        }
        Command::SetPostProcess(settings) => {
            let validated = settings.validated()?;
            let previous = scene.post_process;
            let previous_npr_bloom = scene.npr_look.bloom_strength;
            scene.post_process = validated;
            scene.npr_look.bloom_strength = if validated.bloom_enabled {
                validated.bloom_intensity.clamp(0.0, 2.0)
            } else {
                0.0
            };
            UndoDelta::RestorePostProcess {
                previous,
                previous_npr_bloom,
            }
        }
        _ => return Err(CoreError::OutOfBounds),
    };
    scene.mark_modified();
    Ok(delta)
}
