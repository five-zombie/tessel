use std::path::PathBuf;

use egui::{CornerRadius, RichText, Stroke};
use tessel_core::{AppMode, Command, HistoryStack, SceneState};

use super::theme::tokens;

pub struct QuickStartState {
    pub open: bool,
    pub has_recovery: bool,
    pub error_message: Option<String>,
}

pub enum QuickStartAction {
    None,
    NewPixel,
    NewVoxel,
    NewCharacter,
    OpenAiWorkspace,
    RecoverSession,
    OpenRecent(PathBuf),
}

pub fn show_quick_start(
    ctx: &egui::Context,
    state: &mut QuickStartState,
    recent_projects: &[PathBuf],
) -> QuickStartAction {
    if !state.open {
        return QuickStartAction::None;
    }

    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.open = false;
        return QuickStartAction::None;
    }

    let mut action = QuickStartAction::None;
    let mut open = state.open;

    egui::Window::new("Welcome to Tessel")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .fixed_size(egui::vec2(480.0, 320.0))
        .show(ctx, |ui| {
            if state.has_recovery {
                egui::Frame::NONE
                    .fill(tokens::SURFACE_HIGH)
                    .stroke(Stroke::new(1.0, tokens::PRIMARY))
                    .corner_radius(CornerRadius::same(4))
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Unsaved session found from a previous run.")
                                    .size(11.5)
                                    .color(tokens::ON_SURFACE),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("Recover Session")
                                                    .strong()
                                                    .color(tokens::ON_PRIMARY),
                                            )
                                            .fill(tokens::PRIMARY_CONTAINER),
                                        )
                                        .clicked()
                                    {
                                        action = QuickStartAction::RecoverSession;
                                    }
                                },
                            );
                        });
                    });
                ui.add_space(8.0);
            }

            ui.label(
                RichText::new("Start Creating")
                    .strong()
                    .size(13.0)
                    .color(tokens::ON_SURFACE),
            );
            ui.add_space(6.0);

            egui::Grid::new("quick_start_cards")
                .spacing(egui::vec2(8.0, 8.0))
                .show(ui, |ui| {
                    if quick_card(ui, "New Character", "Biped rig & animation").clicked() {
                        action = QuickStartAction::NewCharacter;
                    }
                    if quick_card(ui, "New Voxel", "3D blocky studio").clicked() {
                        action = QuickStartAction::NewVoxel;
                    }
                    ui.end_row();
                    if quick_card(ui, "New Pixel", "32x32 layered sprite").clicked() {
                        action = QuickStartAction::NewPixel;
                    }
                    if quick_card(ui, "Create with AI", "Direct AI & MCP tools").clicked() {
                        action = QuickStartAction::OpenAiWorkspace;
                    }
                    ui.end_row();
                });

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(6.0);
            ui.label(
                RichText::new("Recent Projects")
                    .strong()
                    .size(12.5)
                    .color(tokens::ON_SURFACE),
            );
            ui.add_space(4.0);

            if recent_projects.is_empty() {
                ui.label(
                    RichText::new("No recent .tessel files yet. Save a project with Ctrl+S.")
                        .size(11.5)
                        .color(tokens::ON_SURFACE_VARIANT),
                );
            } else {
                for path in recent_projects {
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("project.tessel");
                    if ui
                        .selectable_label(false, format!("{name}  ({})", path.display()))
                        .clicked()
                    {
                        action = QuickStartAction::OpenRecent(path.clone());
                    }
                }
            }

            if let Some(err) = &state.error_message {
                ui.add_space(6.0);
                ui.label(RichText::new(err).size(11.0).color(tokens::ERROR));
            }
        });

    if !matches!(action, QuickStartAction::None) {
        state.open = false;
    } else {
        state.open = open;
    }
    action
}

fn quick_card(ui: &mut egui::Ui, title: &str, subtitle: &str) -> egui::Response {
    let btn = egui::Button::new(
        RichText::new(format!("{title}\n{subtitle}"))
            .size(12.0)
            .color(tokens::ON_SURFACE),
    )
    .min_size(egui::vec2(220.0, 48.0))
    .fill(tokens::SURFACE_HIGH)
    .corner_radius(CornerRadius::same(4));
    ui.add(btn)
}

pub fn apply_quick_start_preset(
    action: &QuickStartAction,
    scene: &mut SceneState,
    history: &mut HistoryStack,
) {
    match action {
        QuickStartAction::NewPixel => {
            let _ = history.execute(scene, Command::SetMode(AppMode::Pixel));
        }
        QuickStartAction::NewVoxel | QuickStartAction::OpenAiWorkspace => {
            let _ = history.execute(scene, Command::SetMode(AppMode::Voxel));
        }
        QuickStartAction::NewCharacter => {
            let _ = history.execute(
                scene,
                Command::Transaction(vec![
                    Command::ApplyBipedRig,
                    Command::SetMode(AppMode::Character),
                ]),
            );
        }
        QuickStartAction::None
        | QuickStartAction::RecoverSession
        | QuickStartAction::OpenRecent(_) => {}
    }
}
