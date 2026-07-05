//! Encrypted secret storage for provider API tokens.
//!
//! ## Architecture
//!
//! Tokens are stored via the OS keyring (using the `keyring` crate).
//! On platforms or environments where the system keyring is unavailable
//! (headless Linux, WSL, etc.), an AES-256-GCM encrypted file is used
//! as a transparent fallback. The encryption key is randomly generated
//! on first use and stored in a file with restricted permissions (0o600).
//!
//! ## Security Properties
//!
//! - CPA-1: Tokens are NEVER stored as plaintext on disk.
//! - NF-3: No plaintext secrets are written to disk unprotected.
//! - File permissions are set to 0o600 (owner-only) on Unix systems.

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// An encrypted payload stored on disk as JSON.
#[derive(Debug, Serialize, Deserialize)]
struct EncryptedPayload {
    nonce: String,
    ciphertext: String,
}

/// Manages encrypted storage of provider API tokens.
pub struct SecretsManager {
    secrets_path: PathBuf,
    key_path: PathBuf,
    tokens: HashMap<String, String>,
}

impl SecretsManager {
    /// Creates a new `SecretsManager` for the given data directory.
    ///
    /// Loads any previously saved tokens during construction.
    pub fn new(data_dir: PathBuf) -> Self {
        let secrets_path = data_dir.join("secrets.enc");
        let key_path = data_dir.join(".vpn_key");
        let mut mgr = Self {
            secrets_path,
            key_path,
            tokens: HashMap::new(),
        };
        let _ = mgr.load();
        mgr
    }

    /// Returns or generates a 32-byte AES-256 key.
    ///
    /// The key is stored in a file with restricted permissions (0o600).
    fn load_or_generate_key(&self) -> [u8; 32] {
        if let Ok(data) = fs::read(&self.key_path) {
            if data.len() == 32 {
                let mut key = [0u8; 32];
                key.copy_from_slice(&data);
                return key;
            }
        }
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        let _ = fs::write(&self.key_path, key);

        #[cfg(unix)]
        {
            let _ = fs::set_permissions(&self.key_path, fs::Permissions::from_mode(0o600));
        }
        key
    }

    /// Saves a token for the given provider.
    ///
    /// The token is added to the in-memory map and immediately flushed
    /// to the encrypted file. The OS keyring is also updated when available.
    pub fn save_token(&mut self, provider: &str, token: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Try OS keyring; silently continue if unavailable
        match keyring::Entry::new("wireguard-vpn", provider) {
            Ok(entry) => {
                let _ = entry.set_password(token);
            }
            Err(_) => {
                log::info!("OS keyring unavailable for provider '{}'; using encrypted file", provider);
            }
        }

        self.tokens.insert(provider.to_string(), token.to_string());
        self.flush()?;
        Ok(())
    }

    /// Retrieves a token for the given provider.
    ///
    /// Tries the OS keyring first, then falls back to the encrypted file.
    pub fn get_token(&self, provider: &str) -> Result<String, Box<dyn std::error::Error>> {
        // Try OS keyring first
        if let Ok(entry) = keyring::Entry::new("wireguard-vpn", provider) {
            if let Ok(token) = entry.get_password() {
                return Ok(token);
            }
        }

        // Fall back to encrypted file
        self.tokens
            .get(provider)
            .cloned()
            .ok_or_else(|| format!("Token for '{}' not found", provider).into())
    }

    /// Deletes the token for the given provider.
    pub fn delete_token(&mut self, provider: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Remove from OS keyring if present
        if let Ok(entry) = keyring::Entry::new("wireguard-vpn", provider) {
            let _ = entry.delete_credential();
        }

        self.tokens.remove(provider);
        self.flush()?;
        Ok(())
    }

    /// Lists all provider names that have saved tokens.
    pub fn list_tokens(&self) -> Vec<String> {
        let mut providers: Vec<String> = self.tokens.keys().cloned().collect();
        providers.sort();
        providers
    }

    /// Encrypts the in-memory token map and writes it to the secrets file.
    fn flush(&self) -> Result<(), Box<dyn std::error::Error>> {
        let key_bytes = self.load_or_generate_key();
        let key = aes_gcm::Key::<Aes256Gcm>::from_slice(&key_bytes);
        let cipher = Aes256Gcm::new(key);

        let json = serde_json::to_string(&self.tokens)?;

        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, json.as_bytes())
            .map_err(|e| format!("AES-256-GCM encryption failed: {}", e))?;

        let payload = EncryptedPayload {
            nonce: BASE64.encode(nonce_bytes),
            ciphertext: BASE64.encode(&ciphertext),
        };

        let payload_json = serde_json::to_string_pretty(&payload)?;
        if let Some(parent) = self.secrets_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.secrets_path, payload_json)?;

        #[cfg(unix)]
        {
            let _ = fs::set_permissions(&self.secrets_path, fs::Permissions::from_mode(0o600));
        }

        Ok(())
    }

    /// Loads tokens from the encrypted secrets file.
    fn load(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if !self.secrets_path.exists() {
            return Ok(());
        }

        let data = fs::read_to_string(&self.secrets_path)?;
        let payload: EncryptedPayload = serde_json::from_str(&data)?;

        let key_bytes = self.load_or_generate_key();
        let key = aes_gcm::Key::<Aes256Gcm>::from_slice(&key_bytes);
        let cipher = Aes256Gcm::new(key);

        let nonce_bytes = BASE64.decode(&payload.nonce)?;
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = BASE64.decode(&payload.ciphertext)?;

        let plaintext = cipher
            .decrypt(nonce, ciphertext.as_ref())
            .map_err(|e| format!("AES-256-GCM decryption failed: {}", e))?;

        self.tokens = serde_json::from_slice(&plaintext)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static TEST_CTR: AtomicU64 = AtomicU64::new(0);
        let ctr = TEST_CTR.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("vpn_secrets_test_{}", ctr));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_save_and_get_token() {
        let dir = temp_dir();
        let mut mgr = SecretsManager::new(dir.clone());

        mgr.save_token("digitalocean", "do_test_token_123").unwrap();
        let token = mgr.get_token("digitalocean").unwrap();
        assert_eq!(token, "do_test_token_123");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_token_not_found() {
        let dir = temp_dir();
        let mgr = SecretsManager::new(dir.clone());

        let result = mgr.get_token("nonexistent");
        assert!(result.is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_delete_token() {
        let dir = temp_dir();
        let mut mgr = SecretsManager::new(dir.clone());

        mgr.save_token("hetzner", "h_token_456").unwrap();
        assert!(mgr.get_token("hetzner").is_ok());

        mgr.delete_token("hetzner").unwrap();
        assert!(mgr.get_token("hetzner").is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_list_tokens() {
        let dir = temp_dir();
        let mut mgr = SecretsManager::new(dir.clone());

        mgr.save_token("digitalocean", "do_1").unwrap();
        mgr.save_token("oracle", "or_2").unwrap();
        mgr.save_token("hetzner", "hz_3").unwrap();

        let providers = mgr.list_tokens();
        assert_eq!(providers.len(), 3);
        assert!(providers.contains(&"digitalocean".to_string()));
        assert!(providers.contains(&"oracle".to_string()));
        assert!(providers.contains(&"hetzner".to_string()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_persistence_across_reload() {
        let dir = temp_dir();
        {
            let mut mgr = SecretsManager::new(dir.clone());
            mgr.save_token("digitalocean", "persist_me").unwrap();
        }
        // Reload from disk
        let mgr2 = SecretsManager::new(dir.clone());
        let token = mgr2.get_token("digitalocean").unwrap();
        assert_eq!(token, "persist_me");
        let _ = fs::remove_dir_all(&dir);
    }
}
