use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tessel_ai::AiProviderKind;

const MAX_RECENT_PROJECTS: usize = 6;

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub settings_file: PathBuf,
    pub recovery_file: PathBuf,
    pub log_file: PathBuf,
}

impl AppPaths {
    pub fn resolve() -> Self {
        let (config_dir, cache_dir) =
            match directories::ProjectDirs::from("org", "fivezombie", "tessel") {
                Some(dirs) => (
                    dirs.config_dir().to_path_buf(),
                    dirs.cache_dir().to_path_buf(),
                ),
                None => {
                    let base = std::env::temp_dir().join("tessel");
                    (base.join("config"), base.join("cache"))
                }
            };
        let settings_file = config_dir.join("settings.json");
        let recovery_file = cache_dir.join("recovery").join("autosave.tessel");
        let log_file = cache_dir.join("logs").join("tessel.log");
        Self {
            config_dir,
            settings_file,
            recovery_file,
            log_file,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderProfile {
    pub model: String,
    pub base_url: String,
}

impl ProviderProfile {
    pub fn default_for(provider: AiProviderKind) -> Self {
        Self {
            model: provider.default_model().into(),
            base_url: provider.default_base_url().into(),
        }
    }
}

/// Non-secret user settings persisted to `<config_dir>/settings.json`.
/// API keys are stored separately in the OS Keyring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub ai_provider: AiProviderKind,
    pub ai_model: String,
    pub ai_base_url: String,
    #[serde(default)]
    pub provider_profiles: BTreeMap<AiProviderKind, ProviderProfile>,
    pub allowed_export_dir: PathBuf,
    pub recent_projects: Vec<PathBuf>,
}

impl Default for AppSettings {
    fn default() -> Self {
        let allowed_export_dir = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("exports");
        let mut provider_profiles = BTreeMap::new();
        for kind in AiProviderKind::ALL {
            provider_profiles.insert(kind, ProviderProfile::default_for(kind));
        }
        Self {
            ai_provider: AiProviderKind::Ollama,
            ai_model: AiProviderKind::Ollama.default_model().into(),
            ai_base_url: AiProviderKind::Ollama.default_base_url().into(),
            provider_profiles,
            allowed_export_dir,
            recent_projects: Vec::new(),
        }
    }
}

impl AppSettings {
    pub fn load(paths: &AppPaths) -> Self {
        let Ok(raw) = fs::read_to_string(&paths.settings_file) else {
            return Self::default();
        };
        serde_json::from_str(&raw).unwrap_or_default()
    }

    pub fn save(&self, paths: &AppPaths) {
        if fs::create_dir_all(&paths.config_dir).is_err() {
            return;
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(&paths.settings_file, json);
        }
    }

    pub fn profile_for(&self, provider: AiProviderKind) -> ProviderProfile {
        self.provider_profiles
            .get(&provider)
            .cloned()
            .unwrap_or_else(|| ProviderProfile::default_for(provider))
    }

    pub fn set_profile(&mut self, provider: AiProviderKind, profile: ProviderProfile) {
        self.provider_profiles.insert(provider, profile);
    }

    pub fn record_recent_project(&mut self, path: PathBuf) {
        self.recent_projects.retain(|p| p != &path);
        self.recent_projects.insert(0, path);
        self.recent_projects.truncate(MAX_RECENT_PROJECTS);
    }
}

fn keyring_account(provider: AiProviderKind) -> &'static str {
    match provider {
        AiProviderKind::OpenAi => "direct_ai_openai",
        AiProviderKind::Ollama => "direct_ai_ollama",
        AiProviderKind::Gemini => "direct_ai_gemini",
        AiProviderKind::Anthropic => "direct_ai_anthropic",
    }
}

pub fn load_api_key_from_keyring(provider: AiProviderKind) -> String {
    let Ok(entry) = keyring::Entry::new("tessel", keyring_account(provider)) else {
        return String::new();
    };
    entry.get_password().unwrap_or_default()
}

pub fn save_api_key_to_keyring(provider: AiProviderKind, key: &str) -> Result<(), String> {
    let entry =
        keyring::Entry::new("tessel", keyring_account(provider)).map_err(|e| e.to_string())?;
    if key.trim().is_empty() {
        let _ = entry.delete_credential();
        Ok(())
    } else {
        entry.set_password(key.trim()).map_err(|e| e.to_string())
    }
}
