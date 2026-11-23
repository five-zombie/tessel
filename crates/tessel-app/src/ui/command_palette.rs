use egui::RichText;
use tessel_core::{
    AppMode, Command, HistoryStack, PaletteIndex, PrimitiveKind, SceneState, VoxelCoord,
};
use tessel_render::{OrbitCamera, PixelCamera2D};

use super::theme::tokens;

#[derive(Default)]
pub struct CommandPaletteState {
    pub open: bool,
    pub query: String,
}

#[derive(Clone, Copy)]
enum PaletteAction {
    Mode(AppMode),
    OpenExport,
    Undo,
    Redo,
    ResetCamera,
    SpawnStarterBipedVoxel,
    NewPixelCanvas,
    NewLowPolyCube,
    ApplyBipedRig,
}

struct ActionEntry {
    label: &'static str,
    shortcut: &'static str,
    action: PaletteAction,
}

const ACTIONS: &[ActionEntry] = &[
    ActionEntry {
        label: "Switch Mode: Pixel",
        shortcut: "1",
        action: PaletteAction::Mode(AppMode::Pixel),
    },
    ActionEntry {
        label: "Switch Mode: Voxel",
        shortcut: "2",
        action: PaletteAction::Mode(AppMode::Voxel),
    },
    ActionEntry {
        label: "Switch Mode: Low-Poly",
        shortcut: "3",
        action: PaletteAction::Mode(AppMode::LowPoly),
    },
    ActionEntry {
        label: "Switch Mode: Character",
        shortcut: "4",
        action: PaletteAction::Mode(AppMode::Character),
    },
    ActionEntry {
        label: "Export Asset (.glb / .obj / .stl / .vox / .png / .gif)",
        shortcut: "Ctrl+E",
        action: PaletteAction::OpenExport,
    },
    ActionEntry {
        label: "Starter Template: Spawn Biped Voxel Character",
        shortcut: "",
        action: PaletteAction::SpawnStarterBipedVoxel,
    },
    ActionEntry {
        label: "New 32x32 Pixel Canvas",
        shortcut: "",
        action: PaletteAction::NewPixelCanvas,
    },
    ActionEntry {
        label: "Create Low-Poly Cube",
        shortcut: "",
        action: PaletteAction::NewLowPolyCube,
    },
    ActionEntry {
        label: "Apply Biped Character Rig (Idle + Walk)",
        shortcut: "",
        action: PaletteAction::ApplyBipedRig,
    },
    ActionEntry {
        label: "Undo Last Action",
        shortcut: "Ctrl+Z",
        action: PaletteAction::Undo,
    },
    ActionEntry {
        label: "Redo Action",
        shortcut: "Ctrl+Y",
        action: PaletteAction::Redo,
    },
    ActionEntry {
        label: "Reset Camera View",
        shortcut: "F",
        action: PaletteAction::ResetCamera,
    },
];

#[allow(clippy::too_many_arguments)]
pub fn show_command_palette(
    ctx: &egui::Context,
    state: &mut CommandPaletteState,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    camera_3d: &mut OrbitCamera,
    camera_2d: &mut PixelCamera2D,
    export_open: &mut bool,
) {
    if !state.open {
        return;
    }

    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.open = false;
        state.query.clear();
        return;
    }

    let mut triggered: Option<PaletteAction> = None;
    let mut open = state.open;

    egui::Window::new("Command Palette")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 64.0))
        .fixed_size(egui::vec2(420.0, 280.0))
        .show(ctx, |ui| {
            let input = ui.add(
                egui::TextEdit::singleline(&mut state.query)
                    .desired_width(f32::INFINITY)
                    .hint_text("Type a command..."),
            );
            input.request_focus();

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(4.0);

            let needle = state.query.trim().to_lowercase();
            let filtered: Vec<&ActionEntry> = ACTIONS
                .iter()
                .filter(|a| needle.is_empty() || a.label.to_lowercase().contains(&needle))
                .collect();

            if filtered.is_empty() {
                ui.label(
                    RichText::new(
                        "No matching commands. Clear the search box to view all actions.",
                    )
                    .size(11.5)
                    .color(tokens::ON_SURFACE_VARIANT),
                );
            } else {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for entry in filtered {
                        ui.horizontal(|ui| {
                            if ui.selectable_label(false, entry.label).clicked() {
                                triggered = Some(entry.action);
                            }
                            if !entry.shortcut.is_empty() {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            RichText::new(entry.shortcut)
                                                .monospace()
                                                .size(10.5)
                                                .color(tokens::ON_SURFACE_VARIANT),
                                        );
                                    },
                                );
                            }
                        });
                    }
                });
            }
        });

    if let Some(action) = triggered {
        execute_action(action, scene, history, camera_3d, camera_2d, export_open);
        state.open = false;
        state.query.clear();
    } else {
        state.open = open;
    }
}

fn execute_action(
    action: PaletteAction,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    camera_3d: &mut OrbitCamera,
    camera_2d: &mut PixelCamera2D,
    export_open: &mut bool,
) {
    match action {
        PaletteAction::Mode(mode) => {
            if scene.active_mode != mode {
                let _ = history.execute(scene, Command::SetMode(mode));
            }
        }
        PaletteAction::OpenExport => {
            *export_open = true;
        }
        PaletteAction::Undo => {
            let _ = history.undo(scene);
        }
        PaletteAction::Redo => {
            let _ = history.redo(scene);
        }
        PaletteAction::ResetCamera => match scene.active_mode {
            AppMode::Pixel => camera_2d.reset(),
            _ => camera_3d.reset(),
        },
        PaletteAction::NewPixelCanvas => {
            let _ = history.execute(
                scene,
                Command::Transaction(vec![
                    Command::CreatePixelCanvas {
                        name: "Sprite 32x32".into(),
                        width: 32,
                        height: 32,
                    },
                    Command::SetMode(AppMode::Pixel),
                ]),
            );
        }
        PaletteAction::NewLowPolyCube => {
            let _ = history.execute(
                scene,
                Command::Transaction(vec![
                    Command::CreatePrimitiveMesh {
                        name: "Cube".into(),
                        kind: PrimitiveKind::Cube,
                        size: 0.8,
                        material: scene.active_color,
                    },
                    Command::SetMode(AppMode::LowPoly),
                ]),
            );
        }
        PaletteAction::ApplyBipedRig => {
            let _ = history.execute(
                scene,
                Command::Transaction(vec![
                    Command::ApplyBipedRig,
                    Command::SetMode(AppMode::Character),
                ]),
            );
        }
        PaletteAction::SpawnStarterBipedVoxel => {
            let _ = history.execute(
                scene,
                Command::CreateVoxelVolume {
                    name: "Starter Biped".into(),
                },
            );
            if let Some(node_id) = scene.active_voxel_node() {
                let c_body = PaletteIndex::new(7).unwrap_or(scene.active_color);
                let c_head = PaletteIndex::new(8).unwrap_or(scene.active_color);
                let c_legs = PaletteIndex::new(2).unwrap_or(scene.active_color);
                let _ = history.execute(
                    scene,
                    Command::Transaction(vec![
                        // Left & Right Legs
                        Command::FillVoxelBox {
                            node_id,
                            min: VoxelCoord::new(-2, 0, -1),
                            max: VoxelCoord::new(-1, 6, 1),
                            color: Some(c_legs),
                            hollow: false,
                        },
                        Command::FillVoxelBox {
                            node_id,
                            min: VoxelCoord::new(1, 0, -1),
                            max: VoxelCoord::new(2, 6, 1),
                            color: Some(c_legs),
                            hollow: false,
                        },
                        // Torso
                        Command::FillVoxelBox {
                            node_id,
                            min: VoxelCoord::new(-3, 7, -2),
                            max: VoxelCoord::new(3, 13, 2),
                            color: Some(c_body),
                            hollow: false,
                        },
                        // Arms
                        Command::FillVoxelBox {
                            node_id,
                            min: VoxelCoord::new(-5, 8, -1),
                            max: VoxelCoord::new(-4, 13, 1),
                            color: Some(c_body),
                            hollow: false,
                        },
                        Command::FillVoxelBox {
                            node_id,
                            min: VoxelCoord::new(4, 8, -1),
                            max: VoxelCoord::new(5, 13, 1),
                            color: Some(c_body),
                            hollow: false,
                        },
                        // Head
                        Command::FillVoxelBox {
                            node_id,
                            min: VoxelCoord::new(-2, 14, -2),
                            max: VoxelCoord::new(2, 18, 2),
                            color: Some(c_head),
                            hollow: false,
                        },
                        Command::SetMode(AppMode::Voxel),
                    ]),
                );
            }
        }
    }
}
