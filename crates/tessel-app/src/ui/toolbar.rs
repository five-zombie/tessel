use egui::{CornerRadius, Frame, Margin, RichText, Stroke};
use tessel_core::{
    AppMode, Command, FaceId, HistoryStack, PixelTool, PrimitiveKind, SceneState, VoxelTool,
};

use super::theme::tokens;

pub fn show_toolbar(ctx: &egui::Context, scene: &mut SceneState, history: &mut HistoryStack) {
    let frame = Frame::NONE
        .fill(tokens::SURFACE_LOW)
        .inner_margin(Margin::symmetric(6, 10))
        .stroke(Stroke::new(1.0, tokens::OUTLINE_VARIANT));

    egui::SidePanel::left("tessel_toolbar")
        .exact_width(72.0)
        .resizable(false)
        .frame(frame)
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("TOOLS")
                        .size(10.0)
                        .color(tokens::ON_SURFACE_VARIANT),
                );
                ui.add_space(6.0);

                match scene.active_mode {
                    AppMode::Pixel => {
                        for tool in PixelTool::ALL {
                            if tool_button(ui, tool.label(), scene.pixel_tool == tool) {
                                scene.pixel_tool = tool;
                            }
                        }
                        ui.add_space(6.0);
                        ui.separator();
                        ui.label(
                            RichText::new("SYM")
                                .size(10.0)
                                .color(tokens::ON_SURFACE_VARIANT),
                        );
                        toggle_chip(ui, "X", &mut scene.pixel_symmetry.x);
                        toggle_chip(ui, "Y", &mut scene.pixel_symmetry.y);
                        ui.separator();
                        if let Some(pixel_node_id) = scene.active_pixel_node() {
                            if action_button(ui, "To Voxel") {
                                let _ = history.execute(
                                    scene,
                                    Command::Transaction(vec![
                                        Command::ConvertPixelToVoxel {
                                            pixel_node_id,
                                            depth: 3,
                                            name: "Voxelized Sprite".into(),
                                        },
                                        Command::SetMode(AppMode::Voxel),
                                    ]),
                                );
                            }
                        }
                    }
                    AppMode::Voxel => {
                        for tool in VoxelTool::ALL {
                            if tool_button(ui, tool.label(), scene.voxel_tool == tool) {
                                scene.voxel_tool = tool;
                            }
                        }
                        ui.add_space(6.0);
                        ui.separator();
                        ui.label(
                            RichText::new("MIRROR")
                                .size(10.0)
                                .color(tokens::ON_SURFACE_VARIANT),
                        );
                        toggle_chip(ui, "X", &mut scene.voxel_symmetry.x);
                        toggle_chip(ui, "Y", &mut scene.voxel_symmetry.y);
                        toggle_chip(ui, "Z", &mut scene.voxel_symmetry.z);
                        ui.separator();
                        if let Some(voxel_node_id) = scene.active_voxel_node() {
                            if action_button(ui, "To Mesh") {
                                let _ = history.execute(
                                    scene,
                                    Command::Transaction(vec![
                                        Command::ConvertVoxelToLowPoly {
                                            voxel_node_id,
                                            name: "LowPoly Mesh".into(),
                                        },
                                        Command::SetMode(AppMode::LowPoly),
                                    ]),
                                );
                            }
                        }
                    }
                    AppMode::LowPoly => {
                        for kind in PrimitiveKind::ALL {
                            if action_button(ui, kind.label()) {
                                let _ = history.execute(
                                    scene,
                                    Command::CreatePrimitiveMesh {
                                        name: kind.label().into(),
                                        kind,
                                        size: 0.8,
                                        material: scene.active_color,
                                    },
                                );
                            }
                        }
                        ui.separator();
                        if let Some(node_id) = scene.active_lowpoly_node() {
                            if action_button(ui, "Extrude") {
                                let face_id = scene.selected_face.unwrap_or(FaceId(0));
                                let _ = history.execute(
                                    scene,
                                    Command::ExtrudeMeshFace {
                                        node_id,
                                        face_id,
                                        distance: 0.25,
                                    },
                                );
                            }
                        }
                    }
                    AppMode::Character => {
                        if action_button(ui, "Biped Rig") {
                            let _ = history.execute(scene, Command::ApplyBipedRig);
                        }
                    }
                }
            });
        });
}

fn tool_button(ui: &mut egui::Ui, label: &str, active: bool) -> bool {
    let fill = if active {
        tokens::PRIMARY_CONTAINER
    } else {
        tokens::SURFACE_HIGH
    };
    let text_color = if active {
        tokens::ON_PRIMARY
    } else {
        tokens::ON_SURFACE
    };
    let btn = egui::Button::new(RichText::new(label).size(11.0).color(text_color))
        .min_size(egui::vec2(58.0, 26.0))
        .fill(fill)
        .corner_radius(CornerRadius::same(4));
    ui.add(btn).clicked()
}

fn action_button(ui: &mut egui::Ui, label: &str) -> bool {
    let btn = egui::Button::new(RichText::new(label).size(10.5).color(tokens::ON_SURFACE))
        .min_size(egui::vec2(58.0, 24.0))
        .fill(tokens::SURFACE_HIGH)
        .corner_radius(CornerRadius::same(4));
    ui.add(btn).clicked()
}

fn toggle_chip(ui: &mut egui::Ui, label: &str, value: &mut bool) {
    let fill = if *value {
        tokens::PRIMARY_CONTAINER
    } else {
        tokens::SURFACE_HIGH
    };
    let text_color = if *value {
        tokens::ON_PRIMARY
    } else {
        tokens::ON_SURFACE
    };
    let btn = egui::Button::new(RichText::new(label).size(11.0).color(text_color))
        .min_size(egui::vec2(58.0, 22.0))
        .fill(fill)
        .corner_radius(CornerRadius::same(4));
    if ui.add(btn).clicked() {
        *value = !*value;
    }
}
