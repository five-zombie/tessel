use std::path::PathBuf;
use std::time::{Duration, Instant};

use tessel_core::SceneState;
use tessel_io::{load_tessel_project, save_tessel_project_atomic};

const AUTOSAVE_DEBOUNCE: Duration = Duration::from_secs(5);
const AUTOSAVE_MIN_INTERVAL: Duration = Duration::from_secs(30);

/// Manages native OS Save/Open `.tessel` dialogs and debounced crash-recovery snapshots.
pub struct ProjectFileManager {
    pub current_path: Option<PathBuf>,
    pub recovery_path: PathBuf,
    last_saved_revision: u64,
    last_autosaved_revision: u64,
    last_observed_revision: u64,
    last_change_at: Instant,
    last_autosave_at: Instant,
    autosave_paused: bool,
}

impl Default for ProjectFileManager {
    fn default() -> Self {
        let recovery_path = std::env::temp_dir()
            .join("tessel_recovery")
            .join("autosave.tessel");
        let now = Instant::now();
        Self {
            current_path: None,
            recovery_path,
            last_saved_revision: 0,
            last_autosaved_revision: 0,
            last_observed_revision: 0,
            last_change_at: now,
            last_autosave_at: now,
            autosave_paused: false,
        }
    }
}

impl ProjectFileManager {
    pub fn with_recovery_path(recovery_path: PathBuf) -> Self {
        let now = Instant::now();
        Self {
            current_path: None,
            recovery_path,
            last_saved_revision: 0,
            last_autosaved_revision: 0,
            last_observed_revision: 0,
            last_change_at: now,
            last_autosave_at: now,
            autosave_paused: false,
        }
    }

    pub fn mark_baseline_revision(&mut self, revision: u64) {
        self.last_saved_revision = revision;
        self.last_autosaved_revision = revision;
        self.last_observed_revision = revision;
        self.last_change_at = Instant::now();
    }

    pub fn has_unsaved_changes(&self, scene_revision: u64) -> bool {
        scene_revision != self.last_saved_revision
    }

    pub fn set_autosave_paused(&mut self, paused: bool) {
        self.autosave_paused = paused;
    }

    pub fn has_recovery_snapshot(&self) -> bool {
        self.recovery_path.exists()
    }

    pub fn load_recovery_snapshot(&mut self) -> Result<SceneState, String> {
        let loaded = load_tessel_project(&self.recovery_path).map_err(|e| e.to_string())?;
        self.last_saved_revision = loaded.revision;
        Ok(loaded)
    }

    pub fn load_from_path(&mut self, path: PathBuf) -> Result<SceneState, String> {
        let loaded = load_tessel_project(&path).map_err(|e| e.to_string())?;
        self.current_path = Some(path);
        self.last_saved_revision = loaded.revision;
        Ok(loaded)
    }

    pub fn save_with_dialog(
        &mut self,
        scene: &SceneState,
        thumbnail_png: Option<&[u8]>,
    ) -> Result<Option<PathBuf>, String> {
        let target = match &self.current_path {
            Some(p) => Some(p.clone()),
            None => rfd::FileDialog::new()
                .add_filter("Tessel Project", &["tessel"])
                .set_file_name("untitled.tessel")
                .save_file(),
        };
        let Some(path) = target else {
            return Ok(None);
        };
        tessel_io::save_tessel_project_with_thumbnail(&path, scene, thumbnail_png)
            .map_err(|e| e.to_string())?;
        self.current_path = Some(path.clone());
        self.last_saved_revision = scene.revision;
        let _ = std::fs::remove_file(&self.recovery_path);
        Ok(Some(path))
    }

    pub fn open_with_dialog(&mut self) -> Result<Option<SceneState>, String> {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Tessel Project", &["tessel"])
            .pick_file()
        else {
            return Ok(None);
        };
        let loaded = load_tessel_project(&path).map_err(|e| e.to_string())?;
        self.current_path = Some(path);
        self.last_saved_revision = loaded.revision;
        Ok(Some(loaded))
    }

    /// Writes a debounced atomic recovery snapshot after edits settle (`AUTOSAVE_DEBOUNCE`)
    /// and at most once per `AUTOSAVE_MIN_INTERVAL`.
    pub fn tick_autosave(&mut self, scene: &SceneState) {
        let now = Instant::now();
        if scene.revision != self.last_observed_revision {
            self.last_observed_revision = scene.revision;
            self.last_change_at = now;
        }
        if self.autosave_paused
            || scene.revision == self.last_saved_revision
            || scene.revision == self.last_autosaved_revision
        {
            return;
        }
        if now.duration_since(self.last_change_at) < AUTOSAVE_DEBOUNCE {
            return;
        }
        if now.duration_since(self.last_autosave_at) < AUTOSAVE_MIN_INTERVAL {
            return;
        }
        self.last_autosave_at = now;
        match save_tessel_project_atomic(&self.recovery_path, scene) {
            Ok(()) => {
                self.last_autosaved_revision = scene.revision;
            }
            Err(err) => {
                tracing::warn!("Autosave recovery snapshot failed: {err}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paused_or_baseline_autosave_never_overwrites_existing_recovery_snapshot() {
        let dir = std::env::temp_dir().join("tessel_recovery_guard_test");
        let _ = std::fs::create_dir_all(&dir);
        let recovery_path = dir.join("autosave.tessel");
        std::fs::write(&recovery_path, b"PREVIOUS_CRASH_SNAPSHOT").unwrap();

        let mut scene = SceneState::new();
        scene.mark_modified();
        scene.mark_modified();

        let mut mgr = ProjectFileManager::with_recovery_path(recovery_path.clone());
        mgr.mark_baseline_revision(scene.revision);
        mgr.set_autosave_paused(true);
        mgr.last_autosave_at = Instant::now() - Duration::from_secs(120);

        // Even with elapsed > 60s and a new modification while paused, recovery snapshot is preserved.
        scene.mark_modified();
        mgr.tick_autosave(&scene);
        assert_eq!(
            std::fs::read(&recovery_path).unwrap(),
            b"PREVIOUS_CRASH_SNAPSHOT"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn autosave_debounces_active_edits_and_writes_after_idle_delay() {
        let dir = std::env::temp_dir().join("tessel_autosave_debounce_test");
        let _ = std::fs::create_dir_all(&dir);
        let recovery_path = dir.join("autosave.tessel");
        let _ = std::fs::remove_file(&recovery_path);

        let mut scene = SceneState::new();
        let mut mgr = ProjectFileManager::with_recovery_path(recovery_path.clone());
        mgr.mark_baseline_revision(scene.revision);
        mgr.last_autosave_at = Instant::now() - Duration::from_secs(60);

        // Active edit resets debounce clock -> should NOT write immediately
        scene.mark_modified();
        mgr.tick_autosave(&scene);
        assert!(!recovery_path.exists());

        // Simulate idle past debounce delay -> should write recovery snapshot
        mgr.last_change_at = Instant::now() - (AUTOSAVE_DEBOUNCE + Duration::from_secs(1));
        mgr.tick_autosave(&scene);
        assert!(recovery_path.exists());
        // Autosave must NOT clear has_unsaved_changes() (Close Guard must remain armed!)
        assert!(mgr.has_unsaved_changes(scene.revision));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
