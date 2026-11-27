use std::fs;
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tessel_core::scene::MAX_NODE_NAME_LEN;
use tessel_core::CharacterRig;

use crate::error::IoError;
use crate::mesh_io::import_glb_rig_bytes;

/// Current `.tesselrig.json` standalone rig template schema version.
pub const RIG_TEMPLATE_SCHEMA_VERSION: u32 = 1;

const MAX_RIG_JSON_BYTES: usize = 8 * 1024 * 1024;
const MAX_GLB_RIG_BYTES: u64 = 64 * 1024 * 1024;
const MAX_RIG_BONES: usize = 1024;
const MAX_RIG_CLIPS: usize = 256;
const MAX_KEYFRAMES_PER_CHANNEL: usize = 16_384;

/// Standalone portable character rig template (`.tesselrig.json`, schema v1).
/// Scene-specific `NodeId` bindings, masks, opt-outs, and per-node animation tracks
/// are stripped so rigs transfer cleanly across projects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RigTemplateFile {
    pub schema_version: u32,
    #[serde(default)]
    pub name: String,
    pub rig: CharacterRig,
}

fn strip_scene_specific_data(rig: &CharacterRig) -> CharacterRig {
    let mut cleaned = rig.clone();
    cleaned.bindings.clear();
    cleaned.node_bone_masks.clear();
    cleaned.unrigged_nodes.clear();
    cleaned.node_visibility.clear();
    cleaned.node_parent_overrides.clear();
    cleaned.node_transform_overrides.clear();
    cleaned.morph_weights.clear();
    cleaned.playback_time = 0.0;
    cleaned.playing = false;
    for clip in &mut cleaned.animations {
        clip.morph_tracks.clear();
        clip.visibility_tracks.clear();
        clip.parent_switch_tracks.clear();
        clip.node_transform_tracks.clear();
    }
    cleaned
}

fn validate_and_sanitize_rig(rig: &mut CharacterRig) -> Result<(), IoError> {
    if rig.bones.len() > MAX_RIG_BONES {
        return Err(IoError::PayloadTooLarge(format!(
            "rig bone count {} exceeds limit {MAX_RIG_BONES}",
            rig.bones.len()
        )));
    }
    if rig.animations.len() > MAX_RIG_CLIPS {
        return Err(IoError::PayloadTooLarge(format!(
            "rig animation clip count {} exceeds limit {MAX_RIG_CLIPS}",
            rig.animations.len()
        )));
    }
    for clip in &rig.animations {
        for ch in &clip.channels {
            if ch.keyframes.len() > MAX_KEYFRAMES_PER_CHANNEL {
                return Err(IoError::PayloadTooLarge(format!(
                    "animation channel in '{}' exceeds limit {MAX_KEYFRAMES_PER_CHANNEL}",
                    clip.name
                )));
            }
        }
    }
    rig.validate_and_sanitize().map_err(|_| {
        IoError::InvalidFormat(
            "rig template failed validation (invalid hierarchy, non-finite/non-positive transform, or dangling bone reference)".into(),
        )
    })?;
    rig.playback_time = 0.0;
    rig.playing = false;
    Ok(())
}

/// Serializes `rig` as a deterministic `.tesselrig.json` (schema v1) byte vector,
/// stripping all scene-specific `NodeId` bindings, masks, opt-outs, and node animation tracks.
pub fn export_rig_json_bytes(rig: &CharacterRig, name: &str) -> Result<Vec<u8>, IoError> {
    let mut clean = strip_scene_specific_data(rig);
    validate_and_sanitize_rig(&mut clean)?;
    let template_name: String = name
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NODE_NAME_LEN)
        .collect();
    let final_name = if template_name.is_empty() {
        "TesselRig".to_string()
    } else {
        template_name
    };
    let file = RigTemplateFile {
        schema_version: RIG_TEMPLATE_SCHEMA_VERSION,
        name: final_name,
        rig: clean,
    };
    Ok(serde_json::to_vec_pretty(&file)?)
}

/// Parses and validates a `.tesselrig.json` (schema v1) buffer into a clean `CharacterRig`.
/// Rejects unsupported schema versions, oversized payloads, cyclic hierarchies, non-finite values,
/// negative/zero scales, and dangling bone references.
pub fn import_rig_json_bytes(bytes: &[u8]) -> Result<CharacterRig, IoError> {
    if bytes.len() > MAX_RIG_JSON_BYTES {
        return Err(IoError::PayloadTooLarge(
            "rig JSON payload exceeds 8 MiB safety limit".into(),
        ));
    }
    let template: RigTemplateFile = serde_json::from_slice(bytes)?;
    if template.schema_version != RIG_TEMPLATE_SCHEMA_VERSION {
        return Err(IoError::UnsupportedSchemaVersion(template.schema_version));
    }
    let mut clean = strip_scene_specific_data(&template.rig);
    validate_and_sanitize_rig(&mut clean)?;
    Ok(clean)
}

fn read_regular_file_bounded(path: &Path, max_bytes: u64, label: &str) -> Result<Vec<u8>, IoError> {
    // Check stat() BEFORE File::open() so a Unix FIFO / device cannot block open() indefinitely.
    let pre_meta = fs::metadata(path)?;
    if !pre_meta.is_file() {
        return Err(IoError::InvalidFormat(format!(
            "{label} path is not a regular file"
        )));
    }
    if pre_meta.len() > max_bytes {
        return Err(IoError::PayloadTooLarge(format!(
            "{label} exceeds {} MiB safety limit",
            max_bytes / (1024 * 1024)
        )));
    }
    let file = fs::File::open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(IoError::InvalidFormat(format!(
            "{label} path is not a regular file"
        )));
    }
    let mut bytes = Vec::with_capacity(meta.len().min(max_bytes) as usize);
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(IoError::PayloadTooLarge(format!(
            "{label} exceeds {} MiB safety limit",
            max_bytes / (1024 * 1024)
        )));
    }
    Ok(bytes)
}

/// Loads a standalone rig from disk, supporting `.tesselrig.json` / `.json` and `.glb` skeletons.
pub fn import_rig_from_path(path: &Path) -> Result<CharacterRig, IoError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match ext.as_str() {
        "json" => {
            let bytes =
                read_regular_file_bounded(path, MAX_RIG_JSON_BYTES as u64, "rig JSON file")?;
            import_rig_json_bytes(&bytes)
        }
        "glb" => {
            let bytes = read_regular_file_bounded(path, MAX_GLB_RIG_BYTES, "GLB file")?;
            import_glb_rig_bytes(&bytes)
        }
        other => Err(IoError::InvalidFormat(format!(
            "unsupported rig format '.{other}' (expected .tesselrig.json or .glb)"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use tessel_core::{BoneId, NodeId, SkinBinding};

    #[test]
    fn rig_json_roundtrip_strips_scene_nodes_and_is_deterministic() {
        let mut rig = CharacterRig::humanoid_template();
        rig.bindings.push(SkinBinding {
            node_id: NodeId(42),
            bone_id: BoneId(0),
        });
        rig.node_bone_masks.insert(NodeId(42), vec![BoneId(0)]);
        rig.unrigged_nodes.insert(NodeId(99));
        rig.node_visibility.insert(NodeId(42), false);
        rig.animations[0].upsert_visibility_key(NodeId(42), 0.5, false);

        let bytes_a = export_rig_json_bytes(&rig, "HeroRig").unwrap();
        let bytes_b = export_rig_json_bytes(&rig, "HeroRig").unwrap();
        assert_eq!(
            bytes_a, bytes_b,
            "rig JSON export must be byte-deterministic"
        );

        let imported = import_rig_json_bytes(&bytes_a).unwrap();
        assert_eq!(imported.bones.len(), 19);
        assert_eq!(imported.animations.len(), 2);
        assert!(imported.bindings.is_empty());
        assert!(imported.node_bone_masks.is_empty());
        assert!(imported.unrigged_nodes.is_empty());
        assert!(imported.node_visibility.is_empty());
        assert!(imported.animations[0].visibility_tracks.is_empty());
    }

    #[test]
    fn rig_json_import_rejects_bad_schema_cycles_and_non_finite() {
        let rig = CharacterRig::biped_template();
        let bytes = export_rig_json_bytes(&rig, "Biped").unwrap();

        // Unsupported schema version
        let mut val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        val["schema_version"] = serde_json::json!(99);
        let bad_ver = serde_json::to_vec(&val).unwrap();
        assert!(matches!(
            import_rig_json_bytes(&bad_ver),
            Err(IoError::UnsupportedSchemaVersion(99))
        ));

        // Cyclic hierarchy (Root -> Head -> Torso -> Root)
        let mut cyclic_val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        cyclic_val["rig"]["bones"][0]["parent"] = serde_json::json!(2);
        let bad_cycle = serde_json::to_vec(&cyclic_val).unwrap();
        assert!(matches!(
            import_rig_json_bytes(&bad_cycle),
            Err(IoError::InvalidFormat(_))
        ));

        // Zero-length quaternion
        let mut bad_quat_val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        bad_quat_val["rig"]["bones"][0]["rest_local"]["rotation"] =
            serde_json::json!([0.0, 0.0, 0.0, 0.0]);
        let bad_quat = serde_json::to_vec(&bad_quat_val).unwrap();
        assert!(matches!(
            import_rig_json_bytes(&bad_quat),
            Err(IoError::InvalidFormat(_))
        ));

        // Direct export rejects NaN translation, zero scale, and negative scale
        let mut nan_rig = CharacterRig::biped_template();
        nan_rig.bones[0].rest_local.translation = Vec3::new(f32::NAN, 0.0, 0.0);
        assert!(matches!(
            export_rig_json_bytes(&nan_rig, "Broken"),
            Err(IoError::InvalidFormat(_))
        ));
        for bad_scale_arr in [[0.0, 1.0, 1.0], [-1.0, 1.0, 1.0]] {
            let mut scale_val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            scale_val["rig"]["bones"][0]["rest_local"]["scale"] = serde_json::json!(bad_scale_arr);
            let bad_scale = serde_json::to_vec(&scale_val).unwrap();
            assert!(matches!(
                import_rig_json_bytes(&bad_scale),
                Err(IoError::InvalidFormat(_))
            ));
        }

        // Empty clip name must be rejected
        let mut empty_clip_val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        empty_clip_val["rig"]["animations"][0]["name"] = serde_json::json!("   ");
        let bad_clip = serde_json::to_vec(&empty_clip_val).unwrap();
        assert!(matches!(
            import_rig_json_bytes(&bad_clip),
            Err(IoError::InvalidFormat(_))
        ));
    }

    #[test]
    fn rig_json_import_topologically_sorts_out_of_order_bones() {
        let mut rig = CharacterRig::biped_template();
        let bytes = export_rig_json_bytes(&rig, "Biped").unwrap();
        let mut val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        val["rig"]["bones"].as_array_mut().unwrap().reverse();
        let reversed = serde_json::to_vec(&val).unwrap();

        let imported = import_rig_json_bytes(&reversed).unwrap();
        let root_idx = imported
            .bones
            .iter()
            .position(|b| b.id == BoneId(0))
            .unwrap();
        let head_idx = imported
            .bones
            .iter()
            .position(|b| b.id == BoneId(2))
            .unwrap();
        assert!(root_idx < head_idx);
        rig.validate_and_sort_bones().unwrap();
        assert_eq!(imported.world_rest_matrices(), rig.world_rest_matrices());
    }
}
