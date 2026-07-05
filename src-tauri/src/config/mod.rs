//! Application configuration and secret management.
//!
//! ## AppConfig
//!
//! Persisted as JSON in the Tauri app data directory. Handles corruption
//! recovery by backing up the corrupted file and loading defaults.
//!
//! ## Secrets
//!
//! Provider API tokens are stored via the OS keyring with an AES-256-GCM
//! encrypted file fallback. See [`secrets::SecretsManager`] for details.

pub mod secrets;

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// User preferences and application state.
///
/// This struct is serialized to JSON and stored in the app data directory.
/// It does NOT contain sensitive data (tokens are stored separately via
/// the secrets manager).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// ISO 639-1 language code: "en" or "es".
    pub language: String,

    /// Theme preference: "light", "dark", or "system".
    pub theme: String,

    /// Optional reference to the last active server for reconnection.
    pub last_server_id: Option<String>,

    /// References to saved provider tokens (not the tokens themselves).
    pub saved_token_refs: Vec<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            language: "en".to_string(),
            theme: "system".to_string(),
            last_server_id: None,
            saved_token_refs: Vec::new(),
        }
    }
}

/// Manager for reading, writing, and recovering the application config.
pub struct ConfigManager {
    config_path: PathBuf,
    config: AppConfig,
}

impl ConfigManager {
    /// Creates a new `ConfigManager`, loading config from `config_path`
    /// or falling back to defaults if the file is missing or corrupted.
    pub fn new(app_data_dir: PathBuf) -> Self {
        let config_path = app_data_dir.join("config.json");
        let config = Self::load_or_default(&config_path);
        Self { config_path, config }
    }

    fn load_or_default(path: &PathBuf) -> AppConfig {
        match fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(config) => config,
                Err(e) => {
                    // Corruption recovery: back up the corrupted file, then use defaults
                    let bak_path = path.with_extension("json.bak");
                    if let Err(rename_err) = fs::rename(path, &bak_path) {
                        log::warn!(
                            "Failed to back up corrupted config to {:?}: {}",
                            bak_path,
                            rename_err
                        );
                    }
                    log::warn!(
                        "Corrupted config file backed up to {:?}: {}",
                        bak_path,
                        e
                    );
                    AppConfig::default()
                }
            },
            Err(_) => {
                // File doesn't exist — first launch
                AppConfig::default()
            }
        }
    }

    /// Returns a reference to the current config.
    pub fn get(&self) -> &AppConfig {
        &self.config
    }

    /// Replaces the current config with a new value and persists it.
    pub fn update(&mut self, config: AppConfig) -> Result<(), std::io::Error> {
        self.config = config;
        self.save()
    }

    /// Persists the current config to disk as JSON.
    pub fn save(&self) -> Result<(), std::io::Error> {
        let json = serde_json::to_string_pretty(&self.config)?;
        if let Some(parent) = self.config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.config_path, json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static TEST_CTR: AtomicU64 = AtomicU64::new(0);
        let ctr = TEST_CTR.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("vpn_config_test_{}", ctr));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_default_config_on_first_launch() {
        let dir = temp_dir();
        let mgr = ConfigManager::new(dir.clone());
        let config = mgr.get();
        assert_eq!(config.language, "en");
        assert_eq!(config.theme, "system");
        assert!(config.last_server_id.is_none());
        assert!(config.saved_token_refs.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_save_and_load_config() {
        let dir = temp_dir();
        let mut mgr = ConfigManager::new(dir.clone());

        let updated = AppConfig {
            language: "es".to_string(),
            theme: "dark".to_string(),
            last_server_id: Some("srv-abc".to_string()),
            saved_token_refs: vec!["digitalocean".to_string()],
        };
        mgr.update(updated).unwrap();

        // Create a new manager to verify persistence
        let mgr2 = ConfigManager::new(dir.clone());
        assert_eq!(mgr2.get().language, "es");
        assert_eq!(mgr2.get().theme, "dark");
        assert_eq!(mgr2.get().last_server_id, Some("srv-abc".to_string()));
        assert_eq!(mgr2.get().saved_token_refs, vec!["digitalocean"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_corruption_recovery() {
        let dir = temp_dir();
        let config_path = dir.join("config.json");

        // Write invalid JSON
        let mut file = fs::File::create(&config_path).unwrap();
        file.write_all(b"this is not valid json").unwrap();

        let mgr = ConfigManager::new(dir.clone());

        // Should recover with defaults
        assert_eq!(mgr.get().language, "en");

        // Corrupted file should have been backed up
        assert!(config_path.with_extension("json.bak").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_save_preserves_roundtrip() {
        let dir = temp_dir();
        let mut mgr = ConfigManager::new(dir.clone());

        let original = AppConfig {
            language: "es".to_string(),
            theme: "dark".to_string(),
            last_server_id: None,
            saved_token_refs: vec![],
        };
        mgr.update(original.clone()).unwrap();

        // Read the file back directly
        let content = fs::read_to_string(dir.join("config.json")).unwrap();
        let deserialized: AppConfig = serde_json::from_str(&content).unwrap();
        assert_eq!(deserialized.language, original.language);
        assert_eq!(deserialized.theme, original.theme);
        let _ = fs::remove_dir_all(&dir);
    }
}
