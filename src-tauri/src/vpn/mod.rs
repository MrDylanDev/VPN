//! VPN connection state machine.
//!
//! Defines the connection lifecycle: Disconnected → Connecting → Connected → Disconnecting → Disconnected.
//! Emits events to the frontend via Tauri's event system.

/// Valid states in the VPN connection state machine.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub enum VpnState {
    Disconnected,
    Connecting,
    Connected,
    Disconnecting,
}

impl VpnState {
    /// Returns true if transitioning from `self` to `next` is valid.
    pub fn can_transition_to(&self, next: &VpnState) -> bool {
        matches!(
            (self, next),
            (VpnState::Disconnected, VpnState::Connecting)
                | (VpnState::Connecting, VpnState::Connected)
                | (VpnState::Connecting, VpnState::Disconnected)
                | (VpnState::Connected, VpnState::Disconnecting)
                | (VpnState::Disconnecting, VpnState::Disconnected)
        )
    }
}
