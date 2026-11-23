use egui::{Frame, Margin, RichText, Stroke};
use tessel_core::{AppMode, HistoryStack, SceneState};
use tessel_render::{OrbitCamera, PixelCamera2D};

use super::theme::tokens;

pub fn show_status_bar(
    ctx: &egui::Context,
    scene: &SceneState,
    history: &HistoryStack,
    camera_3d: &OrbitCamera,
    camera_2d: &PixelCamera2D,
) {
    let frame = Frame::NONE
        .fill(tokens::SURFACE)
        .inner_margin(Margin::symmetric(12, 4))
        .stroke(Stroke::new(1.0, tokens::OUTLINE_VARIANT));

    egui::TopBottomPanel::bottom("tessel_status_bar")
        .exact_height(24.0)
        .frame(frame)
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                let hints = match scene.active_mode {
                    AppMode::Pixel => {
                        "LMB: Draw  |  RMB/MMB: Pan  |  Scroll: Zoom  |  Alt+LMB: Picker  |  B/E/G: Tools  |  X/Y: Symmetry"
                    }
                    AppMode::Voxel | AppMode::LowPoly | AppMode::Character => {
                        "RMB: Orbit  |  Shift+RMB: Pan  |  Scroll: Zoom  |  F: Reset View  |  1-4: Mode"
                    }
                };
                ui.label(
                    RichText::new(hints)
                        .size(11.0)
                        .color(tokens::ON_SURFACE_VARIANT),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let right_text = match scene.active_mode {
                        AppMode::Pixel => format!(
                            "Mode: Pixel  |  Zoom: {:.0}x  |  Rev: {}  |  History: {}",
                            camera_2d.zoom,
                            scene.revision,
                            history.undo_len()
                        ),
                        _ => {
                            let pos = camera_3d.position();
                            format!(
                                "Mode: {}  |  Rev: {}  |  History: {}  |  Cam: ({:.1}, {:.1}, {:.1})",
                                scene.active_mode.label(),
                                scene.revision,
                                history.undo_len(),
                                pos.x,
                                pos.y,
                                pos.z
                            )
                        }
                    };
                    ui.label(
                        RichText::new(right_text)
                            .monospace()
                            .size(11.0)
                            .color(tokens::ON_SURFACE_VARIANT),
                    );
                });
            });
        });
}
