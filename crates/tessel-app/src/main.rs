mod app;
mod input;
mod project_files;
mod settings;
mod ui;

use std::fs::{self, OpenOptions};
use std::io::Write;
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
