use std::collections::BTreeMap;
use std::path::PathBuf;

use egui::{CornerRadius, RichText};
use tessel_ai::{AiProviderKind, DirectAiConfig, LiveMcpServerHandle};

use crate::settings::{
    load_api_key_from_keyring, save_api_key_to_keyring, AppPaths, AppSettings, ProviderProfile,
};

use super::theme::tokens;

pub struct SettingsDialogState {
    pub open: bool,
    pub active_provider: AiProviderKind,
    pub profiles: BTreeMap<AiProviderKind, ProviderProfile>,
    pub api_keys: BTreeMap<AiProviderKind, String>,
    pub export_dir_text: String,
    pub status_message: Option<Result<String, String>>,
}

impl SettingsDialogState {
    pub fn from_settings(settings: &AppSettings) -> Self {
        let mut profiles = BTreeMap::new();
        let mut api_keys = BTreeMap::new();
        for kind in AiProviderKind::ALL {
            profiles.insert(kind, settings.profile_for(kind));
            api_keys.insert(kind, load_api_key_from_keyring(kind));
        }
        Self {
            open: false,
            active_provider: settings.ai_provider,
            profiles,
            api_keys,
            export_dir_text: settings.allowed_export_dir.display().to_string(),
            status_message: None,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn show_settings_dialog(
    ctx: &egui::Context,
    state: &mut SettingsDialogState,
    settings: &mut AppSettings,
    paths: &AppPaths,
    ai_config: &mut DirectAiConfig,
    export_dir: &mut PathBuf,
    live_mcp: Option<&LiveMcpServerHandle>,
    mcp_dialog_open: &mut bool,
) {
    if !state.open {
        return;
    }

    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.open = false;
        return;
    }

    let mut open = state.open;
    egui::Window::new("Settings — AI Providers, MCP & Export")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .fixed_size(egui::vec2(470.0, 350.0))
        .show(ctx, |ui| {
            ui.label(
                RichText::new("AI Provider Profiles")
                    .strong()
                    .size(13.0)
                    .color(tokens::ON_SURFACE),
            );
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for kind in AiProviderKind::ALL {
                    if ui
                        .selectable_label(state.active_provider == kind, kind.label())
                        .clicked()
                    {
                        state.active_provider = kind;
                    }
                }
            });

            ui.add_space(6.0);
            let kind = state.active_provider;
            let profile = state
                .profiles
                .entry(kind)
                .or_insert_with(|| ProviderProfile::default_for(kind));
            let key_buf = state.api_keys.entry(kind).or_default();

            egui::Grid::new("settings_provider_grid")
                .num_columns(2)
                .spacing(egui::vec2(10.0, 8.0))
                .show(ui, |ui| {
                    ui.label("Base URL:");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut profile.base_url).desired_width(250.0),
                        );
                        if ui.small_button("Reset").clicked() {
                            profile.base_url = kind.default_base_url().into();
                        }
                    });
                    ui.end_row();

                    ui.label("Model:");
                    ui.add(egui::TextEdit::singleline(&mut profile.model).desired_width(305.0));
                    ui.end_row();

                    ui.label("API Key:");
                    ui.add(
                        egui::TextEdit::singleline(key_buf)
                            .password(true)
                            .desired_width(305.0)
                            .hint_text(if kind == AiProviderKind::Ollama {
                                "Optional for local Ollama"
                            } else {
                                "Stored securely in OS Keyring"
                            }),
                    );
                    ui.end_row();
                });

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(6.0);

            ui.label(
                RichText::new("Export & MCP")
                    .strong()
                    .size(13.0)
                    .color(tokens::ON_SURFACE),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Allowed Export Dir:");
                ui.add(egui::TextEdit::singleline(&mut state.export_dir_text).desired_width(220.0));
                if ui.small_button("Browse...").clicked() {
                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                        state.export_dir_text = folder.display().to_string();
                    }
                }
            });

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let status = match live_mcp {
                    Some(mcp) => format!("Live MCP: http://127.0.0.1:{}/mcp", mcp.port),
                    None => "Live MCP: Offline".into(),
                };
                ui.label(RichText::new(status).monospace().size(11.0).color(
                    if live_mcp.is_some() {
                        tokens::PRIMARY
                    } else {
                        tokens::ON_SURFACE_VARIANT
                    },
                ));
                if ui.small_button("Open MCP Setup...").clicked() {
                    *mcp_dialog_open = true;
                }
            });

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let save_btn = egui::Button::new(
                    RichText::new("Save Settings")
                        .strong()
                        .color(tokens::ON_PRIMARY),
                )
                .fill(tokens::PRIMARY_CONTAINER)
                .corner_radius(CornerRadius::same(4));

                if ui.add(save_btn).clicked() {
                    let mut keyring_err: Option<String> = None;
                    for &p_kind in &AiProviderKind::ALL {
                        if let Some(prof) = state.profiles.get(&p_kind) {
                            settings.set_profile(p_kind, prof.clone());
                        }
                        if let Some(k) = state.api_keys.get(&p_kind) {
                            if let Err(err) = save_api_key_to_keyring(p_kind, k) {
                                keyring_err = Some(err);
                            }
                        }
                    }

                    settings.ai_provider = state.active_provider;
                    let active_prof = settings.profile_for(state.active_provider);
                    settings.ai_model = active_prof.model.clone();
                    settings.ai_base_url = active_prof.base_url.clone();

                    let next_export = PathBuf::from(state.export_dir_text.trim());
                    if !state.export_dir_text.trim().is_empty() {
                        settings.allowed_export_dir = next_export.clone();
                        *export_dir = next_export;
                    }
                    settings.save(paths);

                    ai_config.provider = settings.ai_provider;
                    ai_config.model = active_prof.model;
                    ai_config.base_url = active_prof.base_url;
                    ai_config.api_key = state
                        .api_keys
                        .get(&settings.ai_provider)
                        .cloned()
                        .unwrap_or_default();

                    state.status_message = match keyring_err {
                        Some(err) => Some(Err(format!(
                            "Settings saved, but OS Keyring returned: {err}"
                        ))),
                        None => Some(Ok("Settings & API keys saved.".into())),
                    };
                }

                if ui.button("Close").clicked() {
                    state.open = false;
                }
            });

            if let Some(msg) = &state.status_message {
                ui.add_space(6.0);
                match msg {
                    Ok(ok) => {
                        ui.label(RichText::new(ok).size(11.0).color(tokens::PRIMARY));
                    }
                    Err(err) => {
                        ui.label(RichText::new(err).size(11.0).color(tokens::ERROR));
                    }
                }
            }
        });

    state.open = open && state.open;
}
