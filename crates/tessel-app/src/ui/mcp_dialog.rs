use std::io::Write;
use std::path::Path;
use std::process::{Command as ProcessCommand, Stdio};

use egui::RichText;
use tessel_ai::LiveMcpServerHandle;

use super::icons::{self, ToolIcon};
use super::theme::tokens;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum McpClientTarget {
    #[default]
    OpenCode,
    Codex,
    Agy,
    Claude,
}

impl McpClientTarget {
    pub const ALL: [Self; 4] = [Self::OpenCode, Self::Codex, Self::Agy, Self::Claude];

    pub const fn label(self) -> &'static str {
        match self {
            Self::OpenCode => "OpenCode",
            Self::Codex => "Codex",
            Self::Agy => "AGY",
            Self::Claude => "Claude",
        }
    }

    pub const fn config_path_hint(self) -> &'static str {
        match self {
            Self::OpenCode => "opencode.json  or  ~/.config/opencode/opencode.json",
            Self::Codex => "~/.codex/config.toml  or  .codex/config.toml",
            Self::Agy => "~/.gemini/config/mcp_config.json",
            Self::Claude => "claude_desktop_config.json  or  .mcp.json",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum McpTransportMode {
    #[default]
    LiveHttp,
    Stdio,
}

impl McpTransportMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::LiveHttp => "Live HTTP",
            Self::Stdio => "Stdio (CLI)",
        }
    }
}

pub const REDACTED_TOKEN_PLACEHOLDER: &str = "<REDACTED_MCP_TOKEN>";

#[derive(Default)]
pub struct McpDialogState {
    pub open: bool,
    pub client: McpClientTarget,
    pub transport: McpTransportMode,
    pub reveal_token: bool,
    pub status_message: Option<String>,
}

pub struct McpDialogAction {
    pub toggle_server: bool,
    pub rotate_token: bool,
}

#[cfg(target_os = "linux")]
const LINUX_CLIPBOARD_CANDIDATES: &[(&str, &[&str])] = &[
    ("wl-copy", &[]),
    ("xclip", &["-selection", "clipboard"]),
    ("xsel", &["--clipboard", "--input"]),
];

fn wait_child_bounded(child: &mut std::process::Child, timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    // Tool stayed alive (e.g. X11 xclip/xsel holding selection): treat as active.
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(err) => {
                tracing::warn!("clipboard helper wait failed: {err}");
                return false;
            }
        }
    }
}

/// Copies text to both egui's clipboard and the OS native clipboard tool on a background thread
/// so slow or selection-holding clipboard helpers never block the UI thread.
pub fn copy_text_reliable(ctx: &egui::Context, text: &str) {
    ctx.copy_text(text.to_string());
    let owned = text.to_string();
    let timeout = std::time::Duration::from_millis(250);

    #[cfg(target_os = "linux")]
    std::thread::spawn(move || {
        for (bin, args) in LINUX_CLIPBOARD_CANDIDATES {
            if let Ok(mut child) = ProcessCommand::new(bin)
                .args(*args)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                if let Some(mut stdin) = child.stdin.take() {
                    if let Err(err) = stdin.write_all(owned.as_bytes()) {
                        tracing::warn!("clipboard write to {bin} failed: {err}");
                    }
                }
                if wait_child_bounded(&mut child, timeout) {
                    break;
                }
            }
        }
    });

    #[cfg(target_os = "macos")]
    std::thread::spawn(move || {
        if let Ok(mut child) = ProcessCommand::new("pbcopy")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                if let Err(err) = stdin.write_all(owned.as_bytes()) {
                    tracing::warn!("clipboard write to pbcopy failed: {err}");
                }
            }
            let _ = wait_child_bounded(&mut child, timeout);
        }
    });

    #[cfg(target_os = "windows")]
    std::thread::spawn(move || {
        if let Ok(mut child) = ProcessCommand::new("clip")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                if let Err(err) = stdin.write_all(owned.as_bytes()) {
                    tracing::warn!("clipboard write to clip failed: {err}");
                }
            }
            let _ = wait_child_bounded(&mut child, timeout);
        }
    });
}

pub fn build_mcp_config_snippet(
    client: McpClientTarget,
    transport: McpTransportMode,
    live_mcp: Option<&LiveMcpServerHandle>,
    mcp_token: &str,
    export_dir: &Path,
    reveal_token: bool,
) -> String {
    let export_dir_escaped = export_dir
        .display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let (port, raw_token) = match live_mcp {
        Some(mcp) => (mcp.port, mcp.token.as_str()),
        None => (38421, mcp_token),
    };
    let token = if reveal_token {
        raw_token
    } else {
        REDACTED_TOKEN_PLACEHOLDER
    };
    let url = format!("http://127.0.0.1:{port}/mcp");

    match (client, transport) {
        (McpClientTarget::OpenCode, McpTransportMode::LiveHttp) => format!(
            "{{\n  \"$schema\": \"https://opencode.ai/config.json\",\n  \"mcp\": {{\n    \"tessel\": {{\n      \"type\": \"remote\",\n      \"url\": \"{url}\",\n      \"headers\": {{\n        \"Authorization\": \"Bearer {token}\"\n      }},\n      \"enabled\": true\n    }}\n  }}\n}}"
        ),
        (McpClientTarget::OpenCode, McpTransportMode::Stdio) => format!(
            "{{\n  \"$schema\": \"https://opencode.ai/config.json\",\n  \"mcp\": {{\n    \"tessel\": {{\n      \"type\": \"local\",\n      \"command\": [\"tessel\", \"mcp\", \"--export-dir\", \"{export_dir_escaped}\"],\n      \"enabled\": true\n    }}\n  }}\n}}"
        ),
        (McpClientTarget::Codex, McpTransportMode::LiveHttp) => format!(
            "[mcp_servers.tessel]\nurl = \"{url}\"\nenabled = true\n\n[mcp_servers.tessel.http_headers]\nAuthorization = \"Bearer {token}\""
        ),
        (McpClientTarget::Codex, McpTransportMode::Stdio) => format!(
            "[mcp_servers.tessel]\ncommand = \"tessel\"\nargs = [\"mcp\", \"--export-dir\", \"{export_dir_escaped}\"]\nenabled = true"
        ),
        (McpClientTarget::Agy | McpClientTarget::Claude, McpTransportMode::LiveHttp) => format!(
            "{{\n  \"mcpServers\": {{\n    \"tessel\": {{\n      \"url\": \"{url}\",\n      \"headers\": {{\n        \"Authorization\": \"Bearer {token}\"\n      }}\n    }}\n  }}\n}}"
        ),
        (McpClientTarget::Agy | McpClientTarget::Claude, McpTransportMode::Stdio) => format!(
            "{{\n  \"mcpServers\": {{\n    \"tessel\": {{\n      \"command\": \"tessel\",\n      \"args\": [\"mcp\", \"--export-dir\", \"{export_dir_escaped}\"]\n    }}\n  }}\n}}"
        ),
    }
}

pub fn show_mcp_dialog(
    ctx: &egui::Context,
    state: &mut McpDialogState,
    live_mcp: Option<&LiveMcpServerHandle>,
    mcp_token: &str,
    export_dir: &Path,
) -> McpDialogAction {
    let mut action = McpDialogAction {
        toggle_server: false,
        rotate_token: false,
    };
    if !state.open {
        return action;
    }

    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.open = false;
        state.reveal_token = false;
        return action;
    }

    let mut open = state.open;
    egui::Window::new("MCP Server & Client Setup")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .fixed_size(egui::vec2(520.0, 410.0))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let (status_text, status_color, srv_icon, srv_tip) = match live_mcp {
                    Some(mcp) => (
                        format!("Live HTTP: http://127.0.0.1:{}/mcp", mcp.port),
                        tokens::PRIMARY,
                        ToolIcon::Stop,
                        "Stop Server",
                    ),
                    None => (
                        "Live HTTP: Offline".into(),
                        tokens::ON_SURFACE_VARIANT,
                        ToolIcon::Play,
                        "Start Server",
                    ),
                };
                ui.label(
                    RichText::new(status_text)
                        .monospace()
                        .strong()
                        .size(11.5)
                        .color(status_color),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icons::hud_icon_button(ui, srv_icon, srv_tip, live_mcp.is_some()) {
                        action.toggle_server = true;
                    }
                });
            });

            ui.add_space(4.0);
            let active_token = live_mcp.map_or(mcp_token, |m| m.token.as_str());
            let mut token_display = active_token.to_string();
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Bearer Token:")
                        .size(11.0)
                        .color(tokens::ON_SURFACE_VARIANT),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut token_display)
                        .password(!state.reveal_token)
                        .desired_width(210.0)
                        .code_editor(),
                );
                let (eye_icon, eye_tip) = if state.reveal_token {
                    (ToolIcon::EyeClosed, "Hide Bearer Token")
                } else {
                    (ToolIcon::EyeOpen, "Reveal Bearer Token")
                };
                if icons::hud_icon_button(ui, eye_icon, eye_tip, state.reveal_token) {
                    state.reveal_token = !state.reveal_token;
                }
                if icons::hud_icon_button(ui, ToolIcon::Copy, "Copy", false) {
                    copy_text_reliable(ui.ctx(), active_token);
                    state.status_message = Some("Copied Bearer Token to clipboard.".into());
                }
                if icons::hud_icon_button(
                    ui,
                    ToolIcon::RotateButton,
                    "Generate a new persistent MCP token",
                    false,
                ) {
                    action.rotate_token = true;
                }
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Client:")
                        .strong()
                        .size(12.0)
                        .color(tokens::ON_SURFACE),
                );
                for client in McpClientTarget::ALL {
                    if ui
                        .selectable_label(state.client == client, client.label())
                        .clicked()
                    {
                        state.client = client;
                        state.status_message = None;
                    }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    for mode in [McpTransportMode::Stdio, McpTransportMode::LiveHttp] {
                        if ui
                            .selectable_label(state.transport == mode, mode.label())
                            .clicked()
                        {
                            state.transport = mode;
                            state.status_message = None;
                        }
                    }
                });
            });

            ui.add_space(4.0);
            ui.label(
                RichText::new(format!("Config file: {}", state.client.config_path_hint()))
                    .monospace()
                    .size(10.5)
                    .color(tokens::ON_SURFACE_VARIANT),
            );

            ui.add_space(6.0);
            let snippet = build_mcp_config_snippet(
                state.client,
                state.transport,
                live_mcp,
                mcp_token,
                export_dir,
                state.reveal_token,
            );
            let mut snippet_display = snippet.clone();

            egui::ScrollArea::vertical()
                .max_height(210.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut snippet_display)
                            .code_editor()
                            .desired_width(f32::INFINITY)
                            .desired_rows(11),
                    );
                });

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let copy_tip = if state.reveal_token || state.transport == McpTransportMode::Stdio {
                    format!("Copy {} Config", state.client.label())
                } else {
                    format!("Copy {} Config (token redacted)", state.client.label())
                };
                if icons::hud_icon_button(ui, ToolIcon::Copy, &copy_tip, false) {
                    copy_text_reliable(ui.ctx(), &snippet);
                    state.status_message = if state.reveal_token
                        || state.transport == McpTransportMode::Stdio
                    {
                        Some(format!(
                            "Copied {} ({}) config to clipboard.",
                            state.client.label(),
                            state.transport.label()
                        ))
                    } else {
                        Some(format!(
                            "Copied {} ({}) config with redacted token (reveal token to copy full secret).",
                            state.client.label(),
                            state.transport.label()
                        ))
                    };
                }

                if icons::hud_icon_button(ui, ToolIcon::Close, "Close", false) {
                    state.open = false;
                }
            });

            if let Some(msg) = &state.status_message {
                ui.add_space(6.0);
                ui.label(RichText::new(msg).size(11.0).color(tokens::PRIMARY));
            }
        });

    state.open = open && state.open;
    if !state.open {
        state.reveal_token = false;
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_config_snippets_match_client_schemas() {
        let dir = Path::new("/tmp/tessel_exports");
        let tok = "tsl_testtoken1234";
        for client in [
            McpClientTarget::OpenCode,
            McpClientTarget::Agy,
            McpClientTarget::Claude,
        ] {
            for mode in [McpTransportMode::LiveHttp, McpTransportMode::Stdio] {
                let snippet = build_mcp_config_snippet(client, mode, None, tok, dir, true);
                let parsed: serde_json::Value = serde_json::from_str(&snippet).unwrap();
                assert!(parsed.is_object());
            }
        }

        let codex_http = build_mcp_config_snippet(
            McpClientTarget::Codex,
            McpTransportMode::LiveHttp,
            None,
            tok,
            dir,
            true,
        );
        assert!(codex_http.contains("[mcp_servers.tessel]"));
        assert!(codex_http.contains("[mcp_servers.tessel.http_headers]"));
        assert!(codex_http.contains(tok));

        let codex_stdio = build_mcp_config_snippet(
            McpClientTarget::Codex,
            McpTransportMode::Stdio,
            None,
            tok,
            dir,
            true,
        );
        assert!(codex_stdio.contains("command = \"tessel\""));
    }

    #[test]
    fn mcp_config_snippets_redact_token_when_hidden() {
        let dir = Path::new("/tmp/tessel_exports");
        let tok = "tsl_supersecret987654";
        assert!(!McpDialogState::default().reveal_token);

        for client in McpClientTarget::ALL {
            let redacted =
                build_mcp_config_snippet(client, McpTransportMode::LiveHttp, None, tok, dir, false);
            assert!(!redacted.contains(tok));
            assert!(redacted.contains(REDACTED_TOKEN_PLACEHOLDER));

            if client != McpClientTarget::Codex {
                let parsed: serde_json::Value = serde_json::from_str(&redacted).unwrap();
                assert!(parsed.is_object());
            }

            let revealed =
                build_mcp_config_snippet(client, McpTransportMode::LiveHttp, None, tok, dir, true);
            assert!(revealed.contains(tok));
            assert!(!revealed.contains(REDACTED_TOKEN_PLACEHOLDER));
        }
    }

    #[test]
    fn clipboard_fallback_stops_only_on_zero_exit_or_running_daemon() {
        let mut ok_child = std::process::Command::new("true").spawn().unwrap();
        let mut fail_child = std::process::Command::new("false").spawn().unwrap();
        let timeout = std::time::Duration::from_millis(250);
        assert!(wait_child_bounded(&mut ok_child, timeout));
        assert!(!wait_child_bounded(&mut fail_child, timeout));

        #[cfg(target_os = "linux")]
        {
            let bins: Vec<&str> = LINUX_CLIPBOARD_CANDIDATES.iter().map(|(b, _)| *b).collect();
            assert_eq!(bins, vec!["wl-copy", "xclip", "xsel"]);
        }
    }
}
