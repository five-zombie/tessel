mod app;
mod icon;
mod input;
mod project_files;
mod settings;
mod ui;

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Mutex;

use settings::AppPaths;
use tracing_subscriber::EnvFilter;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("mcp") {
        let export_dir = parse_export_dir(&args[2..]);
        let mut server = tessel_ai::McpServer::new(export_dir);
        if let Err(err) = server.run_stdio() {
            eprintln!("Tessel MCP server error: {err}");
            std::process::exit(1);
        }
        return;
    }
    if args.get(1).map(String::as_str) == Some("batch") {
        if let Err(err) = run_batch_cli(&args[2..]) {
            eprintln!("Tessel batch error: {err}");
            std::process::exit(1);
        }
        return;
    }

    let paths = AppPaths::resolve();
    init_logging(&paths);
    install_panic_hook(&paths);

    if let Err(err) = app::run() {
        tracing::error!("Tessel exited with error: {err}");
        eprintln!("Tessel failed to start: {err}");
        std::process::exit(1);
    }
}

fn parse_export_dir(args: &[String]) -> PathBuf {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--export-dir" {
            if let Some(dir) = iter.next() {
                return PathBuf::from(dir);
            }
        }
    }
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("exports")
}

#[derive(Debug, PartialEq)]
struct BatchCliArgs {
    open_path: Option<PathBuf>,
    script_path: Option<String>,
    export_dir: PathBuf,
    save_path: Option<PathBuf>,
    export_filename: Option<String>,
    preset: tessel_io::ExportPreset,
    show_help: bool,
}

fn parse_batch_args(args: &[String]) -> Result<BatchCliArgs, String> {
    let mut out = BatchCliArgs {
        open_path: None,
        script_path: None,
        export_dir: std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("exports"),
        save_path: None,
        export_filename: None,
        preset: tessel_io::ExportPreset::Godot4,
        show_help: false,
    };

    let mut i = 0;
    while i < args.len() {
        let flag = args[i].as_str();
        let next_val = |idx: &mut usize, name: &str| -> Result<String, String> {
            *idx += 1;
            args.get(*idx)
                .cloned()
                .ok_or_else(|| format!("missing value for {name}"))
        };
        match flag {
            "-h" | "--help" => {
                out.show_help = true;
                return Ok(out);
            }
            "--open" => out.open_path = Some(PathBuf::from(next_val(&mut i, "--open")?)),
            "--script" => out.script_path = Some(next_val(&mut i, "--script")?),
            "--export-dir" => out.export_dir = PathBuf::from(next_val(&mut i, "--export-dir")?),
            "--save" => out.save_path = Some(PathBuf::from(next_val(&mut i, "--save")?)),
            "--export" => out.export_filename = Some(next_val(&mut i, "--export")?),
            "--preset" => {
                let raw = next_val(&mut i, "--preset")?;
                out.preset = tessel_ai::parse_export_preset(&raw).map_err(|e| format!("{e}"))?;
            }
            other => {
                return Err(format!(
                    "unknown flag '{other}' (run `tessel batch --help`)"
                ))
            }
        }
        i += 1;
    }

    Ok(out)
}

fn run_batch_cli(args: &[String]) -> Result<(), String> {
    let cli = parse_batch_args(args)?;
    if cli.show_help {
        println!(
            "Usage: tessel batch [OPTIONS]\n\
             \n\
             Options:\n\
               --open <file.tessel>      Load existing .tessel project before running script\n\
               --script <ops.json|->     JSON batch operations file (or '-' for stdin)\n\
               --export-dir <dir>        Sandbox directory for exports (default: ./exports)\n\
               --save <out.tessel>       Save resulting .tessel project atomically\n\
               --export <filename>       Export asset into --export-dir (.glb|.obj|.stl|.vox|.png|.gif)\n\
               --preset <preset>         Export preset: godot4|unity|unreal|threejs|print3d\n\
               -h, --help                Show this help"
        );
        return Ok(());
    }

    const MAX_BATCH_SCRIPT_BYTES: u64 = 16 * 1024 * 1024;
    let operations = match cli.script_path.as_deref() {
        Some("-") => {
            let mut buf = String::new();
            let n = std::io::stdin()
                .take(MAX_BATCH_SCRIPT_BYTES + 1)
                .read_to_string(&mut buf)
                .map_err(|e| format!("failed to read batch script from stdin: {e}"))?;
            if n as u64 > MAX_BATCH_SCRIPT_BYTES {
                return Err("batch script exceeds 16 MiB limit".into());
            }
            tessel_ai::parse_batch_script(&buf).map_err(|e| format!("{e}"))?
        }
        Some(path) => {
            let meta = fs::metadata(path)
                .map_err(|e| format!("failed to inspect batch script '{path}': {e}"))?;
            if meta.len() > MAX_BATCH_SCRIPT_BYTES {
                return Err(format!("batch script '{path}' exceeds 16 MiB limit"));
            }
            let raw = fs::read_to_string(path)
                .map_err(|e| format!("failed to read batch script '{path}': {e}"))?;
            tessel_ai::parse_batch_script(&raw).map_err(|e| format!("{e}"))?
        }
        None => Vec::new(),
    };

    let report = tessel_ai::run_batch(&tessel_ai::BatchConfig {
        open_path: cli.open_path,
        export_dir: cli.export_dir,
        save_path: cli.save_path,
        export_filename: cli.export_filename,
        export_preset: cli.preset,
        operations,
    })
    .map_err(|e| format!("{e}"))?;

    let json = serde_json::to_string_pretty(&report)
        .map_err(|e| format!("failed to serialize batch report: {e}"))?;
    println!("{json}");
    Ok(())
}

fn init_logging(paths: &AppPaths) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    if let Some(parent) = paths.log_file.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&paths.log_file)
    {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(false)
            .with_ansi(false)
            .with_writer(Mutex::new(file))
            .try_init();
    } else {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(false)
            .try_init();
    }
}

fn install_panic_hook(paths: &AppPaths) {
    let default_hook = std::panic::take_hook();
    let log_dir = paths
        .log_file
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("fatal panic: {info}");
        let _ = fs::create_dir_all(&log_dir);
        if let Ok(mut f) = fs::File::create(log_dir.join("crash-latest.log")) {
            let _ = writeln!(f, "Tessel crash report: {info}");
        }
        default_hook(info);
    }));
}

#[cfg(test)]
mod batch_cli_tests {
    use super::*;

    #[test]
    fn parse_batch_args_parses_all_flags_and_rejects_unknown() {
        let raw: Vec<String> = vec![
            "--open",
            "in.tessel",
            "--script",
            "ops.json",
            "--export-dir",
            "/tmp/out",
            "--save",
            "out.tessel",
            "--export",
            "hero.glb",
            "--preset",
            "unity",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let parsed = parse_batch_args(&raw).unwrap();
        assert_eq!(parsed.open_path, Some(PathBuf::from("in.tessel")));
        assert_eq!(parsed.script_path.as_deref(), Some("ops.json"));
        assert_eq!(parsed.export_dir, PathBuf::from("/tmp/out"));
        assert_eq!(parsed.save_path, Some(PathBuf::from("out.tessel")));
        assert_eq!(parsed.export_filename.as_deref(), Some("hero.glb"));
        assert_eq!(parsed.preset, tessel_io::ExportPreset::Unity);
        assert!(!parsed.show_help);

        let bad = vec!["--bogus".to_string()];
        assert!(parse_batch_args(&bad).is_err());
    }
}
