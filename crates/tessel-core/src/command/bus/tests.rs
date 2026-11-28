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

#[test]
fn h1_post_process_and_material_contracts_validate_sync_and_undo() {
    let mut scene = SceneState::new();
    let mut history = crate::HistoryStack::default();

    // Command::SetPostProcess validates finite inputs, syncs npr_look.bloom_strength, and undoes cleanly.
    let bad_pp = crate::PostProcessSettings {
        exposure: f32::NAN,
        ..Default::default()
    };
    assert_eq!(
        history.execute(&mut scene, Command::SetPostProcess(bad_pp)),
        Err(CoreError::OutOfBounds)
    );

    let custom_pp = crate::PostProcessSettings {
        bloom_enabled: true,
        bloom_threshold: 0.85,
        bloom_intensity: 0.75,
        exposure: 1.4,
        aces_enabled: true,
        fog_enabled: true,
        fog_color: [0.1, 0.2, 0.3],
        fog_density: 0.05,
        fog_start: 4.0,
        fog_height_falloff: 0.4,
    };
    history
        .execute(&mut scene, Command::SetPostProcess(custom_pp))
        .unwrap();
    assert_eq!(scene.post_process, custom_pp);
    assert!((scene.npr_look.bloom_strength - 0.75).abs() < 1e-6);

    history.undo(&mut scene).unwrap();
    assert_eq!(scene.post_process, crate::PostProcessSettings::default());
    assert_eq!(scene.npr_look.bloom_strength, 0.0);

    // Legacy SetNprLookParams stays synced with post_process bloom fields.
    history
        .execute(
            &mut scene,
            Command::SetNprLookParams {
                params: crate::NprLookParams {
                    hatching_strength: 0.2,
                    bloom_strength: 1.1,
                    msaa_samples: 4,
                },
            },
        )
        .unwrap();
    assert!(scene.post_process.bloom_enabled);
    assert!((scene.post_process.bloom_intensity - 1.1).abs() < 1e-6);
    history.undo(&mut scene).unwrap();
    assert!(!scene.post_process.bloom_enabled);

    // PaletteEntry & MeshMaterial reject NaN and clamp HDR emissive / transmission / IOR.
    let idx = PaletteIndex::new(5).unwrap();
    let mut bad_pal = crate::PaletteEntry::matte("Bad", crate::Rgba8::rgb(255, 0, 0));
    bad_pal.ior = f32::INFINITY;
    assert_eq!(
        history.execute(
            &mut scene,
            Command::SetPaletteEntry {
                index: idx,
                entry: bad_pal,
            }
        ),
        Err(CoreError::OutOfBounds)
    );
    let mut neon_glass = crate::PaletteEntry::matte("NeonGlass", crate::Rgba8::rgb(80, 220, 255));
    neon_glass.emissive = 25.0;
    neon_glass.transmission = 1.5;
    neon_glass.ior = 0.5;
    history
        .execute(
            &mut scene,
            Command::SetPaletteEntry {
                index: idx,
                entry: neon_glass,
            },
        )
        .unwrap();
    let saved = scene.palette.get(idx).unwrap();
    assert_eq!(saved.emissive, 20.0);
    assert_eq!(saved.transmission, 1.0);
    assert_eq!(saved.ior, 1.0);

    history
        .execute(
            &mut scene,
            Command::CreatePrimitiveMesh {
                name: "Gem".into(),
                kind: crate::PrimitiveKind::Cube,
                size: 1.0,
                material: idx,
            },
        )
        .unwrap();
    let mesh_id = scene.selected_node.unwrap();
    let bad_mat = crate::MeshMaterial {
        emissive_intensity: f32::NAN,
        ..Default::default()
    };
    assert_eq!(
        history.execute(
            &mut scene,
            Command::SetMeshMaterial {
                node_id: mesh_id,
                material: bad_mat,
            }
        ),
        Err(CoreError::OutOfBounds)
    );
    history
        .execute(
            &mut scene,
            Command::SetMeshMaterial {
                node_id: mesh_id,
                material: crate::MeshMaterial {
                    emissive: [0.2, 0.9, 1.0],
                    emissive_intensity: 50.0,
                    transmission: 0.9,
                    ior: 2.8,
                    ..Default::default()
                },
            },
        )
        .unwrap();
    let mat = scene.lowpoly_meshes[&mesh_id].material;
    assert_eq!(mat.emissive_intensity, 20.0);
    assert!((mat.transmission - 0.9).abs() < 1e-6);
    assert_eq!(mat.ior, 2.5);
}

#[test]
fn pixel_canvas_4k_bounds_and_resize_undo_redo() {
    use crate::pixel::{PixelCoord, MAX_PIXEL_CANVAS_DIM};
    use crate::HistoryStack;

    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();

    // Exceeding 4096 or 0 must fail cleanly.
    assert_eq!(
        history.execute(
            &mut scene,
            Command::CreatePixelCanvas {
                name: "TooWide".into(),
                width: MAX_PIXEL_CANVAS_DIM + 1,
                height: 16,
            },
        ),
        Err(CoreError::OutOfBounds)
    );
    assert_eq!(
        history.execute(
            &mut scene,
            Command::CreatePixelCanvas {
                name: "Zero".into(),
                width: 0,
                height: 16,
            },
        ),
        Err(CoreError::OutOfBounds)
    );

    // 4096-wide canvas is supported.
    history
        .execute(
            &mut scene,
            Command::CreatePixelCanvas {
                name: "Wide4K".into(),
                width: MAX_PIXEL_CANVAS_DIM,
                height: 4,
            },
        )
        .unwrap();
    let wide_id = scene.selected_node.unwrap();
    assert_eq!(scene.pixel_canvases[&wide_id].width, 4096);
    assert_eq!(scene.pixel_canvases[&wide_id].height, 4);

    // Create a small canvas, paint inside and outside crop region, add a 2nd frame, then resize.
    history
        .execute(
            &mut scene,
            Command::CreatePixelCanvas {
                name: "Resizable".into(),
                width: 8,
                height: 8,
            },
        )
        .unwrap();
    let id = scene.selected_node.unwrap();
    let layer_id = scene.pixel_canvases[&id].frames[0].layers[0].id;
    let color = PaletteIndex::new(4).unwrap();
    history
        .execute(
            &mut scene,
            Command::DrawPixels {
                node_id: id,
                frame: FrameIndex(0),
                layer_id,
                edits: vec![
                    PixelEdit {
                        coord: PixelCoord::new(1, 2),
                        color: Some(color),
                    },
                    PixelEdit {
                        coord: PixelCoord::new(6, 7),
                        color: Some(color),
                    },
                ],
            },
        )
        .unwrap();
    history
        .execute(&mut scene, Command::AddPixelFrame { node_id: id })
        .unwrap();

    // Grow to 16x12: both pixels and both frames stay intact.
    history
        .execute(
            &mut scene,
            Command::ResizePixelCanvas {
                node_id: id,
                width: 16,
                height: 12,
            },
        )
        .unwrap();
    {
        let c = &scene.pixel_canvases[&id];
        assert_eq!((c.width, c.height), (16, 12));
        assert_eq!(c.frames.len(), 2);
        assert_eq!(
            c.frames[0].layers[0].get(PixelCoord::new(1, 2)),
            Some(color)
        );
        assert_eq!(
            c.frames[0].layers[0].get(PixelCoord::new(6, 7)),
            Some(color)
        );
        assert_eq!(c.frames[1].layers[0].width(), 16);
        assert_eq!(c.frames[1].layers[0].height(), 12);
    }

    // Shrink to 4x4: (1,2) survives, (6,7) is cropped.
    history
        .execute(
            &mut scene,
            Command::ResizePixelCanvas {
                node_id: id,
                width: 4,
                height: 4,
            },
        )
        .unwrap();
    {
        let c = &scene.pixel_canvases[&id];
        assert_eq!((c.width, c.height), (4, 4));
        assert_eq!(
            c.frames[0].layers[0].get(PixelCoord::new(1, 2)),
            Some(color)
        );
        assert_eq!(c.frames[0].layers[0].get(PixelCoord::new(6, 7)), None);
    }

    // Out-of-bounds resize fails without mutating canvas.
    assert_eq!(
        history.execute(
            &mut scene,
            Command::ResizePixelCanvas {
                node_id: id,
                width: MAX_PIXEL_CANVAS_DIM + 1,
                height: 4,
            },
        ),
        Err(CoreError::OutOfBounds)
    );
    assert_eq!(scene.pixel_canvases[&id].width, 4);

    // Undo restores 16x12 and cropped pixel (6,7).
    history.undo(&mut scene).unwrap();
    {
        let c = &scene.pixel_canvases[&id];
        assert_eq!((c.width, c.height), (16, 12));
        assert_eq!(
            c.frames[0].layers[0].get(PixelCoord::new(6, 7)),
            Some(color)
        );
    }

    // Redo reapplies 4x4 shrink.
    history.redo(&mut scene).unwrap();
    assert_eq!(scene.pixel_canvases[&id].width, 4);
    assert_eq!(scene.pixel_canvases[&id].height, 4);
}

#[test]
fn flood_fill_pixels_uses_compact_rle_undo_and_roundtrips_undo_redo() {
    use crate::pixel::{FrameIndex, PixelCoord};

    let mut scene = SceneState::new();
    let mut history = crate::HistoryStack::default();
    history
        .execute(
            &mut scene,
            Command::CreatePixelCanvas {
                name: "Fill4KTest".into(),
                width: 64,
                height: 64,
            },
        )
        .unwrap();
    let id = scene.active_pixel_node().unwrap();
    let layer_id = scene.pixel_canvases[&id].frames[0].layers[0].id;
    let fill_color = PaletteIndex::new(3).unwrap();

    // Direct apply verifies compact RLE undo delta (1 run for empty layer).
    let delta = (Command::FloodFillPixels {
        node_id: id,
        frame: FrameIndex(0),
        layer_id,
        start: PixelCoord::new(0, 0),
        color: Some(fill_color),
    })
    .apply(&mut scene)
    .unwrap();
    match &delta {
        UndoDelta::RestorePixelLayerRle { rle_pixels, .. } => {
            assert_eq!(rle_pixels.len(), 1);
            assert_eq!(rle_pixels[0], (64 * 64, None));
        }
        other => panic!("expected RestorePixelLayerRle, got {other:?}"),
    }
    assert_eq!(
        scene.pixel_canvases[&id].frames[0].layers[0].get(PixelCoord::new(63, 63)),
        Some(fill_color)
    );
    delta.revert(&mut scene).unwrap();
    assert_eq!(
        scene.pixel_canvases[&id].frames[0].layers[0].get(PixelCoord::new(63, 63)),
        None
    );

    // HistoryStack execute -> undo -> redo roundtrip + OOB rejection.
    history
        .execute(
            &mut scene,
            Command::FloodFillPixels {
                node_id: id,
                frame: FrameIndex(0),
                layer_id,
                start: PixelCoord::new(10, 10),
                color: Some(fill_color),
            },
        )
        .unwrap();
    assert_eq!(
        scene.pixel_canvases[&id].frames[0].layers[0].get(PixelCoord::new(0, 0)),
        Some(fill_color)
    );
    history.undo(&mut scene).unwrap();
    assert_eq!(
        scene.pixel_canvases[&id].frames[0].layers[0].get(PixelCoord::new(0, 0)),
        None
    );
    history.redo(&mut scene).unwrap();
    assert_eq!(
        scene.pixel_canvases[&id].frames[0].layers[0].get(PixelCoord::new(0, 0)),
        Some(fill_color)
    );
    assert_eq!(
        history.execute(
            &mut scene,
            Command::FloodFillPixels {
                node_id: id,
                frame: FrameIndex(0),
                layer_id,
                start: PixelCoord::new(64, 0),
                color: Some(fill_color),
            },
        ),
        Err(CoreError::OutOfBounds)
    );
}
