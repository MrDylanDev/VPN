//! WireGuard tunnel lifecycle management.
//!
//! Probes for system WireGuard (`wg-quick`) and falls back to embedded
//! `wireguard-go` if unavailable. Manages up/down/status of the tunnel.

/// Current tunnel state as reported by the engine.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub enum TunnelStatus {
    Down,
    Up { handshake_ms: u64, bytes_sent: u64, bytes_received: u64 },
}

/// Common interface for system and embedded tunnel engines.
pub trait TunnelEngine {
    fn up(&self, config_path: &str) -> Result<(), Box<dyn std::error::Error>>;
    fn down(&self) -> Result<(), Box<dyn std::error::Error>>;
    fn status(&self) -> Result<TunnelStatus, Box<dyn std::error::Error>>;
}
