use super::helpers::{expand_mixed_step_keyframes, has_group_ancestor, pack_cubicspline};
use super::writer::export_glb_bytes;
use crate::ExportPreset;
use glam::Vec3;
use serde_json::Value;
use tessel_core::{Command, PaletteIndex, PrimitiveKind, Rgba8, SceneState, VoxelCoord};

#[test]
fn glb_export_embeds_painted_mesh_rgba_texture() {
    let mut scene = SceneState::new();
    Command::CreatePrimitiveMesh {
        name: "PaintedCube".into(),
        kind: PrimitiveKind::Cube,
        size: 1.0,
        material: PaletteIndex::new(1).unwrap(),
    }
    .apply(&mut scene)
    .unwrap();
    let node_id = scene.active_lowpoly_node().unwrap();
    Command::PaintMeshTexture {
        node_id,
        uv: [0.5, 0.75],
        radius_px: 12.0,
        color: Rgba8::rgb(255, 255, 255),
    }
    .apply(&mut scene)
    .unwrap();

    let (glb, tris) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
    assert_eq!(tris, 12);
    let parsed = gltf::Gltf::from_slice(&glb).unwrap();
    assert_eq!(parsed.images().count(), 1);
    assert_eq!(parsed.materials().count(), 1);

    // Verify PBR metallic, roughness, emissive, and doubleSided factors export cleanly into glTF 2.0
    Command::SetMeshMaterial {
        node_id,
        material: tessel_core::MeshMaterial {
            metallic: 0.9,
            roughness: 0.2,
            emissive: [0.1, 0.5, 0.8],
            double_sided: true,
            ..Default::default()
        },
    }
    .apply(&mut scene)
    .unwrap();
    let (glb_pbr, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
    let parsed_pbr = gltf::Gltf::from_slice(&glb_pbr).unwrap();
    let mat = parsed_pbr.materials().next().unwrap();
    assert!((mat.pbr_metallic_roughness().metallic_factor() - 0.9).abs() < 1e-4);
    assert!(mat.double_sided());
    assert!((mat.pbr_metallic_roughness().roughness_factor() - 0.2).abs() < 1e-4);
    assert!((mat.emissive_factor()[2] - 0.8).abs() < 1e-4);

    // Verify Normal Map embeds as glTF normalTexture
    Command::SetMeshNormalMap {
        node_id,
        normal_map: Some(tessel_core::MeshTexture::solid(
            16,
            16,
            Rgba8::rgb(128, 128, 255),
        )),
    }
    .apply(&mut scene)
    .unwrap();
    let (glb_norm, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
    let parsed_norm = gltf::Gltf::from_slice(&glb_norm).unwrap();
    assert!(parsed_norm
        .materials()
        .next()
        .unwrap()
        .normal_texture()
        .is_some());

    // Verify AO Map embeds as glTF occlusionTexture (shares TEXCOORD_0 when base texture exists)
    Command::SetMeshAoMap {
        node_id,
        ao_map: Some(tessel_core::MeshTexture::solid(
            16,
            16,
            Rgba8::rgb(200, 200, 200),
        )),
    }
    .apply(&mut scene)
    .unwrap();
    let (glb_ao, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
    let parsed_ao = gltf::Gltf::from_slice(&glb_ao).unwrap();
    let occ = parsed_ao
        .materials()
        .next()
        .unwrap()
        .occlusion_texture()
        .expect("expected glTF occlusionTexture");
    assert_eq!(occ.tex_coord(), 0);

    // When base texture is cleared (palette UV on TEXCOORD_0), occlusionTexture uses TEXCOORD_1
    Command::SetMeshTexture {
        node_id,
        texture: None,
    }
    .apply(&mut scene)
    .unwrap();
    let (glb_ao_pal, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
    let parsed_ao_pal = gltf::Gltf::from_slice(&glb_ao_pal).unwrap();
    let occ_pal = parsed_ao_pal
        .materials()
        .next()
        .unwrap()
        .occlusion_texture()
        .expect("expected glTF occlusionTexture with palette base");
    assert_eq!(occ_pal.tex_coord(), 1);
}

#[test]
fn glb_export_passes_gltf_parser_validation_with_animations() {
    let mut scene = SceneState::new();
    Command::CreateVoxelVolume {
        name: "Hero".into(),
    }
    .apply(&mut scene)
    .unwrap();
    let id = scene.active_voxel_node().unwrap();
    Command::FillVoxelBox {
        node_id: id,
        min: VoxelCoord::new(0, 0, 0),
        max: VoxelCoord::new(1, 2, 1),
        color: Some(PaletteIndex::new(4).unwrap()),
        hollow: false,
    }
    .apply(&mut scene)
    .unwrap();
    Command::ApplyBipedRig.apply(&mut scene).unwrap();

    let (glb, tris) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
    assert_eq!(tris, 12);

    let parsed = gltf::Gltf::from_slice(&glb).unwrap();
    assert_eq!(parsed.meshes().count(), 1);
    assert_eq!(parsed.materials().count(), 1);
    assert_eq!(parsed.images().count(), 1);
    assert_eq!(parsed.animations().count(), 2); // Idle + Walk
    assert_eq!(parsed.skins().count(), 1);
    let skin = parsed.skins().next().unwrap();
    assert_eq!(skin.joints().count(), 7);
    assert!(skin.inverse_bind_matrices().is_some());
    assert!(parsed.nodes().next().unwrap().skin().is_some());
    assert!(parsed.nodes().count() >= 8); // Mesh + 7 biped bones
    let blob = parsed.blob.as_ref().unwrap();
    let prim = parsed.meshes().next().unwrap().primitives().next().unwrap();
    let reader = prim.reader(|_| Some(blob.as_slice()));
    let joints: Vec<[u16; 4]> = reader.read_joints(0).unwrap().into_u16().collect();
    let weights: Vec<[f32; 4]> = reader.read_weights(0).unwrap().into_f32().collect();
    assert!(!joints.is_empty());
    assert_eq!(joints.len(), weights.len());
    for w in &weights {
        let sum: f32 = w.iter().sum();
        assert!((sum - 1.0).abs() < 1e-4, "weights must sum to 1.0: {w:?}");
    }
}

#[test]
fn glb_export_writes_translation_and_scale_channels_for_trs_keyframes() {
    use tessel_core::{BoneId, Keyframe};

    let mut scene = SceneState::new();
    Command::CreateVoxelVolume {
        name: "Hero".into(),
    }
    .apply(&mut scene)
    .unwrap();
    let id = scene.active_voxel_node().unwrap();
    Command::FillVoxelBox {
        node_id: id,
        min: tessel_core::VoxelCoord::new(0, 0, 0),
        max: tessel_core::VoxelCoord::new(0, 0, 0),
        color: Some(PaletteIndex::new(4).unwrap()),
        hollow: false,
    }
    .apply(&mut scene)
    .unwrap();
    Command::ApplyBipedRig.apply(&mut scene).unwrap();
    Command::AddAnimationClip {
        name: "Bounce".into(),
        duration_secs: 1.0,
    }
    .apply(&mut scene)
    .unwrap();
    Command::SetAnimationKeyframe {
        clip_index: 2, // Idle + Walk ship with the rig
        bone_id: BoneId(1),
        keyframe: Keyframe {
            time_secs: 0.5,
            translation: Vec3::new(0.0, 0.1, 0.0),
            rotation: glam::Quat::IDENTITY,
            scale: Vec3::new(1.0, 1.0, 2.0),
            interpolation: Default::default(),
        },
    }
    .apply(&mut scene)
    .unwrap();

    let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
    let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let json: serde_json::Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();
    let animations = json["animations"].as_array().unwrap();
    let bounce = animations
        .iter()
        .find(|a| a["name"] == "Bounce")
        .expect("Bounce clip must export");
    let mut paths: Vec<&str> = bounce["channels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["target"]["path"].as_str().unwrap())
        .collect();
    paths.sort_unstable();
    assert_eq!(paths, vec!["rotation", "scale", "translation"]);

    // The built-in Idle clip breathes via translation offsets: those were
    // silently dropped before and must now export as absolute values.
    let idle = animations
        .iter()
        .find(|a| a["name"] == "Idle")
        .expect("Idle clip must export");
    let idle_paths: Vec<&str> = idle["channels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["target"]["path"].as_str().unwrap())
        .collect();
    assert!(idle_paths.contains(&"translation"));
    // No scale keyframes exist anywhere in the starter clips.
    assert!(!idle_paths.contains(&"scale"));

    // Rotation-only Walk stays rotation-only (skip rule).
    let walk = animations
        .iter()
        .find(|a| a["name"] == "Walk")
        .expect("Walk clip must export");
    let walk_paths: Vec<&str> = walk["channels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["target"]["path"].as_str().unwrap())
        .collect();
    assert!(walk_paths.iter().all(|p| *p == "rotation"));
}

#[test]
fn glb_export_prefers_manual_weights_over_auto_solver() {
    let mut scene = SceneState::new();
    Command::CreatePrimitiveMesh {
        name: "PaintedCube".into(),
        kind: PrimitiveKind::Cube,
        size: 1.0,
        material: PaletteIndex::new(1).unwrap(),
    }
    .apply(&mut scene)
    .unwrap();
    let node_id = scene.active_lowpoly_node().unwrap();
    Command::SetMeshVertexWeights {
        node_id,
        vertex_ids: (0..8).collect(),
        joints: [3, 0, 0, 0],
        weights: [1.0, 0.0, 0.0, 0.0],
    }
    .apply(&mut scene)
    .unwrap();
    Command::ApplyBipedRig.apply(&mut scene).unwrap();

    let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
    let parsed = gltf::Gltf::from_slice(&glb).unwrap();
    let blob = parsed.blob.as_ref().unwrap();
    let prim = parsed.meshes().next().unwrap().primitives().next().unwrap();
    let reader = prim.reader(|_| Some(blob.as_slice()));
    let joints: Vec<[u16; 4]> = reader.read_joints(0).unwrap().into_u16().collect();
    let weights: Vec<[f32; 4]> = reader.read_weights(0).unwrap().into_f32().collect();
    assert!(!joints.is_empty());
    assert_eq!(joints.len(), weights.len());
    for (j, w) in joints.iter().zip(weights.iter()) {
        assert_eq!(*j, [3, 0, 0, 0]);
        assert_eq!(*w, [1.0, 0.0, 0.0, 0.0]);
    }
}

#[test]
fn glb_export_writes_morph_targets_for_morphed_mesh() {
    let mut scene = SceneState::new();
    Command::CreatePrimitiveMesh {
        name: "Smiler".into(),
        kind: PrimitiveKind::Cube,
        size: 1.0,
        material: PaletteIndex::new(1).unwrap(),
    }
    .apply(&mut scene)
    .unwrap();
    let node_id = scene.active_lowpoly_node().unwrap();
    Command::AddMorphDelta {
        node_id,
        name: "Smile".into(),
        vertex_ids: vec![0],
        delta: Vec3::new(0.0, 0.5, 0.0),
    }
    .apply(&mut scene)
    .unwrap();

    let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
    // The binary must stay parseable with morphs present.
    gltf::Gltf::from_slice(&glb).unwrap();
    let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let json: serde_json::Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();
    let mesh_json = json["meshes"].as_array().unwrap().last().unwrap();
    let prim = mesh_json["primitives"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p.get("targets").is_some())
        .expect("a primitive with morph targets must export");
    let targets = prim["targets"].as_array().unwrap();
    assert_eq!(targets.len(), 1);
    assert_eq!(mesh_json["weights"], serde_json::json!([0.0]));

    // Target deltas land on the triangulated corners of source vertex 0.
    let mesh = scene.lowpoly_meshes.get(&node_id).unwrap();
    let tri = mesh.to_triangulated_flat();
    let expected: usize = tri.source_indices.iter().filter(|s| **s == 0).count();
    assert!(expected > 0);
    let acc_idx = targets[0]["POSITION"].as_u64().unwrap() as usize;
    let acc = json["accessors"].as_array().unwrap()[acc_idx].clone();
    assert_eq!(acc["count"], tri.vertices.len());
    assert_eq!(acc["type"], "VEC3");
    // Sum of absolute y deltas equals |delta.y| * corners sharing vertex 0.
    // BIN data starts right after the 12-byte header + 8-byte JSON chunk
    // header + padded JSON (BIN chunk header is 8 bytes).
    let bv = acc["bufferView"].as_u64().unwrap() as usize;
    let bvs = json["bufferViews"].as_array().unwrap();
    let byte_offset: usize = bvs[bv]["byteOffset"].as_u64().unwrap_or(0) as usize;
    let bin_start = 12 + 8 + json_len + 8;
    let mut sum = 0.0f32;
    for i in 0..tri.vertices.len() {
        let off = bin_start + byte_offset + i * 12 + 4; // y component
        let y = f32::from_le_bytes(glb[off..off + 4].try_into().unwrap()).abs();
        sum += y;
    }
    assert!((sum - 0.5 * expected as f32).abs() < 1e-3, "sum {sum}");
}

#[test]
fn glb_export_writes_morph_weight_channels() {
    let mut scene = SceneState::new();
    Command::CreatePrimitiveMesh {
        name: "Talker".into(),
        kind: PrimitiveKind::Cube,
        size: 1.0,
        material: PaletteIndex::new(1).unwrap(),
    }
    .apply(&mut scene)
    .unwrap();
    let node_id = scene.active_lowpoly_node().unwrap();
    Command::AddMorphDelta {
        node_id,
        name: "Smile".into(),
        vertex_ids: vec![0],
        delta: Vec3::new(0.0, 0.5, 0.0),
    }
    .apply(&mut scene)
    .unwrap();
    Command::ApplyBipedRig.apply(&mut scene).unwrap();
    Command::AddAnimationClip {
        name: "Talk".into(),
        duration_secs: 1.0,
    }
    .apply(&mut scene)
    .unwrap();
    Command::SetMorphWeightKeyframe {
        clip_index: 2,
        node_id,
        target: "Smile".into(),
        time_secs: 0.0,
        weight: 0.0,
    }
    .apply(&mut scene)
    .unwrap();
    Command::SetMorphWeightKeyframe {
        clip_index: 2,
        node_id,
        target: "Smile".into(),
        time_secs: 1.0,
        weight: 1.0,
    }
    .apply(&mut scene)
    .unwrap();

    let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
    let parsed = gltf::Gltf::from_slice(&glb).unwrap();
    // Clips export in order: Idle, Walk, Talk.
    let anim = parsed.animations().nth(2).expect("Talk clip must export");
    let mut found_weights = false;
    for ch in anim.channels() {
        use gltf::animation::{util::ReadOutputs, Property};
        if ch.target().property() == Property::MorphTargetWeights {
            found_weights = true;
            let reader = ch.reader(|_| parsed.blob.as_deref());
            let inputs: Vec<f32> = reader.read_inputs().unwrap().collect();
            assert_eq!(inputs, vec![0.0, 1.0]);
            let Some(ReadOutputs::MorphTargetWeights(out)) = reader.read_outputs() else {
                panic!("weights output must be morph weights");
            };
            let outputs: Vec<f32> = out.into_f32().collect();
            assert_eq!(outputs, vec![0.0, 1.0]);
        }
    }
    assert!(found_weights, "weights channel must export");
}

#[test]
fn glb_export_survives_bone_and_clip_removal() {
    let mut scene = SceneState::new();
    Command::CreatePrimitiveMesh {
        name: "Hero".into(),
        kind: PrimitiveKind::Cube,
        size: 1.0,
        material: PaletteIndex::new(1).unwrap(),
    }
    .apply(&mut scene)
    .unwrap();
    Command::ApplyBipedRig.apply(&mut scene).unwrap();
    // Drop RightArm (leaf) and the Walk clip, then export must parse.
    Command::RemoveBone {
        bone_id: tessel_core::BoneId(4),
    }
    .apply(&mut scene)
    .unwrap();
    let rig = scene.character_rig.as_ref().unwrap();
    assert_eq!(rig.bones.len(), 6);
    Command::RemoveAnimationClip { clip_index: 1 }
        .apply(&mut scene)
        .unwrap();
    assert_eq!(scene.character_rig.as_ref().unwrap().animations.len(), 1);

    let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
    let parsed = gltf::Gltf::from_slice(&glb).unwrap();
    assert_eq!(parsed.animations().count(), 1);
    assert_eq!(parsed.skins().count(), 1);
}

#[test]
fn glb_export_applies_node_transform_to_mesh_positions() {
    let mut scene = SceneState::new();
    Command::CreatePrimitiveMesh {
        name: "ShiftedCube".into(),
        kind: PrimitiveKind::Cube,
        size: 1.0,
        material: PaletteIndex::new(1).unwrap(),
    }
    .apply(&mut scene)
    .unwrap();
    let node_id = scene.active_lowpoly_node().unwrap();
    Command::SetNodeTransform {
        node_id,
        transform: tessel_core::Transform {
            translation: Vec3::new(5.0, 2.0, -3.0),
            rotation: glam::Quat::IDENTITY,
            scale: Vec3::new(2.0, 1.0, 1.0),
        },
    }
    .apply(&mut scene)
    .unwrap();

    let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
    let parsed = gltf::Gltf::from_slice(&glb).unwrap();
    let blob = parsed.blob.as_ref().unwrap();
    let prim = parsed.meshes().next().unwrap().primitives().next().unwrap();
    let reader = prim.reader(|_| Some(blob.as_slice()));
    let positions: Vec<[f32; 3]> = reader.read_positions().unwrap().collect();
    let min_x = positions.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min);
    let max_x = positions
        .iter()
        .map(|p| p[0])
        .fold(f32::NEG_INFINITY, f32::max);
    let min_y = positions.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min);
    assert!((min_x - 4.0).abs() < 1e-4);
    assert!((max_x - 6.0).abs() < 1e-4);
    assert!((min_y - 2.0).abs() < 1e-4);
}

#[test]
fn glb_export_deduplicates_identical_pbr_materials() {
    let mut scene = SceneState::new();
    let mat = tessel_core::MeshMaterial {
        metallic: 0.45,
        roughness: 0.5,
        emissive: [0.0; 3],
        double_sided: false,
        ..Default::default()
    };
    for i in 0..4 {
        Command::CreatePrimitiveMesh {
            name: format!("Part_{i}"),
            kind: PrimitiveKind::Cube,
            size: 0.5,
            material: PaletteIndex::new((i + 1) as u8).unwrap(),
        }
        .apply(&mut scene)
        .unwrap();
        let id = scene.active_lowpoly_node().unwrap();
        Command::SetMeshMaterial {
            node_id: id,
            material: mat,
        }
        .apply(&mut scene)
        .unwrap();
    }
    let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
    let parsed = gltf::Gltf::from_slice(&glb).unwrap();
    // All 4 parts share identical PBR parameters + palette texture -> deduplicated to 1 material!
    assert_eq!(parsed.materials().count(), 1);
}

#[test]
fn glb_export_writes_step_scale_channels_for_visibility_keyframes() {
    let mut scene = SceneState::new();
    Command::CreatePrimitiveMesh {
        name: "MuzzleFlash".into(),
        kind: PrimitiveKind::Sphere,
        size: 0.1,
        material: PaletteIndex::new(6).unwrap(),
    }
    .apply(&mut scene)
    .unwrap();
    let flash_id = scene.active_lowpoly_node().unwrap();
    Command::ApplyHumanoidRig.apply(&mut scene).unwrap();
    Command::SetVisibilityKeyframe {
        clip_index: 0,
        node_id: flash_id,
        time_secs: 0.0,
        visible: false,
    }
    .apply(&mut scene)
    .unwrap();
    Command::SetVisibilityKeyframe {
        clip_index: 0,
        node_id: flash_id,
        time_secs: 0.25,
        visible: true,
    }
    .apply(&mut scene)
    .unwrap();

    let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
    let parsed = gltf::Gltf::from_slice(&glb).unwrap();
    let idle = parsed.animations().next().expect("Idle clip must export");
    let vis_chan = idle
        .channels()
        .find(|c| {
            c.target().property() == gltf::animation::Property::Scale
                && c.sampler().interpolation() == gltf::animation::Interpolation::Step
        })
        .expect("visibility track should export as STEP scale channel");
    // Node 0 = merged mesh, nodes 1..=19 = 19 humanoid bones, node 20 = dedicated MuzzleFlash
    assert_eq!(vis_chan.target().node().index(), 20);
}

#[test]
fn glb_export_hidden_visibility_animated_node_matches_morphed_order() {
    let mut scene = SceneState::new();
    Command::CreatePrimitiveMesh {
        name: "HiddenFlash".into(),
        kind: PrimitiveKind::Cube,
        size: 1.0,
        material: PaletteIndex::new(1).unwrap(),
    }
    .apply(&mut scene)
    .unwrap();
    let flash_id = scene.active_lowpoly_node().unwrap();
    Command::ApplyHumanoidRig.apply(&mut scene).unwrap();
    // Hide the node *before* export: it must still export a dedicated node
    // because a visibility track animates it (morphed_order includes it).
    Command::SetNodeVisibility {
        node_id: flash_id,
        visible: false,
    }
    .apply(&mut scene)
    .unwrap();
    Command::SetVisibilityKeyframe {
        clip_index: 0,
        node_id: flash_id,
        time_secs: 0.0,
        visible: false,
    }
    .apply(&mut scene)
    .unwrap();
    Command::SetVisibilityKeyframe {
        clip_index: 0,
        node_id: flash_id,
        time_secs: 0.25,
        visible: true,
    }
    .apply(&mut scene)
    .unwrap();
    // A second, visible morphed node keeps `morphed` non-empty so the
    // order mismatch surfaces as the assert panic (not EmptyCanvas).
    Command::CreatePrimitiveMesh {
        name: "Smiler".into(),
        kind: PrimitiveKind::Cube,
        size: 1.0,
        material: PaletteIndex::new(1).unwrap(),
    }
    .apply(&mut scene)
    .unwrap();
    let smiler_id = scene.active_lowpoly_node().unwrap();
    Command::AddMorphDelta {
        node_id: smiler_id,
        name: "Smile".into(),
        vertex_ids: vec![0],
        delta: Vec3::new(0.0, 0.5, 0.0),
    }
    .apply(&mut scene)
    .unwrap();

    // Pre-fix this panicked on the morphed/morphed_order debug_assert!.
    let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
    let parsed = gltf::Gltf::from_slice(&glb).unwrap();
    assert!(parsed.meshes().len() >= 2);
    let idle = parsed.animations().next().expect("Idle clip must export");
    assert!(
        idle.channels().any(|c| {
            c.target().property() == gltf::animation::Property::Scale
                && c.sampler().interpolation() == gltf::animation::Interpolation::Step
        }),
        "hidden visibility-animated node must keep its STEP scale channel"
    );
}

#[test]
fn glb_export_writes_npr_color0_alpha_unlit_and_cubicspline() {
    let mut scene = SceneState::new();
    Command::CreatePrimitiveMesh {
        name: "InkBanner".into(),
        kind: PrimitiveKind::Cube,
        size: 1.0,
        material: PaletteIndex::new(2).unwrap(),
    }
    .apply(&mut scene)
    .unwrap();
    let node_id = scene.active_lowpoly_node().unwrap();
    Command::SetMeshMaterial {
        node_id,
        material: tessel_core::MeshMaterial {
            alpha_mode: tessel_core::AlphaMode::Mask { cutoff: 0.45 },
            unlit: true,
            ..tessel_core::MeshMaterial::default()
        },
    }
    .apply(&mut scene)
    .unwrap();
    Command::SetMeshVertexColors {
        node_id,
        updates: vec![(0, [0.8, 0.5, 0.9, 1.0])],
    }
    .apply(&mut scene)
    .unwrap();
    Command::ApplyBipedRig.apply(&mut scene).unwrap();
    Command::SetAnimationKeyframe {
        clip_index: 0,
        bone_id: tessel_core::BoneId(1),
        keyframe: tessel_core::Keyframe {
            time_secs: 0.0,
            translation: Vec3::ZERO,
            rotation: glam::Quat::IDENTITY,
            scale: Vec3::ONE,
            interpolation: tessel_core::KeyframeInterpolation::CubicBezier {
                p1x: 0.25,
                p1y: 0.1,
                p2x: 0.25,
                p2y: 1.0,
            },
        },
    }
    .apply(&mut scene)
    .unwrap();

    let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
    let parsed = gltf::Gltf::from_slice(&glb).unwrap();
    // Verify COLOR_0 attribute on primitive
    let has_color0 = parsed.meshes().any(|m| {
        m.primitives()
            .any(|p| p.get(&gltf::Semantic::Colors(0)).is_some())
    });
    assert!(
        has_color0,
        "expected COLOR_0 vertex color attribute in glTF"
    );
    // Verify alphaMode MASK + KHR_materials_unlit
    let has_mask = parsed
        .materials()
        .any(|m| m.alpha_mode() == gltf::material::AlphaMode::Mask);
    assert!(has_mask, "expected alphaMode MASK in glTF material");
    let root_json = parsed.as_json();
    assert!(
        root_json
            .extensions_used
            .iter()
            .any(|e| e == "KHR_materials_unlit"),
        "expected KHR_materials_unlit in extensionsUsed"
    );
    // Verify CUBICSPLINE sampler exported for CubicBezier channel with 3*N packed outputs
    let mut cubic_channels = 0usize;
    for anim in parsed.animations() {
        for ch in anim.channels() {
            let sampler = ch.sampler();
            if sampler.interpolation() == gltf::animation::Interpolation::CubicSpline {
                assert_eq!(
                    sampler.output().count(),
                    sampler.input().count() * 3,
                    "CUBICSPLINE output accessor must be 3x input count"
                );
                assert!(
                    sampler.input().count() >= 2,
                    "CUBICSPLINE requires at least 2 keyframes"
                );
                cubic_channels += 1;
            } else {
                assert_eq!(
                    sampler.output().count(),
                    sampler.input().count(),
                    "LINEAR/STEP output accessor must match input count"
                );
            }
        }
    }
    assert!(
        cubic_channels >= 2,
        "expected CUBICSPLINE on both rotation and translation channels, got {cubic_channels}"
    );
}

#[test]
fn obj_export_keeps_vertex_color_meshes_and_glb_respects_stepped_fps() {
    let mut scene = SceneState::new();
    Command::CreatePrimitiveMesh {
        name: "ColoredCube".into(),
        kind: PrimitiveKind::Cube,
        size: 1.0,
        material: PaletteIndex::new(3).unwrap(),
    }
    .apply(&mut scene)
    .unwrap();
    let node_id = scene.active_lowpoly_node().unwrap();
    Command::SetMeshVertexColors {
        node_id,
        updates: vec![(0, [1.0, 0.5, 0.5, 1.0])],
    }
    .apply(&mut scene)
    .unwrap();
    // OBJ export must NOT drop meshes that only have vertex_colors
    let (obj, _) = crate::export_obj_string(&scene, ExportPreset::Godot4).unwrap();
    assert!(obj.contains("v ") && obj.contains("f "));

    // Stepped FPS on a clip must automatically export STEP channels to GLB
    Command::ApplyBipedRig.apply(&mut scene).unwrap();
    if let Some(rig) = scene.character_rig.as_mut() {
        rig.animations[1].stepped_fps = Some(8.0);
    }
    let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
    let parsed = gltf::Gltf::from_slice(&glb).unwrap();
    let walk = parsed.animations().nth(1).unwrap();
    assert!(walk
        .channels()
        .all(|c| c.sampler().interpolation() == gltf::animation::Interpolation::Step));
}

#[test]
fn gltf_easing_interpolations_export_as_cubicspline_with_tangents() {
    let mut scene = SceneState::new();
    Command::CreatePrimitiveMesh {
        name: "RigMesh".into(),
        kind: PrimitiveKind::Cube,
        size: 0.5,
        material: PaletteIndex::new(1).unwrap(),
    }
    .apply(&mut scene)
    .unwrap();
    Command::ApplyBipedRig.apply(&mut scene).unwrap();
    let bone = tessel_core::BoneId(1);
    for (clip_idx, interp) in [
        (0usize, tessel_core::KeyframeInterpolation::EaseIn),
        (1usize, tessel_core::KeyframeInterpolation::EaseOut),
    ] {
        Command::SetAnimationKeyframe {
            clip_index: clip_idx,
            bone_id: bone,
            keyframe: tessel_core::Keyframe {
                time_secs: 0.0,
                translation: Vec3::ZERO,
                rotation: glam::Quat::IDENTITY,
                scale: Vec3::ONE,
                interpolation: interp,
            },
        }
        .apply(&mut scene)
        .unwrap();
        Command::SetAnimationKeyframe {
            clip_index: clip_idx,
            bone_id: bone,
            keyframe: tessel_core::Keyframe {
                time_secs: 1.0,
                translation: Vec3::new(0.0, 1.0, 0.0),
                rotation: glam::Quat::from_rotation_x(0.5),
                scale: Vec3::ONE,
                interpolation: tessel_core::KeyframeInterpolation::EaseInOut,
            },
        }
        .apply(&mut scene)
        .unwrap();
    }

    let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
    let parsed = gltf::Gltf::from_slice(&glb).unwrap();
    let mut cubic_count = 0usize;
    for anim in parsed.animations() {
        for ch in anim.channels() {
            let sampler = ch.sampler();
            if sampler.interpolation() == gltf::animation::Interpolation::CubicSpline {
                assert_eq!(sampler.output().count(), sampler.input().count() * 3);
                cubic_count += 1;
            }
        }
    }
    assert!(
        cubic_count >= 4,
        "EaseIn/EaseOut/EaseInOut channels must export as CUBICSPLINE (got {cubic_count})"
    );

    // Verify exact Hermite tangents for EaseIn (0, 2) and EaseOut (2, 0) on [0 -> 1] over dt=1.0
    let packed_in = pack_cubicspline(
        &[0.0, 1.0],
        &[
            tessel_core::KeyframeInterpolation::EaseIn,
            tessel_core::KeyframeInterpolation::EaseIn,
        ],
        &[[0.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    );
    assert!((packed_in[2][1] - 0.0).abs() < 1e-6); // out-tangent at k=0 is 0
    assert!((packed_in[3][1] - 2.0).abs() < 1e-6); // in-tangent at k=1 is 2*delta/dt = 2

    let packed_out = pack_cubicspline(
        &[0.0, 1.0],
        &[
            tessel_core::KeyframeInterpolation::EaseOut,
            tessel_core::KeyframeInterpolation::EaseOut,
        ],
        &[[0.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    );
    assert!((packed_out[2][1] - 2.0).abs() < 1e-6); // out-tangent at k=0 is 2*delta/dt = 2
    assert!((packed_out[3][1] - 0.0).abs() < 1e-6); // in-tangent at k=1 is 0

    // Mixed Step + EaseInOut expands Step segment with a hold keyframe at t_next - 1e-4
    let (exp_t, _, exp_v) = expand_mixed_step_keyframes(
        &[0.0, 0.5, 1.0],
        &[
            tessel_core::KeyframeInterpolation::Step,
            tessel_core::KeyframeInterpolation::EaseInOut,
            tessel_core::KeyframeInterpolation::EaseInOut,
        ],
        &[0.0f32, 1.0, 2.0],
    );
    assert_eq!(exp_t.len(), 4);
    assert!((exp_t[1] - (0.5 - 1e-4)).abs() < 1e-6);
    assert!((exp_v[1] - 0.0).abs() < 1e-6);

    // NodeTransformTrack easing also exports as CUBICSPLINE
    let node_id = scene.active_lowpoly_node().unwrap();
    if let Some(rig) = scene.character_rig.as_mut() {
        rig.animations[0]
            .node_transform_tracks
            .push(tessel_core::NodeTransformTrack {
                node: node_id,
                keyframes: vec![
                    tessel_core::Keyframe {
                        time_secs: 0.0,
                        translation: Vec3::ZERO,
                        rotation: glam::Quat::IDENTITY,
                        scale: Vec3::ONE,
                        interpolation: tessel_core::KeyframeInterpolation::Step,
                    },
                    tessel_core::Keyframe {
                        time_secs: 0.5,
                        translation: Vec3::new(0.0, 1.0, 0.0),
                        rotation: glam::Quat::IDENTITY,
                        scale: Vec3::ONE,
                        interpolation: tessel_core::KeyframeInterpolation::EaseInOut,
                    },
                    tessel_core::Keyframe {
                        time_secs: 1.0,
                        translation: Vec3::new(0.0, 2.0, 0.0),
                        rotation: glam::Quat::IDENTITY,
                        scale: Vec3::ONE,
                        interpolation: tessel_core::KeyframeInterpolation::EaseInOut,
                    },
                ],
            });
    }
    let (glb2, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
    let parsed2 = gltf::Gltf::from_slice(&glb2).unwrap();
    let idle = parsed2.animations().next().unwrap();
    let has_expanded_nt_cubic = idle.channels().any(|c| {
        let s = c.sampler();
        s.interpolation() == gltf::animation::Interpolation::CubicSpline
            && s.input().count() == 4
            && s.output().count() == 12
    });
    assert!(
        has_expanded_nt_cubic,
        "mixed Step+EaseInOut NodeTransformTrack must export as expanded CUBICSPLINE"
    );
}

#[test]
fn v5_4_bone_grid_morph_normals_cage_ribbon_and_stick_gizmo_export_roundtrip() {
    let mut scene = SceneState::new();
    let mut history = tessel_core::HistoryStack::new();

    history
        .execute(
            &mut scene,
            Command::CreatePrimitiveMesh {
                name: "CapeMesh".to_string(),
                kind: PrimitiveKind::Cube,
                size: 1.0,
                material: PaletteIndex::new(1).unwrap(),
            },
        )
        .unwrap();
    let cape_id = scene.active_lowpoly_node().unwrap();

    // 1. Bone Grid + Cloth Wind Bake
    history
        .execute(
            &mut scene,
            Command::CreateBoneGrid {
                spec: tessel_core::BoneGridSpec {
                    prefix: "Cape".to_string(),
                    cols: 3,
                    rows: 3,
                    ..Default::default()
                },
                bind_node: Some(cape_id),
            },
        )
        .unwrap();
    history
        .execute(
            &mut scene,
            Command::BakeClothGridAnimation {
                bone_prefix: "Cape".to_string(),
                params: tessel_core::ClothWindParams {
                    clip_name: "CapeWind".to_string(),
                    duration_secs: 1.0,
                    fps: 6.0,
                    ..Default::default()
                },
            },
        )
        .unwrap();

    // 2. Proxy Cage baked into MorphTarget (with position + normal deltas) + default weight
    history
        .execute(
            &mut scene,
            Command::ApplyProxyCageDeform {
                node_id: cape_id,
                divisions: [2, 2, 2],
                control_deltas: vec![
                    (0, Vec3::new(0.0, -0.3, 0.2)),
                    (7, Vec3::new(0.2, 0.3, 0.0)),
                ],
                floor_y: Some(-0.5),
                bake_as_morph: Some("CageFlutter".to_string()),
            },
        )
        .unwrap();
    history
        .execute(
            &mut scene,
            Command::SetMorphDefaultWeight {
                node_id: cape_id,
                name: "CageFlutter".to_string(),
                weight: 0.45,
            },
        )
        .unwrap();

    // 3. Procedural Ribbon with STEP visibility track
    history
        .execute(
            &mut scene,
            Command::BakeProceduralRibbon {
                params: tessel_core::RibbonParams {
                    name: "SwordTrail".to_string(),
                    source_bone: Some(tessel_core::BoneId(2)),
                    source_node: None,
                    explicit_points: Vec::new(),
                    clip_index: 0,
                    start_time: 0.1,
                    end_time: 0.9,
                    samples: 8,
                    width_start: 0.15,
                    width_end: 0.02,
                    material: PaletteIndex::new(2).unwrap(),
                    bake_visibility_track: true,
                },
            },
        )
        .unwrap();

    // 4. Stick Gizmo Twist + Undo/Redo
    let prev_rot = scene.character_rig.as_ref().unwrap().bones[0]
        .pose_local
        .rotation;
    history
        .execute(
            &mut scene,
            Command::ApplyStickGizmoPose {
                params: tessel_core::StickGizmoParams {
                    bone_id: tessel_core::BoneId(0),
                    mode: tessel_core::StickGizmoMode::Twist,
                    angle_deg: 45.0,
                    chain_len: 3,
                    snap_deg: 15.0,
                    mirror_lr: false,
                    axis_lock: None,
                },
            },
        )
        .unwrap();
    assert_ne!(
        scene.character_rig.as_ref().unwrap().bones[0]
            .pose_local
            .rotation,
        prev_rot
    );
    history.undo(&mut scene).unwrap();
    assert_eq!(
        scene.character_rig.as_ref().unwrap().bones[0]
            .pose_local
            .rotation,
        prev_rot
    );
    history.redo(&mut scene).unwrap();

    // 5. Round-trip .tessel archive & glTF 2.0 .glb validation
    let archive = crate::encode_tessel_project(&scene).unwrap();
    let decoded = crate::decode_tessel_project(&archive).unwrap();
    assert_eq!(
        decoded.lowpoly_meshes[&cape_id].morphs[0].name,
        "CageFlutter"
    );
    assert!((decoded.lowpoly_meshes[&cape_id].morphs[0].default_weight - 0.45).abs() < 1e-4);

    let (glb, _) = export_glb_bytes(&decoded, ExportPreset::Godot4).unwrap();
    let parsed = gltf::Gltf::from_slice(&glb).unwrap();
    assert!(parsed.skins().count() >= 1);
    assert!(parsed.animations().count() >= 1);
    let morphed_mesh = parsed
        .meshes()
        .find(|m| m.weights().is_some_and(|w| !w.is_empty()))
        .expect("morphed mesh with weights must export");
    assert!((morphed_mesh.weights().unwrap()[0] - 0.45).abs() < 1e-4);
    let prim = morphed_mesh.primitives().next().unwrap();
    let target = prim.morph_targets().next().unwrap();
    assert!(target.positions().is_some());
    assert!(target.normals().is_some());

    // 6. Verify GLB re-import preserves normal_deltas and default_weight
    let reimported = crate::import_glb_bytes(&glb, PaletteIndex::new(1).unwrap()).unwrap();
    assert!(!reimported.morphs.is_empty());
    assert!(!reimported.morphs[0].normal_deltas.is_empty());
    assert!((reimported.morphs[0].default_weight - 0.45).abs() < 1e-4);

    // 7. Verify SetMorphDefaultWeight undo also clears rig.morph_weights cache
    history
        .execute(
            &mut scene,
            Command::SetMorphDefaultWeight {
                node_id: cape_id,
                name: "CageFlutter".to_string(),
                weight: 0.9,
            },
        )
        .unwrap();
    assert!(
        (scene.character_rig.as_ref().unwrap().morph_weights
            [&(cape_id, "CageFlutter".to_string())]
            - 0.9)
            .abs()
            < 1e-4
    );
    history.undo(&mut scene).unwrap();
    assert!(
        (scene.character_rig.as_ref().unwrap().morph_weights
            [&(cape_id, "CageFlutter".to_string())]
            - 0.45)
            .abs()
            < 1e-4
    );
}

#[test]
fn e5_gltf_spec_invariants_and_deep_group_hierarchy_beyond_16_levels() {
    let mut scene = SceneState::new();
    let mut history = tessel_core::HistoryStack::default();

    // Build an 18-level nested Group hierarchy (g0 -> g1 -> ... -> g17) via CreateGroupNode
    let mut groups: Vec<tessel_core::NodeId> = Vec::with_capacity(18);
    let mut prev_group: Option<tessel_core::NodeId> = None;
    for depth in 0..18 {
        history
            .execute(
                &mut scene,
                Command::CreateGroupNode {
                    name: format!("Group_{depth}"),
                    transform: tessel_core::Transform::default(),
                    parent: prev_group,
                },
            )
            .unwrap();
        let gid = scene.selected_node.expect("group created");
        groups.push(gid);
        prev_group = Some(gid);
    }

    history
        .execute(
            &mut scene,
            Command::CreatePrimitiveMesh {
                name: "DeepCube".into(),
                kind: tessel_core::PrimitiveKind::Cube,
                size: 1.0,
                material: PaletteIndex::DEFAULT_ACTIVE,
            },
        )
        .unwrap();
    let mesh_id = scene.selected_node.unwrap();

    // SetNodeParent enforces the 16-hop cycle/depth cap (depth 18 fails closed with OutOfBounds).
    assert!(matches!(
        history.execute(
            &mut scene,
            Command::SetNodeParent {
                node_id: mesh_id,
                parent: Some(groups[17]),
            },
        ),
        Err(tessel_core::CoreError::OutOfBounds)
    ));

    // Parenting within the 16-hop cap (Group_14, 15 ancestors) succeeds.
    history
        .execute(
            &mut scene,
            Command::SetNodeParent {
                node_id: mesh_id,
                parent: Some(groups[14]),
            },
        )
        .unwrap();

    assert!(has_group_ancestor(&scene, mesh_id));

    let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();

    // Full gltf crate document + GLB binary buffer + accessor/node validation
    let parsed = gltf::Gltf::from_slice(&glb).unwrap();
    let doc = &parsed.document;
    assert_eq!(doc.as_json().asset.version, "2.0");
    let bin_len = parsed.blob.as_ref().map_or(0, |b| b.len());
    for bv in doc.views() {
        assert!(bv.offset() + bv.length() <= bin_len);
    }
    for acc in doc.accessors() {
        if let Some(bv) = acc.view() {
            assert!(acc.offset() + acc.count() * acc.size() <= bv.length());
        }
    }

    // Verify all 18 group nodes are present and linked in parent -> child chain via GLB JSON
    let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let raw_json: Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();
    let json_nodes = raw_json["nodes"].as_array().unwrap();
    let group_count = json_nodes
        .iter()
        .filter(|n| n["name"].as_str().is_some_and(|s| s.starts_with("Group_")))
        .count();
    assert_eq!(group_count, 18);

    let mut curr_idx = json_nodes
        .iter()
        .position(|n| n["name"].as_str() == Some("Group_0"))
        .expect("Group_0 root node");
    let mut group14_idx = None;
    for expected_depth in 1..18 {
        let expected_name = format!("Group_{expected_depth}");
        let children = json_nodes[curr_idx]["children"]
            .as_array()
            .expect("expected nested group children");
        curr_idx = children
            .iter()
            .filter_map(|v| v.as_u64().map(|u| u as usize))
            .find(|&i| json_nodes[i]["name"].as_str() == Some(expected_name.as_str()))
            .expect("expected nested group child");
        if expected_depth == 14 {
            group14_idx = Some(curr_idx);
        }
    }
    let g14 = group14_idx.expect("Group_14 node index");
    assert!(
        json_nodes[g14]["children"]
            .as_array()
            .is_some_and(|cs| cs.iter().any(|v| {
                v.as_u64()
                    .is_some_and(|i| json_nodes[i as usize]["mesh"].is_u64())
            })),
        "Group_14 should parent the exported mesh node"
    );
}

#[test]
fn glb_export_omits_empty_primitives_mesh_and_validates_cubicspline_step_scale() {
    let mut scene = SceneState::new();
    Command::CreatePrimitiveMesh {
        name: "MorphedOnly".into(),
        kind: tessel_core::PrimitiveKind::Cube,
        size: 1.0,
        material: PaletteIndex::new(1).unwrap(),
    }
    .apply(&mut scene)
    .unwrap();
    let node_id = scene.active_lowpoly_node().unwrap();
    Command::SetMorphTargetDeltas {
        node_id,
        name: "Blink".into(),
        position_deltas: vec![(0, Vec3::new(0.0, 0.1, 0.0))],
        normal_deltas: vec![],
        replace: true,
    }
    .apply(&mut scene)
    .unwrap();

    Command::AddBone {
        name: "Root".into(),
        parent: None,
        position: Vec3::ZERO,
    }
    .apply(&mut scene)
    .unwrap();
    let bone_id = scene.character_rig.as_ref().unwrap().bones[0].id;
    Command::AddAnimationClip {
        name: "ScaleCubicStep".into(),
        duration_secs: 1.0,
    }
    .apply(&mut scene)
    .unwrap();
    let clip_idx = scene.character_rig.as_ref().unwrap().active_animation;
    for (t, s, interp) in [
        (
            0.0,
            Vec3::ONE,
            tessel_core::KeyframeInterpolation::CubicBezier {
                p1x: 0.42,
                p1y: 0.0,
                p2x: 0.58,
                p2y: 1.0,
            },
        ),
        (
            0.5,
            Vec3::splat(1.5),
            tessel_core::KeyframeInterpolation::Step,
        ),
        (1.0, Vec3::ONE, tessel_core::KeyframeInterpolation::Linear),
    ] {
        Command::SetAnimationKeyframe {
            clip_index: clip_idx,
            bone_id,
            keyframe: tessel_core::Keyframe {
                time_secs: t,
                rotation: glam::Quat::IDENTITY,
                translation: Vec3::ZERO,
                scale: s,
                interpolation: interp,
            },
        }
        .apply(&mut scene)
        .unwrap();
    }

    let (glb, _) = export_glb_bytes(&scene, ExportPreset::default()).unwrap();
    let parsed = gltf::Gltf::from_slice(&glb).expect("GLB must be valid glTF 2.0");
    // Every exported mesh must have at least 1 primitive (no empty primitives: []).
    for mesh in parsed.meshes() {
        assert!(
            mesh.primitives().len() >= 1,
            "mesh {} must have >= 1 primitive",
            mesh.index()
        );
    }
    // For the CUBICSPLINE + mixed Step scale channel, output count must equal input count * 3.
    let scale_ch = parsed
        .animations()
        .flat_map(|a| a.channels())
        .find(|c| c.target().property() == gltf::animation::Property::Scale)
        .expect("scale channel");
    assert_eq!(
        scale_ch.sampler().interpolation(),
        gltf::animation::Interpolation::CubicSpline
    );
    assert_eq!(
        scale_ch.sampler().output().count(),
        scale_ch.sampler().input().count() * 3
    );
}

#[test]
fn gltf_export_is_byte_deterministic_across_repeated_runs() {
    let build_scene = || {
        let mut scene = SceneState::new();
        let mut history = tessel_core::HistoryStack::new();
        history
            .execute(
                &mut scene,
                Command::CreateGroupNode {
                    name: "RootGroup".into(),
                    transform: tessel_core::Transform::default(),
                    parent: None,
                },
            )
            .unwrap();
        let group_id = scene.selected_node.unwrap();

        history
            .execute(
                &mut scene,
                Command::CreatePrimitiveMesh {
                    name: "HeroBody".into(),
                    kind: PrimitiveKind::Cube,
                    size: 1.0,
                    material: PaletteIndex::new(1).unwrap(),
                },
            )
            .unwrap();
        let mesh_id = scene.active_lowpoly_node().unwrap();
        history
            .execute(
                &mut scene,
                Command::SetNodeParent {
                    node_id: mesh_id,
                    parent: Some(group_id),
                },
            )
            .unwrap();
        history
            .execute(
                &mut scene,
                Command::SetMeshVertexColors {
                    node_id: mesh_id,
                    updates: vec![(0, [1.0, 0.25, 0.5, 1.0]), (1, [0.5, 0.75, 1.0, 1.0])],
                },
            )
            .unwrap();
        history
            .execute(
                &mut scene,
                Command::SetMorphTargetDeltas {
                    node_id: mesh_id,
                    name: "Smile".into(),
                    position_deltas: vec![(0, Vec3::new(0.0, 0.05, 0.0))],
                    normal_deltas: vec![],
                    replace: true,
                },
            )
            .unwrap();
        history.execute(&mut scene, Command::ApplyBipedRig).unwrap();
        scene
    };

    let scene_a = build_scene();
    let scene_b = build_scene();

    let (glb_a1, tris_a1) = export_glb_bytes(&scene_a, ExportPreset::Godot4).unwrap();
    let (glb_a2, tris_a2) = export_glb_bytes(&scene_a, ExportPreset::Godot4).unwrap();
    let (glb_b1, tris_b1) = export_glb_bytes(&scene_b, ExportPreset::Godot4).unwrap();

    assert_eq!(tris_a1, tris_a2);
    assert_eq!(tris_a1, tris_b1);
    assert_eq!(
        glb_a1, glb_a2,
        "repeated GLB export on same scene must be byte-identical"
    );
    assert_eq!(
        glb_a1, glb_b1,
        "GLB export on equivalent scenes must be byte-identical"
    );
}

#[test]
fn g1_glb_export_separates_unrigged_nodes_from_skinned_nodes() {
    let mut scene = SceneState::new();
    Command::CreatePrimitiveMesh {
        name: "Hero".into(),
        kind: PrimitiveKind::Cube,
        size: 1.0,
        material: PaletteIndex::new(1).unwrap(),
    }
    .apply(&mut scene)
    .unwrap();
    let hero_id = scene.selected_node.unwrap();

    Command::CreatePrimitiveMesh {
        name: "StaticTerrain".into(),
        kind: PrimitiveKind::Cube,
        size: 2.0,
        material: PaletteIndex::new(2).unwrap(),
    }
    .apply(&mut scene)
    .unwrap();
    let terrain_id = scene.selected_node.unwrap();

    Command::ApplyBipedRig.apply(&mut scene).unwrap();
    Command::SetNodeRigSkinned {
        node_id: terrain_id,
        skinned: false,
    }
    .apply(&mut scene)
    .unwrap();

    // Mixed scene: Hero (skinned) + StaticTerrain (unrigged)
    let (glb, tris) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
    assert_eq!(tris, 24);
    let parsed = gltf::Gltf::from_slice(&glb).unwrap();
    let skinned_nodes: Vec<_> = parsed
        .nodes()
        .filter(|n| n.mesh().is_some() && n.skin().is_some())
        .collect();
    let static_nodes: Vec<_> = parsed
        .nodes()
        .filter(|n| n.mesh().is_some() && n.skin().is_none())
        .collect();
    assert_eq!(skinned_nodes.len(), 1, "Hero must be exported as skinned");
    assert_eq!(
        static_nodes.len(),
        1,
        "StaticTerrain must be exported as a separate unskinned node"
    );
    let skinned_prim = skinned_nodes[0]
        .mesh()
        .unwrap()
        .primitives()
        .next()
        .unwrap();
    assert!(skinned_prim.get(&gltf::Semantic::Joints(0)).is_some());
    assert!(skinned_prim.get(&gltf::Semantic::Weights(0)).is_some());

    let static_prim = static_nodes[0].mesh().unwrap().primitives().next().unwrap();
    assert!(static_prim.get(&gltf::Semantic::Joints(0)).is_none());
    assert!(static_prim.get(&gltf::Semantic::Weights(0)).is_none());

    // All-unrigged scene: neither node should carry skin or JOINTS_0/WEIGHTS_0
    Command::SetNodeRigSkinned {
        node_id: hero_id,
        skinned: false,
    }
    .apply(&mut scene)
    .unwrap();
    let (glb_all_static, tris_static) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
    assert_eq!(tris_static, 24);
    let parsed_static = gltf::Gltf::from_slice(&glb_all_static).unwrap();
    assert_eq!(
        parsed_static.skins().count(),
        0,
        "all-unrigged export must not emit orphaned glTF skins"
    );
    for node in parsed_static.nodes() {
        assert!(
            node.skin().is_none(),
            "no node should carry skin when all scene objects are unrigged"
        );
        if let Some(mesh) = node.mesh() {
            for prim in mesh.primitives() {
                assert!(prim.get(&gltf::Semantic::Joints(0)).is_none());
                assert!(prim.get(&gltf::Semantic::Weights(0)).is_none());
            }
        }
    }

    // Backward-compatibility: legacy rig JSON without `unrigged_nodes` deserializes cleanly.
    let mut rig_json = serde_json::to_value(scene.character_rig.as_ref().unwrap()).unwrap();
    rig_json.as_object_mut().unwrap().remove("unrigged_nodes");
    let legacy_rig: tessel_core::CharacterRig = serde_json::from_value(rig_json).unwrap();
    assert!(legacy_rig.unrigged_nodes.is_empty());
    assert!(legacy_rig.should_skin_node(terrain_id));
}

#[test]
fn h1_glb_export_writes_hdr_emissive_transmission_and_ior_extensions() {
    // Legacy PaletteEntry & MeshMaterial JSON without H1 fields deserialize with safe defaults.
    let legacy_pal_json = r#"{"name":"Clay","color":{"r":200,"g":150,"b":100,"a":255},"roughness":0.8,"metallic":0.0,"emissive":0.0}"#;
    let legacy_pal: tessel_core::PaletteEntry = serde_json::from_str(legacy_pal_json).unwrap();
    assert_eq!(legacy_pal.transmission, 0.0);
    assert!((legacy_pal.ior - 1.5).abs() < 1e-6);

    let legacy_mat_json = r#"{"metallic":0.2,"roughness":0.6,"emissive":[0.5,0.0,0.0]}"#;
    let legacy_mat: tessel_core::MeshMaterial = serde_json::from_str(legacy_mat_json).unwrap();
    assert!((legacy_mat.emissive_intensity - 1.0).abs() < 1e-6);
    assert_eq!(legacy_mat.transmission, 0.0);
    assert!((legacy_mat.ior - 1.5).abs() < 1e-6);

    let mut scene = SceneState::new();
    let neon_idx = PaletteIndex::new(10).unwrap();
    let glass_idx = PaletteIndex::new(11).unwrap();
    let matte_idx = PaletteIndex::new(1).unwrap();

    // Configure glowing neon voxel palette slot & glass voxel palette slot.
    let mut neon = tessel_core::PaletteEntry::matte("NeonCyan", Rgba8::rgb(40, 220, 255));
    neon.emissive = 6.0;
    Command::SetPaletteEntry {
        index: neon_idx,
        entry: neon,
    }
    .apply(&mut scene)
    .unwrap();

    let mut glass = tessel_core::PaletteEntry::matte("Crystal", Rgba8::rgb(180, 235, 255));
    glass.transmission = 0.85;
    glass.ior = 1.45;
    Command::SetPaletteEntry {
        index: glass_idx,
        entry: glass,
    }
    .apply(&mut scene)
    .unwrap();

    Command::CreateVoxelVolume {
        name: "MixedVoxels".into(),
    }
    .apply(&mut scene)
    .unwrap();
    let vox_id = scene.selected_node.unwrap();
    Command::SetVoxels {
        node_id: vox_id,
        edits: vec![
            tessel_core::VoxelEdit {
                coord: VoxelCoord::new(0, 0, 0),
                color: Some(matte_idx),
            },
            tessel_core::VoxelEdit {
                coord: VoxelCoord::new(2, 0, 0),
                color: Some(neon_idx),
            },
            tessel_core::VoxelEdit {
                coord: VoxelCoord::new(4, 0, 0),
                color: Some(glass_idx),
            },
        ],
    }
    .apply(&mut scene)
    .unwrap();

    // Add two Low-Poly meshes with identical base PBR but different emissive_intensity so
    // MaterialKey deduplication must keep them as distinct materials.
    for (name, intensity) in [("LampLow", 2.0_f32), ("LampHigh", 8.5_f32)] {
        Command::CreatePrimitiveMesh {
            name: name.into(),
            kind: PrimitiveKind::Cube,
            size: 0.5,
            material: matte_idx,
        }
        .apply(&mut scene)
        .unwrap();
        let nid = scene.selected_node.unwrap();
        Command::SetMeshMaterial {
            node_id: nid,
            material: tessel_core::MeshMaterial {
                emissive: [1.0, 0.5, 0.1],
                emissive_intensity: intensity,
                transmission: 0.25,
                ior: 1.6,
                ..Default::default()
            },
        }
        .apply(&mut scene)
        .unwrap();
    }

    let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
    // Validate GLB container and schema with gltf parser, and inspect raw JSON chunk for KHR extensions.
    let _parsed = gltf::Gltf::from_slice(&glb).unwrap();
    let json_len = u32::from_le_bytes([glb[12], glb[13], glb[14], glb[15]]) as usize;
    let json_val: serde_json::Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();

    // Verify conditional extensionsUsed contains all three H1 extensions (and NOT unlit).
    let ext_used = json_val["extensionsUsed"]
        .as_array()
        .expect("extensionsUsed must be present");
    let ext_names: Vec<&str> = ext_used.iter().filter_map(|v| v.as_str()).collect();
    assert!(ext_names.contains(&"KHR_materials_emissive_strength"));
    assert!(ext_names.contains(&"KHR_materials_transmission"));
    assert!(ext_names.contains(&"KHR_materials_ior"));
    assert!(!ext_names.contains(&"KHR_materials_unlit"));

    // 3 voxel palette material groups (matte, neon, glass) + 2 distinct LowPoly materials = 5 materials.
    let mats = json_val["materials"].as_array().unwrap();
    assert_eq!(
        mats.len(),
        5,
        "expected 3 palette groups + 2 distinct lowpoly materials, got {mats:?}"
    );

    // Verify emissiveStrength values (6.0 for neon voxel, 2.0 & 8.5 for lowpoly lamps).
    let strengths: Vec<f64> = mats
        .iter()
        .filter_map(|m| {
            m.get("extensions")?
                .get("KHR_materials_emissive_strength")?
                .get("emissiveStrength")?
                .as_f64()
        })
        .collect();
    assert!(strengths.iter().any(|&s| (s - 6.0).abs() < 1e-3));
    assert!(strengths.iter().any(|&s| (s - 2.0).abs() < 1e-3));
    assert!(strengths.iter().any(|&s| (s - 8.5).abs() < 1e-3));

    // Verify transmission & IOR extensions are populated.
    assert!(mats.iter().any(|m| {
        m.get("extensions")
            .and_then(|e| e.get("KHR_materials_transmission"))
            .and_then(|t| t.get("transmissionFactor"))
            .and_then(|v| v.as_f64())
            .is_some_and(|tf| (tf - 0.85).abs() < 1e-3)
    }));
    assert!(mats.iter().any(|m| {
        m.get("extensions")
            .and_then(|e| e.get("KHR_materials_ior"))
            .and_then(|t| t.get("ior"))
            .and_then(|v| v.as_f64())
            .is_some_and(|ior| (ior - 1.45).abs() < 1e-3)
    }));
}
