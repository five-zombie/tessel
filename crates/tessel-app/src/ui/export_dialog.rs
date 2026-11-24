use std::path::PathBuf;

use egui::{CornerRadius, RichText};
use tessel_ai::headless::TurntableArgs;
use tessel_core::{AppMode, SceneState};
use tessel_io::{
    export_scene_to_dir, resolve_safe_export_path, ExportFormat, ExportPreset, IoError,
};
use tessel_render::OrbitCamera;

use super::icons::{self, ToolIcon};
use super::theme::tokens;

pub enum ExportAction {
    None,
    TurntableGif(TurntableArgs),
}

pub struct ExportDialogState {
    pub open: bool,
    pub format: ExportFormat,
    pub preset: ExportPreset,
    pub filename: String,
    pub output_dir: PathBuf,
    pub turntable_size: u32,
    pub turntable_frames: u32,
    pub turntable_fps: u32,
    pub turntable_clip: Option<usize>,
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
            turntable_size: 320,
            turntable_frames: 16,
            turntable_fps: 12,
            turntable_clip: None,
            status_message: None,
        }
    }
}

/// Returns true when `.gif` export should render a 3D turntable rather than a 2D pixel animation.
pub fn is_3d_turntable_gif(scene: &SceneState, format: ExportFormat) -> bool {
    format == ExportFormat::Gif
        && (scene.active_mode != AppMode::Pixel || scene.active_pixel_node().is_none())
}

/// Validates and builds [`TurntableArgs`] from the current dialog state and 3D viewport camera.
pub fn build_turntable_args(
    state: &ExportDialogState,
    camera: &OrbitCamera,
) -> Result<TurntableArgs, IoError> {
    let target_path = resolve_safe_export_path(&state.output_dir, &state.filename)?;
    let size = state.turntable_size.clamp(64, 512);
    Ok(TurntableArgs {
        target_path,
        width: size,
        height: size,
        frames: state.turntable_frames.clamp(4, 36),
        fps: state.turntable_fps.clamp(2, 30),
        preview: true,
        clip_name: None,
        clip_index: state.turntable_clip,
        camera: *camera,
    })
}

pub fn show_export_dialog(
    ctx: &egui::Context,
    scene: &SceneState,
    camera_3d: &OrbitCamera,
    state: &mut ExportDialogState,
) -> ExportAction {
    if !state.open {
        return ExportAction::None;
    }

    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.open = false;
        return ExportAction::None;
    }

    let mut action = ExportAction::None;
    let mut open = state.open;
    let turntable_mode = is_3d_turntable_gif(scene, state.format);

    egui::Window::new("Export Asset")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .min_width(390.0)
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
            if turntable_mode {
                ui.label(
                    RichText::new("3D Turntable GIF")
                        .size(11.5)
                        .color(tokens::ON_SURFACE_VARIANT),
                );
                ui.horizontal(|ui| {
                    ui.label("Resolution:");
                    for sz in [256u32, 320, 512] {
                        if ui
                            .selectable_label(state.turntable_size == sz, format!("{sz}px"))
                            .clicked()
                        {
                            state.turntable_size = sz;
                        }
                    }
                });
                ui.add(egui::Slider::new(&mut state.turntable_frames, 8..=36).text("Frames"));
                ui.add(egui::Slider::new(&mut state.turntable_fps, 4..=24).text("FPS"));
                if let Some(rig) = scene.character_rig.as_ref() {
                    if !rig.animations.is_empty() {
                        if let Some(ci) = state.turntable_clip {
                            if ci >= rig.animations.len() {
                                state.turntable_clip = None;
                            }
                        }
                        ui.horizontal_wrapped(|ui| {
                            ui.label("Animation:");
                            if ui
                                .selectable_label(state.turntable_clip.is_none(), "Static")
                                .clicked()
                            {
                                state.turntable_clip = None;
                            }
                            for (idx, clip) in rig.animations.iter().enumerate() {
                                if ui
                                    .selectable_label(state.turntable_clip == Some(idx), &clip.name)
                                    .clicked()
                                {
                                    state.turntable_clip = Some(idx);
                                }
                            }
                        });
                    }
                }
            } else {
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
            }

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
                    if turntable_mode {
                        match build_turntable_args(state, camera_3d) {
                            Ok(args) => {
                                action = ExportAction::TurntableGif(args);
                            }
                            Err(err) => {
                                state.status_message = Some(Err(err.to_string()));
                            }
                        }
                    } else {
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
                }
                if icons::hud_icon_button(ui, ToolIcon::Close, "Close", false) {
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
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use tessel_core::{Command, HistoryStack};

    #[test]
    fn is_3d_turntable_gif_routes_by_mode_and_canvas() {
        let mut scene = SceneState::new();
        scene.active_mode = AppMode::LowPoly;
        assert!(is_3d_turntable_gif(&scene, ExportFormat::Gif));
        assert!(!is_3d_turntable_gif(&scene, ExportFormat::Glb));

        scene.active_mode = AppMode::Pixel;
        assert!(
            is_3d_turntable_gif(&scene, ExportFormat::Gif),
            "pixel mode without pixel canvas falls back to 3D turntable"
        );

        let mut history = HistoryStack::new();
        history
            .execute(
                &mut scene,
                Command::CreatePixelCanvas {
                    name: "Sprite".into(),
                    width: 16,
                    height: 16,
                },
            )
            .unwrap();
        assert!(
            !is_3d_turntable_gif(&scene, ExportFormat::Gif),
            "pixel mode with active pixel canvas uses 2D pixel GIF export"
        );
    }

    #[test]
    fn build_turntable_args_validates_path_and_clamps_values() {
        let cam = OrbitCamera::new();
        let mut state = ExportDialogState {
            output_dir: std::env::temp_dir(),
            filename: "../escape.gif".into(),
            ..Default::default()
        };
        assert!(build_turntable_args(&state, &cam).is_err());

        state.filename = "showcase.gif".into();
        state.turntable_size = 2048;
        state.turntable_frames = 99;
        state.turntable_fps = 1;
        state.turntable_clip = Some(1);
        let args = build_turntable_args(&state, &cam).unwrap();
        assert_eq!(args.width, 512);
        assert_eq!(args.height, 512);
        assert_eq!(args.frames, 36);
        assert_eq!(args.fps, 2);
        assert_eq!(args.clip_index, Some(1));
        assert!(args.preview);
    }
}
