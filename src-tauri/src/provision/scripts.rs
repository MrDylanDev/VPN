//! Embedded bash scripts for WireGuard server provisioning.
//!
//! All scripts are embedded at compile time via `include_str!()` and target
//! Ubuntu 24.04. Each script uses `set -euo pipefail` and checks
//! `/etc/os-release` before proceeding.

/// Install WireGuard and generate server keypair.
pub const INSTALL_WIREGUARD: &str = include_str!("scripts/install-wireguard.sh");

/// Configure UFW to allow WireGuard traffic on port 51820/udp.
pub const CONFIGURE_FIREWALL: &str = include_str!("scripts/configure-firewall.sh");

/// Enable IPv4 and IPv6 forwarding via sysctl.
pub const CONFIGURE_SYSCTL: &str = include_str!("scripts/configure-sysctl.sh");

/// Set DNS resolvers to 1.1.1.1 and 1.0.0.1.
pub const CONFIGURE_DNS: &str = include_str!("scripts/configure-dns.sh");

/// All provisioning scripts in execution order.
pub const ALL_SCRIPTS: &[&str] = &[
    INSTALL_WIREGUARD,
    CONFIGURE_FIREWALL,
    CONFIGURE_SYSCTL,
    CONFIGURE_DNS,
];

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_wireguard_is_valid_utf8() {
        assert!(std::str::from_utf8(INSTALL_WIREGUARD.as_bytes()).is_ok());
    }

    #[test]
    fn configure_firewall_is_valid_utf8() {
        assert!(std::str::from_utf8(CONFIGURE_FIREWALL.as_bytes()).is_ok());
    }

    #[test]
    fn configure_sysctl_is_valid_utf8() {
        assert!(std::str::from_utf8(CONFIGURE_SYSCTL.as_bytes()).is_ok());
    }

    #[test]
    fn configure_dns_is_valid_utf8() {
        assert!(std::str::from_utf8(CONFIGURE_DNS.as_bytes()).is_ok());
    }

    #[test]
    fn all_scripts_non_empty() {
        for (i, script) in ALL_SCRIPTS.iter().enumerate() {
            assert!(!script.is_empty(), "script at index {i} is empty");
        }
    }

    #[test]
    fn all_scripts_start_with_shebang() {
        for (i, script) in ALL_SCRIPTS.iter().enumerate() {
            assert!(
                script.starts_with("#!/usr/bin/env bash"),
                "script at index {i} does not start with shebang"
            );
        }
    }

    #[test]
    fn all_scripts_have_euo_pipefail() {
        for (i, script) in ALL_SCRIPTS.iter().enumerate() {
            assert!(
                script.contains("set -euo pipefail"),
                "script at index {i} is missing 'set -euo pipefail'"
            );
        }
    }
}
