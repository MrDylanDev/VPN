//! Server provisioning orchestration.
//!
//! Handles the end-to-end flow of creating a VPS, installing WireGuard, and
//! generating peer configuration. Manages rollback on failure.

/// The peer configuration returned after successful provisioning.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PeerConfig {
    pub endpoint: String,
    pub server_public_key: String,
    pub client_private_key: String,
    pub client_public_key: String,
    pub allowed_ips: String,
    pub dns: String,
}

impl Default for PeerConfig {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            server_public_key: String::new(),
            client_private_key: String::new(),
            client_public_key: String::new(),
            allowed_ips: "0.0.0.0/0, ::/0".to_string(),
            dns: "1.1.1.1, 1.0.0.1".to_string(),
        }
    }
}
