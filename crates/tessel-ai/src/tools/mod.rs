mod character;
mod mesh;
mod pixel;
mod project;
mod scene;
mod voxel;

use serde_json::Value;

/// Single source of truth for tool schemas shared by both Direct AI and MCP `tools/list`.
pub fn tool_definitions() -> Vec<Value> {
    let mut tools = Vec::with_capacity(160);
    tools.extend(scene::scene_tools());
    tools.extend(voxel::voxel_tools());
    tools.extend(pixel::pixel_tools());
    tools.extend(mesh::mesh_tools());
    tools.extend(character::character_tools());
    tools.extend(project::project_tools());
    tools
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use serde_json::json;
    use tessel_core::{HistoryStack, SceneState};

    use super::tool_definitions;
    use crate::error::AiError;
    use crate::mcp::dispatch_mcp_message;
    use crate::translator::execute_tool_call;

    #[test]
    fn tool_definitions_has_160_unique_tools() {
        let defs = tool_definitions();
        assert_eq!(defs.len(), 167);
        let names: HashSet<&str> = defs
            .iter()
            .filter_map(|d| d.get("name").and_then(|v| v.as_str()))
            .collect();
        assert_eq!(names.len(), defs.len());
    }

    #[test]
    fn mcp_schema_to_handler_parity_for_all_tools() {
        let defs = tool_definitions();
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let export_dir = std::env::temp_dir().join("tessel_j3_parity_test");

        // Verify tools/list returns exact parity with tool_definitions()
        let list_resp = dispatch_mcp_message(
            &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}}),
            &mut scene,
            &mut history,
            &export_dir,
        )
        .expect("tools/list must reply");
        let listed = list_resp["result"]["tools"]
            .as_array()
            .expect("tools array");
        assert_eq!(listed.len(), defs.len());

        for def in &defs {
            let name = def["name"]
                .as_str()
                .expect("every tool definition must have a string name");
            let desc = def["description"]
                .as_str()
                .expect("every tool definition must have a description");
            assert!(
                !desc.trim().is_empty(),
                "tool '{name}' has empty description"
            );
            let schema = &def["inputSchema"];
            assert_eq!(
                schema["type"].as_str(),
                Some("object"),
                "tool '{name}' inputSchema.type must be 'object'"
            );
            let props = schema["properties"]
                .as_object()
                .unwrap_or_else(|| panic!("tool '{name}' missing inputSchema.properties object"));
            if let Some(req) = schema.get("required").and_then(|v| v.as_array()) {
                for r in req {
                    let key = r
                        .as_str()
                        .unwrap_or_else(|| panic!("tool '{name}' non-string required entry"));
                    assert!(
                        props.contains_key(key),
                        "tool '{name}' required key '{key}' is missing from inputSchema.properties"
                    );
                }
            }

            // Every defined tool must be recognized by execute_tool_call (never UnknownTool).
            let mut local_scene = SceneState::new();
            let mut local_history = HistoryStack::new();
            let res = execute_tool_call(
                name,
                &json!({}),
                &mut local_scene,
                &mut local_history,
                &export_dir,
            );
            assert!(
                !matches!(res, Err(AiError::UnknownTool(_))),
                "tool '{name}' is listed in tool_definitions() but returned UnknownTool in translator"
            );
        }
    }

    #[test]
    fn phase_l_and_m_mcp_tools_roundtrip_and_undo() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let export_dir = std::env::temp_dir().join("tessel_lm_roundtrip_test");

        // Create cube mesh
        let created = execute_tool_call(
            "mesh_create_primitive",
            &json!({"name": "SculptHead", "kind": "cube", "size": 1.0}),
            &mut scene,
            &mut history,
            &export_dir,
        )
        .expect("create cube");
        let node_id = created["node_id"].as_u64().expect("node_id");

        // L2: mesh_set_npr_slot (tune params + reject bad type)
        let bad_npr = execute_tool_call(
            "mesh_set_npr_slot",
            &json!({"node_id": node_id, "slot": "matcap", "matcap_strength": "bad"}),
            &mut scene,
            &mut history,
            &export_dir,
        );
        assert!(bad_npr.is_err());

        let npr_ok = execute_tool_call(
            "mesh_set_npr_slot",
            &json!({
                "node_id": node_id,
                "slot": "face_sdf",
                "matcap_strength": 0.4,
                "hatching_scale": 3.0,
                "face_sdf_light_override": [0.5, 0.2, 0.8]
            }),
            &mut scene,
            &mut history,
            &export_dir,
        )
        .expect("mesh_set_npr_slot");
        assert_eq!(npr_ok["ok"], json!(true));
        let mat = scene.lowpoly_meshes[&tessel_core::NodeId(node_id)].material;
        assert!((mat.matcap_strength - 0.4).abs() < 1e-4);
        assert!((mat.hatching_scale - 3.0).abs() < 1e-4);
        assert!(mat.face_sdf_light_override.is_some());

        // M1: mesh_set_uv_seams + unwrap + mesh_normalize_texel_density + undo
        let seam_res = execute_tool_call(
            "mesh_set_uv_seams",
            &json!({"node_id": node_id, "edges": [[0, 1], [1, 2]], "replace": true, "unwrap": true}),
            &mut scene,
            &mut history,
            &export_dir,
        )
        .expect("mesh_set_uv_seams");
        assert_eq!(seam_res["seam_count"], json!(2));

        let td_res = execute_tool_call(
            "mesh_normalize_texel_density",
            &json!({"node_id": node_id, "texture_size": 256, "padding_uv": 0.01}),
            &mut scene,
            &mut history,
            &export_dir,
        )
        .expect("mesh_normalize_texel_density");
        assert_eq!(td_res["ok"], json!(true));

        // M2: mesh_sculpt_stroke + undo restores exact vertices
        let verts_before = scene.lowpoly_meshes[&tessel_core::NodeId(node_id)]
            .vertices
            .clone();
        let sculpt_res = execute_tool_call(
            "mesh_sculpt_stroke",
            &json!({
                "node_id": node_id,
                "brush": "inflate",
                "center": [0.5, 0.5, 0.5],
                "radius": 0.8,
                "strength": 0.5,
                "mirror_x": true
            }),
            &mut scene,
            &mut history,
            &export_dir,
        )
        .expect("mesh_sculpt_stroke");
        assert_eq!(sculpt_res["ok"], json!(true));
        assert_ne!(
            scene.lowpoly_meshes[&tessel_core::NodeId(node_id)].vertices,
            verts_before
        );

        history.undo(&mut scene).expect("undo sculpt stroke");
        assert_eq!(
            scene.lowpoly_meshes[&tessel_core::NodeId(node_id)].vertices,
            verts_before
        );
    }

    #[test]
    fn g1_character_set_node_skinned_roundtrip_and_inspect_weights() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let export_dir = std::env::temp_dir().join("tessel_g1_skinned_test");

        let created = execute_tool_call(
            "mesh_create_primitive",
            &json!({"name": "Ground", "kind": "cube", "size": 2.0}),
            &mut scene,
            &mut history,
            &export_dir,
        )
        .expect("create ground mesh");
        let node_id = created["node_id"].as_u64().expect("node_id");
        execute_tool_call(
            "character_apply_biped_rig",
            &json!({"preset": "starter"}),
            &mut scene,
            &mut history,
            &export_dir,
        )
        .expect("apply biped rig");

        let set_false = execute_tool_call(
            "character_set_node_skinned",
            &json!({"node_id": node_id, "skinned": false}),
            &mut scene,
            &mut history,
            &export_dir,
        )
        .expect("set skinned=false");
        assert_eq!(set_false["ok"], json!(true));
        assert_eq!(set_false["skinned"], json!(false));

        let inspected = execute_tool_call(
            "character_inspect_weights",
            &json!({"node_id": node_id}),
            &mut scene,
            &mut history,
            &export_dir,
        )
        .expect("inspect weights");
        assert_eq!(inspected["skinned"], json!(false));
        assert_eq!(inspected["influences"], json!([]));

        history.undo(&mut scene).expect("undo set_node_skinned");
        assert!(scene
            .character_rig
            .as_ref()
            .unwrap()
            .should_skin_node(tessel_core::NodeId(node_id)));

        // Validation guards: node_id=0 and nonexistent/invalid node_id must return InvalidArguments.
        assert!(matches!(
            execute_tool_call(
                "character_set_node_skinned",
                &json!({"node_id": 0, "skinned": false}),
                &mut scene,
                &mut history,
                &export_dir,
            ),
            Err(AiError::InvalidArguments(_))
        ));
        assert!(matches!(
            execute_tool_call(
                "character_inspect_weights",
                &json!({"node_id": 99999}),
                &mut scene,
                &mut history,
                &export_dir,
            ),
            Err(AiError::InvalidArguments(_))
        ));
        assert!(matches!(
            execute_tool_call(
                "character_inspect_weights",
                &json!({"node_id": "bad", "point": [0.0, 1.0, 0.0]}),
                &mut scene,
                &mut history,
                &export_dir,
            ),
            Err(AiError::InvalidArguments(_))
        ));

        // All bones deform=false should report skinned=false & empty influences instead of "has no bones".
        if let Some(rig) = scene.character_rig.as_mut() {
            for b in &mut rig.bones {
                b.deform = false;
            }
        }
        let non_deform_res = execute_tool_call(
            "character_inspect_weights",
            &json!({"node_id": node_id}),
            &mut scene,
            &mut history,
            &export_dir,
        )
        .expect("inspect weights on all-non-deform rig");
        assert_eq!(non_deform_res["skinned"], json!(false));
        assert_eq!(non_deform_res["influences"], json!([]));
    }

    #[test]
    fn g3_character_export_and_import_rig_mcp_roundtrip_undo_and_path_guard() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let export_dir = std::env::temp_dir().join("tessel_g3_rig_io_mcp_test");
        let _ = std::fs::remove_dir_all(&export_dir);
        std::fs::create_dir_all(&export_dir).unwrap();

        // Create static terrain mesh (unrigged) + humanoid rig
        let ground = execute_tool_call(
            "mesh_create_primitive",
            &json!({"name": "Ground", "kind": "cube", "size": 2.0}),
            &mut scene,
            &mut history,
            &export_dir,
        )
        .expect("create ground");
        let ground_id = ground["node_id"].as_u64().unwrap();
        execute_tool_call(
            "character_apply_biped_rig",
            &json!({"preset": "humanoid"}),
            &mut scene,
            &mut history,
            &export_dir,
        )
        .expect("apply humanoid rig");
        execute_tool_call(
            "character_set_node_skinned",
            &json!({"node_id": ground_id, "skinned": false}),
            &mut scene,
            &mut history,
            &export_dir,
        )
        .expect("opt ground out");

        // Export standalone .tesselrig.json
        let exp = execute_tool_call(
            "character_export_rig",
            &json!({"filename": "hero.tesselrig.json", "name": "HeroHumanoid"}),
            &mut scene,
            &mut history,
            &export_dir,
        )
        .expect("export rig json");
        assert_eq!(exp["ok"], json!(true));
        assert_eq!(exp["bone_count"], json!(19));

        // Path traversal, non-json extension, and directory-root targets (even on fresh dir) must be blocked
        assert!(execute_tool_call(
            "character_export_rig",
            &json!({"filename": "../evil.tesselrig.json"}),
            &mut scene,
            &mut history,
            &export_dir,
        )
        .is_err());
        assert!(execute_tool_call(
            "character_export_rig",
            &json!({"filename": "."}),
            &mut scene,
            &mut history,
            &export_dir,
        )
        .is_err());
        assert!(execute_tool_call(
            "character_export_rig",
            &json!({"filename": "hero.glb"}),
            &mut scene,
            &mut history,
            &export_dir,
        )
        .is_err());
        let fresh_export_dir = export_dir.join("nonexistent_subdir");
        assert!(execute_tool_call(
            "character_export_rig",
            &json!({"filename": "."}),
            &mut scene,
            &mut history,
            &fresh_export_dir,
        )
        .is_err());
        assert!(execute_tool_call(
            "character_import_rig",
            &json!({"filename": "../evil.tesselrig.json"}),
            &mut scene,
            &mut history,
            &export_dir,
        )
        .is_err());

        // Switch to 7-bone starter rig, then import the 19-bone template back via MCP
        execute_tool_call(
            "character_apply_biped_rig",
            &json!({"preset": "starter"}),
            &mut scene,
            &mut history,
            &export_dir,
        )
        .expect("apply starter rig");
        assert_eq!(scene.character_rig.as_ref().unwrap().bones.len(), 7);

        let imp = execute_tool_call(
            "character_import_rig",
            &json!({"filename": "hero.tesselrig.json"}),
            &mut scene,
            &mut history,
            &export_dir,
        )
        .expect("import rig json");
        assert_eq!(imp["ok"], json!(true));
        assert_eq!(imp["bone_count"], json!(19));
        assert!(!scene
            .character_rig
            .as_ref()
            .unwrap()
            .should_skin_node(tessel_core::NodeId(ground_id)));

        // Undo reverts imported 19-bone rig back to 7-bone starter rig
        history.undo(&mut scene).expect("undo import rig");
        assert_eq!(scene.character_rig.as_ref().unwrap().bones.len(), 7);

        // Also test importing skeleton from exported .glb
        execute_tool_call(
            "character_set_node_skinned",
            &json!({"node_id": ground_id, "skinned": true}),
            &mut scene,
            &mut history,
            &export_dir,
        )
        .expect("re-enable ground skinning for glb");
        execute_tool_call(
            "project_export",
            &json!({"filename": "hero.glb", "format": "glb"}),
            &mut scene,
            &mut history,
            &export_dir,
        )
        .expect("export glb");
        let imp_glb = execute_tool_call(
            "character_import_rig",
            &json!({"filename": "hero.glb"}),
            &mut scene,
            &mut history,
            &export_dir,
        )
        .expect("import glb rig");
        assert_eq!(imp_glb["ok"], json!(true));
        assert_eq!(imp_glb["bone_count"], json!(7));

        let _ = std::fs::remove_dir_all(&export_dir);
    }
}
