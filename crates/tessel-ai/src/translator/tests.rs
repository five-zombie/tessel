use super::*;
use tessel_core::{BoneId, NodeId, PaletteIndex};

#[test]
fn ai_tools_pose_and_keyframe_character_rig() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir();

    execute_tool_call(
        "character_apply_biped_rig",
        &json!({}),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "character_set_bone_pose",
        &json!({ "bone_id": 4, "rotation_deg": [0.0, 0.0, 45.0] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "character_set_keyframe",
        &json!({
            "clip_name": "Wave",
            "duration_secs": 1.0,
            "bone_id": 4,
            "time_secs": 0.5,
            "rotation_deg": [0.0, 0.0, 90.0]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();

    let rig = scene.character_rig.as_ref().unwrap();
    assert_eq!(rig.animations.len(), 3);
    assert_eq!(rig.animations[2].name, "Wave");
    assert_eq!(rig.animations[2].keyframe_times(Some(BoneId(4))), vec![0.5]);
}

#[test]
fn ai_tool_mesh_import_loads_exported_obj_and_glb() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir().join("tessel_ai_mesh_import_test");
    let _ = std::fs::create_dir_all(&dir);

    execute_tool_call(
        "mesh_create_primitive",
        &json!({ "kind": "cube", "size": 1.0 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "project_export",
        &json!({ "filename": "cube_test.glb", "format": "glb" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();

    let res = execute_tool_call(
        "mesh_import",
        &json!({ "filename": "cube_test.glb", "name": "Imported Cube" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(res["ok"], true);
    assert_eq!(res["faces"], 12);
    assert_eq!(scene.lowpoly_meshes.len(), 2);

    let imported_id = res["node_id"].as_u64().unwrap();
    let del = execute_tool_call(
        "scene_delete_node",
        &json!({ "node_id": imported_id }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(del["ok"], true);
    assert_eq!(scene.lowpoly_meshes.len(), 1);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ai_tools_set_node_transform_and_primitive_position() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir();

    let created = execute_tool_call(
        "mesh_create_primitive",
        &json!({
            "kind": "cube",
            "size": 1.0,
            "position": [1.5, 2.0, -0.5],
            "scale": [0.5, 2.0, 0.5]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let node_id = NodeId(created["node_id"].as_u64().unwrap());
    let node = scene.nodes.get(&node_id).unwrap();
    assert_eq!(node.transform.translation, glam::Vec3::new(1.5, 2.0, -0.5));
    assert_eq!(node.transform.scale, glam::Vec3::new(0.5, 2.0, 0.5));

    execute_tool_call(
        "set_node_transform",
        &json!({
            "node_id": node_id.0,
            "position": [-2.0, 0.5, 1.0],
            "rotation_deg": [0.0, 90.0, 0.0]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let updated = scene.nodes.get(&node_id).unwrap();
    assert_eq!(
        updated.transform.translation,
        glam::Vec3::new(-2.0, 0.5, 1.0)
    );
    assert_eq!(updated.transform.scale, glam::Vec3::new(0.5, 2.0, 0.5));
    let summary = scene_summary_json(&scene);
    assert_eq!(summary["nodes"][0]["position"], json!([-2.0, 0.5, 1.0]));

    // Verify palette_set_color, mesh_inspect, mesh_inset_face, mesh_extrude_face, scene_inspect
    let pal_res = execute_tool_call(
        "palette_set_color",
        &json!({ "palette_index": 12, "name": "CamoOlive", "hex": "#4B5320" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(pal_res["palette_index"], 12);

    let minspect = execute_tool_call(
        "mesh_inspect",
        &json!({ "node_id": node_id.0 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(minspect["face_count"], 6);

    let inset = execute_tool_call(
        "mesh_inset_face",
        &json!({ "node_id": node_id.0, "face_id": 0, "factor": 0.25 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(inset["total_faces"], 10);

    let extrude = execute_tool_call(
        "mesh_extrude_face",
        &json!({ "node_id": node_id.0, "face_id": 0, "distance": 0.2 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(extrude["total_faces"], 14);

    let sinspect =
        execute_tool_call("scene_inspect", &json!({}), &mut scene, &mut history, &dir).unwrap();
    assert_eq!(sinspect["ok"], true);
    assert_eq!(
        sinspect["export_dir"],
        crate::translator::canonical_export_dir_string(&dir)
    );

    let beveled = execute_tool_call(
        "mesh_bevel",
        &json!({ "node_id": node_id.0, "amount": 0.15 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(beveled["ok"], true);

    let sphere = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "kind": "sphere", "size": 1.0, "position": [0.0, 2.0, 0.0] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(sphere["ok"], true);
    let sphere_id = sphere["node_id"].as_u64().unwrap();

    let parented = execute_tool_call(
        "scene_set_parent",
        &json!({ "node_id": sphere_id, "parent_node_id": node_id.0 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(parented["parent_node_id"], node_id.0);

    let strap = execute_tool_call(
        "mesh_create_curve_tube",
        &json!({
            "name": "ShoulderStrap",
            "points": [[0.0, 0.2, 0.0], [0.2, 0.6, 0.2], [0.0, 1.0, 0.0]],
            "radius": 0.08,
            "flatness": 0.25
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(strap["ok"], true);
    let strap_id = strap["node_id"].as_u64().unwrap();

    let dec = execute_tool_call(
        "mesh_decimate",
        &json!({ "node_id": strap_id, "ratio": 0.5 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(dec["ok"], true);

    let bone = execute_tool_call(
        "character_add_bone",
        &json!({ "name": "FlapHinge", "position": [0.0, 1.0, 0.0] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(bone["bone_id"], 0);

    let bind = execute_tool_call(
        "character_bind_node",
        &json!({ "node_id": strap_id, "bone_id": 0 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(bind["bone_id"], 0);

    let light = execute_tool_call(
        "scene_set_lighting",
        &json!({ "azimuth_deg": 45.0, "elevation_deg": 60.0 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(light["ok"], true);
}

#[test]
fn ai_tools_move_vertices_set_vertex_and_face_uvs() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir();

    let created = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "kind": "cube", "size": 1.0 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let node_id = created["node_id"].as_u64().unwrap();
    let mesh = scene.lowpoly_meshes.get(&NodeId(node_id)).unwrap();
    let before_v0 = mesh.vertices[0];
    let before_v1 = mesh.vertices[1];

    let moved = execute_tool_call(
        "mesh_move_vertices",
        &json!({ "node_id": node_id, "vertex_ids": [0], "delta": [0.25, 0.0, 0.0] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(moved["ok"], true);
    let mesh = scene.lowpoly_meshes.get(&NodeId(node_id)).unwrap();
    assert!((mesh.vertices[0].x - (before_v0.x + 0.25)).abs() < 1e-6);
    assert_eq!(mesh.vertices[1], before_v1);

    let placed = execute_tool_call(
        "mesh_set_vertex",
        &json!({ "node_id": node_id, "vertex_id": 1, "position": [0.1, 0.2, 0.3] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(placed["ok"], true);
    let mesh = scene.lowpoly_meshes.get(&NodeId(node_id)).unwrap();
    assert_eq!(mesh.vertices[1], glam::Vec3::new(0.1, 0.2, 0.3));

    let uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let unwrapped = execute_tool_call(
        "mesh_set_face_uvs",
        &json!({ "node_id": node_id, "face_id": 0, "uvs": uvs }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(unwrapped["ok"], true);
    let mesh = scene.lowpoly_meshes.get(&NodeId(node_id)).unwrap();
    assert_eq!(mesh.faces[0].uvs, uvs);

    // Rejected inputs surface as errors, not silent no-ops.
    assert!(execute_tool_call(
        "mesh_move_vertices",
        &json!({ "node_id": node_id, "vertex_ids": [], "delta": [0.0, 0.0, 0.0] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .is_err());
    assert!(execute_tool_call(
        "mesh_set_face_uvs",
        &json!({ "node_id": node_id, "face_id": 0, "uvs": [[0.0, 0.0]] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .is_err());

    // Undo restores the mesh snapshot taken before the last command.
    history.undo(&mut scene).unwrap();
    let mesh = scene.lowpoly_meshes.get(&NodeId(node_id)).unwrap();
    assert!(mesh.faces[0].uvs.is_empty());
}

#[test]
fn ai_tool_set_keyframe_stores_trs() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir();

    execute_tool_call(
        "character_apply_biped_rig",
        &json!({}),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let res = execute_tool_call(
        "character_set_keyframe",
        &json!({
            "clip_name": "Bounce",
            "duration_secs": 2.0,
            "bone_id": 1,
            "time_secs": 0.5,
            "rotation_deg": [0.0, 0.0, 45.0],
            "translation": [0.0, 0.1, 0.0],
            "scale": [1.0, 1.0, 2.0]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(res["ok"], true);
    // f32 JSON round-trip is approximate: compare with tolerance.
    let got_t: Vec<f32> = serde_json::from_value(res["translation"].clone()).unwrap();
    let got_s: Vec<f32> = serde_json::from_value(res["scale"].clone()).unwrap();
    for (got, want) in got_t.iter().zip([0.0f32, 0.1, 0.0]) {
        assert!((got - want).abs() < 1e-6);
    }
    for (got, want) in got_s.iter().zip([1.0f32, 1.0, 2.0]) {
        assert!((got - want).abs() < 1e-6);
    }

    let rig = scene.character_rig.as_ref().unwrap();
    let clip = rig.animations.iter().find(|c| c.name == "Bounce").unwrap();
    let ch = clip
        .channels
        .iter()
        .find(|c| c.bone_id == BoneId(1))
        .unwrap();
    assert_eq!(ch.keyframes.len(), 1);
    assert_eq!(ch.keyframes[0].translation, glam::Vec3::new(0.0, 0.1, 0.0));
    assert_eq!(ch.keyframes[0].scale, glam::Vec3::new(1.0, 1.0, 2.0));

    // Rotation-only callers keep working with TRS defaults.
    execute_tool_call(
        "character_set_keyframe",
        &json!({
            "clip_name": "Bounce",
            "bone_id": 1,
            "time_secs": 1.0,
            "rotation_deg": [0.0, 0.0, 0.0]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let rig = scene.character_rig.as_ref().unwrap();
    let clip = rig.animations.iter().find(|c| c.name == "Bounce").unwrap();
    let ch = clip
        .channels
        .iter()
        .find(|c| c.bone_id == BoneId(1))
        .unwrap();
    assert_eq!(ch.keyframes.len(), 2);

    // Malformed TRS arrays are rejected, not stored.
    assert!(execute_tool_call(
        "character_set_keyframe",
        &json!({
            "clip_name": "Bounce",
            "bone_id": 1,
            "time_secs": 1.5,
            "rotation_deg": [0.0, 0.0, 0.0],
            "translation": [0.0, 0.0]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .is_err());
}

#[test]
fn ai_tool_camera_calls_need_live_viewport() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir();

    // Headless dispatch has no camera: every camera_* tool must fail
    // closed with a clear message instead of misrouting to UnknownTool.
    for (name, args) in [
        ("camera_get", json!({})),
        (
            "camera_set_pose",
            json!({ "yaw_deg": 45.0, "pitch_deg": 20.0 }),
        ),
        ("camera_look_at", json!({ "target": [0.0, 0.5, 0.0] })),
        ("camera_zoom", json!({ "factor": 0.5 })),
        ("camera_orbit", json!({ "dx": 10.0, "dy": 0.0 })),
        ("camera_pan", json!({ "dx": 10.0, "dy": 0.0 })),
        ("camera_reset", json!({})),
        ("camera_preset", json!({ "name": "front" })),
    ] {
        let err = execute_tool_call(name, &args, &mut scene, &mut history, &dir)
            .expect_err("camera tools must fail without a live viewport");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("live GUI viewport"),
            "unexpected error for {name}: {msg}"
        );
    }
}

#[test]
fn ai_tool_solve_ik_reaches_target() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir();

    // Straight 3-bone leg: hip(0,1,0) -> knee(0,0.5,0) -> ankle(0,0,0).
    for (name, parent, pos) in [
        ("Hip", None, [0.0, 1.0, 0.0]),
        ("Knee", Some(0), [0.0, -0.5, 0.0]),
        ("Ankle", Some(1), [0.0, -0.5, 0.0]),
    ] {
        let mut args = serde_json::Map::new();
        args.insert("name".into(), json!(name));
        args.insert("position".into(), json!(pos));
        if let Some(pid) = parent {
            args.insert("parent_bone_id".into(), json!(pid));
        }
        execute_tool_call(
            "character_add_bone",
            &Value::Object(args),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
    }

    let solved = execute_tool_call(
        "character_solve_ik",
        &json!({
            "root_bone_id": 0,
            "mid_bone_id": 1,
            "end_bone_id": 2,
            "target": [0.0, 0.5, 0.5]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(solved["ok"], true);
    let reached: Vec<f32> = serde_json::from_value(solved["reached"].clone()).unwrap();
    let target = glam::Vec3::new(0.0, 0.5, 0.5);
    assert!(
        (glam::Vec3::from_array([reached[0], reached[1], reached[2]]) - target).length() < 1e-3,
        "reached {reached:?}"
    );

    // Undo restores the straight pose.
    history.undo(&mut scene).unwrap();
    let rig = scene.character_rig.as_ref().unwrap();
    let world = rig.world_pose_matrices();
    let ankle = world
        .get(&BoneId(2))
        .unwrap()
        .transform_point3(glam::Vec3::ZERO);
    assert!((ankle - glam::Vec3::new(0.0, 0.0, 0.0)).length() < 1e-5);

    // Non-linked triple is rejected, not solved.
    assert!(execute_tool_call(
        "character_solve_ik",
        &json!({
            "root_bone_id": 0,
            "mid_bone_id": 2,
            "end_bone_id": 1,
            "target": [0.0, 0.5, 0.5]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .is_err());
}

#[test]
fn ai_tool_vertex_weights_paint_and_clear() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir();

    let created = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "kind": "cube", "size": 1.0 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let node_id = created["node_id"].as_u64().unwrap();

    let painted = execute_tool_call(
        "mesh_set_vertex_weights",
        &json!({ "node_id": node_id, "vertex_ids": [0, 1], "joints": [2, 0, 0, 0], "weights": [3.0, 1.0, 0.0, 0.0] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(painted["ok"], true);
    assert_eq!(painted["painted"], 2);
    let mesh = scene.lowpoly_meshes.get(&NodeId(node_id)).unwrap();
    assert!((mesh.skin_weights[&0].weights[0] - 0.75).abs() < 1e-6);

    let inspect = execute_tool_call(
        "mesh_inspect",
        &json!({ "node_id": node_id }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(inspect["manual_weights"], 2);

    // Undo restores the unpainted mesh.
    history.undo(&mut scene).unwrap();
    let mesh = scene.lowpoly_meshes.get(&NodeId(node_id)).unwrap();
    assert!(mesh.skin_weights.is_empty());

    // Re-paint then clear one vertex.
    execute_tool_call(
        "mesh_set_vertex_weights",
        &json!({ "node_id": node_id, "vertex_ids": [0, 1], "joints": [2, 0, 0, 0], "weights": [1.0, 0.0, 0.0, 0.0] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "mesh_clear_vertex_weights",
        &json!({ "node_id": node_id, "vertex_ids": [0] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let mesh = scene.lowpoly_meshes.get(&NodeId(node_id)).unwrap();
    assert_eq!(mesh.skin_weights.len(), 1);

    // Rejected inputs: empty ids, short quads, negative weights.
    for bad in [
        json!({ "node_id": node_id, "vertex_ids": [], "joints": [0, 0, 0, 0], "weights": [1.0, 0.0, 0.0, 0.0] }),
        json!({ "node_id": node_id, "vertex_ids": [0], "joints": [0, 0, 0], "weights": [1.0, 0.0, 0.0, 0.0] }),
        json!({ "node_id": node_id, "vertex_ids": [0], "joints": [0, 0, 0, 0], "weights": [-1.0, 1.0, 0.0, 0.0] }),
    ] {
        assert!(execute_tool_call(
            "mesh_set_vertex_weights",
            &bad,
            &mut scene,
            &mut history,
            &dir
        )
        .is_err());
    }
}

#[test]
fn ai_tool_rig_introspection_and_small_batch() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir();

    // No rig yet: introspection fails closed.
    assert!(execute_tool_call(
        "character_list_bones",
        &json!({}),
        &mut scene,
        &mut history,
        &dir
    )
    .is_err());

    execute_tool_call(
        "character_apply_biped_rig",
        &json!({}),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();

    // List bones: 7-bone biped with parents.
    let listed = execute_tool_call(
        "character_list_bones",
        &json!({}),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(listed["bone_count"], 7);
    assert_eq!(listed["bones"][1]["name"], "Torso");
    assert_eq!(listed["bones"][1]["parent_bone_id"], 0);
    assert!(listed["coordinate_space"].as_str().is_some());
    let torso_local = listed["bones"][1]["local_rest_position"]
        .as_array()
        .unwrap();
    let torso_world = listed["bones"][1]["world_rest_position"]
        .as_array()
        .unwrap();
    assert_eq!(torso_local.len(), 3);
    assert_eq!(torso_world.len(), 3);
    let head_world_y = listed["bones"][2]["world_rest_position"][1]
        .as_f64()
        .unwrap();
    let head_local_y = listed["bones"][2]["local_rest_position"][1]
        .as_f64()
        .unwrap();
    assert!(head_world_y > head_local_y);

    // Inspect the Idle clip (index 0): Torso channel has 3 keys.
    let insp = execute_tool_call(
        "character_inspect_clip",
        &json!({ "clip_name": "Idle" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(insp["name"], "Idle");
    assert!(insp["is_active"].as_bool().unwrap());
    let torso_ch = insp["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["bone_id"] == 1)
        .unwrap();
    assert_eq!(torso_ch["keyframes"].as_array().unwrap().len(), 3);

    // Sample the pose halfway: torso lifted by half the breathing offset.
    let sampled = execute_tool_call(
        "character_sample_pose",
        &json!({ "clip_name": "Idle", "time_secs": 0.5 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let torso = sampled["bones"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["bone_id"] == 1)
        .unwrap();
    assert!((torso["world_position"][1].as_f64().unwrap() - 1.115).abs() < 1e-3);

    // Switch active clip to Walk, then remove a keyframe off it.
    execute_tool_call(
        "character_set_active_clip",
        &json!({ "clip_name": "Walk" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let rig = scene.character_rig.as_ref().unwrap();
    assert_eq!(rig.animations[rig.active_animation].name, "Walk");
    let left_arm_keys = |scene: &SceneState| {
        scene.character_rig.as_ref().unwrap().animations[1]
            .channels
            .iter()
            .find(|c| c.bone_id == BoneId(3))
            .map(|c| c.keyframes.len())
            .unwrap_or(0)
    };
    assert_eq!(left_arm_keys(&scene), 3);
    execute_tool_call(
        "character_remove_keyframe",
        &json!({ "clip_index": 1, "bone_id": 3, "time_secs": 0.0 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(left_arm_keys(&scene), 2);

    // Duplicate a low-poly node: cube -> copy with offset + inherited name.
    let created = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "kind": "cube", "size": 1.0 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let node_id = created["node_id"].as_u64().unwrap();
    let dup = execute_tool_call(
        "scene_duplicate_node",
        &json!({ "node_id": node_id, "name": "Twin" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_ne!(dup["node_id"], node_id);
    let twin = scene
        .lowpoly_meshes
        .get(&NodeId(dup["node_id"].as_u64().unwrap()))
        .unwrap();
    assert_eq!(twin.vertices.len(), 8);
    // Undo drops the copy.
    history.undo(&mut scene).unwrap();
    assert_eq!(scene.lowpoly_meshes.len(), 1);

    // Visibility toggle with undo.
    execute_tool_call(
        "scene_set_node_visibility",
        &json!({ "node_id": node_id, "visible": false }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert!(!scene.nodes.get(&NodeId(node_id)).unwrap().visible);
    history.undo(&mut scene).unwrap();
    assert!(scene.nodes.get(&NodeId(node_id)).unwrap().visible);

    // Clear texture round-trips through undo.
    execute_tool_call(
        "mesh_fill_texture",
        &json!({ "node_id": node_id, "palette_index": 5 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert!(scene
        .lowpoly_meshes
        .get(&NodeId(node_id))
        .unwrap()
        .texture
        .is_some());
    execute_tool_call(
        "mesh_clear_texture",
        &json!({ "node_id": node_id, "target": "color" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert!(scene
        .lowpoly_meshes
        .get(&NodeId(node_id))
        .unwrap()
        .texture
        .is_none());
    history.undo(&mut scene).unwrap();
    assert!(scene
        .lowpoly_meshes
        .get(&NodeId(node_id))
        .unwrap()
        .texture
        .is_some());
    assert!(execute_tool_call(
        "mesh_clear_texture",
        &json!({ "node_id": node_id, "target": "bogus" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .is_err());

    // Remove one clip, then one leaf bone (RightArm has no children;
    // removing Torso would drop the body chain).
    let clips_before = scene.character_rig.as_ref().unwrap().animations.len();
    execute_tool_call(
        "character_remove_clip",
        &json!({ "clip_name": "Idle" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(
        scene.character_rig.as_ref().unwrap().animations.len(),
        clips_before - 1
    );
    let removed = execute_tool_call(
        "character_remove_bone",
        &json!({ "bone_id": 4 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(removed["removed_bones"], 1);
    let listed = execute_tool_call(
        "character_list_bones",
        &json!({}),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(listed["bone_count"], 6);
    assert!(scene
        .character_rig
        .as_ref()
        .unwrap()
        .bones
        .iter()
        .all(|b| b.name != "RightArm"));
    // Unknown bone / clip fail cleanly.
    assert!(execute_tool_call(
        "character_remove_bone",
        &json!({ "bone_id": 99 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .is_err());
    assert!(execute_tool_call(
        "character_remove_clip",
        &json!({ "clip_name": "Nope" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .is_err());
}

#[test]
fn ai_tool_morph_delta_and_clear() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir();

    let created = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "kind": "cube", "size": 1.0 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let node_id = created["node_id"].as_u64().unwrap();

    let added = execute_tool_call(
        "mesh_morph_delta",
        &json!({ "node_id": node_id, "name": "Smile", "vertex_ids": [0], "delta": [0.0, 0.1, 0.0] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(added["ok"], true);
    assert_eq!(added["morph"], "Smile");

    let inspect = execute_tool_call(
        "mesh_inspect",
        &json!({ "node_id": node_id }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(inspect["morphs"], json!(["Smile"]));

    // Undo drops the morph; re-add then clear by name.
    history.undo(&mut scene).unwrap();
    let mesh = scene.lowpoly_meshes.get(&NodeId(node_id)).unwrap();
    assert!(mesh.morphs.is_empty());
    execute_tool_call(
        "mesh_morph_delta",
        &json!({ "node_id": node_id, "name": "Smile", "vertex_ids": [0], "delta": [0.0, 0.1, 0.0] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "mesh_morph_clear",
        &json!({ "node_id": node_id, "name": "Smile" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let mesh = scene.lowpoly_meshes.get(&NodeId(node_id)).unwrap();
    assert!(mesh.morphs.is_empty());

    // Rejected inputs: empty name, empty ids, short delta.
    for bad in [
        json!({ "node_id": node_id, "name": "", "vertex_ids": [0], "delta": [0.0, 0.0, 0.0] }),
        json!({ "node_id": node_id, "name": "X", "vertex_ids": [], "delta": [0.0, 0.0, 0.0] }),
        json!({ "node_id": node_id, "name": "X", "vertex_ids": [0], "delta": [0.0, 0.0] }),
    ] {
        assert!(
            execute_tool_call("mesh_morph_delta", &bad, &mut scene, &mut history, &dir).is_err()
        );
    }
}

#[test]
fn ai_tool_morph_weight_keyframes() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir();

    let created = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "kind": "cube", "size": 1.0 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let node_id = created["node_id"].as_u64().unwrap();
    execute_tool_call(
        "mesh_morph_delta",
        &json!({ "node_id": node_id, "name": "Smile", "vertex_ids": [0], "delta": [0.0, 0.1, 0.0] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "character_apply_biped_rig",
        &json!({}),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();

    let set = execute_tool_call(
        "character_set_morph_weight",
        &json!({ "clip_name": "Talk", "node_id": node_id, "target": "Smile", "time_secs": 0.5, "weight": 0.75 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(set["ok"], true);
    let clip_index = set["clip_index"].as_u64().unwrap() as usize;
    let rig = scene.character_rig.as_ref().unwrap();
    let clip = rig.animations.iter().find(|c| c.name == "Talk").unwrap();
    assert_eq!(clip.morph_tracks.len(), 1);
    assert_eq!(clip.morph_tracks[0].keys.len(), 1);

    let removed = execute_tool_call(
        "character_remove_morph_weight",
        &json!({ "clip_index": clip_index, "node_id": node_id, "target": "Smile", "time_secs": 0.5 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(removed["ok"], true);
    let rig = scene.character_rig.as_ref().unwrap();
    let clip = rig.animations.iter().find(|c| c.name == "Talk").unwrap();
    assert!(clip.morph_tracks.is_empty());

    // Unknown clip index is rejected.
    assert!(execute_tool_call(
        "character_remove_morph_weight",
        &json!({ "clip_index": 99, "node_id": node_id, "target": "Smile", "time_secs": 0.5 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .is_err());
}

#[test]
fn ai_tool_list_vertices_paginates_cube_corners() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir();

    let created = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "kind": "cube", "size": 1.0 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let node_id = created["node_id"].as_u64().unwrap();

    let first = execute_tool_call(
        "mesh_list_vertices",
        &json!({ "node_id": node_id, "offset": 0, "limit": 4 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(first["vertex_count"], 8);
    assert_eq!(first["vertices"].as_array().unwrap().len(), 4);
    assert_eq!(first["vertices"][0]["vertex_id"], 0);
    assert_eq!(first["vertices"][0]["position"], json!([-0.5, 0.0, 0.5]));

    let rest = execute_tool_call(
        "mesh_list_vertices",
        &json!({ "node_id": node_id, "offset": 6, "limit": 64 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(rest["vertices"].as_array().unwrap().len(), 2);
    assert_eq!(rest["vertices"][1]["vertex_id"], 7);

    assert!(execute_tool_call(
        "mesh_list_vertices",
        &json!({ "node_id": 9999 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .is_err());
}

#[test]
fn lowpoly_1to1_modeling_and_humanoid_rig_tools_roundtrip_and_undo() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir();

    // 1. mesh_create_raw with per-face materials
    let raw = execute_tool_call(
        "mesh_create_raw",
        &json!({
            "name": "CustomVisor",
            "vertices": [
                [-0.1, 0.0, 0.0], [0.1, 0.0, 0.0], [0.1, 0.2, 0.0], [-0.1, 0.2, 0.0]
            ],
            "faces": [
                { "vertices": [0, 1, 2, 3], "palette_index": 2 }
            ],
            "smooth": true
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let raw_id = raw["node_id"].as_u64().unwrap();
    assert_eq!(raw["vertices"], 4);
    assert_eq!(raw["faces"], 1);

    // 2. mesh_set_vertices (batch) & mesh_set_face_material
    let batch = execute_tool_call(
        "mesh_set_vertices",
        &json!({
            "node_id": raw_id,
            "updates": [
                { "vertex_id": 2, "position": [0.12, 0.22, 0.02] },
                { "vertex_id": 3, "position": [-0.12, 0.22, 0.02] }
            ]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(batch["updated"], 2);

    let mat = execute_tool_call(
        "mesh_set_face_material",
        &json!({ "node_id": raw_id, "face_ids": [0], "palette_index": 5 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(mat["palette_index"], 5);

    // 3. mesh_taper, mesh_bend, mesh_mirror, mesh_set_origin
    execute_tool_call(
        "mesh_taper",
        &json!({ "node_id": raw_id, "axis": "y", "start_scale": 0.8, "end_scale": 1.1 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "mesh_bend",
        &json!({ "node_id": raw_id, "bend_axis": "x", "curve_axis": "z", "angle_deg": 35.0 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let mir = execute_tool_call(
        "mesh_mirror",
        &json!({ "node_id": raw_id, "axis": "x", "merge_original": true }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(mir["faces"], 2);
    execute_tool_call(
        "mesh_set_origin",
        &json!({ "node_id": raw_id, "mode": "center" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();

    // 4. Thin primitive (size 0.015m) is NOT clamped to 0.1m anymore!
    let thin = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "name": "Tie", "kind": "cube", "size": 0.015, "palette_index": 1 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(thin["clamped"], false);
    let thin_id = thin["node_id"].as_u64().unwrap();

    // 5. mesh_join merges thin into raw_id and deletes thin_id; undo restores both
    let joined = execute_tool_call(
        "mesh_join",
        &json!({ "target_node_id": raw_id, "source_node_ids": [thin_id] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(joined["faces"], 8);
    assert!(!scene.nodes.contains_key(&NodeId(thin_id)));
    history.undo(&mut scene).unwrap();
    assert!(scene.nodes.contains_key(&NodeId(thin_id)));

    // 6. Humanoid 19-bone rig (Left=+X, Right=-X) + character_set_bone_rest_position
    let rig = execute_tool_call(
        "character_apply_biped_rig",
        &json!({ "preset": "humanoid" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(rig["bones"], 19);
    let bones = execute_tool_call(
        "character_list_bones",
        &json!({}),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let left_shoulder = bones["bones"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == "LeftShoulder")
        .unwrap();
    assert!(left_shoulder["rest_position"][0].as_f64().unwrap() > 0.0);

    execute_tool_call(
        "character_set_bone_rest_position",
        &json!({ "bone_id": 4, "position": [0.0, 0.15, 0.01] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert!(
        (scene.character_rig.as_ref().unwrap().bones[4]
            .rest_local
            .translation
            .y
            - 0.15)
            .abs()
            < 1e-5
    );
}

#[test]
fn audit_p1_input_validation_and_transaction_undo_revision() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir();

    // 1. i64 wrapping bypass in parse_ivec3 must be rejected (4_294_967_296 wraps to 0 as i32)
    assert!(execute_tool_call(
        "voxel_fill_box",
        &json!({ "min": [4_294_967_296_i64, 0, 0], "max": [1, 1, 1], "palette_index": 4 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .is_err());

    // 2. Oversized ellipsoid fails fast before allocating edits
    assert!(matches!(
        execute_tool_call(
            "voxel_fill_ellipsoid",
            &json!({ "center": [0, 0, 0], "radii": [200.0, 200.0, 200.0], "palette_index": 4 }),
            &mut scene,
            &mut history,
            &dir,
        ),
        Err(AiError::BatchLimitExceeded)
    ));

    // 3. Out-of-range rgb in palette_set_color is rejected (no silent unwrap_or(0))
    assert!(execute_tool_call(
        "palette_set_color",
        &json!({ "palette_index": 9, "rgb": [300, 10, 10] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .is_err());

    // 4. Undo of a Transaction bumps scene.revision and sets dirty_viewport
    let created = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "name": "Box", "kind": "cube", "size": 1.0, "palette_index": 4 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let nid = created["node_id"].as_u64().unwrap();
    execute_tool_call(
        "mesh_set_origin",
        &json!({ "node_id": nid, "mode": "center" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    scene.dirty_viewport = false;
    let rev_before = scene.revision;
    history.undo(&mut scene).unwrap();
    assert!(scene.revision > rev_before);
    assert!(scene.dirty_viewport);
}

#[test]
fn texture_paint_line_rect_uv_project_and_local_face_subdivide_roundtrip() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir();

    let created = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "name": "Torso", "kind": "cube", "size": 1.0, "palette_index": 3 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let nid = created["node_id"].as_u64().unwrap();

    // Project cylindrical UVs
    let proj = execute_tool_call(
        "mesh_project_uvs",
        &json!({ "node_id": nid, "mode": "cylindrical" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(proj["faces_projected"], 6);
    assert_eq!(scene.lowpoly_meshes[&NodeId(nid)].faces[0].uvs.len(), 4);

    // Paint line & rect on RGBA texture + undo
    execute_tool_call(
        "mesh_paint_line",
        &json!({
            "node_id": nid,
            "uv_start": [0.1, 0.2],
            "uv_end": [0.9, 0.2],
            "width_px": 6.0,
            "palette_index": 1
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "mesh_paint_rect",
        &json!({
            "node_id": nid,
            "uv_min": [0.4, 0.4],
            "uv_max": [0.6, 0.6],
            "palette_index": 5
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert!(scene.lowpoly_meshes[&NodeId(nid)].texture.is_some());

    // Local face subdivision: 6 faces -> 1 quad replaced by 4 quads = 9 faces
    let sub = execute_tool_call(
        "mesh_subdivide_face",
        &json!({ "node_id": nid, "face_id": 0 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(sub["faces"], 9);
    history.undo(&mut scene).unwrap();
    assert_eq!(scene.lowpoly_meshes[&NodeId(nid)].faces.len(), 6);

    // Apply transform bakes world offset + scale into vertices so mirror_x works across world X=0
    execute_tool_call(
        "scene_set_node_transform",
        &json!({ "node_id": nid, "position": [0.3, 0.0, 0.0], "scale": [0.1, 0.1, 0.1] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "mesh_apply_transform",
        &json!({ "node_id": nid }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(
        scene.nodes[&NodeId(nid)].transform,
        tessel_core::Transform::default()
    );
    execute_tool_call(
        "mesh_mirror",
        &json!({ "node_id": nid, "axis": "x", "merge_original": true }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let (lo, hi) = scene.lowpoly_meshes[&NodeId(nid)].bounds();
    assert!((lo.x + 0.35).abs() < 1e-4);
    assert!((hi.x - 0.35).abs() < 1e-4);
}

#[test]
fn mcp_project_save_open_and_node_bone_mask_roundtrip() {
    let dir = std::env::temp_dir().join("tessel_mcp_proj_io_test");
    let _ = std::fs::remove_dir_all(&dir);
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();

    let created = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "name": "Legs", "kind": "cube", "size": 0.4, "palette_index": 7, "position": [-0.15, 0.7, 0.0] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let nid = created["node_id"].as_u64().unwrap();
    execute_tool_call(
        "character_apply_biped_rig",
        &json!({ "preset": "humanoid" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();

    // Restrict Legs node to Root(0) + leg bones (13..=18) so hanging RightHand(12) never pulls hip
    execute_tool_call(
        "character_set_node_bone_mask",
        &json!({ "node_id": nid, "bone_ids": [0, 13, 14, 15, 16, 17, 18] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let inspected = execute_tool_call(
        "character_inspect_weights",
        &json!({ "node_id": nid, "point": [-0.25, 0.85, 0.0] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    for inf in inspected["influences"].as_array().unwrap() {
        let j = inf["joint_index"].as_u64().unwrap();
        assert!(
            j == 0 || (13..=18).contains(&j),
            "unexpected arm bone influence {j} on masked legs node"
        );
    }

    // Save .tessel atomically inside allowed_export_dir
    let saved = execute_tool_call(
        "project_save",
        &json!({ "filename": "hero_checkpoint.tessel" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert!(saved["file_size_bytes"].as_u64().unwrap() > 0);

    // Path traversal must be denied on both save and open
    assert!(execute_tool_call(
        "project_save",
        &json!({ "filename": "../escape.tessel" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .is_err());
    assert!(execute_tool_call(
        "project_open",
        &json!({ "filename": "../escape.tessel" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .is_err());

    // Load into a fresh SceneState and verify rig + bone mask survived round-trip
    let mut fresh_scene = SceneState::new();
    let mut fresh_history = HistoryStack::new();
    let opened = execute_tool_call(
        "project_open",
        &json!({ "filename": "hero_checkpoint.tessel" }),
        &mut fresh_scene,
        &mut fresh_history,
        &dir,
    )
    .unwrap();
    assert_eq!(opened["has_character_rig"], true);
    assert!(fresh_scene
        .character_rig
        .as_ref()
        .unwrap()
        .node_bone_masks
        .contains_key(&NodeId(nid)));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mcp_v18_animation_scrub_pose_reset_rename_and_clear_roundtrip() {
    let dir = std::env::temp_dir();
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();

    // 1. scene_clear removes all nodes atomically and is undoable
    execute_tool_call(
        "mesh_create_primitive",
        &json!({ "name": "StarterA", "kind": "cube", "size": 0.5, "palette_index": 3 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "mesh_create_primitive",
        &json!({ "name": "StarterB", "kind": "sphere", "size": 0.5, "palette_index": 4 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let before = scene.nodes.len();
    assert_eq!(before, 2);
    let cleared =
        execute_tool_call("scene_clear", &json!({}), &mut scene, &mut history, &dir).unwrap();
    assert_eq!(cleared["removed_nodes"], before);
    assert!(scene.nodes.is_empty());
    history.undo(&mut scene).unwrap();
    assert_eq!(scene.nodes.len(), before);

    // 2. scene_rename_node renames cleanly + undo
    let created = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "name": "TempPart", "kind": "cube", "size": 0.5, "palette_index": 3 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let nid = created["node_id"].as_u64().unwrap();
    execute_tool_call(
        "scene_rename_node",
        &json!({ "node_id": nid, "name": "PUBG_Helmet" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(scene.nodes[&NodeId(nid)].name, "PUBG_Helmet");
    history.undo(&mut scene).unwrap();
    assert_eq!(scene.nodes[&NodeId(nid)].name, "TempPart");

    // 3. character_set_playback_time scrubs Walk clip live + character_reset_pose resets
    execute_tool_call(
        "character_apply_biped_rig",
        &json!({ "preset": "humanoid" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "character_set_playback_time",
        &json!({ "clip_name": "Walk", "time_secs": 0.25 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let rig = scene.character_rig.as_ref().unwrap();
    assert_eq!(rig.active_animation, 1);
    assert!(
        rig.bone(BoneId(13))
            .unwrap()
            .pose_local
            .rotation
            .angle_between(glam::Quat::IDENTITY)
            > 0.1
    );

    execute_tool_call(
        "character_reset_pose",
        &json!({}),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let rig = scene.character_rig.as_ref().unwrap();
    assert!(
        rig.bone(BoneId(13))
            .unwrap()
            .pose_local
            .rotation
            .angle_between(glam::Quat::IDENTITY)
            < 1e-5
    );
}

#[test]
fn mcp_v19_batch_atomic_rollback_weld_and_flip_normals() {
    let dir = std::env::temp_dir();
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();

    // 1. Atomic scene_batch succeeds and returns per-call results
    let ok = execute_tool_call(
        "scene_batch",
        &json!({
            "calls": [
                { "name": "mesh_create_primitive", "arguments": { "name": "Half", "kind": "cube", "size": 1.0, "palette_index": 4, "position": [0.5, 0.0, 0.0] } },
                { "name": "mesh_apply_transform", "arguments": {} },
                { "name": "mesh_mirror", "arguments": { "axis": "x", "merge_original": true } }
            ]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(ok["count"], 3);
    let nid = ok["results"][0]["node_id"].as_u64().unwrap();
    assert_eq!(scene.lowpoly_meshes[&NodeId(nid)].vertices.len(), 16);

    // 2. mesh_weld_vertices collapses the 4 duplicated center-seam vertices at X=0 (16 -> 12)
    let welded = execute_tool_call(
        "mesh_weld_vertices",
        &json!({ "node_id": nid, "tolerance": 0.001 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(welded["removed"], 4);
    assert_eq!(welded["vertices_after"], 12);

    // 3. mesh_flip_normals reverses face 0 winding
    let orig_f0 = scene.lowpoly_meshes[&NodeId(nid)].faces[0].vertices.clone();
    execute_tool_call(
        "mesh_flip_normals",
        &json!({ "node_id": nid, "face_ids": [0] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let mut expected = orig_f0;
    expected.reverse();
    assert_eq!(
        scene.lowpoly_meshes[&NodeId(nid)].faces[0].vertices,
        expected
    );

    // 4. Failed scene_batch rolls back completely (scene node count unchanged!)
    let count_before = scene.nodes.len();
    let failed = execute_tool_call(
        "scene_batch",
        &json!({
            "calls": [
                { "name": "mesh_create_primitive", "arguments": { "name": "WillRollback", "kind": "cube", "size": 1.0, "palette_index": 2 } },
                { "name": "scene_delete_node", "arguments": { "node_id": 999999 } }
            ]
        }),
        &mut scene,
        &mut history,
        &dir,
    );
    assert!(failed.is_err());
    assert_eq!(scene.nodes.len(), count_before);
}

#[test]
fn v27_mcp_friction_fixes_ik_morph_and_glb_roundtrip() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir().join("tessel_v27_mcp_roundtrip");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // 1. scene_set_mode accepts "Low-Poly" (matching scene_inspect output)
    let mode_res = execute_tool_call(
        "scene_set_mode",
        &json!({ "mode": "Low-Poly" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(mode_res["active_mode"], "Low-Poly");

    // 2. mesh_create_primitive accepts non-uniform size [sx, sy, sz]
    let prim = execute_tool_call(
        "mesh_create_primitive",
        &json!({
            "name": "Torso",
            "kind": "cube",
            "size": [0.42, 0.56, 0.24],
            "palette_index": 2
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let node_id = prim["node_id"].as_u64().unwrap();
    let (lo, hi) = scene.lowpoly_meshes[&tessel_core::NodeId(node_id)].bounds();
    assert!(((hi.x - lo.x) - 0.42).abs() < 1e-4 && ((hi.y - lo.y) - 0.56).abs() < 1e-4);

    // 3. Apply humanoid rig & verify character_solve_ik moves arm bones
    execute_tool_call(
        "character_apply_biped_rig",
        &json!({ "preset": "humanoid" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let rig = scene.character_rig.as_ref().unwrap();
    let upper = rig
        .bones
        .iter()
        .find(|b| b.name == "RightUpperArm")
        .unwrap()
        .id
        .0;
    let mid = rig
        .bones
        .iter()
        .find(|b| b.name == "RightForearm")
        .unwrap()
        .id
        .0;
    let end = rig
        .bones
        .iter()
        .find(|b| b.name == "RightHand")
        .unwrap()
        .id
        .0;
    let before_rot = rig
        .bone(tessel_core::BoneId(upper))
        .unwrap()
        .pose_local
        .rotation;
    execute_tool_call(
        "character_solve_ik",
        &json!({
            "root_bone_id": upper,
            "mid_bone_id": mid,
            "end_bone_id": end,
            "target": [-0.35, 1.35, 0.45],
            "pole": [-0.5, 1.1, -0.3]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let after_rot = scene
        .character_rig
        .as_ref()
        .unwrap()
        .bone(tessel_core::BoneId(upper))
        .unwrap()
        .pose_local
        .rotation;
    assert!(before_rot.angle_between(after_rot) > 1e-3);

    // 4. Verify morph target creation & weight application
    execute_tool_call(
        "mesh_morph_delta",
        &json!({
            "node_id": node_id,
            "name": "Breathe",
            "vertex_ids": [0, 1],
            "delta": [0.0, 0.04, 0.02]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let morph_res = execute_tool_call(
        "character_set_morph_weight",
        &json!({
            "clip_name": "Idle",
            "node_id": node_id,
            "target": "Breathe",
            "time_secs": 0.5,
            "weight": 0.75
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(morph_res["ok"], true);
    let idle = scene
        .character_rig
        .as_ref()
        .unwrap()
        .animations
        .iter()
        .find(|c| c.name == "Idle")
        .unwrap();
    assert!(idle
        .morph_tracks
        .iter()
        .any(|t| t.target == "Breathe" && !t.keys.is_empty()));

    // 5. Export .glb (using absolute path inside exports dir) & round-trip parse it
    let abs_out = dir.join("v27_character.glb");
    let exp = execute_tool_call(
        "project_export",
        &json!({
            "format": "glb",
            "filename": abs_out.to_string_lossy(),
            "preset": "godot4"
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert!(exp["file_size_bytes"].as_u64().unwrap() > 0);
    assert!(exp["triangles"].as_u64().unwrap() > 0);
    let bytes = std::fs::read(&abs_out).unwrap();
    let parsed =
        tessel_io::import_glb_bytes(&bytes, tessel_core::PaletteIndex::new(1).unwrap()).unwrap();
    assert!(!parsed.vertices.is_empty());
    assert!(!parsed.faces.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mesh_create_lathe_builds_helmet_dome_and_visor() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir();

    let dome = execute_tool_call(
        "mesh_create_lathe",
        &json!({
            "name": "AltynDome",
            "profile": [[0.14, 0.0], [0.15, 0.08], [0.11, 0.16], [0.0, 0.19]],
            "segments": 16,
            "angle_deg": 360.0,
            "cap_ends": true,
            "palette_index": 3,
            "position": [0.0, 1.55, 0.0]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(dome["ok"], true);
    assert_eq!(dome["vertices"], 49);
    assert_eq!(dome["faces"], 49);

    let visor = execute_tool_call(
        "mesh_create_lathe",
        &json!({
            "name": "AltynVisor",
            "profile": [[0.155, 0.0], [0.155, 0.10]],
            "segments": 8,
            "angle_deg": 160.0,
            "palette_index": 5
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(visor["ok"], true);
    assert_eq!(visor["faces"], 8);
}

#[test]
fn v29_ik_end_rotation_snapshot_keyframe_and_morph_bounds() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir();

    execute_tool_call(
        "character_apply_biped_rig",
        &json!({ "preset": "humanoid" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let rig = scene.character_rig.as_ref().unwrap();
    let upper = rig
        .bones
        .iter()
        .find(|b| b.name == "RightUpperArm")
        .unwrap()
        .id
        .0;
    let mid = rig
        .bones
        .iter()
        .find(|b| b.name == "RightForearm")
        .unwrap()
        .id
        .0;
    let end = rig
        .bones
        .iter()
        .find(|b| b.name == "RightHand")
        .unwrap()
        .id
        .0;

    // 1. Solve IK with pole direction + end_rotation_deg and inspect returned rotations_deg
    let ik = execute_tool_call(
        "character_solve_ik",
        &json!({
            "root_bone_id": upper,
            "mid_bone_id": mid,
            "end_bone_id": end,
            "target": [-0.25, 1.30, 0.40],
            "pole_vector": [0.0, -1.0, -1.0],
            "end_rotation_deg": [0.0, 20.0, 0.0]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert!(ik["rotations_deg"]["root"].is_array());
    assert!((ik["rotations_deg"]["end"][1].as_f64().unwrap() - 20.0).abs() < 1e-2);

    // 2. Snapshot solved pose directly into a new clip keyframe
    let snap = execute_tool_call(
        "character_snapshot_pose_to_keyframe",
        &json!({
            "clip_name": "AimClip",
            "duration_secs": 1.0,
            "time_secs": 0.5,
            "bone_ids": [upper, mid, end]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(snap["ok"], true);
    assert_eq!(snap["keyframed_bones"], 3);
    let aim_clip = scene
        .character_rig
        .as_ref()
        .unwrap()
        .animations
        .iter()
        .find(|c| c.name == "AimClip")
        .unwrap();
    assert_eq!(aim_clip.channels.len(), 3);

    // 3. Spatial bounding-box morph target selection (without manual vertex_ids)
    let mesh_res = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "name": "Head", "kind": "cube", "size": 1.0, "palette_index": 2 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let node_id = mesh_res["node_id"].as_u64().unwrap();
    let morph = execute_tool_call(
        "mesh_morph_delta",
        &json!({
            "node_id": node_id,
            "name": "RaiseTop",
            "bounds_min": [-1.0, 0.5, -1.0],
            "bounds_max": [1.0, 1.5, 1.0],
            "delta": [0.0, 0.2, 0.0]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(morph["ok"], true);
    assert_eq!(morph["affected_vertices"], 4);
}

#[test]
fn v30_extruded_polygon_and_ik_world_end_rotation() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir();

    // 1. Create necktie silhouette via mesh_create_extruded_polygon
    let tie = execute_tool_call(
        "mesh_create_extruded_polygon",
        &json!({
            "name": "Necktie",
            "points": [[-0.03, 0.0], [0.03, 0.0], [0.045, -0.32], [0.0, -0.38], [-0.045, -0.32]],
            "depth": 0.015,
            "palette_index": 1,
            "position": [0.0, 1.42, 0.12]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(tie["ok"], true);
    assert_eq!(tie["vertices"], 10);
    assert_eq!(tie["faces"], 11); // 3 front tris + 3 back tris + 5 side quads

    // 2. IK with pole_target (world point) + end_world_rotation_deg (world-space aim)
    execute_tool_call(
        "character_apply_biped_rig",
        &json!({ "preset": "humanoid" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let rig = scene.character_rig.as_ref().unwrap();
    let upper = rig
        .bones
        .iter()
        .find(|b| b.name == "RightUpperArm")
        .unwrap()
        .id;
    let mid = rig
        .bones
        .iter()
        .find(|b| b.name == "RightForearm")
        .unwrap()
        .id;
    let end = rig.bones.iter().find(|b| b.name == "RightHand").unwrap().id;

    execute_tool_call(
        "character_solve_ik",
        &json!({
            "root_bone_id": upper.0,
            "mid_bone_id": mid.0,
            "end_bone_id": end.0,
            "target": [-0.18, 1.25, 0.35],
            "pole_target": [-0.35, 0.90, -0.15],
            "end_world_rotation_deg": [0.0, 0.0, 0.0]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let end_world_mat = scene.character_rig.as_ref().unwrap().world_pose_matrices()[&end];
    let end_world_rot = glam::Quat::from_mat4(&end_world_mat).normalize();
    assert!(
        end_world_rot.angle_between(glam::Quat::IDENTITY) < 1e-2,
        "end_bone_id world rotation should match identity [0,0,0] regardless of forearm IK"
    );
}

#[test]
fn v32_loft_mesh_and_multi_character_subrig_append() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir();

    // 1. Create tapered torso via mesh_create_loft
    let loft = execute_tool_call(
        "mesh_create_loft",
        &json!({
            "name": "TaperedTorso",
            "sections": [
                [[0.0, 0.0, 0.2], [0.25, 0.0, 0.0], [0.0, 0.0, -0.2], [-0.25, 0.0, 0.0]],
                [[0.0, 0.5, 0.16], [0.20, 0.5, 0.0], [0.0, 0.5, -0.16], [-0.20, 0.5, 0.0]]
            ],
            "cap_ends": true,
            "palette_index": 3
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(loft["ok"], true);
    assert_eq!(loft["vertices"], 8);
    assert_eq!(loft["faces"], 6);

    // 2. Apply P1 humanoid rig, then append P2 humanoid sub-rig at z=2.0 facing -Z (yaw 180)
    execute_tool_call(
        "character_apply_biped_rig",
        &json!({ "preset": "humanoid" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let p2 = execute_tool_call(
        "character_apply_biped_rig",
        &json!({
            "preset": "humanoid",
            "append": true,
            "prefix": "P2_",
            "offset": [0.0, 0.0, 2.0],
            "yaw_deg": 180.0
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(p2["ok"], true);
    assert_eq!(p2["bones_added"], 19);
    assert_eq!(p2["bone_ids"]["P2_Root"], 19);
    assert_eq!(scene.character_rig.as_ref().unwrap().bones.len(), 38);
}

#[test]
fn v33_scene_batch_chains_prior_results_and_bakes_vector_size() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir();

    let batch = execute_tool_call(
        "scene_batch",
        &json!({
            "calls": [
                {
                    "name": "mesh_create_primitive",
                    "arguments": {
                        "name": "Helmet",
                        "kind": "cube",
                        "size": [0.3, 0.2, 0.25],
                        "palette_index": 3
                    }
                },
                {
                    "name": "character_apply_biped_rig",
                    "arguments": { "preset": "humanoid" }
                },
                {
                    "name": "character_bind_node",
                    "arguments": {
                        "node_id": "$0.node_id",
                        "bone_id": 4
                    }
                }
            ]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(batch["ok"], true);
    assert_eq!(batch["count"], 3);
    let helmet_id = batch["results"][0]["node_id"].as_u64().unwrap();
    let actual_sz = batch["results"][0]["actual_size"].as_array().unwrap();
    assert!((actual_sz[0].as_f64().unwrap() - 0.3).abs() < 1e-5);
    assert!((actual_sz[1].as_f64().unwrap() - 0.2).abs() < 1e-5);
    assert!((actual_sz[2].as_f64().unwrap() - 0.25).abs() < 1e-5);
    let (lo, hi) = scene.lowpoly_meshes[&tessel_core::NodeId(helmet_id)].bounds();
    assert!(((hi.x - lo.x) - 0.3).abs() < 1e-4);
    assert!(((hi.y - lo.y) - 0.2).abs() < 1e-4);
    assert!(((hi.z - lo.z) - 0.25).abs() < 1e-4);
    assert!(scene
        .character_rig
        .as_ref()
        .unwrap()
        .bindings
        .iter()
        .any(|b| b.node_id == tessel_core::NodeId(helmet_id) && b.bone_id == BoneId(4)));
}

#[test]
fn v34_closes_all_gaps_v2_items() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir();

    // 2.2 & 2.3: Append P2 humanoid facing -Z (yaw 180) with default clips remapped
    execute_tool_call(
        "character_apply_biped_rig",
        &json!({ "preset": "humanoid" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let p2 = execute_tool_call(
        "character_apply_biped_rig",
        &json!({
            "preset": "humanoid",
            "append": true,
            "prefix": "P2_",
            "offset": [0.0, 0.0, 2.0],
            "yaw_deg": 180.0
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let p2_root = p2["bone_ids"]["P2_Root"].as_u64().unwrap() as u32;
    let p2_left_thigh = p2["bone_ids"]["P2_LeftThigh"].as_u64().unwrap() as u32;

    // Walk clip now animates P2's LeftThigh automatically (#2.2)
    let walk = scene
        .character_rig
        .as_ref()
        .unwrap()
        .animations
        .iter()
        .find(|c| c.name == "Walk")
        .unwrap();
    assert!(walk
        .channels
        .iter()
        .any(|c| c.bone_id == BoneId(p2_left_thigh)));

    // P2_Root carries yaw=180 rest rotation (#2.3) so child local axes are character-local
    let p2_root_bone = scene
        .character_rig
        .as_ref()
        .unwrap()
        .bone(BoneId(p2_root))
        .unwrap();
    assert!(
        p2_root_bone
            .rest_local
            .rotation
            .angle_between(glam::Quat::from_rotation_y(std::f32::consts::PI))
            < 1e-4
    );

    // 2.5: Pose supports translation + snapshot preserves translation delta
    execute_tool_call(
        "character_set_bone_pose",
        &json!({
            "bone_id": p2_root,
            "translation": [0.25, -0.1, 0.0],
            "rotation_deg": [0.0, 15.0, 0.0]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "character_snapshot_pose_to_keyframe",
        &json!({
            "clip_name": "Dodge",
            "duration_secs": 1.0,
            "time_secs": 0.4,
            "bone_ids": [p2_root]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let dodge = scene
        .character_rig
        .as_ref()
        .unwrap()
        .animations
        .iter()
        .find(|c| c.name == "Dodge")
        .unwrap();
    let kf = dodge.sample_bone(BoneId(p2_root), 0.4).unwrap();
    assert!((kf.translation.x - 0.25).abs() < 1e-4);
    assert!((kf.translation.y - (-0.1)).abs() < 1e-4);

    // 2.4: Per-node visibility keyframes toggle live visibility on playback scrub
    let flash = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "name": "MuzzleFlash", "kind": "sphere", "size": 0.08, "palette_index": 6 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let flash_id = flash["node_id"].as_u64().unwrap();
    execute_tool_call(
        "character_set_visibility_keyframe",
        &json!({ "clip_name": "Dodge", "node_id": flash_id, "time_secs": 0.0, "visible": false }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "character_set_visibility_keyframe",
        &json!({ "clip_name": "Dodge", "node_id": flash_id, "time_secs": 0.3, "visible": true }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "character_set_visibility_keyframe",
        &json!({ "clip_name": "Dodge", "node_id": flash_id, "time_secs": 0.5, "visible": false }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();

    execute_tool_call(
        "character_set_playback_time",
        &json!({ "clip_name": "Dodge", "time_secs": 0.1 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert!(!scene.character_rig.as_ref().unwrap().node_visibility[&NodeId(flash_id)]);
    execute_tool_call(
        "character_set_playback_time",
        &json!({ "clip_name": "Dodge", "time_secs": 0.35 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert!(scene.character_rig.as_ref().unwrap().node_visibility[&NodeId(flash_id)]);
}

#[test]
fn v36_clip_layers_interpolation_and_floor_clamp() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir();

    execute_tool_call(
        "character_apply_biped_rig",
        &json!({ "preset": "humanoid" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let p2 = execute_tool_call(
        "character_apply_biped_rig",
        &json!({
            "preset": "humanoid",
            "append": true,
            "prefix": "P2_",
            "offset": [0.0, 0.0, 2.0],
            "yaw_deg": 180.0
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let p2_root = p2["bone_ids"]["P2_Root"].as_u64().unwrap() as u32;

    // 1. Step & ease_in_out keyframe interpolation on ShotFall clip
    execute_tool_call(
        "character_set_keyframe",
        &json!({
            "clip_name": "ShotFall",
            "duration_secs": 1.0,
            "bone_id": p2_root,
            "time_secs": 0.0,
            "rotation_deg": [0.0, 0.0, 0.0],
            "interpolation": "step"
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "character_set_keyframe",
        &json!({
            "clip_name": "ShotFall",
            "bone_id": p2_root,
            "time_secs": 0.5,
            "rotation_deg": [-85.0, 0.0, 0.0],
            "interpolation": "ease_out"
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let shot = scene
        .character_rig
        .as_ref()
        .unwrap()
        .animations
        .iter()
        .find(|c| c.name == "ShotFall")
        .unwrap();
    // Step interpolation holds 0° until t=0.5s
    let mid_kf = shot.sample_bone(BoneId(p2_root), 0.25).unwrap();
    assert!(mid_kf.rotation.angle_between(glam::Quat::IDENTITY) < 1e-4);

    // 2. Simultaneous per-character clip layers: P1 Walk + P2 ShotFall
    execute_tool_call(
        "character_set_clip_layer",
        &json!({ "clip_name": "ShotFall", "bone_prefix": "P2_" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "character_set_playback_time",
        &json!({ "clip_name": "Walk", "time_secs": 0.5 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let rig = scene.character_rig.as_ref().unwrap();
    let p2_bone = rig.bone(BoneId(p2_root)).unwrap();
    let p2_delta = p2_bone.rest_local.rotation.inverse() * p2_bone.pose_local.rotation;
    assert!(p2_delta.angle_between(glam::Quat::IDENTITY) > 1.0);

    // 3. Floor clamp shifts P2_Root Y so lowest joint sits flush on y = 0.0
    let clamp = execute_tool_call(
        "character_clamp_to_floor",
        &json!({ "floor_y": 0.0, "bone_prefix": "P2_" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(clamp["ok"], true);
    let mats = scene.character_rig.as_ref().unwrap().world_pose_matrices();
    let min_y = scene
        .character_rig
        .as_ref()
        .unwrap()
        .bones
        .iter()
        .filter(|b| b.name.starts_with("P2_"))
        .map(|b| mats[&b.id].transform_point3(glam::Vec3::ZERO).y)
        .fold(f32::INFINITY, f32::min);
    assert!(
        min_y.abs() < 1e-3,
        "min_y after floor clamp should be ~0, got {min_y}"
    );
}

#[test]
fn v39_snapshot_interpolation_bake_clip_layers_and_subframe_floor_clamp() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir();

    execute_tool_call(
        "mesh_create_primitive",
        &json!({ "name": "HeroBody", "kind": "cube", "size": 1.0 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "character_apply_biped_rig",
        &json!({ "preset": "starter" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let p2 = execute_tool_call(
        "character_apply_biped_rig",
        &json!({
            "preset": "starter",
            "append": true,
            "prefix": "P2_",
            "offset": [0.0, 0.0, 2.0],
            "yaw_deg": 180.0
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let p2_root = p2["bone_ids"]["P2_Root"].as_u64().unwrap() as u32;

    // 1. character_snapshot_pose_to_keyframe with interpolation: "ease_out"
    execute_tool_call(
        "character_set_bone_pose",
        &json!({ "bone_id": p2_root, "rotation_deg": [-86.0, 0.0, 0.0] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "character_snapshot_pose_to_keyframe",
        &json!({
            "clip_name": "FallArc",
            "duration_secs": 1.0,
            "time_secs": 0.0,
            "bone_ids": [p2_root],
            "interpolation": "ease_out"
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    // Reset root to upright at t=0 and fallen at t=1.0
    execute_tool_call(
        "character_set_keyframe",
        &json!({
            "clip_name": "FallArc",
            "bone_id": p2_root,
            "time_secs": 0.0,
            "rotation_deg": [0.0, 0.0, 0.0],
            "interpolation": "ease_out"
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "character_set_keyframe",
        &json!({
            "clip_name": "FallArc",
            "bone_id": p2_root,
            "time_secs": 1.0,
            "rotation_deg": [-86.0, 0.0, 0.0],
            "interpolation": "linear"
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let fall_clip = scene
        .character_rig
        .as_ref()
        .unwrap()
        .animations
        .iter()
        .find(|c| c.name == "FallArc")
        .unwrap();
    let ch = fall_clip
        .channels
        .iter()
        .find(|c| c.bone_id == BoneId(p2_root))
        .unwrap();
    assert_eq!(
        ch.keyframes[0].interpolation,
        tessel_core::KeyframeInterpolation::EaseOut
    );

    // 2. Sub-frame floor clamp keeps P2 flush on y=0.0 even at mid-arc t=0.5s
    let clamped = execute_tool_call(
        "character_clamp_clip_to_floor",
        &json!({ "clip_name": "FallArc", "bone_prefix": "P2_", "floor_y": 0.0, "fps": 20 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(clamped["ok"], true);
    execute_tool_call(
        "character_set_playback_time",
        &json!({ "clip_name": "FallArc", "time_secs": 0.5 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let mats = scene.character_rig.as_ref().unwrap().world_pose_matrices();
    let mid_min_y = scene
        .character_rig
        .as_ref()
        .unwrap()
        .bones
        .iter()
        .filter(|b| b.name.starts_with("P2_"))
        .map(|b| mats[&b.id].transform_point3(glam::Vec3::ZERO).y)
        .fold(f32::INFINITY, f32::min);
    assert!(
        mid_min_y.abs() < 0.02,
        "mid-arc min_y after sub-frame floor clamp should be ~0, got {mid_min_y}"
    );

    // 3. Bake multi-character clip_layers into 1 standalone clip ready for .glb
    execute_tool_call(
        "character_set_clip_layer",
        &json!({ "clip_name": "Walk", "bone_prefix": "" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "character_set_clip_layer",
        &json!({ "clip_name": "FallArc", "bone_prefix": "P2_" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let baked = execute_tool_call(
        "character_bake_clip_layers",
        &json!({ "name": "CombinedBattle", "fps": 12, "floor_y": 0.0, "clear_layers": true }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(baked["ok"], true);
    assert_eq!(baked["name"], "CombinedBattle");
    assert!(baked["channels"].as_u64().unwrap() >= 14);
    assert!(scene.character_rig.as_ref().unwrap().clip_layers.is_empty());

    // Verify .glb export includes the baked clip cleanly
    execute_tool_call(
        "scene_set_mode",
        &json!({ "mode": "character" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let (glb, _) = tessel_io::export_glb_bytes(&scene, tessel_io::ExportPreset::Godot4).unwrap();
    assert!(glb.starts_with(b"glTF"));
    let json_len = u32::from_le_bytes([glb[12], glb[13], glb[14], glb[15]]) as usize;
    let glb_json = std::str::from_utf8(&glb[20..20 + json_len]).unwrap();
    assert!(glb_json.contains("\"CombinedBattle\""));
}

#[test]
fn v40_dynamic_parent_switch_weapon_drop_and_glb_export() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir();

    execute_tool_call(
        "scene_set_mode",
        &json!({ "mode": "character" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let rifle = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "name": "Rifle", "kind": "cube", "size": [0.1, 0.1, 0.8] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let rifle_id = rifle["node_id"].as_u64().unwrap();
    execute_tool_call(
        "character_apply_biped_rig",
        &json!({ "preset": "starter" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let right_arm = scene
        .character_rig
        .as_ref()
        .unwrap()
        .bones
        .iter()
        .find(|b| b.name == "RightArm")
        .unwrap()
        .id
        .0 as u64;

    // Hold rifle in RightArm at t=0.0, detach & drop to floor at t=0.5 -> t=1.0
    execute_tool_call(
        "character_set_parent_switch_keyframe",
        &json!({
            "clip_name": "WeaponDrop",
            "duration_secs": 1.0,
            "node_id": rifle_id,
            "time_secs": 0.0,
            "bone_id": right_arm
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "character_set_parent_switch_keyframe",
        &json!({
            "clip_name": "WeaponDrop",
            "node_id": rifle_id,
            "time_secs": 0.5,
            "bone_id": null,
            "position": [0.5, 0.8, 0.3],
            "rotation_deg": [0.0, 0.0, 0.0]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "character_set_parent_switch_keyframe",
        &json!({
            "clip_name": "WeaponDrop",
            "node_id": rifle_id,
            "time_secs": 1.0,
            "bone_id": null,
            "position": [0.8, 0.05, 0.6],
            "rotation_deg": [0.0, 45.0, 90.0]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();

    // At t=0.25 rifle is still attached to RightArm
    execute_tool_call(
        "character_set_playback_time",
        &json!({ "clip_name": "WeaponDrop", "time_secs": 0.25 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let ov_hold = scene
        .character_rig
        .as_ref()
        .unwrap()
        .node_parent_overrides
        .get(&NodeId(rifle_id))
        .copied()
        .unwrap();
    assert_eq!(ov_hold.bone_id, Some(BoneId(right_arm as u32)));

    // At t=0.75 rifle is detached and interpolated halfway along drop arc
    execute_tool_call(
        "character_set_playback_time",
        &json!({ "clip_name": "WeaponDrop", "time_secs": 0.75 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let ov_drop = scene
        .character_rig
        .as_ref()
        .unwrap()
        .node_parent_overrides
        .get(&NodeId(rifle_id))
        .copied()
        .unwrap();
    assert_eq!(ov_drop.bone_id, None);
    assert!((ov_drop.position.x - 0.65).abs() < 1e-3);
    assert!((ov_drop.position.y - 0.425).abs() < 1e-3);

    // Export .glb includes WeaponDrop animation with translation + rotation channels
    let (glb, _) = tessel_io::export_glb_bytes(&scene, tessel_io::ExportPreset::Godot4).unwrap();
    assert!(glb.starts_with(b"glTF"));
    let json_len = u32::from_le_bytes([glb[12], glb[13], glb[14], glb[15]]) as usize;
    let glb_json = std::str::from_utf8(&glb[20..20 + json_len]).unwrap();
    assert!(glb_json.contains("\"WeaponDrop\""));
    assert!(glb_json.contains("\"translation\""));
    assert!(glb_json.contains("\"rotation\""));
}

#[test]
fn floor_clamp_uses_lowest_skinned_mesh_vertex_below_foot_joint() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir();

    // Create a boot mesh whose sole sits at y = -0.06 (below humanoid foot joint).
    let boot = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "name": "LeftBoot", "kind": "cube", "size": [0.18, 0.14, 0.28] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let boot_id = boot["node_id"].as_u64().unwrap();
    execute_tool_call(
        "scene_set_node_transform",
        &json!({ "node_id": boot_id, "position": [0.11, -0.06, 0.0] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();

    execute_tool_call(
        "character_apply_biped_rig",
        &json!({ "preset": "humanoid" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();

    // 1. Live pose floor clamp lifts Root by +0.06 so boot sole sits at y = 0.0.
    let clamp = execute_tool_call(
        "character_clamp_to_floor",
        &json!({ "floor_y": 0.0 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(clamp["ok"], true);
    let min_y_before = clamp["min_y_before"].as_f64().unwrap() as f32;
    assert!(
        (min_y_before - (-0.06)).abs() < 1e-3,
        "expected skinned boot sole min_y_before = -0.06, got {min_y_before}"
    );
    let cache =
        tessel_core::SkinnedBoundsCache::from_scene(&scene, scene.character_rig.as_ref().unwrap())
            .unwrap();
    let min_y_after = cache
        .sub_skeleton_min_y(scene.character_rig.as_ref().unwrap(), "")
        .unwrap();
    assert!(
        min_y_after.abs() < 1e-3,
        "expected skinned boot sole min_y_after ~ 0.0, got {min_y_after}"
    );

    // 2. Clip floor clamp keeps skinned boot sole flush on y = 0.0 across Walk frames.
    let clip_clamp = execute_tool_call(
        "character_clamp_clip_to_floor",
        &json!({ "clip_name": "Walk", "floor_y": 0.0, "fps": 12 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(clip_clamp["ok"], true);
    execute_tool_call(
        "character_set_playback_time",
        &json!({ "clip_name": "Walk", "time_secs": 0.25 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let walk_min_y = cache
        .sub_skeleton_min_y(scene.character_rig.as_ref().unwrap(), "")
        .unwrap();
    assert!(
        walk_min_y.abs() < 0.02,
        "expected clamped Walk skinned boot sole min_y ~ 0.0, got {walk_min_y}"
    );
}

#[test]
fn mcp_quick_wins_batch_schema_double_sided_duplicate_bulk_keyframes_and_playing() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir();

    // 1. scene_batch schema maxItems is synchronized to 512
    let batch_def = crate::tools::tool_definitions()
        .into_iter()
        .find(|t| t["name"] == "scene_batch")
        .unwrap();
    assert_eq!(
        batch_def["inputSchema"]["properties"]["calls"]["maxItems"],
        512
    );

    // 2. mesh_set_material supports double_sided and exports glTF doubleSided: true
    let grass = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "name": "GrassBlade", "kind": "plane", "size": 0.4 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let grass_id = grass["node_id"].as_u64().unwrap();
    let mat_res = execute_tool_call(
        "mesh_set_material",
        &json!({ "node_id": grass_id, "double_sided": true, "roughness": 0.9 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(mat_res["double_sided"], true);
    assert!(
        scene.lowpoly_meshes[&NodeId(grass_id)]
            .material
            .double_sided
    );

    // 3. scene_duplicate_node accepts position, rotation_deg, scale directly
    let dup = execute_tool_call(
        "scene_duplicate_node",
        &json!({
            "node_id": grass_id,
            "name": "GrassBlade_2",
            "position": [1.5, 0.0, -2.0],
            "rotation_deg": [0.0, 45.0, 0.0],
            "scale": [1.2, 1.5, 1.2]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let dup_id = NodeId(dup["node_id"].as_u64().unwrap());
    let dup_tr = scene.nodes[&dup_id].transform;
    assert!((dup_tr.translation - glam::Vec3::new(1.5, 0.0, -2.0)).length() < 1e-4);
    assert!((dup_tr.scale - glam::Vec3::new(1.2, 1.5, 1.2)).length() < 1e-4);

    // 4. character_set_keyframe supports bulk keyframes in 1 atomic call
    execute_tool_call(
        "character_apply_biped_rig",
        &json!({ "preset": "starter" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let bulk = execute_tool_call(
        "character_set_keyframe",
        &json!({
            "clip_name": "WindSway",
            "duration_secs": 2.0,
            "keyframes": [
                { "bone_id": 1, "time_secs": 0.0, "rotation_deg": [0.0, 0.0, 0.0] },
                { "bone_id": 1, "time_secs": 1.0, "rotation_deg": [0.0, 0.0, 12.0] },
                { "bone_id": 2, "time_secs": 0.0, "rotation_deg": [0.0, 0.0, 0.0] },
                { "bone_id": 2, "time_secs": 1.0, "rotation_deg": [0.0, 0.0, -8.0] }
            ]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(bulk["ok"], true);
    assert_eq!(bulk["count"], 4);

    // 5. character_set_playback_time supports playing: true/false even while in Low-Poly mode
    execute_tool_call(
        "scene_set_mode",
        &json!({ "mode": "lowpoly" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let play_res = execute_tool_call(
        "character_set_playback_time",
        &json!({ "clip_name": "WindSway", "time_secs": 1.0, "playing": true }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(play_res["playing"], true);
    assert!(scene.character_rig.as_ref().unwrap().playing);
    let pause_res = execute_tool_call(
        "character_set_playback_time",
        &json!({ "playing": false }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(pause_res["playing"], false);
    assert!(!scene.character_rig.as_ref().unwrap().playing);
}

#[test]
fn scene_group_and_per_node_transform_animation_without_bones_exports_valid_glb() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir();

    // 1. Create an empty Group folder "Trees" at [2, 0, 0]
    let grp = execute_tool_call(
        "scene_create_group",
        &json!({ "name": "Trees", "position": [2.0, 0.0, 0.0] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let grp_id = grp["node_id"].as_u64().unwrap();
    assert_eq!(
        scene.nodes[&NodeId(grp_id)].kind,
        tessel_core::NodeKind::Group
    );

    // 2. Create a Pine mesh under "Trees" and a static Rock at origin
    let pine = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "name": "Pine_1", "kind": "cone", "size": 1.0, "position": [0.5, 0.0, 0.0] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let pine_id = pine["node_id"].as_u64().unwrap();
    execute_tool_call(
        "scene_set_parent",
        &json!({ "node_id": pine_id, "parent_node_id": grp_id }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let rock = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "name": "Rock_Static", "kind": "cube", "size": 0.6 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let rock_id = rock["node_id"].as_u64().unwrap();

    // 3. Animate the Group and Pine directly via scene_set_node_keyframe (0 bones!)
    let kf_res = execute_tool_call(
        "scene_set_node_keyframe",
        &json!({
            "clip_name": "WindBreeze",
            "duration_secs": 2.0,
            "keyframes": [
                { "node_id": grp_id, "time_secs": 0.0, "translation": [0.0, 0.0, 0.0] },
                { "node_id": grp_id, "time_secs": 1.0, "translation": [0.0, 0.5, 0.0] },
                { "node_id": pine_id, "time_secs": 0.0, "rotation_deg": [0.0, 0.0, 0.0] },
                { "node_id": pine_id, "time_secs": 1.0, "rotation_deg": [0.0, 0.0, 15.0] }
            ]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(kf_res["ok"], true);
    assert_eq!(kf_res["count"], 4);
    assert!(scene.character_rig.as_ref().unwrap().bones.is_empty());

    // 4. Scrub to t=1.0: Pine world Y lifts by +0.5 from parent Group; Rock stays at Y=0
    execute_tool_call(
        "character_set_playback_time",
        &json!({ "clip_name": "WindBreeze", "time_secs": 1.0 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let pine_world = scene
        .node_world_matrix(NodeId(pine_id))
        .transform_point3(glam::Vec3::ZERO);
    assert!((pine_world.x - 2.5).abs() < 1e-3);
    assert!((pine_world.y - 0.5).abs() < 1e-3);
    let rock_world = scene
        .node_world_matrix(NodeId(rock_id))
        .transform_point3(glam::Vec3::ZERO);
    assert!(rock_world.length() < 1e-4);

    // 5. Export .glb and validate Group hierarchy + WindBreeze animation channels
    let (glb, _) = tessel_io::export_glb_bytes(&scene, tessel_io::ExportPreset::Godot4).unwrap();
    assert!(glb.starts_with(b"glTF"));
    let json_len = u32::from_le_bytes([glb[12], glb[13], glb[14], glb[15]]) as usize;
    let glb_json: Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();
    let nodes = glb_json["nodes"].as_array().unwrap();
    let trees_node = nodes.iter().find(|n| n["name"] == "Trees").unwrap();
    let pine_idx = nodes.iter().position(|n| n["name"] == "Pine_1").unwrap();
    let trees_children = trees_node["children"].as_array().unwrap();
    assert!(
        trees_children
            .iter()
            .any(|c| c.as_u64() == Some(pine_idx as u64)),
        "Trees group must parent Pine_1 in glTF"
    );
    let anims = glb_json["animations"].as_array().unwrap();
    let wind = anims.iter().find(|a| a["name"] == "WindBreeze").unwrap();
    assert!(wind["channels"].as_array().unwrap().len() >= 6);
}

#[test]
fn v41_terrain_generator_and_surface_raycast_grounding_with_foot_ik() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir();

    // 1. Procedural fBm terrain with palette bands
    let island = execute_tool_call(
        "mesh_create_terrain",
        &json!({
            "name": "IslandDiorama",
            "width": 8.0,
            "depth": 8.0,
            "subdiv_x": 8,
            "subdiv_z": 8,
            "height_scale": 2.0,
            "seed": 7,
            "island_falloff": 0.6,
            "palette_bands": [2, 3, 4]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(island["ok"], true);
    assert_eq!(island["vertices"], 81);
    assert_eq!(island["triangles"], 128);

    // 2. Explicit sloped ramp terrain: y rises linearly from 0.5 at x=-2 to 2.5 at x=+2
    // (subdiv_x=2, subdiv_z=2 -> 3x3 grid; at x=0 y=1.5, at x=+0.11 y=1.555, at x=-0.11 y=1.445)
    let ramp = execute_tool_call(
        "mesh_create_terrain",
        &json!({
            "name": "SlopedRamp",
            "width": 4.0,
            "depth": 4.0,
            "subdiv_x": 2,
            "subdiv_z": 2,
            "height_scale": 1.0,
            "heights": [
                0.5, 1.5, 2.5,
                0.5, 1.5, 2.5,
                0.5, 1.5, 2.5
            ]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let ramp_id = ramp["node_id"].as_u64().unwrap();

    // 3. Apply humanoid rig at origin and ground onto SlopedRamp with solve_foot_ik=true
    execute_tool_call(
        "character_apply_biped_rig",
        &json!({ "preset": "humanoid" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let grounded = execute_tool_call(
        "character_clamp_to_floor",
        &json!({
            "surface_node_id": ramp_id,
            "solve_foot_ik": true
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(grounded["ok"], true);
    let floor_y = grounded["floor_y"].as_f64().unwrap() as f32;
    assert!(
        floor_y > 1.3 && floor_y < 1.6,
        "expected character grounded onto sloped ramp near y ~ 1.445, got {floor_y}"
    );

    // Verify uphill LeftFoot (+X) was raised higher than downhill RightFoot (-X) by leg IK
    let rig = scene.character_rig.as_ref().unwrap();
    let mats = rig.world_pose_matrices();
    let lf = rig.bones.iter().find(|b| b.name == "LeftFoot").unwrap().id;
    let rf = rig.bones.iter().find(|b| b.name == "RightFoot").unwrap().id;
    let ly = mats[&lf].transform_point3(glam::Vec3::ZERO).y;
    let ry = mats[&rf].transform_point3(glam::Vec3::ZERO).y;
    assert!(
        ly > ry + 0.05,
        "expected uphill LeftFoot ({ly}) higher than downhill RightFoot ({ry}) via leg IK"
    );
}

#[test]
fn mesh_bake_maps_bakes_attaches_ao_and_exports_pngs() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir().join("tessel_v42_bake_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let created = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "name": "Crate", "kind": "cube", "size": 1.0 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let node_id = NodeId(created["node_id"].as_u64().unwrap());

    let res = execute_tool_call(
        "mesh_bake_maps",
        &json!({
            "node_id": node_id.0,
            "maps": ["ao", "curvature", "thickness"],
            "resolution": 32,
            "samples": 8,
            "dilation_px": 2,
            "output_prefix": "crate_baked",
            "attach_ao": true
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(res["ok"], true);
    assert_eq!(res["ao_attached"], true);
    assert!(res["covered_texels"].as_u64().unwrap() > 0);
    assert!(scene.lowpoly_meshes[&node_id].ao_map.is_some());
    assert!(dir.join("crate_baked_ao.png").exists());
    assert!(dir.join("crate_baked_curvature.png").exists());
    assert!(dir.join("crate_baked_thickness.png").exists());

    // Undo detaches the AO map cleanly.
    history.undo(&mut scene).unwrap();
    assert!(scene.lowpoly_meshes[&node_id].ao_map.is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mesh_paint_spot_across_seams_with_camera_culling_and_undo() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir().join("tessel_v43_paint_seams_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let created = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "name": "SeamCube", "kind": "cube", "size": 1.0 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let node_id = NodeId(created["node_id"].as_u64().unwrap());

    // Paint right on the +X (x=0.5) / +Y (y=1.0) seam edge with camera visibility culling enabled.
    let res = execute_tool_call(
        "mesh_paint_spot",
        &json!({
            "node_id": node_id.0,
            "point": [0.5, 1.0, 0.0],
            "radius_world": 0.25,
            "across_seams": true,
            "camera_pos": [3.0, 3.0, 0.0],
            "palette_index": 5
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(res["ok"], true);
    assert_eq!(res["across_seams"], true);
    assert_eq!(res["camera_culled"], true);

    let mesh = &scene.lowpoly_meshes[&node_id];
    let tex = mesh.texture.as_ref().expect("texture created by 3D paint");
    let (min_b, max_b) = mesh.bounds();
    let expected_color = scene
        .palette
        .get(PaletteIndex::new(5).unwrap())
        .unwrap()
        .color;
    let expected_px = [
        expected_color.r,
        expected_color.g,
        expected_color.b,
        expected_color.a,
    ];
    let sample = |uv: [f32; 2]| -> [u8; 4] {
        let px = ((uv[0].clamp(0.0, 1.0) * (tex.width - 1) as f32).round() as usize)
            .min(tex.width as usize - 1);
        let py = ((uv[1].clamp(0.0, 1.0) * (tex.height - 1) as f32).round() as usize)
            .min(tex.height as usize - 1);
        let off = (py * tex.width as usize + px) * 4;
        [
            tex.rgba[off],
            tex.rgba[off + 1],
            tex.rgba[off + 2],
            tex.rgba[off + 3],
        ]
    };
    let uv_x =
        tessel_core::box_atlas_uv(glam::Vec3::new(0.5, 0.92, 0.0), glam::Vec3::X, min_b, max_b);
    let uv_y =
        tessel_core::box_atlas_uv(glam::Vec3::new(0.42, 1.0, 0.0), glam::Vec3::Y, min_b, max_b);
    let uv_back = tessel_core::box_atlas_uv(
        glam::Vec3::new(-0.5, 0.92, 0.0),
        -glam::Vec3::X,
        min_b,
        max_b,
    );
    assert_eq!(sample(uv_x), expected_px);
    assert_eq!(sample(uv_y), expected_px);
    assert_ne!(sample(uv_back), expected_px);

    // Undo removes the newly created texture cleanly.
    history.undo(&mut scene).unwrap();
    assert!(scene.lowpoly_meshes[&node_id].texture.is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mesh_fill_smart_material_generates_texture_attaches_ao_and_undoes() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir().join("tessel_v44_smart_mat_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let created = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "name": "WornCrate", "kind": "cube", "size": 1.0 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let node_id = NodeId(created["node_id"].as_u64().unwrap());

    let res = execute_tool_call(
        "mesh_fill_smart_material",
        &json!({
            "node_id": node_id.0,
            "base_palette_index": 3,
            "edge_strength": 0.7,
            "cavity_strength": 0.6,
            "resolution": 32,
            "samples": 8,
            "attach_ao": true
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(res["ok"], true);
    assert_eq!(res["width"], 32);
    assert_eq!(res["ao_attached"], true);
    assert!(res["covered_texels"].as_u64().unwrap() > 0);
    assert!(scene.lowpoly_meshes[&node_id].texture.is_some());
    assert!(scene.lowpoly_meshes[&node_id].ao_map.is_some());

    // Single undo reverts both texture and ao_map atomically.
    history.undo(&mut scene).unwrap();
    assert!(scene.lowpoly_meshes[&node_id].texture.is_none());
    assert!(scene.lowpoly_meshes[&node_id].ao_map.is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mesh_paint_layers_non_destructive_workflow_and_undo() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir().join("tessel_v45_paint_layers_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let created = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "name": "LayeredCube", "kind": "cube", "size": 1.0 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let node_id = NodeId(created["node_id"].as_u64().unwrap());

    // Base solid fill (palette 3), then add transparent Decal layer at 50% opacity.
    execute_tool_call(
        "mesh_fill_texture",
        &json!({ "node_id": node_id.0, "palette_index": 3 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let added = execute_tool_call(
        "mesh_layer_add",
        &json!({ "node_id": node_id.0, "name": "Decal", "opacity": 0.5 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(added["layer_count"], 2);
    let decal_id = added["layer_id"].as_u64().unwrap();

    // Paint palette 5 on the Decal layer at uv [0.5, 0.5].
    execute_tool_call(
        "mesh_paint_spot",
        &json!({
            "node_id": node_id.0,
            "uv": [0.5, 0.5],
            "radius_px": 12.0,
            "palette_index": 5
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();

    let base_c = scene
        .palette
        .get(PaletteIndex::new(3).unwrap())
        .unwrap()
        .color;
    let decal_c = scene
        .palette
        .get(PaletteIndex::new(5).unwrap())
        .unwrap()
        .color;
    let sample_center = |s: &SceneState| -> [u8; 4] {
        let tex = s.lowpoly_meshes[&node_id].texture.as_ref().unwrap();
        let px = (tex.width as usize) / 2;
        let py = (tex.height as usize) / 2;
        let off = (py * tex.width as usize + px) * 4;
        [
            tex.rgba[off],
            tex.rgba[off + 1],
            tex.rgba[off + 2],
            tex.rgba[off + 3],
        ]
    };

    // At 50% opacity, composited pixel is blended between base_c and decal_c.
    let blended = sample_center(&scene);
    assert_ne!(blended, [base_c.r, base_c.g, base_c.b, base_c.a]);
    assert_ne!(blended, [decal_c.r, decal_c.g, decal_c.b, decal_c.a]);

    // Hide Decal layer -> composited pixel reverts cleanly to base_c.
    execute_tool_call(
        "mesh_layer_set",
        &json!({ "node_id": node_id.0, "layer_id": decal_id, "visible": false }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(
        sample_center(&scene),
        [base_c.r, base_c.g, base_c.b, base_c.a]
    );

    // List reports both layers.
    let listed = execute_tool_call(
        "mesh_layer_list",
        &json!({ "node_id": node_id.0 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(listed["layers"].as_array().unwrap().len(), 2);

    // Delete Decal layer, then undo restores it.
    let del = execute_tool_call(
        "mesh_layer_delete",
        &json!({ "node_id": node_id.0, "layer_id": decal_id }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(del["layer_count"], 1);
    history.undo(&mut scene).unwrap();
    assert_eq!(scene.lowpoly_meshes[&node_id].paint_layers.len(), 2);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn v49_grounding_v2_multi_surface_overhang_and_foot_normal() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir().join("tessel_v49_grounding_v2_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Sloped ramp at y ~ 0.0..1.0 + overhead bridge at y = 3.0
    let ramp = execute_tool_call(
        "mesh_create_terrain",
        &json!({
            "name": "Slope",
            "width": 4.0,
            "depth": 4.0,
            "subdiv_x": 2,
            "subdiv_z": 2,
            "heights": [0.0, 0.5, 1.0, 0.0, 0.5, 1.0, 0.0, 0.5, 1.0]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let ramp_id = ramp["node_id"].as_u64().unwrap();

    let bridge = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "name": "Bridge", "kind": "plane", "size": 6.0, "position": [0.0, 3.0, 0.0] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let bridge_id = bridge["node_id"].as_u64().unwrap();

    execute_tool_call(
        "character_apply_biped_rig",
        &json!({ "preset": "humanoid" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();

    let res = execute_tool_call(
        "character_clamp_to_floor",
        &json!({
            "surface_node_ids": [ramp_id, bridge_id],
            "solve_foot_ik": true,
            "align_foot_normal": true,
            "pelvis_tilt": true,
            "step_up_clearance": 1.0
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(res["ok"], true);
    assert_eq!(res["align_foot_normal"], true);
    let floor_y = res["floor_y"].as_f64().unwrap() as f32;
    // Must land on the lower sloped ramp (~0.45), NOT snap onto the y=3.0 bridge!
    assert!(
        floor_y < 1.2,
        "character under y=3.0 bridge must stay on lower ramp, got {floor_y}"
    );
    // LeftFoot rotation should be tilted to align with the sloped normal.
    let rig = scene.character_rig.as_ref().unwrap();
    let lf = rig.bones.iter().find(|b| b.name == "LeftFoot").unwrap();
    assert!(
        lf.pose_local.rotation.angle_between(glam::Quat::IDENTITY) > 0.05,
        "expected LeftFoot tilted to match slope normal"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn v5_1_to_v5_3_mcp_tools_chain_ik_stepped_npr_and_shading_mode() {
    let dir = std::env::temp_dir().join("tessel_v5_1_to_5_3_mcp_test");
    let _ = std::fs::create_dir_all(&dir);
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();

    // v5.2: vertex colors, crease edges, topology validator, alpha_mode + unlit
    let cube = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "name": "InkCube", "kind": "cube", "size": 1.0 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let node_id = cube["node_id"].as_u64().unwrap();

    let vc = execute_tool_call(
        "mesh_set_vertex_colors",
        &json!({ "node_id": node_id, "fill": [0.7, 0.5, 0.9, 1.0] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(vc["ok"], true);
    assert!(vc["updated_vertices"].as_u64().unwrap() >= 8);

    let ce = execute_tool_call(
        "mesh_set_crease_edges",
        &json!({ "node_id": node_id, "edges": [[0, 1], [1, 2]] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(ce["crease_edge_count"], 2);

    let mat = execute_tool_call(
        "mesh_set_material",
        &json!({
            "node_id": node_id,
            "alpha_mode": "mask",
            "alpha_cutoff": 0.4,
            "unlit": true
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(mat["unlit"], true);

    let topo = execute_tool_call(
        "mesh_validate_topology",
        &json!({ "node_id": node_id }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(topo["report"]["is_clean"], true);

    // v5.3: viewport_set_shading_mode
    let mode = execute_tool_call(
        "viewport_set_shading_mode",
        &json!({ "mode": "npr_sumie" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(mode["ok"], true);
    assert_eq!(
        scene.shading_mode,
        tessel_core::ViewportShadingMode::NprSumie
    );

    // v5.1: chain IK + stepped FPS
    execute_tool_call(
        "character_apply_biped_rig",
        &json!({ "preset": "humanoid" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let chain = execute_tool_call(
        "character_solve_chain_ik",
        &json!({
            "bone_ids": [0, 1, 2, 3],
            "target": [0.2, 1.5, 0.3],
            "max_iterations": 16
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(chain["ok"], true);

    let stepped = execute_tool_call(
        "character_set_clip_stepped_fps",
        &json!({ "clip_name": "Idle", "stepped_fps": 12.0, "bake_stepped": true }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(stepped["ok"], true);
    assert_eq!(stepped["baked"], true);

    // Audit regressions:
    // 1. mesh_set_material updates alpha_cutoff without resending alpha_mode
    let mat2 = execute_tool_call(
        "mesh_set_material",
        &json!({ "node_id": node_id, "alpha_cutoff": 0.72 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(mat2["alpha_mode"]["mask"]["cutoff"], 0.72_f32 as f64);

    // 2. SetMeshVertexColors is atomic on error (no partial mutation)
    let before_vc = scene.lowpoly_meshes[&tessel_core::NodeId(node_id)]
        .vertex_colors
        .clone();
    assert!(execute_tool_call(
        "mesh_set_vertex_colors",
        &json!({
            "node_id": node_id,
            "updates": [
                { "vertex_id": 0, "rgba": [0.1, 0.2, 0.3, 1.0] },
                { "vertex_id": 9999, "rgba": [1.0, 1.0, 1.0, 1.0] }
            ]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .is_err());
    assert_eq!(
        scene.lowpoly_meshes[&tessel_core::NodeId(node_id)].vertex_colors,
        before_vc
    );

    // 3. cubic_bezier rejects malformed bezier array
    assert!(execute_tool_call(
        "character_set_keyframe",
        &json!({
            "clip_name": "Idle",
            "bone_id": 1,
            "time_secs": 0.5,
            "rotation_deg": [10.0, 0.0, 0.0],
            "interpolation": "cubic_bezier",
            "bezier": [0.25, 0.1]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .is_err());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pixel_tools_create_layer_draw_animate_and_export() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir().join("tessel_pixel_tools_test");
    let _ = std::fs::create_dir_all(&dir);

    // 1. Create a 32x32 canvas with one transparent layer.
    let created = execute_tool_call(
        "pixel_create_canvas",
        &json!({ "name": "Ronin Fox", "width": 32, "height": 32 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let node_id = created["node_id"].as_u64().unwrap();
    assert_eq!(created["width"].as_u64().unwrap(), 32);
    assert_eq!(created["layers"].as_array().unwrap().len(), 1);

    // 2. Oversized canvas is rejected before allocation.
    assert!(execute_tool_call(
        "pixel_create_canvas",
        &json!({ "width": 513, "height": 32 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .is_err());

    // 3. Add two layers (ground + scarf) on top of the base layer.
    let ground = execute_tool_call(
        "pixel_add_layer",
        &json!({ "node_id": node_id, "name": "Ground & Glow" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let scarf = execute_tool_call(
        "pixel_add_layer",
        &json!({ "node_id": node_id, "name": "Scarf & Katana" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_ne!(
        ground["layer_id"].as_u64().unwrap(),
        scarf["layer_id"].as_u64().unwrap()
    );

    // 4. Set palette + paint one pixel per layer, erase with null.
    execute_tool_call(
        "palette_set_color",
        &json!({ "palette_index": 5, "hex": "#C8102E" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let drawn = execute_tool_call(
        "pixel_draw",
        &json!({
            "node_id": node_id,
            "layer_id": scarf["layer_id"],
            "pixels": [{ "x": 16, "y": 16, "palette_index": 5 }]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(drawn["pixels_painted"].as_u64().unwrap(), 1);

    // 5. Out-of-bounds pixels are rejected, nothing is painted.
    assert!(execute_tool_call(
        "pixel_draw",
        &json!({
            "node_id": node_id,
            "pixels": [{ "x": 32, "y": 0, "palette_index": 5 }]
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .is_err());

    // 6. Append a frame, switch active frame, set fps, inspect.
    let framed = execute_tool_call(
        "pixel_add_frame",
        &json!({ "node_id": node_id }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(framed["frame"].as_u64().unwrap(), 1);
    execute_tool_call(
        "pixel_set_active_frame",
        &json!({ "node_id": node_id, "frame": 0 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "pixel_set_fps",
        &json!({ "node_id": node_id, "fps": 12 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let inspected = execute_tool_call(
        "pixel_inspect",
        &json!({ "node_id": node_id }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(inspected["frame_count"].as_u64().unwrap(), 2);
    assert_eq!(inspected["fps"].as_u64().unwrap(), 12);
    assert_eq!(inspected["active_frame"].as_u64().unwrap(), 0);

    // 7. Undo restores default fps, redo reapplies it (single command bus).
    history.undo(&mut scene).unwrap();
    let after_undo = execute_tool_call(
        "pixel_inspect",
        &json!({ "node_id": node_id }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(after_undo["fps"].as_u64().unwrap(), 8);
    history.redo(&mut scene).unwrap();
    execute_tool_call(
        "pixel_set_fps",
        &json!({ "node_id": node_id, "fps": 8 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();

    // 8. Export animated GIF + sprite sheet through the export dir.
    let gif = execute_tool_call(
        "project_export",
        &json!({ "filename": "ronin_fox.gif", "format": "gif" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert!(gif["file_size_bytes"].as_u64().unwrap() > 0);
    let sheet = execute_tool_call(
        "pixel_export_sprite_sheet",
        &json!({ "node_id": node_id, "filename": "ronin_fox_sheet.png" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(sheet["sheet_width"].as_u64().unwrap(), 64);
    assert_eq!(sheet["sheet_height"].as_u64().unwrap(), 32);
    assert!(dir.join("ronin_fox.gif").exists());
    assert!(dir.join("ronin_fox_sheet.png").exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pixel_and_voxel_mcp_parity_tools_and_non_mutating_inspect() {
    let dir = std::env::temp_dir().join("tessel_pixel_voxel_mcp_parity_test");
    let _ = std::fs::create_dir_all(&dir);
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();

    // Empty scene: read-only inspect & convert MUST error without mutating scene!
    let rev_before = scene.revision;
    assert!(
        execute_tool_call("pixel_inspect", &json!({}), &mut scene, &mut history, &dir).is_err()
    );
    assert!(
        execute_tool_call("voxel_inspect", &json!({}), &mut scene, &mut history, &dir).is_err()
    );
    assert_eq!(scene.revision, rev_before);

    // Pixel full workflow: create, draw_line + mirror_x, draw_rect, flood_fill, set_layer, onion_skin, convert_to_voxel
    let pc = execute_tool_call(
        "pixel_create_canvas",
        &json!({ "name": "HeroSprite", "width": 16, "height": 16 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let pid = pc["node_id"].as_u64().unwrap();
    execute_tool_call(
        "pixel_draw_line",
        &json!({ "node_id": pid, "start": [2, 2], "end": [2, 10], "palette_index": 3, "mirror_x": true }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "pixel_draw_rect",
        &json!({ "node_id": pid, "min": [4, 4], "max": [11, 11], "palette_index": 4, "filled": false }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let ff = execute_tool_call(
        "pixel_flood_fill",
        &json!({ "node_id": pid, "x": 6, "y": 6, "palette_index": 5 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert!(ff["pixels_filled"].as_u64().unwrap() > 0);

    execute_tool_call(
        "pixel_set_layer",
        &json!({ "node_id": pid, "layer_id": 1, "visible": true, "opacity": 0.75 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "pixel_set_onion_skin",
        &json!({ "node_id": pid, "enabled": true }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let pinsp = execute_tool_call(
        "pixel_inspect",
        &json!({ "node_id": pid }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(pinsp["onion_skin"], true);
    assert_eq!(pinsp["frames"][0]["layers"][0]["opacity"], 0.75);

    let pvox = execute_tool_call(
        "pixel_convert_to_voxel",
        &json!({ "node_id": pid, "depth": 3, "name": "SpriteVoxel" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert!(pvox["new_node_id"].as_u64().is_some());

    // Voxel full workflow: create_volume, set_voxels + mirror_x, fill_box carve (null), inspect, export+import .vox
    let vv = execute_tool_call(
        "voxel_create_volume",
        &json!({ "name": "CarvedBlock" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let vid = vv["node_id"].as_u64().unwrap();
    execute_tool_call(
        "voxel_fill_box",
        &json!({ "node_id": vid, "min": [0, 0, 0], "max": [2, 2, 2], "palette_index": 2 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "voxel_fill_box",
        &json!({ "node_id": vid, "min": [1, 1, 1], "max": [1, 1, 1], "palette_index": null }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let sv = execute_tool_call(
        "voxel_set_voxels",
        &json!({ "node_id": vid, "voxels": [{ "x": 4, "y": 0, "z": 0, "palette_index": 6 }], "mirror_x": true }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(sv["voxels_modified"], 2);
    let vinsp = execute_tool_call(
        "voxel_inspect",
        &json!({ "node_id": vid }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(vinsp["voxel_count"], 28); // 27 - 1 carved + 2 mirrored

    // Project/Low-Poly/Animation new MCP tools
    execute_tool_call(
        "scene_select_node",
        &json!({ "node_id": vid }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(scene.selected_node, Some(NodeId(vid)));

    let pal = execute_tool_call(
        "palette_set_color",
        &json!({ "palette_index": 9, "hex": "#FF4422CC", "roughness": 0.4, "metallic": 0.2 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(pal["alpha"], 204);

    let cube = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "name": "ProxyCube", "kind": "cube", "size": 1.0 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let cid = cube["node_id"].as_u64().unwrap();
    let pn = execute_tool_call(
        "mesh_set_proxy_normals",
        &json!({ "node_id": cid, "normals": [{ "vertex_id": 0, "normal": [0.0, 1.0, 0.0] }] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(pn["updated_normals"], 1);

    execute_tool_call(
        "character_apply_biped_rig",
        &json!({ "preset": "starter" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let cc = execute_tool_call(
        "character_create_clip",
        &json!({ "name": "Slash", "duration_secs": 1.2 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let cidx = cc["clip_index"].as_u64().unwrap();
    let uc = execute_tool_call(
        "character_update_clip",
        &json!({ "clip_index": cidx, "name": "HeavySlash", "duration_secs": 0.9 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(uc["name"], "HeavySlash");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn b2_pin_primitive_bottom_anchor_bone_rotation_sign_and_png_export_mode() {
    let dir = std::env::temp_dir().join("tessel_b2_quirk_pin_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();

    // 1. Primitive bottom-anchoring: y in [0, size_y], centered in X/Z (all 7 primitives).
    for kind in [
        "cube", "plane", "cylinder", "cone", "sphere", "torus", "capsule",
    ] {
        let res = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "name": format!("Pin_{kind}"), "kind": kind, "size": [0.4, 1.2, 0.6] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let nid = NodeId(res["node_id"].as_u64().unwrap());
        let (bmin, bmax) = scene.lowpoly_meshes[&nid].bounds();
        assert!(
            bmin.y.abs() < 1e-4,
            "{kind} min.y must be 0.0, got {}",
            bmin.y
        );
        if kind == "plane" {
            assert!(
                bmax.y.abs() < 1e-4,
                "plane max.y must be 0.0, got {}",
                bmax.y
            );
        } else if kind == "torus" {
            assert!(
                bmax.y > 0.0,
                "torus max.y must be > 0.0 when bottom-anchored, got {}",
                bmax.y
            );
        } else {
            assert!(
                (bmax.y - 1.2).abs() < 1e-4,
                "{kind} max.y must be 1.2, got {}",
                bmax.y
            );
        }
        assert!((bmin.x + bmax.x).abs() < 1e-4, "{kind} X must be centered");
        assert!((bmin.z + bmax.z).abs() < 1e-4, "{kind} Z must be centered");
    }

    // 2. Bone rotation sign: Right-Handed +Y Up, +Z Front, +X Left.
    execute_tool_call(
        "character_apply_biped_rig",
        &json!({ "preset": "starter" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "character_set_bone_pose",
        &json!({ "bone_id": 2, "rotation_deg": [45.0, 0.0, 0.0] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let pitch_rot = scene
        .character_rig
        .as_ref()
        .unwrap()
        .bone(BoneId(2))
        .unwrap()
        .pose_local
        .rotation;
    assert!(
        (pitch_rot * glam::Vec3::Y).z > 0.5,
        "+X pitch must tilt +Y forward (+Z)"
    );

    execute_tool_call(
        "character_set_bone_pose",
        &json!({ "bone_id": 2, "rotation_deg": [0.0, 45.0, 0.0] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let yaw_rot = scene
        .character_rig
        .as_ref()
        .unwrap()
        .bone(BoneId(2))
        .unwrap()
        .pose_local
        .rotation;
    assert!(
        (yaw_rot * glam::Vec3::Z).x > 0.5,
        "+Y yaw must turn +Z toward +X (Left)"
    );

    execute_tool_call(
        "character_set_bone_pose",
        &json!({ "bone_id": 2, "rotation_deg": [0.0, 0.0, 45.0] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let roll_rot = scene
        .character_rig
        .as_ref()
        .unwrap()
        .bone(BoneId(2))
        .unwrap()
        .pose_local
        .rotation;
    assert!(
        (roll_rot * glam::Vec3::X).y > 0.5,
        "+Z roll must tilt +X toward +Y"
    );

    // 3. Mode-dependent PNG/GIF export: LowPoly mode rejects PNG & GIF (requires PixelCanvas), allows GLB.
    let png_err = execute_tool_call(
        "project_export",
        &json!({ "filename": "should_fail.png", "format": "png" }),
        &mut scene,
        &mut history,
        &dir,
    );
    assert!(png_err.is_err(), "PNG export in LowPoly mode must error");
    let gif_err = execute_tool_call(
        "project_export",
        &json!({ "filename": "should_fail.gif", "format": "gif" }),
        &mut scene,
        &mut history,
        &dir,
    );
    assert!(gif_err.is_err(), "GIF export in LowPoly mode must error");

    execute_tool_call(
        "pixel_create_canvas",
        &json!({ "name": "PinSprite", "width": 8, "height": 8 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "pixel_draw",
        &json!({ "pixels": [{ "x": 0, "y": 0, "palette_index": 4 }] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let png_ok = execute_tool_call(
        "project_export",
        &json!({ "filename": "pixel_ok.png", "format": "png" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert!(png_ok.is_object());
    let bytes = std::fs::read(dir.join("pixel_ok.png")).unwrap();
    assert_eq!(&bytes[1..4], b"PNG");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn b3_explicit_primitive_anchor_and_dry_run_previews() {
    let dir = std::env::temp_dir().join("tessel_b3_anchor_dry_run_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();

    // 1. Explicit anchor='center' and 'top' on scalar and non-uniform primitives.
    let center = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "name": "CenteredCube", "kind": "cube", "size": 2.0, "anchor": "center" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(center["anchor"], "center");
    let cid = NodeId(center["node_id"].as_u64().unwrap());
    let (clo, chi) = scene.lowpoly_meshes[&cid].bounds();
    assert!((clo.y + 1.0).abs() < 1e-4 && (chi.y - 1.0).abs() < 1e-4);

    let top = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "name": "HangingCyl", "kind": "cylinder", "size": [0.4, 1.5, 0.4], "anchor": "top" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(top["anchor"], "top");
    let tid = NodeId(top["node_id"].as_u64().unwrap());
    let (tlo, thi) = scene.lowpoly_meshes[&tid].bounds();
    assert!((tlo.y + 1.5).abs() < 1e-4 && thi.y.abs() < 1e-4);

    let bad_anchor = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "name": "Bad", "kind": "cube", "anchor": "middle" }),
        &mut scene,
        &mut history,
        &dir,
    );
    assert!(bad_anchor.is_err());
    let non_str_anchor = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "name": "BadType", "kind": "cube", "anchor": 123 }),
        &mut scene,
        &mut history,
        &dir,
    );
    assert!(non_str_anchor.is_err());

    // Non-boolean dry_run must fail closed without mutating scene
    let rev_guard = scene.revision;
    let bad_dry = execute_tool_call(
        "scene_set_node_transform",
        &json!({ "node_id": cid.0, "position": [9.0, 9.0, 9.0], "dry_run": "true" }),
        &mut scene,
        &mut history,
        &dir,
    );
    assert!(bad_dry.is_err());
    assert_eq!(scene.revision, rev_guard);

    // 2. scene_set_node_transform dry_run previews world position/bounds without mutating state.
    let rev_before = scene.revision;
    let transform_before = scene.nodes[&cid].transform;
    let tx_preview = execute_tool_call(
        "scene_set_node_transform",
        &json!({
            "node_id": cid.0,
            "position": [1.0, 2.0, 3.0],
            "rotation_deg": [0.0, 90.0, 0.0],
            "dry_run": true
        }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(tx_preview["dry_run"], true);
    assert_eq!(tx_preview["world_position"], json!([1.0, 2.0, 3.0]));
    assert!(tx_preview["world_forward_dir"][0].as_f64().unwrap() > 0.99);
    assert!(
        (tx_preview["world_bounds"]["min"][1].as_f64().unwrap() - 1.0).abs() < 1e-4
            && (tx_preview["world_bounds"]["max"][1].as_f64().unwrap() - 3.0).abs() < 1e-4
    );
    assert_eq!(scene.revision, rev_before);
    assert_eq!(scene.nodes[&cid].transform, transform_before);

    // 3. character_set_bone_pose dry_run previews bone direction without mutating rig or undo stack.
    execute_tool_call(
        "character_apply_biped_rig",
        &json!({ "preset": "starter" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let rig_rev_before = scene.revision;
    let pose_before = scene
        .character_rig
        .as_ref()
        .unwrap()
        .bone(BoneId(2))
        .unwrap()
        .pose_local;
    let bone_preview = execute_tool_call(
        "character_set_bone_pose",
        &json!({ "bone_id": 2, "rotation_deg": [45.0, 0.0, 0.0], "dry_run": true }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(bone_preview["dry_run"], true);
    assert!(bone_preview["local_up_dir"][2].as_f64().unwrap() > 0.5);
    assert_eq!(scene.revision, rig_rev_before);
    assert_eq!(
        scene
            .character_rig
            .as_ref()
            .unwrap()
            .bone(BoneId(2))
            .unwrap()
            .pose_local,
        pose_before
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn character_apply_biped_rig_respects_include_default_clips_false_without_append() {
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let dir = std::env::temp_dir().join("tessel_test_rig_no_default_clips");
    let _ = std::fs::create_dir_all(&dir);

    // Starter preset with include_default_clips: false -> 7 bones, 0 clips
    let res = execute_tool_call(
        "character_apply_biped_rig",
        &json!({ "preset": "starter", "include_default_clips": false }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(res["ok"], true);
    assert_eq!(res["bones"], 7);
    assert_eq!(res["clips"], 0);
    assert_eq!(scene.character_rig.as_ref().unwrap().animations.len(), 0);

    // Single undo restores no-rig state cleanly
    history.undo(&mut scene).unwrap();
    assert!(scene.character_rig.is_none());

    // Humanoid preset with include_default_clips: false -> 19 bones, 0 clips
    let res2 = execute_tool_call(
        "character_apply_biped_rig",
        &json!({ "preset": "humanoid", "include_default_clips": false }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(res2["bones"], 19);
    assert_eq!(res2["clips"], 0);
    assert_eq!(scene.character_rig.as_ref().unwrap().animations.len(), 0);

    // Default (omitted) still includes Idle & Walk
    let res3 = execute_tool_call(
        "character_apply_biped_rig",
        &json!({ "preset": "starter" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(res3["clips"], 2);
    assert_eq!(scene.character_rig.as_ref().unwrap().animations.len(), 2);

    // Non-boolean include_default_clips is rejected fail-closed
    let bad = execute_tool_call(
        "character_apply_biped_rig",
        &json!({ "preset": "starter", "include_default_clips": "false" }),
        &mut scene,
        &mut history,
        &dir,
    );
    assert!(bad.is_err());

    // append=true + include_default_clips=false on an empty scene is atomic and leaves 0 clips
    let mut empty_scene = SceneState::new();
    let mut empty_history = HistoryStack::new();
    let res4 = execute_tool_call(
        "character_apply_biped_rig",
        &json!({ "preset": "starter", "append": true, "include_default_clips": false }),
        &mut empty_scene,
        &mut empty_history,
        &dir,
    )
    .unwrap();
    assert_eq!(res4["ok"], true);
    assert_eq!(
        empty_scene.character_rig.as_ref().unwrap().animations.len(),
        0
    );
    empty_history.undo(&mut empty_scene).unwrap();
    assert!(empty_scene.character_rig.is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn phase_h_strict_validation_and_animation_ergonomics() {
    let dir = std::env::temp_dir().join("tessel_phase_h_tests");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();

    // H2: i64 -> i32 overflow rejected in voxel_set_voxels
    let err = execute_tool_call(
        "voxel_set_voxels",
        &json!({ "voxels": [{ "x": 4294967297_i64, "y": 0, "z": 0, "palette_index": 1 }] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap_err();
    assert!(matches!(err, AiError::OutOfBounds));

    // H2: invalid project_export format maps to InvalidArguments (not InternalError)
    let err = execute_tool_call(
        "project_export",
        &json!({ "filename": "out.glb", "format": "invalid_fmt" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap_err();
    assert!(matches!(err, AiError::InvalidArguments(_)));

    // H2: wrong argument type in mesh_create_terrain rejected
    let err = execute_tool_call(
        "mesh_create_terrain",
        &json!({ "width": "wide" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap_err();
    assert!(matches!(err, AiError::InvalidArguments(_)));

    // H4: scene_create_clip & character_create_clip work without a prior rig
    let res = execute_tool_call(
        "scene_create_clip",
        &json!({ "name": "WindmillSpin", "duration_secs": 3.0 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(res["ok"], true);
    assert_eq!(res["name"], "WindmillSpin");

    // H4: character_add_bone supports space="world"
    let root = execute_tool_call(
        "character_add_bone",
        &json!({ "name": "Root", "position": [1.0, 0.5, 2.0] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let root_id = root["bone_id"].as_u64().unwrap();
    let child = execute_tool_call(
        "character_add_bone",
        &json!({ "name": "Head", "parent_bone_id": root_id, "position": [1.0, 1.5, 2.0], "space": "world" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(child["local_rest_position"], json!([0.0, 1.0, 0.0]));
    assert_eq!(child["world_rest_position"], json!([1.0, 1.5, 2.0]));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn phase_i3_v55_to_v59_mcp_tools_roundtrip_and_undo() {
    let dir = std::env::temp_dir().join("tessel_phase_i3_tests");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();

    // Create mesh + humanoid rig
    let cube = execute_tool_call(
        "mesh_create_primitive",
        &json!({ "name": "Hero", "kind": "cube", "size": 1.0 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let nid = cube["node_id"].as_u64().unwrap();
    execute_tool_call(
        "character_apply_biped_rig",
        &json!({ "preset": "humanoid" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();

    // v5.5: bone scale + rotation limits + deform flag + spring
    execute_tool_call(
        "character_set_bone_scale",
        &json!({ "bone_id": 0, "scale": [1.0, 1.2, 1.0] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "character_set_bone_properties",
        &json!({ "bone_id": 4, "min_deg": [-30.0, -45.0, -20.0], "max_deg": [30.0, 45.0, 20.0], "inherit_rotation": false, "spring_stiffness": 15.0 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();

    // v5.6: constraint + socket
    execute_tool_call(
        "character_set_constraint",
        &json!({ "name": "HeadLook", "kind": "look_at", "bone_id": 4, "target": [0.0, 1.8, 2.0], "weight": 0.5 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "character_set_socket",
        &json!({ "name": "Hand_R_Socket", "bone_id": 12, "position": [0.0, -0.05, 0.08] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();

    // v5.7: pose preset + spring bake + event + foot roll
    execute_tool_call(
        "character_pose_preset",
        &json!({ "action": "save", "name": "HeroIdle" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "character_bake_spring_bones",
        &json!({ "clip_index": 0, "fps": 12.0 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "character_add_event",
        &json!({ "clip_index": 0, "time_secs": 0.25, "name": "footstep_l", "payload": "wood" }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();

    // v5.8 & v5.9: cleanup, modifier stack, metadata, lookdev, preflight
    execute_tool_call(
        "mesh_cleanup",
        &json!({ "node_id": nid, "weld_epsilon": 0.001 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "mesh_add_modifier",
        &json!({ "node_id": nid, "kind": "solidify", "thickness": 0.04 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "scene_set_node_metadata",
        &json!({ "node_id": nid, "extras": { "surface": "wood" }, "collision_shape": "box", "half_extents": [0.5, 0.5, 0.5] }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    execute_tool_call(
        "scene_set_lookdev",
        &json!({ "preset": "sunset", "bookmark_name": "HeroFront", "bloom_strength": 0.4, "msaa_samples": 4 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    let preflight = execute_tool_call(
        "mesh_preflight",
        &json!({ "max_triangles": 10000 }),
        &mut scene,
        &mut history,
        &dir,
    )
    .unwrap();
    assert_eq!(preflight["status"], "pass");

    let _ = std::fs::remove_dir_all(&dir);
}
