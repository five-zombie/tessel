use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tessel_core::{HistoryStack, SceneState};
use tessel_io::{
    export_scene_to_dir, load_tessel_project, save_tessel_project_atomic, ExportFormat,
    ExportPreset, ExportReport,
};

use crate::error::AiError;
use crate::translator::execute_tool_call;

/// Single MCP tool operation inside a headless batch script.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchOperation {
    pub tool: String,
    #[serde(default = "default_empty_object")]
    pub args: Value,
}

fn default_empty_object() -> Value {
    json!({})
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum BatchScriptEnvelope {
    List(Vec<BatchOperation>),
    Object { operations: Vec<BatchOperation> },
}

/// Configuration for a headless `open -> batch -> save/export` pipeline run.
#[derive(Debug, Clone)]
pub struct BatchConfig {
    pub open_path: Option<PathBuf>,
    pub export_dir: PathBuf,
    pub save_path: Option<PathBuf>,
    pub export_filename: Option<String>,
    pub export_preset: ExportPreset,
    pub operations: Vec<BatchOperation>,
}

/// Deterministic JSON report returned by `run_batch`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchReport {
    pub ok: bool,
    pub opened: Option<String>,
    pub operations_executed: usize,
    pub results: Vec<Value>,
    pub saved: Option<String>,
    pub exported: Option<ExportReport>,
}

/// Hard cap on operations per headless batch script.
pub const MAX_BATCH_OPERATIONS: usize = 4_096;

/// Parses a JSON batch script formatted either as a top-level array of `{ tool, args }`
/// or an object `{ "operations": [ { "tool", "args" }, ... ] }`.
pub fn parse_batch_script(json_str: &str) -> Result<Vec<BatchOperation>, AiError> {
    let trimmed = json_str.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let env: BatchScriptEnvelope = serde_json::from_str(trimmed)
        .map_err(|e| AiError::InvalidArguments(format!("invalid batch JSON: {e}")))?;
    let ops = match env {
        BatchScriptEnvelope::List(ops) => ops,
        BatchScriptEnvelope::Object { operations } => operations,
    };
    if ops.len() > MAX_BATCH_OPERATIONS {
        return Err(AiError::InvalidArguments(format!(
            "batch script exceeds maximum of {MAX_BATCH_OPERATIONS} operations"
        )));
    }
    Ok(ops)
}

/// Infers `ExportFormat` from a filename's extension.
pub fn infer_export_format(filename: &str) -> Result<ExportFormat, AiError> {
    let ext = Path::new(filename)
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| {
            AiError::InvalidArguments(format!(
                "missing file extension on export target '{filename}'"
            ))
        })?;
    match ext.as_str() {
        "glb" => Ok(ExportFormat::Glb),
        "obj" => Ok(ExportFormat::Obj),
        "stl" => Ok(ExportFormat::Stl),
        "vox" => Ok(ExportFormat::Vox),
        "png" => Ok(ExportFormat::Png),
        "gif" => Ok(ExportFormat::Gif),
        other => Err(AiError::InvalidArguments(format!(
            "unsupported export extension '.{other}', expected glb|obj|stl|vox|png|gif"
        ))),
    }
}

/// Parses an engine export preset name (`godot4|unity|unreal|threejs|print3d`).
pub fn parse_export_preset(name: &str) -> Result<ExportPreset, AiError> {
    match name.trim().to_ascii_lowercase().as_str() {
        "godot" | "godot4" => Ok(ExportPreset::Godot4),
        "unity" => Ok(ExportPreset::Unity),
        "unreal" => Ok(ExportPreset::Unreal),
        "three" | "threejs" => Ok(ExportPreset::ThreeJs),
        "print" | "print3d" => Ok(ExportPreset::Print3D),
        other => Err(AiError::InvalidArguments(format!(
            "unknown export preset '{other}', expected godot4|unity|unreal|threejs|print3d"
        ))),
    }
}

/// Executes a headless `open -> operations -> save -> export` pipeline deterministically.
pub fn run_batch(config: &BatchConfig) -> Result<BatchReport, AiError> {
    if config.operations.len() > MAX_BATCH_OPERATIONS {
        return Err(AiError::InvalidArguments(format!(
            "batch exceeds maximum of {MAX_BATCH_OPERATIONS} operations"
        )));
    }
    let export_spec = match &config.export_filename {
        Some(filename) => Some((filename.as_str(), infer_export_format(filename)?)),
        None => None,
    };

    fs::create_dir_all(&config.export_dir).map_err(|e| {
        AiError::InvalidArguments(format!(
            "failed to create export dir '{}': {e}",
            config.export_dir.display()
        ))
    })?;

    let mut opened = None;
    let mut scene = if let Some(open_path) = &config.open_path {
        opened = Some(open_path.display().to_string());
        load_tessel_project(open_path)?
    } else {
        SceneState::new()
    };

    let mut results = Vec::with_capacity(config.operations.len());
    for (idx, op) in config.operations.iter().enumerate() {
        if op.tool == "scene_batch" {
            return Err(AiError::InvalidArguments(format!(
                "batch operation #{idx}: nested 'scene_batch' is not allowed inside tessel batch"
            )));
        }
        let mut history = HistoryStack::new();
        let res = execute_tool_call(
            &op.tool,
            &op.args,
            &mut scene,
            &mut history,
            &config.export_dir,
        )
        .map_err(|e| {
            AiError::InvalidArguments(format!(
                "batch operation #{idx} ('{}') failed: {e}",
                op.tool
            ))
        })?;
        results.push(res);
    }

    let saved = if let Some(save_path) = &config.save_path {
        save_tessel_project_atomic(save_path, &scene)?;
        Some(save_path.display().to_string())
    } else {
        None
    };

    let exported = if let Some((filename, format)) = export_spec {
        let report = export_scene_to_dir(
            &scene,
            &config.export_dir,
            filename,
            format,
            config.export_preset,
        )?;
        Some(report)
    } else {
        None
    };

    Ok(BatchReport {
        ok: true,
        opened,
        operations_executed: results.len(),
        results,
        saved,
        exported,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_batch_script_accepts_array_and_object_wrapper() {
        let arr = parse_batch_script(
            r#"[{"tool":"mesh_create_primitive","args":{"name":"Box","kind":"cube"}}]"#,
        )
        .unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0].tool, "mesh_create_primitive");

        let obj = parse_batch_script(
            r#"{"operations":[{"tool":"scene_info"},{"tool":"mesh_create_primitive","args":{"kind":"sphere"}}]}"#,
        )
        .unwrap();
        assert_eq!(obj.len(), 2);
        assert_eq!(obj[0].tool, "scene_info");
        assert_eq!(obj[0].args, json!({}));
    }

    #[test]
    fn run_batch_full_open_mutate_save_export_pipeline() {
        let dir = std::env::temp_dir().join("tessel_v46_batch_pipeline_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let proj_path = dir.join("crate.tessel");

        // Stage 1: build scene and save .tessel
        let stage1 = BatchConfig {
            open_path: None,
            export_dir: dir.clone(),
            save_path: Some(proj_path.clone()),
            export_filename: None,
            export_preset: ExportPreset::Godot4,
            operations: parse_batch_script(
                r#"[
                    {"tool":"mesh_create_primitive","args":{"name":"Crate","kind":"cube","size":1.0}}
                ]"#,
            )
            .unwrap(),
        };
        let rep1 = run_batch(&stage1).unwrap();
        assert!(rep1.ok);
        assert_eq!(rep1.operations_executed, 1);
        assert!(proj_path.exists());

        // Stage 2: open .tessel, apply smart material, save, and export .glb
        let stage2 = BatchConfig {
            open_path: Some(proj_path.clone()),
            export_dir: dir.clone(),
            save_path: Some(proj_path.clone()),
            export_filename: Some("crate_final.glb".into()),
            export_preset: ExportPreset::Godot4,
            operations: parse_batch_script(
                r#"{"operations":[
                    {"tool":"mesh_fill_smart_material","args":{"resolution":32,"samples":8}}
                ]}"#,
            )
            .unwrap(),
        };
        let rep2 = run_batch(&stage2).unwrap();
        assert!(rep2.ok);
        assert!(rep2.opened.is_some());
        let exp = rep2.exported.expect("expected glb export report");
        assert!(exp.file_size_bytes > 0);
        assert!(dir.join("crate_final.glb").exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn infer_format_and_preset_reject_invalid_values() {
        assert_eq!(infer_export_format("hero.GLB").unwrap(), ExportFormat::Glb);
        assert!(infer_export_format("hero.fbx").is_err());
        assert!(infer_export_format("no_ext").is_err());
        assert_eq!(parse_export_preset("unreal").unwrap(), ExportPreset::Unreal);
        assert!(parse_export_preset("blender").is_err());
    }
}
