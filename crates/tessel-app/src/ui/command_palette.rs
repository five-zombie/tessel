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
    pub selected_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaletteAction {
    Mode(AppMode),
    NewProject,
    OpenExport,
    ImportMesh,
    Undo,
    Redo,
    ResetCamera,
    SpawnStarterBipedVoxel,
    NewPixelCanvas,
    NewLowPolyCube,
    ApplyHumanoidRig,
    ApplyBipedRig,
    ResetCharacterPose,
    WeldMeshVertices,
    FlipMeshNormals,
    MirrorMeshX,
    ApplyMeshTransform,
    ClearScene,
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
        label: "New Project (Reset Workspace)",
        shortcut: "Ctrl+N",
        action: PaletteAction::NewProject,
    },
    ActionEntry {
        label: "Export Asset (.glb / .obj / .stl / .vox / .png / .gif)",
        shortcut: "Ctrl+E",
        action: PaletteAction::OpenExport,
    },
    ActionEntry {
        label: "Import 3D Mesh (.obj / .glb)",
        shortcut: "Ctrl+I",
        action: PaletteAction::ImportMesh,
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
        label: "Apply Humanoid Character Rig (19 Bones, Idle + Walk)",
        shortcut: "",
        action: PaletteAction::ApplyHumanoidRig,
    },
    ActionEntry {
        label: "Apply Biped Character Rig (11 Bones, Idle + Walk)",
        shortcut: "",
        action: PaletteAction::ApplyBipedRig,
    },
    ActionEntry {
        label: "Character: Reset All Bones to Rest Pose",
        shortcut: "",
        action: PaletteAction::ResetCharacterPose,
    },
    ActionEntry {
        label: "Mesh: Weld Coincident Vertices",
        shortcut: "",
        action: PaletteAction::WeldMeshVertices,
    },
    ActionEntry {
        label: "Mesh: Flip Face Normals",
        shortcut: "",
        action: PaletteAction::FlipMeshNormals,
    },
    ActionEntry {
        label: "Mesh: Mirror Selected Along X",
        shortcut: "",
        action: PaletteAction::MirrorMeshX,
    },
    ActionEntry {
        label: "Mesh: Apply Transform to Geometry",
        shortcut: "",
        action: PaletteAction::ApplyMeshTransform,
    },
    ActionEntry {
        label: "Scene: Clear All Nodes",
        shortcut: "",
        action: PaletteAction::ClearScene,
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

fn filter_actions(query: &str) -> Vec<&'static ActionEntry> {
    let needle = query.trim().to_lowercase();
    ACTIONS
        .iter()
        .filter(|a| needle.is_empty() || a.label.to_lowercase().contains(&needle))
        .collect()
}

fn step_selection(current: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    let cur = current.min(len - 1) as isize;
    (cur + delta).rem_euclid(len as isize) as usize
}

pub struct CommandPaletteContext<'a> {
    pub state: &'a mut CommandPaletteState,
    pub scene: &'a mut SceneState,
    pub history: &'a mut HistoryStack,
    pub camera_3d: &'a mut OrbitCamera,
    pub camera_2d: &'a mut PixelCamera2D,
    pub export_open: &'a mut bool,
    pub import_mesh: &'a mut bool,
    pub new_project: &'a mut bool,
    pub requested_mode: &'a mut Option<AppMode>,
}

pub fn show_command_palette(ctx: &egui::Context, c: CommandPaletteContext<'_>) {
    let CommandPaletteContext {
        state,
        scene,
        history,
        camera_3d,
        camera_2d,
        export_open,
        import_mesh,
        new_project,
        requested_mode,
    } = c;
    if !state.open {
        return;
    }

    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.open = false;
        state.query.clear();
        state.selected_index = 0;
        return;
    }

    let mut triggered: Option<PaletteAction> = None;
    let mut open = state.open;

    egui::Window::new("Command Palette")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 64.0))
        .fixed_size(egui::vec2(440.0, 310.0))
        .show(ctx, |ui| {
            let input = ui.add(
                egui::TextEdit::singleline(&mut state.query)
                    .desired_width(f32::INFINITY)
                    .hint_text("Type a command... (Up/Down + Enter)"),
            );
            input.request_focus();
            if input.changed() {
                state.selected_index = 0;
            }

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(4.0);

            let filtered = filter_actions(&state.query);
            if !filtered.is_empty() {
                state.selected_index = state.selected_index.min(filtered.len() - 1);
                if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                    state.selected_index = step_selection(state.selected_index, filtered.len(), 1);
                }
                if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                    state.selected_index = step_selection(state.selected_index, filtered.len(), -1);
                }
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    triggered = Some(filtered[state.selected_index].action);
                }
            } else {
                state.selected_index = 0;
            }

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
                    for (idx, entry) in filtered.iter().enumerate() {
                        let is_selected = idx == state.selected_index;
                        ui.horizontal(|ui| {
                            let resp = ui.selectable_label(is_selected, entry.label);
                            if is_selected {
                                resp.scroll_to_me(Some(egui::Align::Center));
                            }
                            if resp.clicked() {
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
        execute_action(
            ctx,
            action,
            CommandPaletteExec {
                scene,
                history,
                camera_3d,
                camera_2d,
                export_open,
                import_mesh,
                new_project,
                requested_mode,
            },
        );
        state.open = false;
        state.query.clear();
        state.selected_index = 0;
    } else {
        state.open = open;
    }
}

struct CommandPaletteExec<'a> {
    scene: &'a mut SceneState,
    history: &'a mut HistoryStack,
    camera_3d: &'a mut OrbitCamera,
    camera_2d: &'a mut PixelCamera2D,
    export_open: &'a mut bool,
    import_mesh: &'a mut bool,
    new_project: &'a mut bool,
    requested_mode: &'a mut Option<AppMode>,
}

fn execute_action(ctx: &egui::Context, action: PaletteAction, e: CommandPaletteExec<'_>) {
    let CommandPaletteExec {
        scene,
        history,
        camera_3d,
        camera_2d,
        export_open,
        import_mesh,
        new_project,
        requested_mode,
    } = e;
    match action {
        PaletteAction::Mode(mode) => {
            if scene.active_mode != mode {
                *requested_mode = Some(mode);
            }
        }
        PaletteAction::NewProject => {
            *new_project = true;
        }
        PaletteAction::OpenExport => {
            *export_open = true;
        }
        PaletteAction::ImportMesh => {
            *import_mesh = true;
        }
        PaletteAction::Undo => {
            super::undo_history(ctx, history, scene);
        }
        PaletteAction::Redo => {
            super::redo_history(ctx, history, scene);
        }
        PaletteAction::ResetCamera => match scene.active_mode {
            AppMode::Pixel => camera_2d.reset(),
            _ => camera_3d.reset(),
        },
        PaletteAction::NewPixelCanvas => {
            if scene.active_mode == AppMode::Pixel {
                super::exec_history(
                    ctx,
                    history,
                    scene,
                    Command::CreatePixelCanvas {
                        name: "Sprite 32x32".into(),
                        width: 32,
                        height: 32,
                    },
                );
            } else {
                *requested_mode = Some(AppMode::Pixel);
            }
        }
        PaletteAction::NewLowPolyCube => {
            if matches!(scene.active_mode, AppMode::LowPoly | AppMode::Character) {
                super::exec_history(
                    ctx,
                    history,
                    scene,
                    Command::CreatePrimitiveMesh {
                        name: "Cube".into(),
                        kind: PrimitiveKind::Cube,
                        size: 0.8,
                        material: scene.active_color,
                    },
                );
            } else {
                *requested_mode = Some(AppMode::LowPoly);
            }
        }
        PaletteAction::ApplyHumanoidRig => {
            super::exec_history(
                ctx,
                history,
                scene,
                Command::Transaction(vec![
                    Command::ApplyHumanoidRig,
                    Command::SetMode(AppMode::Character),
                ]),
            );
        }
        PaletteAction::ApplyBipedRig => {
            super::exec_history(
                ctx,
                history,
                scene,
                Command::Transaction(vec![
                    Command::ApplyBipedRig,
                    Command::SetMode(AppMode::Character),
                ]),
            );
        }
        PaletteAction::ResetCharacterPose => {
            super::exec_history(ctx, history, scene, Command::ResetCharacterPose);
        }
        PaletteAction::WeldMeshVertices => {
            if let Some(node_id) = scene.active_lowpoly_node() {
                super::exec_history(
                    ctx,
                    history,
                    scene,
                    Command::WeldMeshVertices {
                        node_id,
                        tolerance: 1e-4,
                    },
                );
            }
        }
        PaletteAction::FlipMeshNormals => {
            if let Some(node_id) = scene.active_lowpoly_node() {
                super::exec_history(
                    ctx,
                    history,
                    scene,
                    Command::FlipMeshNormals {
                        node_id,
                        face_ids: None,
                    },
                );
            }
        }
        PaletteAction::MirrorMeshX => {
            if let Some(node_id) = scene.active_lowpoly_node() {
                super::exec_history(
                    ctx,
                    history,
                    scene,
                    Command::MirrorLowPolyMesh {
                        node_id,
                        axis: 0,
                        merge_original: true,
                    },
                );
            }
        }
        PaletteAction::ApplyMeshTransform => {
            if let Some(node_id) = scene.active_lowpoly_node() {
                super::exec_history(ctx, history, scene, Command::ApplyMeshTransform { node_id });
            }
        }
        PaletteAction::ClearScene => {
            super::exec_history(ctx, history, scene, Command::ClearScene);
        }
        PaletteAction::SpawnStarterBipedVoxel => {
            if scene.active_mode != AppMode::Voxel {
                *requested_mode = Some(AppMode::Voxel);
                return;
            }
            super::exec_history(
                ctx,
                history,
                scene,
                Command::CreateVoxelVolume {
                    name: "Starter Biped".into(),
                },
            );
            if let Some(node_id) = scene.active_voxel_node() {
                let c_body = PaletteIndex::new(7).unwrap_or(scene.active_color);
                let c_head = PaletteIndex::new(8).unwrap_or(scene.active_color);
                let c_legs = PaletteIndex::new(2).unwrap_or(scene.active_color);
                super::exec_history(
                    ctx,
                    history,
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
                    ]),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_and_keyboard_step_navigation_wrap_cleanly() {
        let hits = filter_actions("humanoid");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].action, PaletteAction::ApplyHumanoidRig);

        assert_eq!(step_selection(0, 5, -1), 4);
        assert_eq!(step_selection(4, 5, 1), 0);
        assert_eq!(step_selection(0, 0, 1), 0);
    }

    #[test]
    fn execute_action_applies_humanoid_rig_and_new_project_flag() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let mut cam3d = OrbitCamera::new();
        let mut cam2d = PixelCamera2D::new();
        let mut export_open = false;
        let mut import_mesh = false;
        let mut new_project = false;
        let mut requested_mode = None;

        let ctx = egui::Context::default();
        execute_action(
            &ctx,
            PaletteAction::ApplyHumanoidRig,
            CommandPaletteExec {
                scene: &mut scene,
                history: &mut history,
                camera_3d: &mut cam3d,
                camera_2d: &mut cam2d,
                export_open: &mut export_open,
                import_mesh: &mut import_mesh,
                new_project: &mut new_project,
                requested_mode: &mut requested_mode,
            },
        );
        assert_eq!(scene.active_mode, AppMode::Character);
        assert_eq!(
            scene.character_rig.as_ref().map(|r| r.bones.len()),
            Some(19)
        );

        execute_action(
            &ctx,
            PaletteAction::NewProject,
            CommandPaletteExec {
                scene: &mut scene,
                history: &mut history,
                camera_3d: &mut cam3d,
                camera_2d: &mut cam2d,
                export_open: &mut export_open,
                import_mesh: &mut import_mesh,
                new_project: &mut new_project,
                requested_mode: &mut requested_mode,
            },
        );
        assert!(new_project);

        execute_action(
            &ctx,
            PaletteAction::Mode(AppMode::Pixel),
            CommandPaletteExec {
                scene: &mut scene,
                history: &mut history,
                camera_3d: &mut cam3d,
                camera_2d: &mut cam2d,
                export_open: &mut export_open,
                import_mesh: &mut import_mesh,
                new_project: &mut new_project,
                requested_mode: &mut requested_mode,
            },
        );
        assert_eq!(requested_mode, Some(AppMode::Pixel));
    }
}
