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

    use super::tool_definitions;

    #[test]
    fn tool_definitions_has_160_unique_tools() {
        let defs = tool_definitions();
        assert_eq!(defs.len(), 160);
        let names: HashSet<&str> = defs
            .iter()
            .filter_map(|d| d.get("name").and_then(|v| v.as_str()))
            .collect();
        assert_eq!(names.len(), 160);
    }
}
