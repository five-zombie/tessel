mod app;
mod input;
mod project_files;
mod ui;

use std::path::PathBuf;

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

    init_logging();
    install_panic_hook();

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

fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("fatal panic: {info}");
        default_hook(info);
    }));
}
