use std::path::PathBuf;
use std::time::{Duration, Instant};

use tessel_core::SceneState;
use tessel_io::{load_tessel_project, save_tessel_project_atomic};

const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(60);

/// Manages native OS Save/Open `.tessel` dialogs and background crash-recovery snapshots.
pub struct ProjectFileManager {
    pub current_path: Option<PathBuf>,
    pub recovery_path: PathBuf,
    last_saved_revision: u64,
    last_autosave_at: Instant,
}

impl Default for ProjectFileManager {
    fn default() -> Self {
        let recovery_path = std::env::temp_dir()
            .join("tessel_recovery")
            .join("autosave.tessel");
        Self {
            current_path: None,
            recovery_path,
            last_saved_revision: 0,
            last_autosave_at: Instant::now(),
        }
    }
}

impl ProjectFileManager {
    pub fn with_recovery_path(recovery_path: PathBuf) -> Self {
        Self {
            current_path: None,
            recovery_path,
            last_saved_revision: 0,
            last_autosave_at: Instant::now(),
        }
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

    /// Writes a lightweight atomic recovery snapshot every 60 seconds if the scene changed.
    pub fn tick_autosave(&mut self, scene: &SceneState) {
        if scene.revision == self.last_saved_revision {
            return;
        }
        if self.last_autosave_at.elapsed() < AUTOSAVE_INTERVAL {
            return;
        }
        if save_tessel_project_atomic(&self.recovery_path, scene).is_ok() {
            self.last_saved_revision = scene.revision;
            self.last_autosave_at = Instant::now();
        }
    }
}
