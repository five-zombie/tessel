use std::path::PathBuf;

use egui::{CornerRadius, RichText};
use tessel_core::SceneState;
use tessel_io::{export_scene_to_dir, ExportFormat, ExportPreset};

use super::theme::tokens;

pub struct ExportDialogState {
    pub open: bool,
    pub format: ExportFormat,
    pub preset: ExportPreset,
    pub filename: String,
    pub output_dir: PathBuf,
    pub status_message: Option<Result<String, String>>,
}

impl Default for ExportDialogState {
    fn default() -> Self {
        let output_dir = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("exports");
        Self {
            open: false,
            format: ExportFormat::Glb,
            preset: ExportPreset::Godot4,
            filename: "asset.glb".into(),
            output_dir,
            status_message: None,
        }
    }
}

pub fn show_export_dialog(ctx: &egui::Context, scene: &SceneState, state: &mut ExportDialogState) {
    if !state.open {
        return;
    }

    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.open = false;
        return;
    }

    let mut open = state.open;
    egui::Window::new("Export Asset")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .fixed_size(egui::vec2(380.0, 240.0))
        .show(ctx, |ui| {
            ui.label(
                RichText::new("Format")
                    .size(11.5)
                    .color(tokens::ON_SURFACE_VARIANT),
            );
            ui.horizontal_wrapped(|ui| {
                for fmt in ExportFormat::ALL {
                    let selected = state.format == fmt;
                    if ui
                        .selectable_label(selected, format!(".{}", fmt.extension()))
                        .clicked()
                    {
                        state.format = fmt;
                        let stem = state
                            .filename
                            .split('.')
                            .next()
                            .filter(|s| !s.is_empty())
                            .unwrap_or("asset");
                        state.filename = format!("{stem}.{}", fmt.extension());
                    }
                }
            });

            ui.add_space(8.0);
            ui.label(
                RichText::new("Engine Preset")
                    .size(11.5)
                    .color(tokens::ON_SURFACE_VARIANT),
            );
            ui.horizontal_wrapped(|ui| {
                for preset in ExportPreset::ALL {
                    if ui
                        .selectable_label(state.preset == preset, preset.label())
                        .clicked()
                    {
                        state.preset = preset;
                    }
                }
            });

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("Filename:");
                ui.text_edit_singleline(&mut state.filename);
            });
            ui.label(
                RichText::new(format!("Output dir: {}", state.output_dir.display()))
                    .monospace()
                    .size(10.5)
                    .color(tokens::ON_SURFACE_VARIANT),
            );

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let export_btn = egui::Button::new(
                    RichText::new("Export Now")
                        .strong()
                        .color(tokens::ON_PRIMARY),
                )
                .fill(tokens::PRIMARY_CONTAINER)
                .corner_radius(CornerRadius::same(4));
                if ui.add(export_btn).clicked() {
                    match export_scene_to_dir(
                        scene,
                        &state.output_dir,
                        &state.filename,
                        state.format,
                        state.preset,
                    ) {
                        Ok(report) => {
                            state.status_message = Some(Ok(format!(
                                "Saved {} ({} bytes, {} tris)",
                                report.output_path, report.file_size_bytes, report.triangles
                            )));
                        }
                        Err(err) => {
                            state.status_message = Some(Err(err.to_string()));
                        }
                    }
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
