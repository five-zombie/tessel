use super::*;
use crate::scene::MAX_NODE_NAME_LEN;

#[test]
fn rename_and_duplicate_enforce_node_name_bounds() {
    let mut scene = SceneState::new();
    let mut history = crate::HistoryStack::default();
    history
        .execute(
            &mut scene,
            Command::CreatePrimitiveMesh {
                name: "Cube".into(),
                kind: crate::PrimitiveKind::Cube,
                size: 1.0,
                material: PaletteIndex::DEFAULT_ACTIVE,
            },
        )
        .unwrap();
    let nid = scene.selected_node.unwrap();

    let too_long = "x".repeat(MAX_NODE_NAME_LEN + 1);
    assert!(matches!(
        history.execute(
            &mut scene,
            Command::CreatePrimitiveMesh {
                name: too_long.clone(),
                kind: crate::PrimitiveKind::Cube,
                size: 1.0,
                material: PaletteIndex::DEFAULT_ACTIVE,
            },
        ),
        Err(CoreError::OutOfBounds)
    ));
    assert!(matches!(
        history.execute(
            &mut scene,
            Command::CreateGroupNode {
                name: "   ".into(),
                transform: Transform::default(),
                parent: None,
            },
        ),
        Err(CoreError::OutOfBounds)
    ));
    assert!(matches!(
        history.execute(
            &mut scene,
            Command::RenameNode {
                node_id: nid,
                name: too_long.clone(),
            },
        ),
        Err(CoreError::OutOfBounds)
    ));
    assert!(matches!(
        history.execute(
            &mut scene,
            Command::DuplicateNode {
                node_id: nid,
                name: Some(too_long),
            },
        ),
        Err(CoreError::OutOfBounds)
    ));

    let max_name = "m".repeat(MAX_NODE_NAME_LEN);
    history
        .execute(
            &mut scene,
            Command::RenameNode {
                node_id: nid,
                name: max_name,
            },
        )
        .unwrap();
    history
        .execute(
            &mut scene,
            Command::DuplicateNode {
                node_id: nid,
                name: None,
            },
        )
        .unwrap();
    let copy_id = scene.selected_node.unwrap();
    assert!(scene.nodes[&copy_id].name.chars().count() <= MAX_NODE_NAME_LEN);
    assert!(scene.nodes[&copy_id].name.ends_with(" Copy"));
}

#[test]
fn phase_i_audit_rejects_inverted_rotation_limits_and_invalid_constraints() {
    let mut scene = SceneState::new();
    let mut history = crate::HistoryStack::default();
    history.execute(&mut scene, Command::ApplyBipedRig).unwrap();
    let bone_id = scene.character_rig.as_ref().unwrap().bones[0].id;

    assert!(matches!(
        history.execute(
            &mut scene,
            Command::SetBoneProperties {
                bone_id,
                rotation_limits_deg: Some(Some(([30.0, 0.0, 0.0], [-30.0, 0.0, 0.0]))),
                inherit_rotation: None,
                inherit_scale: None,
                connected: None,
                deform: None,
                spring_stiffness: None,
                spring_damping: None,
                lod_level: None,
                physics_collider: None,
            },
        ),
        Err(CoreError::OutOfBounds)
    ));

    assert!(matches!(
        history.execute(
            &mut scene,
            Command::UpsertRigConstraint {
                constraint: crate::BoneConstraint::LookAt {
                    name: "bad".into(),
                    bone_id: crate::BoneId(9999),
                    target: glam::Vec3::new(0.0, 1.0, 1.0),
                    up: glam::Vec3::Y,
                    influence: 1.0,
                },
            },
        ),
        Err(CoreError::OutOfBounds)
    ));
}

#[test]
fn phase_i_audit_delete_node_cleans_and_restores_metadata() {
    let mut scene = SceneState::new();
    let mut history = crate::HistoryStack::default();
    history
        .execute(
            &mut scene,
            Command::CreatePrimitiveMesh {
                name: "Prop".into(),
                kind: crate::PrimitiveKind::Cube,
                size: 1.0,
                material: PaletteIndex::DEFAULT_ACTIVE,
            },
        )
        .unwrap();
    let nid = scene.selected_node.unwrap();
    let mut extras = std::collections::BTreeMap::new();
    extras.insert("tag".into(), "hero".into());
    history
        .execute(
            &mut scene,
            Command::SetNodeExtras {
                node_id: nid,
                extras,
            },
        )
        .unwrap();
    history
        .execute(
            &mut scene,
            Command::SetCollisionHull {
                node_id: nid,
                hull: Some(crate::CollisionHull::Box {
                    half_extents: [0.5, 0.5, 0.5],
                    offset: [0.0, 0.0, 0.0],
                }),
            },
        )
        .unwrap();
    assert!(scene.node_extras.contains_key(&nid));
    assert!(scene.collision_hulls.contains_key(&nid));

    history
        .execute(&mut scene, Command::DeleteNode(nid))
        .unwrap();
    assert!(!scene.node_extras.contains_key(&nid));
    assert!(!scene.collision_hulls.contains_key(&nid));

    history.undo(&mut scene).unwrap();
    assert_eq!(
        scene.node_extras.get(&nid).and_then(|m| m.get("tag")),
        Some(&"hero".to_string())
    );
    assert!(scene.collision_hulls.contains_key(&nid));
}

#[test]
fn phase_i_audit_apply_pose_preset_flipped_swaps_lr_and_spring_bake_is_stable() {
    let mut scene = SceneState::new();
    let mut history = crate::HistoryStack::default();
    history.execute(&mut scene, Command::ApplyBipedRig).unwrap();

    let (left_arm, right_arm) = {
        let rig = scene.character_rig.as_ref().unwrap();
        let l = rig.bones.iter().find(|b| b.name == "LeftArm").unwrap().id;
        let r = rig.bones.iter().find(|b| b.name == "RightArm").unwrap().id;
        (l, r)
    };

    let raised = glam::Quat::from_rotation_x(0.8);
    history
        .execute(
            &mut scene,
            Command::SetBoneRotation {
                bone_id: left_arm,
                rotation: raised,
            },
        )
        .unwrap();
    history
        .execute(
            &mut scene,
            Command::SavePosePreset {
                name: "Wave".into(),
            },
        )
        .unwrap();
    history
        .execute(
            &mut scene,
            Command::ApplyPosePreset {
                name: "Wave".into(),
                flipped: true,
            },
        )
        .unwrap();
    let rig = scene.character_rig.as_ref().unwrap();
    let right_rot = rig.bone(right_arm).unwrap().pose_local.rotation;
    assert!(
        (right_rot.w - raised.w).abs() < 0.05 || (right_rot.w + raised.w).abs() < 0.05,
        "flipped apply should transfer left pose onto right bone"
    );

    history
        .execute(
            &mut scene,
            Command::SetBoneProperties {
                bone_id: left_arm,
                rotation_limits_deg: None,
                inherit_rotation: None,
                inherit_scale: None,
                connected: None,
                deform: None,
                spring_stiffness: Some(200.0),
                spring_damping: Some(0.1),
                lod_level: None,
                physics_collider: None,
            },
        )
        .unwrap();
    let baked = crate::bake_spring_bones(scene.character_rig.as_ref().unwrap(), 0, 1.0).unwrap();
    for ch in &baked.channels {
        for kf in &ch.keyframes {
            assert!(kf.rotation.is_finite());
            assert!(kf.translation.is_finite());
            assert!(kf.scale.is_finite());
        }
    }
}

#[test]
fn g1_set_node_rig_skinned_undo_redo_delete_duplicate_and_serde_compat() {
    let mut scene = SceneState::new();
    let mut history = crate::HistoryStack::default();
    history
        .execute(
            &mut scene,
            Command::CreatePrimitiveMesh {
                name: "Floor".into(),
                kind: crate::PrimitiveKind::Cube,
                size: 1.0,
                material: PaletteIndex::DEFAULT_ACTIVE,
            },
        )
        .unwrap();
    let floor_id = scene.selected_node.unwrap();

    // Fails cleanly when no rig exists
    assert!(matches!(
        history.execute(
            &mut scene,
            Command::SetNodeRigSkinned {
                node_id: floor_id,
                skinned: false,
            },
        ),
        Err(CoreError::CharacterRigNotFound)
    ));

    history.execute(&mut scene, Command::ApplyBipedRig).unwrap();
    assert!(scene
        .character_rig
        .as_ref()
        .unwrap()
        .should_skin_node(floor_id));

    // Non-3D geometry nodes (Group / PixelCanvas) must be rejected by SetNodeRigSkinned.
    history
        .execute(
            &mut scene,
            Command::CreateGroupNode {
                name: "Props".into(),
                transform: Transform::default(),
                parent: None,
            },
        )
        .unwrap();
    let group_id = scene.selected_node.unwrap();
    assert!(matches!(
        history.execute(
            &mut scene,
            Command::SetNodeRigSkinned {
                node_id: group_id,
                skinned: false,
            },
        ),
        Err(CoreError::OutOfBounds)
    ));

    // Opt out of rig skinning
    history
        .execute(
            &mut scene,
            Command::SetNodeRigSkinned {
                node_id: floor_id,
                skinned: false,
            },
        )
        .unwrap();
    assert!(!scene
        .character_rig
        .as_ref()
        .unwrap()
        .should_skin_node(floor_id));

    // Duplicate inherits unrigged state, and undoing duplicate cleans it up
    history
        .execute(
            &mut scene,
            Command::DuplicateNode {
                node_id: floor_id,
                name: Some("FloorCopy".into()),
            },
        )
        .unwrap();
    let copy_id = scene.selected_node.unwrap();
    assert!(!scene
        .character_rig
        .as_ref()
        .unwrap()
        .should_skin_node(copy_id));
    history.undo(&mut scene).unwrap();
    assert!(!scene
        .character_rig
        .as_ref()
        .unwrap()
        .unrigged_nodes
        .contains(&copy_id));

    // Undo opt-out restores skinned=true; Redo restores skinned=false
    history.undo(&mut scene).unwrap();
    assert!(scene
        .character_rig
        .as_ref()
        .unwrap()
        .should_skin_node(floor_id));
    history.redo(&mut scene).unwrap();
    assert!(!scene
        .character_rig
        .as_ref()
        .unwrap()
        .should_skin_node(floor_id));

    // DeleteNode cleans unrigged_nodes entry; Undo DeleteNode restores it
    history
        .execute(&mut scene, Command::DeleteNode(floor_id))
        .unwrap();
    assert!(!scene
        .character_rig
        .as_ref()
        .unwrap()
        .unrigged_nodes
        .contains(&floor_id));
    history.undo(&mut scene).unwrap();
    assert!(!scene
        .character_rig
        .as_ref()
        .unwrap()
        .should_skin_node(floor_id));

    // Switching rig presets preserves scene-level unrigged_nodes exclusions.
    history
        .execute(&mut scene, Command::ApplyHumanoidRig)
        .unwrap();
    assert!(!scene
        .character_rig
        .as_ref()
        .unwrap()
        .should_skin_node(floor_id));
}

#[test]
fn g3_set_character_rig_rename_and_reparent_bone_with_anti_cycle_and_undo() {
    let mut scene = SceneState::new();
    let mut history = crate::HistoryStack::default();

    // Empty custom rig is valid and undoable
    history
        .execute(
            &mut scene,
            Command::SetCharacterRig {
                rig: Some(crate::CharacterRig::default()),
            },
        )
        .unwrap();
    assert!(scene.character_rig.as_ref().unwrap().bones.is_empty());
    history.undo(&mut scene).unwrap();
    assert!(scene.character_rig.is_none());

    // Apply biped rig, rename bone, undo/redo
    history
        .execute(
            &mut scene,
            Command::SetCharacterRig {
                rig: Some(crate::CharacterRig::biped_template()),
            },
        )
        .unwrap();
    history
        .execute(
            &mut scene,
            Command::RenameBone {
                bone_id: crate::BoneId(2),
                name: "  Skull  ".into(),
            },
        )
        .unwrap();
    assert_eq!(
        scene
            .character_rig
            .as_ref()
            .unwrap()
            .bone(crate::BoneId(2))
            .unwrap()
            .name,
        "Skull"
    );
    history.undo(&mut scene).unwrap();
    assert_eq!(
        scene
            .character_rig
            .as_ref()
            .unwrap()
            .bone(crate::BoneId(2))
            .unwrap()
            .name,
        "Head"
    );
    // Reject empty/whitespace bone name
    assert!(matches!(
        history.execute(
            &mut scene,
            Command::RenameBone {
                bone_id: crate::BoneId(2),
                name: "   ".into(),
            },
        ),
        Err(CoreError::OutOfBounds)
    ));

    // ReparentBone anti-cycle: Root(0) cannot be reparented under Head(2) (its descendant) or itself
    assert!(matches!(
        history.execute(
            &mut scene,
            Command::ReparentBone {
                bone_id: crate::BoneId(0),
                parent: Some(crate::BoneId(2)),
            },
        ),
        Err(CoreError::OutOfBounds)
    ));
    assert!(matches!(
        history.execute(
            &mut scene,
            Command::ReparentBone {
                bone_id: crate::BoneId(1),
                parent: Some(crate::BoneId(1)),
            },
        ),
        Err(CoreError::OutOfBounds)
    ));

    // Valid reparent: move Torso(1) under LeftLeg(5) and verify topological re-sorting
    // keeps parent-before-child AND remaps any manual mesh.skin_weights joint indices (with Undo).
    history
        .execute(
            &mut scene,
            Command::CreatePrimitiveMesh {
                name: "TorsoArmor".into(),
                kind: crate::PrimitiveKind::Cube,
                size: 0.5,
                material: PaletteIndex::DEFAULT_ACTIVE,
            },
        )
        .unwrap();
    let armor_id = scene.selected_node.unwrap();
    // Before reparent, Torso is at joint index 1.
    history
        .execute(
            &mut scene,
            Command::SetMeshVertexWeights {
                node_id: armor_id,
                vertex_ids: vec![0],
                joints: [1, 0, 0, 0],
                weights: [1.0, 0.0, 0.0, 0.0],
            },
        )
        .unwrap();
    history
        .execute(
            &mut scene,
            Command::ReparentBone {
                bone_id: crate::BoneId(1),
                parent: Some(crate::BoneId(5)),
            },
        )
        .unwrap();
    let rig = scene.character_rig.as_ref().unwrap();
    let leg_pos = rig
        .bones
        .iter()
        .position(|b| b.id == crate::BoneId(5))
        .unwrap();
    let torso_pos = rig
        .bones
        .iter()
        .position(|b| b.id == crate::BoneId(1))
        .unwrap();
    assert!(
        leg_pos < torso_pos,
        "ReparentBone must topologically sort parent before child"
    );
    assert_eq!(
        scene.lowpoly_meshes[&armor_id].skin_weights[&0].joints[0], torso_pos as u16,
        "manual mesh skin_weights joint index must remap when ReparentBone reorders bones"
    );
    let world_rest = rig.world_rest_matrices();
    let leg_w = world_rest[&crate::BoneId(5)].transform_point3(glam::Vec3::ZERO);
    let torso_w = world_rest[&crate::BoneId(1)].transform_point3(glam::Vec3::ZERO);
    assert!(
        (torso_w - (leg_w + glam::Vec3::new(0.0, 0.3, 0.0))).length() < 1e-4,
        "world_rest_matrices must resolve reparented parent before child"
    );
    history.undo(&mut scene).unwrap();
    assert_eq!(
        scene
            .character_rig
            .as_ref()
            .unwrap()
            .bone(crate::BoneId(1))
            .unwrap()
            .parent,
        Some(crate::BoneId(0))
    );
    assert_eq!(
        scene.lowpoly_meshes[&armor_id].skin_weights[&0].joints[0], 1,
        "undoing ReparentBone must also restore original manual skin_weights joint indices"
    );

    // SetCharacterRig rejects cyclic or non-finite rigs
    let mut cyclic = crate::CharacterRig::biped_template();
    cyclic.bone_mut(crate::BoneId(0)).unwrap().parent = Some(crate::BoneId(2));
    assert!(matches!(
        history.execute(&mut scene, Command::SetCharacterRig { rig: Some(cyclic) }),
        Err(CoreError::OutOfBounds)
    ));
    let mut nan_rig = crate::CharacterRig::biped_template();
    nan_rig
        .bone_mut(crate::BoneId(0))
        .unwrap()
        .rest_local
        .translation
        .x = f32::NAN;
    assert!(matches!(
        history.execute(&mut scene, Command::SetCharacterRig { rig: Some(nan_rig) }),
        Err(CoreError::OutOfBounds)
    ));
    let mut neg_scale_rig = crate::CharacterRig::biped_template();
    neg_scale_rig
        .bone_mut(crate::BoneId(0))
        .unwrap()
        .rest_local
        .scale
        .x = -1.0;
    assert!(matches!(
        history.execute(
            &mut scene,
            Command::SetCharacterRig {
                rig: Some(neg_scale_rig)
            }
        ),
        Err(CoreError::OutOfBounds)
    ));
    let mut dangling_node_rig = crate::CharacterRig::biped_template();
    dangling_node_rig.unrigged_nodes.insert(NodeId(99_999));
    assert!(matches!(
        history.execute(
            &mut scene,
            Command::SetCharacterRig {
                rig: Some(dangling_node_rig)
            }
        ),
        Err(CoreError::NodeNotFound(NodeId(99_999)))
    ));
    let mut dup_channel_rig = crate::CharacterRig::biped_template();
    let mut clip = crate::AnimationClip::new_empty("DupChan", 1.0);
    clip.channels.push(crate::BoneChannel {
        bone_id: crate::BoneId(0),
        keyframes: vec![crate::Keyframe::rotation_only(0.0, glam::Quat::IDENTITY)],
    });
    clip.channels.push(crate::BoneChannel {
        bone_id: crate::BoneId(0),
        keyframes: vec![crate::Keyframe::rotation_only(0.5, glam::Quat::IDENTITY)],
    });
    dup_channel_rig.animations.push(clip);
    assert!(matches!(
        history.execute(
            &mut scene,
            Command::SetCharacterRig {
                rig: Some(dup_channel_rig)
            }
        ),
        Err(CoreError::OutOfBounds)
    ));
}
