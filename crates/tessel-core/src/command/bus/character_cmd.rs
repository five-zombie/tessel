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
        Command::UpdateAnimationClipMeta {
            clip_index,
            name,
            duration_secs,
        } => {
            if let Some(d) = duration_secs {
                if !d.is_finite() || *d < 0.05 || *d > 600.0 {
                    return Err(CoreError::OutOfBounds);
                }
            }
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            let clip = rig
                .animations
                .get_mut(*clip_index)
                .ok_or(CoreError::OutOfBounds)?;
            let previous = clip.clone();
            if let Some(n) = name {
                let trimmed = n.trim();
                if !trimmed.is_empty() {
                    clip.name = trimmed.to_string();
                }
            }
            if let Some(d) = duration_secs {
                clip.duration_secs = *d;
            }
            UndoDelta::RestoreAnimationClip {
                clip_index: *clip_index,
                previous,
            }
        }
        Command::ApplyBipedRig => {
            let prev = scene.character_rig.replace(CharacterRig::biped_template());
            UndoDelta::RestoreCharacterRig(prev)
        }
        Command::ApplyHumanoidRig => {
            let prev = scene
                .character_rig
                .replace(CharacterRig::humanoid_template());
            UndoDelta::RestoreCharacterRig(prev)
        }
        Command::AddBone {
            name,
            parent,
            position,
        } => {
            if !position.is_finite() {
                return Err(CoreError::OutOfBounds);
            }
            let prev = scene.character_rig.clone();
            let rig = scene
                .character_rig
                .get_or_insert_with(CharacterRig::default);
            if let Some(pid) = parent {
                if rig.bone(*pid).is_none() {
                    scene.character_rig = prev;
                    return Err(CoreError::OutOfBounds);
                }
            }
            let next_id = rig
                .bones
                .iter()
                .map(|b| b.id.0.saturating_add(1))
                .max()
                .unwrap_or(0);
            rig.bones.push(Bone::new(
                BoneId(next_id),
                name.clone(),
                *parent,
                Transform {
                    translation: *position,
                    rotation: Quat::IDENTITY,
                    scale: glam::Vec3::ONE,
                },
            ));
            UndoDelta::RestoreCharacterRig(prev)
        }
        Command::SetBoneRotation { bone_id, rotation } => {
            if !rotation.is_finite() {
                return Err(CoreError::OutOfBounds);
            }
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            let bone = rig.bone_mut(*bone_id).ok_or(CoreError::OutOfBounds)?;
            let previous = bone.pose_local.rotation;
            bone.pose_local.rotation = bone.clamp_rotation_quat(*rotation);
            UndoDelta::RestoreBoneRotation {
                bone_id: *bone_id,
                previous,
            }
        }
        Command::SetBoneTranslation {
            bone_id,
            translation,
        } => {
            if !translation.is_finite() {
                return Err(CoreError::OutOfBounds);
            }
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            let bone = rig.bone_mut(*bone_id).ok_or(CoreError::OutOfBounds)?;
            let previous = bone.pose_local.translation;
            bone.pose_local.translation = *translation;
            UndoDelta::RestoreBoneTranslation {
                bone_id: *bone_id,
                previous,
            }
        }
        Command::SetBoneRestPosition { bone_id, position } => {
            if !position.is_finite() {
                return Err(CoreError::OutOfBounds);
            }
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            let bone = rig.bone_mut(*bone_id).ok_or(CoreError::OutOfBounds)?;
            let previous_rest = bone.rest_local.translation;
            let previous_pose = bone.pose_local.translation;
            bone.rest_local.translation = *position;
            bone.pose_local.translation = *position;
            UndoDelta::RestoreBoneRestPosition {
                bone_id: *bone_id,
                previous_rest,
                previous_pose,
            }
        }
        Command::SetBoneRestRotation { bone_id, rotation } => {
            if !rotation.is_finite() {
                return Err(CoreError::OutOfBounds);
            }
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            let bone = rig.bone_mut(*bone_id).ok_or(CoreError::OutOfBounds)?;
            let previous_rest = bone.rest_local.rotation;
            let previous_pose = bone.pose_local.rotation;
            bone.rest_local.rotation = *rotation;
            bone.pose_local.rotation = *rotation;
            UndoDelta::RestoreBoneRestRotation {
                bone_id: *bone_id,
                previous_rest,
                previous_pose,
            }
        }
        Command::SolveTwoBoneIk {
            root_bone,
            mid_bone,
            end_bone,
            target,
            pole,
        } => {
            let rig = scene
                .character_rig
                .as_ref()
                .ok_or(CoreError::CharacterRigNotFound)?;
            let (root_local, mid_local) = crate::character::ik::solve_for_rig(
                rig,
                *root_bone,
                *mid_bone,
                *end_bone,
                *target,
                Some(*pole),
            )?;
            let mut applied = Vec::with_capacity(2);
            for (id, rot) in [(*root_bone, root_local), (*mid_bone, mid_local)] {
                let delta = Command::SetBoneRotation {
                    bone_id: id,
                    rotation: rot,
                }
                .apply(scene)?;
                applied.push(delta);
            }
            UndoDelta::Transaction(applied)
        }
        Command::BindNodeToBone { node_id, bone_id } => {
            if !scene.nodes.contains_key(node_id) {
                return Err(CoreError::NodeNotFound(*node_id));
            }
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            if let Some(bid) = bone_id {
                if rig.bone(*bid).is_none() {
                    return Err(CoreError::OutOfBounds);
                }
            }
            let previous = rig
                .bindings
                .iter()
                .find(|b| b.node_id == *node_id)
                .map(|b| b.bone_id);
            rig.bindings.retain(|b| b.node_id != *node_id);
            if let Some(bid) = bone_id {
                rig.bindings.push(SkinBinding {
                    node_id: *node_id,
                    bone_id: *bid,
                });
            }
            UndoDelta::RestoreNodeBoneBinding {
                node_id: *node_id,
                previous,
            }
        }
        Command::SetNodeBoneMask { node_id, bone_ids } => {
            if !scene.nodes.contains_key(node_id) {
                return Err(CoreError::NodeNotFound(*node_id));
            }
            let prev = scene.character_rig.clone();
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            match bone_ids {
                Some(ids) if !ids.is_empty() => {
                    for bid in ids {
                        if rig.bone(*bid).is_none() {
                            scene.character_rig = prev;
                            return Err(CoreError::OutOfBounds);
                        }
                    }
                    rig.node_bone_masks.insert(*node_id, ids.clone());
                }
                _ => {
                    rig.node_bone_masks.remove(node_id);
                }
            }
            UndoDelta::RestoreCharacterRig(prev)
        }
        Command::AddAnimationClip {
            name,
            duration_secs,
        } => {
            if !duration_secs.is_finite() || *duration_secs <= 0.0 {
                return Err(CoreError::OutOfBounds);
            }
            let prev_rig = scene.character_rig.clone();
            let had_rig = scene.character_rig.is_some();
            let rig = scene
                .character_rig
                .get_or_insert_with(CharacterRig::default);
            let previous_active = rig.active_animation;
            let previous_time = rig.playback_time;
            rig.animations
                .push(AnimationClip::new_empty(name.clone(), *duration_secs));
            rig.active_animation = rig.animations.len().saturating_sub(1);
            rig.playback_time = 0.0;
            rig.playing = false;
            if had_rig {
                UndoDelta::PopAnimationClip {
                    previous_active,
                    previous_time,
                }
            } else {
                UndoDelta::RestoreCharacterRig(prev_rig)
            }
        }
        Command::SetAnimationKeyframe {
            clip_index,
            bone_id,
            keyframe,
        } => {
            if !keyframe.time_secs.is_finite() || keyframe.time_secs < 0.0 {
                return Err(CoreError::OutOfBounds);
            }
            if !keyframe.translation.is_finite()
                || !keyframe.scale.is_finite()
                || !keyframe.rotation.is_finite()
            {
                return Err(CoreError::OutOfBounds);
            }
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            if rig.bone(*bone_id).is_none() {
                return Err(CoreError::OutOfBounds);
            }
            let clip = rig
                .animations
                .get_mut(*clip_index)
                .ok_or(CoreError::OutOfBounds)?;
            let previous = clip.clone();
            clip.upsert_keyframe(*bone_id, *keyframe);
            UndoDelta::RestoreAnimationClip {
                clip_index: *clip_index,
                previous,
            }
        }
        Command::RemoveAnimationKeyframe {
            clip_index,
            bone_id,
            time_secs,
        } => {
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            let clip = rig
                .animations
                .get_mut(*clip_index)
                .ok_or(CoreError::OutOfBounds)?;
            let previous = clip.clone();
            clip.remove_keyframe(*bone_id, *time_secs);
            UndoDelta::RestoreAnimationClip {
                clip_index: *clip_index,
                previous,
            }
        }
        Command::SetMorphWeightKeyframe {
            clip_index,
            node_id,
            target,
            time_secs,
            weight,
        } => {
            if !time_secs.is_finite() || *time_secs < 0.0 || !weight.is_finite() {
                return Err(CoreError::OutOfBounds);
            }
            if target.is_empty() {
                return Err(CoreError::OutOfBounds);
            }
            if !scene.nodes.contains_key(node_id) {
                return Err(CoreError::NodeNotFound(*node_id));
            }
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            let clip = rig
                .animations
                .get_mut(*clip_index)
                .ok_or(CoreError::OutOfBounds)?;
            let previous = clip.clone();
            clip.upsert_morph_weight(*node_id, target, *time_secs, *weight);
            UndoDelta::RestoreAnimationClip {
                clip_index: *clip_index,
                previous,
            }
        }
        Command::SetVisibilityKeyframe {
            clip_index,
            node_id,
            time_secs,
            visible,
        } => {
            if !time_secs.is_finite() || *time_secs < 0.0 {
                return Err(CoreError::OutOfBounds);
            }
            if !scene.nodes.contains_key(node_id) {
                return Err(CoreError::NodeNotFound(*node_id));
            }
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            let clip = rig
                .animations
                .get_mut(*clip_index)
                .ok_or(CoreError::OutOfBounds)?;
            let previous = clip.clone();
            clip.upsert_visibility_key(*node_id, *time_secs, *visible);
            UndoDelta::RestoreAnimationClip {
                clip_index: *clip_index,
                previous,
            }
        }
        Command::SetParentSwitchKeyframe {
            clip_index,
            node_id,
            key,
        } => {
            if !key.time_secs.is_finite() || key.time_secs < 0.0 {
                return Err(CoreError::OutOfBounds);
            }
            if !scene.nodes.contains_key(node_id) {
                return Err(CoreError::NodeNotFound(*node_id));
            }
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            if let Some(bid) = key.bone_id {
                if rig.bone(bid).is_none() {
                    return Err(CoreError::OutOfBounds);
                }
            }
            let clip = rig
                .animations
                .get_mut(*clip_index)
                .ok_or(CoreError::OutOfBounds)?;
            let previous = clip.clone();
            clip.upsert_parent_switch_key(*node_id, *key);
            rig.apply_active_animation();
            UndoDelta::RestoreAnimationClip {
                clip_index: *clip_index,
                previous,
            }
        }
        Command::SetClipLayers { layers } => {
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            let previous = std::mem::replace(&mut rig.clip_layers, layers.clone());
            rig.apply_active_animation();
            UndoDelta::RestoreClipLayers(previous)
        }
        Command::UpsertAnimationClip { clip, activate } => {
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            if let Some(idx) = rig.animations.iter().position(|c| c.name == clip.name) {
                let previous = std::mem::replace(&mut rig.animations[idx], clip.clone());
                if *activate {
                    rig.active_animation = idx;
                }
                rig.apply_active_animation();
                UndoDelta::RestoreAnimationClip {
                    clip_index: idx,
                    previous,
                }
            } else {
                let previous_active = rig.active_animation;
                let previous_time = rig.playback_time;
                rig.animations.push(clip.clone());
                if *activate {
                    rig.active_animation = rig.animations.len().saturating_sub(1);
                }
                rig.apply_active_animation();
                UndoDelta::PopAnimationClip {
                    previous_active,
                    previous_time,
                }
            }
        }
        Command::RemoveMorphWeightKeyframe {
            clip_index,
            node_id,
            target,
            time_secs,
        } => {
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            let clip = rig
                .animations
                .get_mut(*clip_index)
                .ok_or(CoreError::OutOfBounds)?;
            let previous = clip.clone();
            clip.remove_morph_weight(*node_id, target, *time_secs);
            UndoDelta::RestoreAnimationClip {
                clip_index: *clip_index,
                previous,
            }
        }
        Command::SetActiveAnimationClip { clip_index } => {
            let previous = scene.character_rig.clone();
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            if *clip_index >= rig.animations.len() {
                return Err(CoreError::OutOfBounds);
            }
            rig.active_animation = *clip_index;
            rig.playback_time = 0.0;
            rig.apply_active_animation();
            UndoDelta::RestoreCharacterRig(previous)
        }
        Command::RemoveAnimationClip { clip_index } => {
            let previous = scene.character_rig.clone();
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            if *clip_index >= rig.animations.len() {
                return Err(CoreError::OutOfBounds);
            }
            rig.animations.remove(*clip_index);
            rig.clip_layers.retain_mut(|layer| {
                if layer.clip_index == *clip_index {
                    false
                } else {
                    if layer.clip_index > *clip_index {
                        layer.clip_index -= 1;
                    }
                    true
                }
            });
            if rig.animations.is_empty() {
                rig.active_animation = 0;
            } else if *clip_index < rig.active_animation {
                rig.active_animation -= 1;
            } else if rig.active_animation >= rig.animations.len() {
                rig.active_animation = rig.animations.len() - 1;
            }
            UndoDelta::RestoreCharacterRig(previous)
        }
        Command::RemoveBone { bone_id } => {
            let previous = scene.character_rig.clone();
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            if rig.bone(*bone_id).is_none() {
                return Err(CoreError::OutOfBounds);
            }
            // Collect the whole subtree: any bone whose parent chain
            // reaches `bone_id` goes too.
            let mut removed = vec![*bone_id];
            loop {
                let mut added = false;
                for bone in rig.bones.iter() {
                    if !removed.contains(&bone.id)
                        && bone.parent.is_some_and(|p| removed.contains(&p))
                    {
                        removed.push(bone.id);
                        added = true;
                    }
                }
                if !added {
                    break;
                }
            }
            rig.bones.retain(|b| !removed.contains(&b.id));
            rig.bindings.retain(|b| !removed.contains(&b.bone_id));
            rig.chain_poles.retain(|bid, _| !removed.contains(bid));
            rig.sockets.retain(|s| !removed.contains(&s.bone_id));
            if rig
                .root_motion_bone
                .is_some_and(|bid| removed.contains(&bid))
            {
                rig.root_motion_bone = None;
            }
            for group in rig.bone_groups.values_mut() {
                group.bone_ids.retain(|bid| !removed.contains(bid));
            }
            rig.bone_groups.retain(|_, g| !g.bone_ids.is_empty());
            for pose in rig.pose_library.values_mut() {
                pose.retain(|bid, _| !removed.contains(bid));
            }
            rig.constraints.retain(|c| match c {
                crate::character::BoneConstraint::IkFkBlend { root, mid, end, .. } => {
                    !removed.contains(root) && !removed.contains(mid) && !removed.contains(end)
                }
                crate::character::BoneConstraint::LookAt { bone_id, .. } => {
                    !removed.contains(bone_id)
                }
                crate::character::BoneConstraint::CopyTransform {
                    bone_id,
                    source_bone_id,
                    ..
                } => !removed.contains(bone_id) && !removed.contains(source_bone_id),
            });
            for mask in rig.node_bone_masks.values_mut() {
                mask.retain(|bid| !removed.contains(bid));
            }
            rig.node_bone_masks.retain(|_, mask| !mask.is_empty());
            for clip in rig.animations.iter_mut() {
                clip.channels.retain(|c| !removed.contains(&c.bone_id));
                for track in clip.parent_switch_tracks.iter_mut() {
                    for k in track.keys.iter_mut() {
                        if k.bone_id.is_some_and(|bid| removed.contains(&bid)) {
                            k.bone_id = None;
                        }
                    }
                }
            }
            UndoDelta::RestoreCharacterRig(previous)
        }
        Command::SetNodeTransformKeyframe {
            clip_index,
            node_id,
            keyframe,
        } => {
            if !scene.nodes.contains_key(node_id) {
                return Err(CoreError::NodeNotFound(*node_id));
            }
            if !keyframe.time_secs.is_finite()
                || keyframe.time_secs < 0.0
                || !keyframe.translation.is_finite()
                || !keyframe.rotation.is_finite()
                || !keyframe.scale.is_finite()
            {
                return Err(CoreError::OutOfBounds);
            }
            let prev = scene.character_rig.clone();
            let rig = scene
                .character_rig
                .get_or_insert_with(CharacterRig::default);
            if rig.animations.is_empty() && *clip_index == 0 {
                rig.animations.push(AnimationClip::new_empty(
                    "SceneAnim",
                    keyframe.time_secs.max(1.0),
                ));
            }
            let clip = rig
                .animations
                .get_mut(*clip_index)
                .ok_or(CoreError::OutOfBounds)?;
            clip.upsert_node_transform_key(*node_id, *keyframe);
            UndoDelta::RestoreCharacterRig(prev)
        }
        Command::SetNodeVisibility { node_id, visible } => {
            let node = scene
                .nodes
                .get_mut(node_id)
                .ok_or(CoreError::NodeNotFound(*node_id))?;
            let previous = node.visible;
            node.visible = *visible;
            UndoDelta::RestoreNodeVisibility {
                node_id: *node_id,
                previous,
            }
        }
        Command::SetAnimationPlaybackTime {
            clip_index,
            time_secs,
            playing,
        } => {
            if !time_secs.is_finite() || *time_secs < 0.0 {
                return Err(CoreError::OutOfBounds);
            }
            let prev = scene.character_rig.clone();
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            if let Some(idx) = *clip_index {
                if idx >= rig.animations.len() {
                    scene.character_rig = prev;
                    return Err(CoreError::OutOfBounds);
                }
                rig.active_animation = idx;
            }
            rig.playback_time = *time_secs;
            rig.playing = playing.unwrap_or(false);
            rig.apply_active_animation();
            UndoDelta::RestoreCharacterRig(prev)
        }
        Command::CreateBoneGrid { spec, bind_node } => {
            let previous = scene.character_rig.clone();
            let rig = scene
                .character_rig
                .get_or_insert_with(CharacterRig::default);
            let result = crate::character::create_bone_grid(rig, spec)?;
            if let Some(nid) = *bind_node {
                if !scene.nodes.contains_key(&nid) {
                    scene.character_rig = previous;
                    return Err(CoreError::NodeNotFound(nid));
                }
                crate::character::assign_bone_grid_mask(rig, nid, result.all_bone_ids());
            }
            UndoDelta::RestoreCharacterRig(previous)
        }
        Command::BakeClothGridAnimation {
            bone_prefix,
            params,
        } => {
            let previous = scene.character_rig.clone();
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            let prefix_tag = format!("{}_c", bone_prefix.trim());
            let mut col_map: std::collections::BTreeMap<usize, Vec<(usize, BoneId)>> =
                std::collections::BTreeMap::new();
            for bone in &rig.bones {
                if let Some(rest) = bone.name.strip_prefix(&prefix_tag) {
                    if let Some((c_str, r_str)) = rest.split_once("_r") {
                        if let (Ok(c), Ok(r)) = (c_str.parse::<usize>(), r_str.parse::<usize>()) {
                            col_map.entry(c).or_default().push((r, bone.id));
                        }
                    }
                }
            }
            if col_map.is_empty() {
                return Err(CoreError::OutOfBounds);
            }
            let mut columns = Vec::with_capacity(col_map.len());
            for (_, mut rows) in col_map {
                rows.sort_by_key(|(r, _)| *r);
                columns.push(rows.into_iter().map(|(_, id)| id).collect());
            }
            let clip_idx = crate::character::bake_cloth_grid_animation(rig, &columns, params)?;
            rig.active_animation = clip_idx;
            rig.apply_active_animation();
            UndoDelta::RestoreCharacterRig(previous)
        }
        Command::ApplyStickGizmoPose { params } => {
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            let previous = crate::character::apply_stick_gizmo_pose(rig, params)?;
            UndoDelta::RestoreBoneRotations(previous)
        }
        Command::SetBoneScale { bone_id, scale } => {
            if !scale.is_finite() {
                return Err(CoreError::OutOfBounds);
            }
            let clamped = glam::Vec3::new(scale.x.max(1e-6), scale.y.max(1e-6), scale.z.max(1e-6));
            let prev = scene.character_rig.clone();
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            let bone = rig.bone_mut(*bone_id).ok_or(CoreError::OutOfBounds)?;
            bone.pose_local.scale = clamped;
            UndoDelta::RestoreCharacterRig(prev)
        }
        Command::SetBoneProperties {
            bone_id,
            rotation_limits_deg,
            inherit_rotation,
            inherit_scale,
            connected,
            deform,
            spring_stiffness,
            spring_damping,
            lod_level,
            physics_collider,
        } => {
            if let Some(Some((min_d, max_d))) = rotation_limits_deg {
                for axis in 0..3 {
                    if !min_d[axis].is_finite()
                        || !max_d[axis].is_finite()
                        || min_d[axis] > max_d[axis]
                    {
                        return Err(CoreError::OutOfBounds);
                    }
                }
            }
            if let Some(v) = spring_stiffness {
                if !v.is_finite() || !(0.0..=200.0).contains(v) {
                    return Err(CoreError::OutOfBounds);
                }
            }
            if let Some(v) = spring_damping {
                if !v.is_finite() || !(0.01..=50.0).contains(v) {
                    return Err(CoreError::OutOfBounds);
                }
            }
            if let Some(Some(col)) = physics_collider {
                let valid = match col {
                    crate::character::BoneColliderMeta::Capsule { radius, height } => {
                        radius.is_finite() && *radius > 0.0 && height.is_finite() && *height > 0.0
                    }
                    crate::character::BoneColliderMeta::Box { half_extents } => {
                        half_extents.iter().all(|v| v.is_finite() && *v > 0.0)
                    }
                };
                if !valid {
                    return Err(CoreError::OutOfBounds);
                }
            }
            let prev = scene.character_rig.clone();
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            let bone = rig.bone_mut(*bone_id).ok_or(CoreError::OutOfBounds)?;
            if let Some(lim) = rotation_limits_deg {
                bone.rotation_limits_deg = *lim;
            }
            if let Some(v) = inherit_rotation {
                bone.inherit_rotation = *v;
            }
            if let Some(v) = inherit_scale {
                bone.inherit_scale = *v;
            }
            if let Some(v) = connected {
                bone.connected = *v;
            }
            if let Some(v) = deform {
                bone.deform = *v;
            }
            if let Some(v) = spring_stiffness {
                bone.spring_stiffness = *v;
            }
            if let Some(v) = spring_damping {
                bone.spring_damping = *v;
            }
            if let Some(v) = lod_level {
                bone.lod_level = *v;
            }
            if let Some(c) = physics_collider {
                bone.physics_collider = c.clone();
            }
            UndoDelta::RestoreCharacterRig(prev)
        }
        Command::SetChainPole { end_bone_id, pole } => {
            let prev = scene.character_rig.clone();
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            if rig.bone(*end_bone_id).is_none() {
                return Err(CoreError::OutOfBounds);
            }
            match pole {
                Some(p) if p.is_finite() => {
                    rig.chain_poles.insert(*end_bone_id, *p);
                }
                Some(_) => return Err(CoreError::OutOfBounds),
                None => {
                    rig.chain_poles.remove(end_bone_id);
                }
            }
            UndoDelta::RestoreCharacterRig(prev)
        }
        Command::UpsertRigConstraint { constraint } => {
            let prev = scene.character_rig.clone();
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            let cname = match constraint {
                crate::character::BoneConstraint::IkFkBlend {
                    name,
                    root,
                    mid,
                    end,
                    target,
                    pole,
                    weight,
                } => {
                    if name.trim().is_empty()
                        || !target.is_finite()
                        || !weight.is_finite()
                        || pole.is_some_and(|p| !p.is_finite())
                        || rig.bone(*root).is_none()
                        || rig.bone(*mid).is_none()
                        || rig.bone(*end).is_none()
                    {
                        return Err(CoreError::OutOfBounds);
                    }
                    name.as_str()
                }
                crate::character::BoneConstraint::LookAt {
                    name,
                    bone_id,
                    target,
                    up,
                    influence,
                } => {
                    if name.trim().is_empty()
                        || !target.is_finite()
                        || !up.is_finite()
                        || !influence.is_finite()
                        || rig.bone(*bone_id).is_none()
                    {
                        return Err(CoreError::OutOfBounds);
                    }
                    name.as_str()
                }
                crate::character::BoneConstraint::CopyTransform {
                    name,
                    bone_id,
                    source_bone_id,
                    influence,
                } => {
                    if name.trim().is_empty()
                        || !influence.is_finite()
                        || rig.bone(*bone_id).is_none()
                        || rig.bone(*source_bone_id).is_none()
                    {
                        return Err(CoreError::OutOfBounds);
                    }
                    name.as_str()
                }
            };
            if let Some(existing) = rig.constraints.iter_mut().find(|c| {
                let n = match c {
                    crate::character::BoneConstraint::IkFkBlend { name, .. }
                    | crate::character::BoneConstraint::LookAt { name, .. }
                    | crate::character::BoneConstraint::CopyTransform { name, .. } => name.as_str(),
                };
                n == cname
            }) {
                *existing = constraint.clone();
            } else {
                rig.constraints.push(constraint.clone());
            }
            crate::character::evaluate_rig_constraints(rig);
            UndoDelta::RestoreCharacterRig(prev)
        }
        Command::UpsertRigSocket { socket } => {
            let prev = scene.character_rig.clone();
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            if rig.bone(socket.bone_id).is_none() {
                return Err(CoreError::OutOfBounds);
            }
            if let Some(existing) = rig.sockets.iter_mut().find(|s| s.name == socket.name) {
                *existing = socket.clone();
            } else {
                rig.sockets.push(socket.clone());
            }
            UndoDelta::RestoreCharacterRig(prev)
        }
        Command::UpsertBoneGroup { group } => {
            let prev = scene.character_rig.clone();
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            rig.bone_groups.insert(group.name.clone(), group.clone());
            UndoDelta::RestoreCharacterRig(prev)
        }
        Command::SavePosePreset { name } => {
            let prev = scene.character_rig.clone();
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            let snapshot = rig.bones.iter().map(|b| (b.id, b.pose_local)).collect();
            rig.pose_library.insert(name.clone(), snapshot);
            UndoDelta::RestoreCharacterRig(prev)
        }
        Command::ApplyPosePreset { name, flipped } => {
            let prev = scene.character_rig.clone();
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            let pose = rig
                .pose_library
                .get(name)
                .cloned()
                .ok_or(CoreError::OutOfBounds)?;
            for (bid, tr) in pose {
                if let Some(b) = rig.bone_mut(bid) {
                    b.pose_local = tr;
                }
            }
            if *flipped {
                crate::character::swap_rig_pose_lr(rig);
            }
            UndoDelta::RestoreCharacterRig(prev)
        }
        Command::MirrorRigPose { left_to_right } => {
            let prev = scene.character_rig.clone();
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            crate::character::mirror_rig_pose(rig, *left_to_right);
            UndoDelta::RestoreCharacterRig(prev)
        }
        Command::AddAnimationEvent { clip_index, event } => {
            let prev = scene.character_rig.clone();
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            let clip = rig
                .animations
                .get_mut(*clip_index)
                .ok_or(CoreError::OutOfBounds)?;
            if !event.time_secs.is_finite() || event.time_secs < 0.0 {
                return Err(CoreError::OutOfBounds);
            }
            clip.events.push(event.clone());
            clip.events
                .sort_by(|a, b| a.time_secs.total_cmp(&b.time_secs));
            UndoDelta::RestoreCharacterRig(prev)
        }
        Command::ApplyFootRoll {
            heel_id,
            ball_id,
            toe_id,
            roll_deg,
        } => {
            let prev = scene.character_rig.clone();
            let rig = scene
                .character_rig
                .as_mut()
                .ok_or(CoreError::CharacterRigNotFound)?;
            if !crate::character::apply_foot_roll(rig, *heel_id, *ball_id, *toe_id, *roll_deg) {
                return Err(CoreError::OutOfBounds);
            }
            UndoDelta::RestoreCharacterRig(prev)
        }
        _ => return Err(CoreError::OutOfBounds),
    };
    scene.mark_modified();
    Ok(delta)
}
