use egui::{CornerRadius, Frame, Margin, RichText, Stroke};
use tessel_core::{AppMode, Command, HistoryStack, SceneState};

use super::theme::tokens;

pub struct TopBarActions {
    pub toggle_mcp: bool,
    pub save_project: bool,
    pub open_project: bool,
    pub import_mesh: bool,
    pub open_settings: bool,
}

pub fn show_top_bar(
    ctx: &egui::Context,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    software_fallback: bool,
    export_open: &mut bool,
    mcp_port: Option<u16>,
) -> TopBarActions {
    let mut actions = TopBarActions {
        toggle_mcp: false,
        save_project: false,
        open_project: false,
        import_mesh: false,
        open_settings: false,
    };
    let frame = Frame::NONE
        .fill(tokens::SURFACE_LOW)
        .inner_margin(Margin::symmetric(12, 6))
        .stroke(Stroke::new(1.0, tokens::OUTLINE_VARIANT));

    egui::TopBottomPanel::top("tessel_top_bar")
        .exact_height(40.0)
        .frame(frame)
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.label(
                    RichText::new("TESSEL")
                        .strong()
                        .size(13.5)
                        .color(tokens::ON_SURFACE),
                );
                if ui.small_button("Open").clicked() {
                    actions.open_project = true;
                }
                if ui.small_button("Save").clicked() {
                    actions.save_project = true;
                }
                if ui
                    .small_button("Import")
                    .on_hover_text("Import 3D Mesh (.obj / .glb)")
                    .clicked()
                {
                    actions.import_mesh = true;
                }
                ui.separator();

                for mode in AppMode::ALL {
                    let selected = scene.active_mode == mode;
                    let text_color = if selected {
                        tokens::ON_PRIMARY
                    } else {
                        tokens::ON_SURFACE
                    };
                    let fill = if selected {
                        tokens::PRIMARY_CONTAINER
                    } else {
                        tokens::SURFACE_HIGH
                    };

                    let btn = egui::Button::new(RichText::new(mode.label()).color(text_color))
                        .fill(fill)
                        .corner_radius(CornerRadius::same(4));

                    if ui.add(btn).clicked() && !selected {
                        let _ = history.execute(scene, Command::SetMode(mode));
                    }
                }

                ui.separator();

                if ui
                    .add_enabled(history.can_undo(), egui::Button::new("Undo"))
                    .clicked()
                {
                    let _ = history.undo(scene);
                }

                if ui
                    .add_enabled(history.can_redo(), egui::Button::new("Redo"))
                    .clicked()
                {
                    let _ = history.redo(scene);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let export_btn = egui::Button::new(
                        RichText::new("Export .glb")
                            .strong()
                            .size(11.5)
                            .color(tokens::ON_PRIMARY),
                    )
                    .fill(tokens::PRIMARY_CONTAINER)
                    .corner_radius(CornerRadius::same(4));
                    if ui.add(export_btn).clicked() {
                        *export_open = true;
                    }
                    if ui.small_button("Settings").clicked() {
                        actions.open_settings = true;
                    }
                    ui.separator();
                    let mcp_label = match mcp_port {
                        Some(port) => format!("MCP: 127.0.0.1:{port}"),
                        None => "MCP: Offline".into(),
                    };
                    let mcp_color = if mcp_port.is_some() {
                        tokens::PRIMARY
                    } else {
                        tokens::ON_SURFACE_VARIANT
                    };
                    if ui
                        .add(
                            egui::Button::new(RichText::new(mcp_label).size(11.5).color(mcp_color))
                                .fill(tokens::SURFACE_HIGH)
                                .corner_radius(CornerRadius::same(4)),
                        )
                        .clicked()
                    {
                        actions.toggle_mcp = true;
                    }
                    if software_fallback {
                        ui.separator();
                        ui.label(
                            RichText::new("Software GPU Fallback")
                                .size(11.5)
                                .color(tokens::ERROR),
                        );
                    }
                });
            });
        });

    actions
}
