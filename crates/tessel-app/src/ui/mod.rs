pub mod command_palette;
pub mod export_dialog;
pub mod icons;
pub mod inspector;
pub mod mcp_dialog;
pub mod quick_start;
pub mod rotate_gizmo;
pub mod settings_dialog;
pub mod status_bar;
pub mod theme;
pub mod timeline;
pub mod toolbar;
pub mod top_bar;
pub mod viewport;

use std::sync::mpsc::{self, Receiver, TryRecvError};
use tessel_core::{Command, HistoryStack, SceneState};

/// Non-blocking single-slot background task handle polled once per frame.
pub struct AsyncJobSlot<T: Send + 'static> {
    rx: Option<Receiver<T>>,
}

impl<T: Send + 'static> Default for AsyncJobSlot<T> {
    fn default() -> Self {
        Self { rx: None }
    }
}

impl<T: Send + 'static> AsyncJobSlot<T> {
    pub fn is_busy(&self) -> bool {
        self.rx.is_some()
    }

    /// Spawns `job` on a background thread if no job is currently in flight.
    pub fn spawn(&mut self, job: impl FnOnce() -> T + Send + 'static) {
        if self.rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(job());
        });
    }

    /// Non-blocking poll; returns `Ok(Some(value))` on completion, `Ok(None)` while running,
    /// or `Err(...)` if the background thread panicked/disconnected without sending a result.
    pub fn poll(&mut self) -> Result<Option<T>, &'static str> {
        let Some(rx) = self.rx.as_ref() else {
            return Ok(None);
        };
        match rx.try_recv() {
            Ok(val) => {
                self.rx = None;
                Ok(Some(val))
            }
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => {
                self.rx = None;
                Err("Background task failed unexpectedly.")
            }
        }
    }
}

const UI_LAST_ERROR_ID: &str = "tessel_ui_last_error";

/// Records a UI-triggered command/history error in egui temporary storage so `app.rs`
/// can surface it in the shared error banner.
pub fn push_ui_error(ctx: &egui::Context, err: impl std::fmt::Display) {
    let msg = err.to_string();
    tracing::warn!("UI command error: {msg}");
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(UI_LAST_ERROR_ID), msg));
}

/// Drains any pending UI command error recorded during the current frame.
pub fn take_ui_error(ctx: &egui::Context) -> Option<String> {
    ctx.data_mut(|d| d.remove_temp::<String>(egui::Id::new(UI_LAST_ERROR_ID)))
}

/// Executes a `Command` through `HistoryStack`, surfacing any error to the UI banner.
pub fn exec_history(
    ctx: &egui::Context,
    history: &mut HistoryStack,
    scene: &mut SceneState,
    command: Command,
) {
    if let Err(err) = history.execute(scene, command) {
        push_ui_error(ctx, err);
    }
}

/// Undoes the last command in `HistoryStack`, surfacing any error to the UI banner.
pub fn undo_history(ctx: &egui::Context, history: &mut HistoryStack, scene: &mut SceneState) {
    if let Err(err) = history.undo(scene) {
        push_ui_error(ctx, err);
    }
}

/// Redoes the last undone command in `HistoryStack`, surfacing any error to the UI banner.
pub fn redo_history(ctx: &egui::Context, history: &mut HistoryStack, scene: &mut SceneState) {
    if let Err(err) = history.redo(scene) {
        push_ui_error(ctx, err);
    }
}

/// Debounces continuous slider / `DragValue` edits so an entire drag produces a single
/// `HistoryStack` entry instead of flooding undo history every frame.
#[allow(clippy::too_many_arguments)]
pub fn commit_debounced_command(
    ctx: &egui::Context,
    id: egui::Id,
    history: &mut HistoryStack,
    scene: &mut SceneState,
    changed: bool,
    dragging: bool,
    drag_stopped: bool,
    restore_cmd: impl FnOnce(&SceneState) -> Option<Command>,
    new_cmd: Command,
) {
    if changed && dragging {
        let has_snapshot = ctx.data(|d| d.get_temp::<Option<Command>>(id).is_some());
        if !has_snapshot {
            ctx.data_mut(|d| d.insert_temp(id, restore_cmd(scene)));
        }
        if let Err(err) = new_cmd.apply(scene) {
            push_ui_error(ctx, err);
        }
        return;
    }

    if drag_stopped {
        let drag_entry = ctx.data_mut(|d| d.remove_temp::<Option<Command>>(id));
        let had_drag_change = drag_entry.is_some();
        if let Some(Some(prev)) = drag_entry {
            if let Err(err) = prev.apply(scene) {
                push_ui_error(ctx, err);
            }
        }
        if had_drag_change || changed {
            exec_history(ctx, history, scene, new_cmd);
        }
        return;
    }

    if changed {
        if let Some(prev) = ctx
            .data_mut(|d| d.remove_temp::<Option<Command>>(id))
            .flatten()
        {
            if let Err(err) = prev.apply(scene) {
                push_ui_error(ctx, err);
            }
        }
        exec_history(ctx, history, scene, new_cmd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_history_surfaces_error_via_ctx() {
        let ctx = egui::Context::default();
        let mut scene = SceneState::new();
        let mut history = HistoryStack::default();

        // Undo on empty history triggers an error that must be captured in ctx.
        undo_history(&ctx, &mut history, &mut scene);
        assert!(take_ui_error(&ctx).is_some());
        assert!(take_ui_error(&ctx).is_none());
    }

    #[test]
    fn commit_debounced_command_ignores_noop_drag_stop() {
        let ctx = egui::Context::default();
        let mut scene = SceneState::new();
        let mut history = HistoryStack::default();
        let az = scene.light_azimuth;
        let el = scene.light_elevation;
        let id = egui::Id::new("test_noop_drag_stop");

        commit_debounced_command(
            &ctx,
            id,
            &mut history,
            &mut scene,
            false,
            false,
            true,
            |_| None,
            Command::SetLightDirection {
                azimuth: az,
                elevation: el,
            },
        );
        assert_eq!(history.undo_len(), 0);
        assert!(take_ui_error(&ctx).is_none());
    }

    #[test]
    fn commit_debounced_command_collapses_drag_to_single_undo_entry() {
        let ctx = egui::Context::default();
        let mut scene = SceneState::new();
        let mut history = HistoryStack::default();
        let initial_az = scene.light_azimuth;
        let el = scene.light_elevation;
        let id = egui::Id::new("test_light_debounce");

        for az in [0.5_f32, 0.9, 1.25] {
            commit_debounced_command(
                &ctx,
                id,
                &mut history,
                &mut scene,
                true,
                true,
                false,
                |s| {
                    Some(Command::SetLightDirection {
                        azimuth: s.light_azimuth,
                        elevation: s.light_elevation,
                    })
                },
                Command::SetLightDirection {
                    azimuth: az,
                    elevation: el,
                },
            );
        }
        assert_eq!(history.undo_len(), 0);
        assert!((scene.light_azimuth - 1.25).abs() < 1e-6);

        commit_debounced_command(
            &ctx,
            id,
            &mut history,
            &mut scene,
            false,
            false,
            true,
            |s| {
                Some(Command::SetLightDirection {
                    azimuth: s.light_azimuth,
                    elevation: s.light_elevation,
                })
            },
            Command::SetLightDirection {
                azimuth: 1.25,
                elevation: el,
            },
        );
        assert_eq!(history.undo_len(), 1);
        assert!((scene.light_azimuth - 1.25).abs() < 1e-6);

        undo_history(&ctx, &mut history, &mut scene);
        assert!((scene.light_azimuth - initial_az).abs() < 1e-6);
    }

    #[test]
    fn async_job_slot_completes_without_duplicate_spawn() {
        let mut slot = AsyncJobSlot::<u32>::default();
        assert!(!slot.is_busy());
        slot.spawn(|| 42);
        assert!(slot.is_busy());
        // Duplicate spawn while busy must be ignored.
        slot.spawn(|| 99);

        let mut got = None;
        for _ in 0..100 {
            if let Ok(Some(v)) = slot.poll() {
                got = Some(v);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(got, Some(42));
        assert!(!slot.is_busy());
    }

    #[test]
    fn async_job_slot_surfaces_worker_panic_as_err() {
        let mut slot = AsyncJobSlot::<u32>::default();
        slot.spawn(|| panic!("boom"));
        let mut err = None;
        for _ in 0..100 {
            match slot.poll() {
                Err(msg) => {
                    err = Some(msg);
                    break;
                }
                Ok(_) => std::thread::sleep(std::time::Duration::from_millis(5)),
            }
        }
        assert!(err.is_some());
        assert!(!slot.is_busy());
    }
}
