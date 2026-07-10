//! SSH session management for server provisioning.
//!
//! Wraps sync `ssh2` inside `tokio::task::spawn_blocking` to stay non-blocking
//! in the Tauri async runtime. Follows design decision #1: spawn_blocking
//! wrapper around ssh2.
//!
//! # Key management
//!
//! Ephemeral ed25519 keypairs are generated in memory using `ed25519-dalek` and
//! converted to PEM / OpenSSH wire format for `ssh2` authentication. The key
//! material is never written to disk.

use std::io::Read;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use base64::Engine;
use ed25519_dalek::{SigningKey, VerifyingKey, SECRET_KEY_LENGTH};
use rand::Rng;
use ssh2::Session;

use crate::provision::error::SshError;
use crate::provision::tofu::TofuStore;

/// An SSH session backed by an ephemeral ed25519 keypair.
///
/// All I/O is synchronous (ssh2). Callers should use
/// `tokio::task::spawn_blocking` to run SSH operations without blocking the
/// async runtime.
pub struct SshSession {
    seed: [u8; SECRET_KEY_LENGTH],
    session: Option<Session>,
    verifying_key: Option<VerifyingKey>,
}

impl SshSession {
    /// Create a new session and generate an ephemeral ed25519 keypair.
    ///
    /// Returns the session handle and the signing key so the caller can extract
    /// the public key for `PeerConfig` generation.
    pub fn with_keypair() -> (Self, SigningKey) {
        let mut seed_bytes = [0u8; SECRET_KEY_LENGTH];
        let mut rng = rand::thread_rng();
        rng.fill(&mut seed_bytes);
        let signing_key = SigningKey::from_bytes(&seed_bytes);
        let verifying_key = signing_key.verifying_key();

        let session = Self {
            seed: signing_key.to_bytes(),
            session: None,
            verifying_key: Some(verifying_key),
        };
        (session, signing_key)
    }

    /// Connect to a remote host via SSH.
    ///
    /// 1. Opens a TCP connection to `addr:port`
    /// 2. Performs the SSH handshake
    /// 3. Verifies the host key fingerprint against the [`TofuStore`]
    /// 4. Authenticates with the ephemeral ed25519 keypair as `root`
    pub fn connect(
        &mut self,
        addr: &str,
        port: u16,
        tcp_timeout: Duration,
        tofu: &TofuStore,
    ) -> Result<(), SshError> {
        let socket_addr = format!("{addr}:{port}")
            .to_socket_addrs()
            .map_err(|e| SshError::Connect(format!("Invalid address: {e}")))?
            .next()
            .ok_or_else(|| SshError::Connect("No address resolved".into()))?;

        let tcp = TcpStream::connect_timeout(&socket_addr, tcp_timeout)
            .map_err(|e| SshError::Connect(format!("TCP connection failed: {e}")))?;

        tcp.set_read_timeout(Some(tcp_timeout))
            .map_err(|e| SshError::Connect(format!("set_read_timeout failed: {e}")))?;
        tcp.set_write_timeout(Some(tcp_timeout))
            .map_err(|e| SshError::Connect(format!("set_write_timeout failed: {e}")))?;

        let mut session = Session::new()
            .map_err(|e| SshError::Connect(format!("Session creation failed: {e}")))?;

        session.set_tcp_stream(tcp);
        session
            .handshake()
            .map_err(|e| SshError::Connect(format!("Handshake failed: {e}")))?;

        // ---- TOFU host key verification (task 2.2) ----
        let fingerprint = session
            .host_key_hash(ssh2::HashType::Sha256)
            .ok_or_else(|| SshError::Connect("Failed to get host key hash".into()))?;
        let fingerprint_hex: String = fingerprint.iter().map(|b| format!("{b:02x}")).collect();
        let fingerprint_prefixed = format!("SHA256:{fingerprint_hex}");

        tofu.verify(addr, &fingerprint_prefixed)?;
        tofu.store(addr, &fingerprint_prefixed);

        // ---- Authenticate with ephemeral ed25519 key ----
        let privkey_pem = private_key_to_pem(&self.seed);
        let pubkey_openssh = encode_openssh_pubkey(
            self.verifying_key
                .as_ref()
                .ok_or_else(|| SshError::Auth("No verifying key available".into()))?,
        );

        session
            .userauth_pubkey_memory("root", Some(&pubkey_openssh), &privkey_pem, None)
            .map_err(|e| SshError::Auth(format!("Authentication failed: {e}")))?;

        if !session.authenticated() {
            return Err(SshError::Auth("Session not authenticated after auth".into()));
        }

        self.session = Some(session);
        Ok(())
    }

    /// Execute a shell command (or script) on the remote host.
    ///
    /// Returns `(stdout, stderr)` on success.
    ///
    /// # Errors
    ///
    /// Returns [`SshError::Exec`] if the command exits with a non-zero status.
    /// Returns [`SshError::Connect`] for transport errors.
    pub fn execute(&mut self, script: &str) -> Result<(String, String), SshError> {
        let session = self
            .session
            .as_ref()
            .ok_or_else(|| SshError::Connect("Not connected".into()))?;

        let mut channel = session
            .channel_session()
            .map_err(|e| SshError::Connect(format!("Channel open failed: {e}")))?;

        channel
            .exec(script)
            .map_err(|e| SshError::Connect(format!("Exec failed: {e}")))?;

        let mut stdout = String::new();
        let mut stderr = String::new();

        channel
            .read_to_string(&mut stdout)
            .map_err(|e| SshError::Connect(format!("Read stdout failed: {e}")))?;

        {
            let mut stderr_stream = channel.stderr();
            stderr_stream
                .read_to_string(&mut stderr)
                .map_err(|e| SshError::Connect(format!("Read stderr failed: {e}")))?;
        }

        channel
            .wait_close()
            .map_err(|e| SshError::Connect(format!("Wait close failed: {e}")))?;

        let exit_code = channel
            .exit_status()
            .map_err(|e| SshError::Connect(format!("Exit status failed: {e}")))?;

        if exit_code == 0 {
            Ok((stdout, stderr))
        } else {
            Err(SshError::Exec {
                code: exit_code,
                stdout,
                stderr,
            })
        }
    }

    /// Close the SSH session and discard the ephemeral key material.
    pub fn disconnect(&mut self) {
        if let Some(session) = self.session.take() {
            let _ = session.disconnect(None, "disconnect", None);
        }
        self.verifying_key = None;
    }

    /// Return a reference to the verifying key, if still available.
    pub fn verifying_key(&self) -> Option<&VerifyingKey> {
        self.verifying_key.as_ref()
    }
}

// ---------------------------------------------------------------------------
// PEM / OpenSSH key serialization
// ---------------------------------------------------------------------------

/// Encode a raw ed25519 seed (32 bytes) as a PKCS#8 PEM string.
///
/// The DER structure is:
/// ```ignore
/// SEQUENCE {
///   INTEGER 0                       -- version
///   SEQUENCE { OID 1.3.101.112 }    -- id-EdDSA25519
///   OCTET STRING {
///     OCTET STRING (32 bytes)       -- private key seed
///   }
/// }
/// ```
fn private_key_to_pem(seed: &[u8; 32]) -> String {
    let mut der = Vec::with_capacity(46);
    der.push(0x30); // SEQUENCE tag
    der.push(0x2E); // length 46
    der.push(0x02); // INTEGER tag
    der.push(0x01); // length 1
    der.push(0x00); // value 0
    der.push(0x30); // SEQUENCE tag
    der.push(0x05); // length 5
    der.push(0x06); // OID tag
    der.push(0x03); // length 3
    der.push(0x2B); // 1.3.101.112 (ed25519)
    der.push(0x65);
    der.push(0x70);
    der.push(0x04); // OCTET STRING tag
    der.push(0x22); // length 34
    der.push(0x04); // OCTET STRING tag
    der.push(0x20); // length 32
    der.extend_from_slice(seed);

    let b64 = base64::engine::general_purpose::STANDARD.encode(&der);
    let mut pem = String::from("-----BEGIN PRIVATE KEY-----\n");
    for chunk in b64.as_bytes().chunks(64) {
        pem.push_str(std::str::from_utf8(chunk).unwrap());
        pem.push('\n');
    }
    pem.push_str("-----END PRIVATE KEY-----\n");
    pem
}

/// Encode an ed25519 verifying key as an OpenSSH-format public key string.
///
/// Produces: `ssh-ed25519 <base64>`
fn encode_openssh_pubkey(verifying_key: &VerifyingKey) -> String {
    let algo = b"ssh-ed25519";
    let key_bytes = verifying_key.to_bytes();

    let mut wire = Vec::with_capacity(4 + algo.len() + 4 + key_bytes.len());
    wire.extend_from_slice(&(algo.len() as u32).to_be_bytes());
    wire.extend_from_slice(algo);
    wire.extend_from_slice(&(key_bytes.len() as u32).to_be_bytes());
    wire.extend_from_slice(&key_bytes);

    let b64 = base64::engine::general_purpose::STANDARD.encode(&wire);
    format!("ssh-ed25519 {b64}")
}

/// Return a hex-encoded SHA-256 fingerprint of an ed25519 public key.
pub fn fingerprint_pubkey(verifying_key: &VerifyingKey) -> String {
    use sha2::Digest;
    let key_bytes = verifying_key.to_bytes();
    let hash = sha2::Sha256::digest(&key_bytes);
    let hex_str: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    format!("SHA256:{hex_str}")
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ── PEM encoding ──────────────────────────────────────────────────────

    #[test]
    fn private_key_to_pem_produces_valid_pem() {
        let seed = [0xABu8; 32];
        let pem = private_key_to_pem(&seed);

        assert!(pem.starts_with("-----BEGIN PRIVATE KEY-----"));
        assert!(pem.ends_with("-----END PRIVATE KEY-----\n"));
    }

    #[test]
    fn private_key_to_pem_is_deterministic() {
        let seed = [0x42u8; 32];
        let pem1 = private_key_to_pem(&seed);
        let pem2 = private_key_to_pem(&seed);
        assert_eq!(pem1, pem2);
    }

    #[test]
    fn private_key_to_pem_different_seed_different_output() {
        let seed_a = [0xAAu8; 32];
        let seed_b = [0xBBu8; 32];
        let pem_a = private_key_to_pem(&seed_a);
        let pem_b = private_key_to_pem(&seed_b);
        assert_ne!(pem_a, pem_b);
    }

    #[test]
    fn private_key_to_pem_base64_decodes_correctly() {
        let seed = [0xABu8; 32];
        let pem = private_key_to_pem(&seed);

        // Extract base64 content between PEM headers
        let body: String = pem
            .lines()
            .filter(|l| !l.starts_with("-----"))
            .collect();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&body)
            .expect("valid base64");

        // DER should start with SEQUENCE tag 0x30
        assert_eq!(decoded[0], 0x30);
        // DER should end with the seed
        assert_eq!(&decoded[decoded.len() - 32..], &seed[..]);
    }

    // ── OpenSSH public key encoding ───────────────────────────────────────

    #[test]
    fn openssh_pubkey_starts_with_ssh_ed25519() {
        let seed = [0x42u8; 32];
        let signing_key = SigningKey::from_bytes(&seed);
        let vk = signing_key.verifying_key();

        let encoded = encode_openssh_pubkey(&vk);
        assert!(encoded.starts_with("ssh-ed25519 "));
    }

    #[test]
    fn openssh_pubkey_base64_decodes_to_valid_wire_format() {
        let seed = [0x42u8; 32];
        let signing_key = SigningKey::from_bytes(&seed);
        let vk = signing_key.verifying_key();

        let encoded = encode_openssh_pubkey(&vk);
        let b64 = encoded.strip_prefix("ssh-ed25519 ").unwrap();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .expect("valid base64");

        // Wire format: len(4) + "ssh-ed25519"(11) + len(4) + key(32)
        assert!(decoded.len() >= 4 + 11 + 4 + 32);

        let algo_len = u32::from_be_bytes([decoded[0], decoded[1], decoded[2], decoded[3]]) as usize;
        assert_eq!(algo_len, 11);
        assert_eq!(&decoded[4..4 + 11], b"ssh-ed25519");

        let key_len_start = 4 + 11;
        let key_len = u32::from_be_bytes([
            decoded[key_len_start],
            decoded[key_len_start + 1],
            decoded[key_len_start + 2],
            decoded[key_len_start + 3],
        ]) as usize;
        assert_eq!(key_len, 32);
        assert_eq!(&decoded[key_len_start + 4..], vk.to_bytes().as_slice());
    }

    // ── SshSession::with_keypair ──────────────────────────────────────────

    #[test]
    fn with_keypair_creates_valid_signing_key() {
        let (session, signing_key) = SshSession::with_keypair();
        assert!(session.verifying_key.is_some());
        let vk = signing_key.verifying_key();
        assert_eq!(
            SigningKey::from_bytes(&signing_key.to_bytes()).verifying_key(),
            vk,
        );
    }

    #[test]
    fn with_keypair_generates_unique_keys() {
        let (_, k1) = SshSession::with_keypair();
        let (_, k2) = SshSession::with_keypair();
        assert_ne!(k1.to_bytes(), k2.to_bytes());
    }

    // ── disconnect ────────────────────────────────────────────────────────

    #[test]
    fn disconnect_clears_session_and_key() {
        let (mut session, _) = SshSession::with_keypair();
        // Even with no active SSH session, disconnect should not panic
        session.disconnect();
        assert!(session.session.is_none());
    }

    // ── Error cases: execute without connect ──────────────────────────────

    #[test]
    fn execute_without_connect_returns_error() {
        let (mut session, _) = SshSession::with_keypair();
        let result = session.execute("echo hello");
        assert!(result.is_err());
        match result.unwrap_err() {
            SshError::Connect(msg) => assert!(msg.contains("Not connected")),
            other => panic!("expected Connect error, got {other:?}"),
        }
    }

    // ── fingerprint_pubkey ────────────────────────────────────────────────

    #[test]
    fn fingerprint_pubkey_returns_sha256_prefixed() {
        let seed = [0x42u8; 32];
        let signing_key = SigningKey::from_bytes(&seed);
        let vk = signing_key.verifying_key();
        let fp = fingerprint_pubkey(&vk);
        assert!(fp.starts_with("SHA256:"));
        let hex_part = fp.strip_prefix("SHA256:").unwrap();
        assert_eq!(hex_part.len(), 64);
        assert!(hex_part.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn fingerprint_pubkey_is_deterministic() {
        let seed = [0x42u8; 32];
        let signing_key = SigningKey::from_bytes(&seed);
        let vk = signing_key.verifying_key();
        assert_eq!(fingerprint_pubkey(&vk), fingerprint_pubkey(&vk));
    }
}
