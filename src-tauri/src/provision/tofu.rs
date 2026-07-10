//! TOFU (Trust On First Use) host key fingerprint store.
//!
//! Stores SSH host key fingerprints per IP address in a JSON file at
//! `data_dir/vpn_known_hosts.json`. Follows design decision #3:
//! per-IP JSON file, trivially testable without SSH.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::provision::error::SshError;

/// Trust-on-first-use store for SSH host key fingerprints.
///
/// Data is persisted as `{ "10.0.0.1": "SHA256:abc123...", ... }` in
/// `<data_dir>/vpn_known_hosts.json`.
///
/// All public methods serialize access through an internal [`Mutex`] to
/// prevent read-modify-write races when called concurrently.
pub struct TofuStore {
    path: PathBuf,
    lock: Mutex<()>,
}

impl TofuStore {
    /// Create a new store rooted at `data_dir`.
    ///
    /// The store file (`vpn_known_hosts.json`) will be created inside `data_dir`
    /// if it does not already exist.
    pub fn new(data_dir: PathBuf) -> Self {
        let path = data_dir.join("vpn_known_hosts.json");
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    /// Verify a fingerprint for the given `ip`.
    ///
    /// - If no fingerprint is stored for `ip`, the verification **passes**
    ///   (first-connect semantics). The caller should then call [`store`] to
    ///   persist the fingerprint.
    /// - If a fingerprint IS stored, this compares it against `fingerprint`.
    ///
    /// # Errors
    ///
    /// Returns [`SshError::HostKeyMismatch`] when the stored fingerprint differs
    /// from the provided value.
    pub fn verify(&self, ip: &str, fingerprint: &str) -> Result<(), SshError> {
        let _guard = self.lock.lock().unwrap();
        let store = self.load_store();
        if let Some(stored) = store.get(ip) {
            if stored != fingerprint {
                return Err(SshError::HostKeyMismatch {
                    ip: ip.to_string(),
                    expected: stored.clone(),
                    actual: fingerprint.to_string(),
                });
            }
        }
        Ok(())
    }

    /// Persist a fingerprint for the given `ip`.
    ///
    /// Overwrites any previously stored fingerprint for this IP.
    pub fn store(&self, ip: &str, fingerprint: &str) {
        let _guard = self.lock.lock().unwrap();
        let mut store = self.load_store();
        store.insert(ip.to_string(), fingerprint.to_string());
        self.save_store(&store);
    }

    /// Remove the stored fingerprint for `ip`, if any.
    pub fn remove(&self, ip: &str) {
        let _guard = self.lock.lock().unwrap();
        let mut store = self.load_store();
        store.remove(ip);
        self.save_store(&store);
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

impl TofuStore {
    fn store_path(&self) -> &std::path::Path {
        &self.path
    }

    fn load_store(&self) -> HashMap<String, String> {
        match fs::read_to_string(self.store_path()) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
            Err(_) => HashMap::new(),
        }
    }

    fn save_store(&self, store: &HashMap<String, String>) {
        if let Some(parent) = self.store_path().parent() {
            let _ = fs::create_dir_all(parent);
        }
        let contents = serde_json::to_string_pretty(store).unwrap_or_default();
        let _ = fs::write(self.store_path(), contents);
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Helper: create a TofuStore in a temporary directory.
    fn make_store() -> (tempfile::TempDir, TofuStore) {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = TofuStore::new(dir.path().to_path_buf());
        (dir, store)
    }

    // ── 4.2.a — First connect: no stored fingerprint → verify passes ──────

    #[test]
    fn first_connect_verify_passes() {
        let (_dir, store) = make_store();
        assert!(store.verify("10.0.0.1", "SHA256:abc123").is_ok());
    }

    // ── 4.2.b — After storing, matching fingerprint passes ───────────────

    #[test]
    fn matching_fingerprint_after_store_verifies() {
        let (_dir, store) = make_store();
        store.store("10.0.0.1", "SHA256:abc123");
        assert!(store.verify("10.0.0.1", "SHA256:abc123").is_ok());
    }

    // ── 4.2.c — Mismatched fingerprint rejects ───────────────────────────

    #[test]
    fn mismatched_fingerprint_rejected() {
        let (_dir, store) = make_store();
        store.store("10.0.0.1", "SHA256:abc123");
        let result = store.verify("10.0.0.1", "SHA256:def456");
        assert!(result.is_err());
        match result.unwrap_err() {
            SshError::HostKeyMismatch { ip, expected, actual } => {
                assert_eq!(ip, "10.0.0.1");
                assert_eq!(expected, "SHA256:abc123");
                assert_eq!(actual, "SHA256:def456");
            }
            other => panic!("expected HostKeyMismatch, got {other:?}"),
        }
    }

    // ── 4.2.d — Missing file creates new store ───────────────────────────

    #[test]
    fn missing_file_creates_empty_store() {
        let (_dir, store) = make_store();
        assert!(!store.store_path().exists());
        // First operation on a missing file should succeed
        assert!(store.verify("10.0.0.1", "key").is_ok());
    }

    // ── 4.2.e — Corrupt file is handled gracefully ───────────────────────

    #[test]
    fn corrupt_file_is_handled() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("vpn_known_hosts.json");
        let mut f = fs::File::create(&path).expect("create corrupt file");
        writeln!(f, "this is not valid json").expect("write garbage");

        let store = TofuStore::new(dir.path().to_path_buf());
        // Should not panic — reads as empty store
        assert!(store.verify("10.0.0.1", "key").is_ok());

        // Store should overwrite the corrupt file with valid JSON
        store.store("10.0.0.1", "key");
        let contents = fs::read_to_string(&path).expect("read back");
        assert!(contents.contains("10.0.0.1"));
    }

    // ── 4.2.f — Remove deletes only the specified IP entry ───────────────

    #[test]
    fn remove_deletes_only_specified_ip() {
        let (_dir, store) = make_store();
        store.store("10.0.0.1", "key-a");
        store.store("10.0.0.2", "key-b");
        store.remove("10.0.0.1");

        assert!(store.verify("10.0.0.2", "key-b").is_ok());
        // After removing 10.0.0.1, it should behave like first-connect
        assert!(store.verify("10.0.0.1", "key-c").is_ok());
    }

    // ── 4.2.g — Persistence across store instances ───────────────────────

    #[test]
    fn store_persists_across_instances() {
        let dir = tempfile::tempdir().expect("temp dir");

        // First instance: store a fingerprint
        let store_a = TofuStore::new(dir.path().to_path_buf());
        store_a.store("10.0.0.1", "SHA256:persisted");

        // Drop store_a, create store_b — should read same file
        drop(store_a);
        let store_b = TofuStore::new(dir.path().to_path_buf());
        assert!(store_b.verify("10.0.0.1", "SHA256:persisted").is_ok());
    }
}
