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
