use tessel_core::{AppMode, Command, HistoryStack, PixelTool, SceneState};
use tessel_render::{OrbitCamera, PixelCamera2D};

/// Processes global and contextual studio shortcuts when text inputs do not hold keyboard focus.
pub fn handle_global_shortcuts(
    ctx: &egui::Context,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    camera_3d: &mut OrbitCamera,
    camera_2d: &mut PixelCamera2D,
) {
    if ctx.wants_keyboard_input() {
        return;
    }

    let (
        undo,
        redo,
        reset_cam,
        delete_selected,
        mode_switch,
        key_b,
        key_e,
        key_g,
        key_i,
        key_x,
        key_y,
    ) = ctx.input(|i| {
        let cmd = i.modifiers.command;
        let shift = i.modifiers.shift;

        let undo = cmd && !shift && i.key_pressed(egui::Key::Z);
        let redo =
            (cmd && shift && i.key_pressed(egui::Key::Z)) || (cmd && i.key_pressed(egui::Key::Y));
        let reset_cam = !cmd && i.key_pressed(egui::Key::F);
        let delete_selected = !cmd && i.key_pressed(egui::Key::Delete);

        let mode = if !cmd {
            if i.key_pressed(egui::Key::Num1) {
                Some(AppMode::Pixel)
            } else if i.key_pressed(egui::Key::Num2) {
                Some(AppMode::Voxel)
            } else if i.key_pressed(egui::Key::Num3) {
                Some(AppMode::LowPoly)
            } else if i.key_pressed(egui::Key::Num4) {
                Some(AppMode::Character)
            } else {
                None
            }
        } else {
            None
        };

        (
            undo,
            redo,
            reset_cam,
            delete_selected,
            mode,
            !cmd && i.key_pressed(egui::Key::B),
            !cmd && i.key_pressed(egui::Key::E),
            !cmd && i.key_pressed(egui::Key::G),
            !cmd && i.key_pressed(egui::Key::I),
            !cmd && i.key_pressed(egui::Key::X),
            !cmd && i.key_pressed(egui::Key::Y),
        )
    });

    if undo {
        let _ = history.undo(scene);
    } else if redo {
        let _ = history.redo(scene);
    }

    if delete_selected {
        if let Some(id) = scene.selected_node {
            let _ = history.execute(scene, Command::DeleteNode(id));
        }
    }

    if let Some(target_mode) = mode_switch {
        if scene.active_mode != target_mode {
            let _ = history.execute(scene, Command::SetMode(target_mode));
        }
    }

    if reset_cam {
        match scene.active_mode {
            AppMode::Pixel => camera_2d.reset(),
            _ => camera_3d.reset(),
        }
    }

    if scene.active_mode == AppMode::Pixel {
        if key_b {
            scene.pixel_tool = PixelTool::Pencil;
        } else if key_e {
            scene.pixel_tool = PixelTool::Eraser;
        } else if key_g {
            scene.pixel_tool = PixelTool::Fill;
        } else if key_i {
            scene.pixel_tool = PixelTool::Eyedropper;
        }
        if key_x {
            scene.pixel_symmetry.x = !scene.pixel_symmetry.x;
        }
        if key_y {
            scene.pixel_symmetry.y = !scene.pixel_symmetry.y;
        }
    }
}
