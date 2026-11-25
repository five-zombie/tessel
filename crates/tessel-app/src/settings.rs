use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tessel_ai::AiProviderKind;

const MAX_RECENT_PROJECTS: usize = 6;

/// Returns the OS-native default export directory: `<home_dir>/tessel-project`
/// (Linux: `/home/<user>/tessel-project`, macOS: `/Users/<user>/tessel-project`,
/// Windows: `C:\Users\<user>\tessel-project`).
pub fn default_export_dir() -> PathBuf {
    if let Some(user_dirs) = directories::UserDirs::new() {
        return user_dirs.home_dir().join("tessel-project");
    }
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("tessel-project")
}

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
        let allowed_export_dir = default_export_dir();
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
        match serde_json::from_str::<Self>(&raw) {
            Ok(mut loaded) => {
                let trimmed = loaded.ai_base_url.trim();
                if !trimmed.is_empty()
                    && !trimmed.starts_with("http://")
                    && !trimmed.starts_with("https://")
                {
                    tracing::warn!(
                        base_url = trimmed,
                        "Invalid ai_base_url scheme in settings.json; resetting to provider default"
                    );
                    loaded.ai_base_url = loaded.ai_provider.default_base_url().into();
                }
                for (&provider, profile) in &mut loaded.provider_profiles {
                    let pt = profile.base_url.trim();
                    if !pt.is_empty() && !pt.starts_with("http://") && !pt.starts_with("https://") {
                        profile.base_url = provider.default_base_url().into();
                    }
                }
                loaded
            }
            Err(err) => {
                tracing::error!(
                    path = %paths.settings_file.display(),
                    "Corrupt settings.json ({err}); backing up to settings.json.corrupt and using defaults"
                );
                let backup = paths.settings_file.with_extension("json.corrupt");
                let _ = fs::copy(&paths.settings_file, &backup);
                Self::default()
            }
        }
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

fn mcp_token_file(paths: &AppPaths) -> PathBuf {
    paths.config_dir.join("mcp_token")
}

/// Loads the persistent Live MCP bearer token (OS Keyring or `<config_dir>/mcp_token`),
/// creating and saving one only on first use so the token stays stable across restarts.
pub fn load_or_create_mcp_token(paths: &AppPaths) -> String {
    #[cfg(not(test))]
    if let Ok(entry) = keyring::Entry::new("tessel", "mcp_live_token") {
        if let Ok(tok) = entry.get_password() {
            let trimmed = tok.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    if let Ok(raw) = fs::read_to_string(mcp_token_file(paths)) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    let token = tessel_ai::mcp::http::generate_local_token();
    save_mcp_token(paths, &token);
    token
}

/// Rotates the persistent Live MCP bearer token on explicit user request.
pub fn rotate_mcp_token(paths: &AppPaths) -> String {
    let token = tessel_ai::mcp::http::generate_local_token();
    save_mcp_token(paths, &token);
    token
}

fn save_mcp_token(paths: &AppPaths, token: &str) {
    #[cfg(not(test))]
    if let Ok(entry) = keyring::Entry::new("tessel", "mcp_live_token") {
        if entry.set_password(token).is_ok() {
            let _ = fs::remove_file(mcp_token_file(paths));
            return;
        }
    }
    let token_path = mcp_token_file(paths);
    if let Some(parent) = token_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        if let Ok(mut file) = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&token_path)
        {
            let _ = file.write_all(token.as_bytes());
        }
    }
    #[cfg(not(unix))]
    {
        let _ = fs::write(&token_path, token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_token_persists_until_explicitly_rotated() {
        let dir = std::env::temp_dir().join("tessel_mcp_token_persist_test");
        let _ = fs::remove_dir_all(&dir);
        let paths = AppPaths {
            config_dir: dir.clone(),
            settings_file: dir.join("settings.json"),
            recovery_file: dir.join("autosave.tessel"),
            log_file: dir.join("tessel.log"),
        };

        let first = load_or_create_mcp_token(&paths);
        assert!(first.starts_with("tsl_"));
        let second = load_or_create_mcp_token(&paths);
        assert_eq!(first, second);

        let rotated = rotate_mcp_token(&paths);
        assert_ne!(first, rotated);
        assert_eq!(load_or_create_mcp_token(&paths), rotated);

        let _ = fs::remove_dir_all(&dir);
    }
}
