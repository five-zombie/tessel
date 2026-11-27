use tessel_core::{AppMode, Command, FrameIndex, HistoryStack, PixelTool, SceneState};
use tessel_render::{OrbitCamera, PixelCamera2D};

const FRAME_STEP_SECS: f32 = 1.0 / 24.0;

pub fn step_character_frame(scene: &mut SceneState, delta_frames: i32) -> bool {
    let Some(rig) = scene.character_rig.as_mut() else {
        return false;
    };
    let duration = rig
        .animations
        .get(rig.active_animation)
        .map(|c| c.duration_secs.max(0.05))
        .unwrap_or(1.0);
    rig.playing = false;
    let next = (rig.playback_time + delta_frames as f32 * FRAME_STEP_SECS).rem_euclid(duration);
    rig.playback_time = next;
    rig.apply_active_animation();
    scene.mark_modified();
    true
}

pub fn rewind_character(scene: &mut SceneState) -> bool {
    let Some(rig) = scene.character_rig.as_mut() else {
        return false;
    };
    rig.playing = false;
    rig.playback_time = 0.0;
    rig.apply_active_animation();
    scene.mark_modified();
    true
}

pub fn step_pixel_frame(scene: &mut SceneState, history: &mut HistoryStack, delta: isize) -> bool {
    let Some(node_id) = scene.active_pixel_node() else {
        return false;
    };
    let Some((active, len)) = scene
        .pixel_canvases
        .get(&node_id)
        .map(|c| (c.active_frame, c.frames.len()))
    else {
        return false;
    };
    if len <= 1 {
        return false;
    }
    let next = (active as isize + delta).rem_euclid(len as isize) as usize;
    if next == active {
        return false;
    }
    history
        .execute(
            scene,
            Command::SetActivePixelFrame {
                node_id,
                frame: FrameIndex(next),
            },
        )
        .is_ok()
}

/// Processes global and contextual studio shortcuts when text inputs do not hold keyboard focus.
/// Returns a user-facing error message if a command or history operation fails.
pub fn handle_global_shortcuts(
    ctx: &egui::Context,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    camera_3d: &mut OrbitCamera,
    camera_2d: &mut PixelCamera2D,
    requested_mode: &mut Option<AppMode>,
) -> Option<String> {
    if ctx.wants_keyboard_input() {
        return None;
    }

    let (
        undo,
        redo,
        duplicate,
        reset_cam,
        delete_selected,
        mode_switch,
        play_toggle,
        step_prev,
        step_next,
        rewind_home,
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
        let duplicate = cmd && !shift && i.key_pressed(egui::Key::D);
        let reset_cam = !cmd && i.key_pressed(egui::Key::F);
        let delete_selected = !cmd && i.key_pressed(egui::Key::Delete);
        let play_toggle = !cmd && i.key_pressed(egui::Key::Space);
        let step_prev = !cmd && i.key_pressed(egui::Key::ArrowLeft);
        let step_next = !cmd && i.key_pressed(egui::Key::ArrowRight);
        let rewind_home = !cmd && i.key_pressed(egui::Key::Home);

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
            duplicate,
            reset_cam,
            delete_selected,
            mode,
            play_toggle,
            step_prev,
            step_next,
            rewind_home,
            !cmd && i.key_pressed(egui::Key::B),
            !cmd && i.key_pressed(egui::Key::E),
            !cmd && i.key_pressed(egui::Key::G),
            !cmd && i.key_pressed(egui::Key::I),
            !cmd && i.key_pressed(egui::Key::X),
            !cmd && i.key_pressed(egui::Key::Y),
        )
    });

    let mut last_err: Option<String> = None;

    if undo {
        if let Err(err) = history.undo(scene) {
            last_err = Some(format!("Undo failed: {err}"));
        }
    } else if redo {
        if let Err(err) = history.redo(scene) {
            last_err = Some(format!("Redo failed: {err}"));
        }
    }

    if duplicate {
        if let Some(node_id) = scene.selected_node {
            if let Err(err) = history.execute(
                scene,
                Command::DuplicateNode {
                    node_id,
                    name: None,
                },
            ) {
                last_err = Some(format!("Duplicate failed: {err}"));
            }
        }
    }

    if delete_selected {
        if let Some(id) = scene.selected_node {
            if let Err(err) = history.execute(scene, Command::DeleteNode(id)) {
                last_err = Some(format!("Delete failed: {err}"));
            }
        }
    }

    if let Some(target_mode) = mode_switch {
        if scene.active_mode != target_mode {
            *requested_mode = Some(target_mode);
        }
    }

    if reset_cam {
        match scene.active_mode {
            AppMode::Pixel => camera_2d.reset(),
            _ => camera_3d.reset(),
        }
    }

    match scene.active_mode {
        AppMode::Character => {
            if play_toggle {
                if let Some(rig) = scene.character_rig.as_mut() {
                    rig.playing = !rig.playing;
                }
            }
            if step_prev {
                step_character_frame(scene, -1);
            } else if step_next {
                step_character_frame(scene, 1);
            } else if rewind_home {
                rewind_character(scene);
            }
        }
        AppMode::Pixel => {
            if step_prev {
                step_pixel_frame(scene, history, -1);
            } else if step_next {
                step_pixel_frame(scene, history, 1);
            } else if rewind_home {
                if let Some(node_id) = scene.active_pixel_node() {
                    if let Err(err) = history.execute(
                        scene,
                        Command::SetActivePixelFrame {
                            node_id,
                            frame: FrameIndex(0),
                        },
                    ) {
                        last_err = Some(format!("Rewind frame failed: {err}"));
                    }
                }
            }
            if key_b {
                scene.set_pixel_tool(PixelTool::Pencil);
            } else if key_e {
                scene.set_pixel_tool(PixelTool::Eraser);
            } else if key_g {
                scene.set_pixel_tool(PixelTool::Fill);
            } else if key_i {
                scene.set_pixel_tool(PixelTool::Eyedropper);
            }
            if key_x {
                scene.toggle_pixel_symmetry_x();
            }
            if key_y {
                scene.toggle_pixel_symmetry_y();
            }
        }
        AppMode::Voxel | AppMode::LowPoly => {}
    }

    last_err
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_frame_step_and_rewind_wrap_cleanly() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        history
            .execute(&mut scene, Command::ApplyHumanoidRig)
            .unwrap();

        assert!(step_character_frame(&mut scene, 1));
        let t1 = scene.character_rig.as_ref().unwrap().playback_time;
        assert!((t1 - FRAME_STEP_SECS).abs() < 1e-4);

        assert!(rewind_character(&mut scene));
        assert!((scene.character_rig.as_ref().unwrap().playback_time - 0.0).abs() < 1e-6);

        // Stepping backwards from 0.0 wraps near duration
        assert!(step_character_frame(&mut scene, -1));
        assert!(scene.character_rig.as_ref().unwrap().playback_time > 0.5);
    }

    #[test]
    fn pixel_frame_step_wraps_across_frames() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        history
            .execute(
                &mut scene,
                Command::CreatePixelCanvas {
                    name: "Sprite".into(),
                    width: 8,
                    height: 8,
                },
            )
            .unwrap();
        let node_id = scene.active_pixel_node().unwrap();
        history
            .execute(&mut scene, Command::AddPixelFrame { node_id })
            .unwrap();
        assert_eq!(scene.pixel_canvases[&node_id].active_frame, 1);

        assert!(step_pixel_frame(&mut scene, &mut history, 1));
        assert_eq!(scene.pixel_canvases[&node_id].active_frame, 0);

        assert!(step_pixel_frame(&mut scene, &mut history, -1));
        assert_eq!(scene.pixel_canvases[&node_id].active_frame, 1);
    }
}
