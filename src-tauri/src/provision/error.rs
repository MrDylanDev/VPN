//! Error types for the SSH provisioning subsystem.
//!
//! Provides two error enums following the same `thiserror` pattern as
//! [`crate::cloud::CloudError`]:
//!
//! - [`SshError`] — low-level SSH session errors
//! - [`ProvisionError`] — high-level orchestration errors

use crate::cloud::CloudError;

// ---------------------------------------------------------------------------
// SshError
// ---------------------------------------------------------------------------

/// Low-level errors that can occur during an SSH session.
#[derive(Debug, thiserror::Error)]
pub enum SshError {
    /// Could not establish a TCP connection to the remote host.
    #[error("SSH connection failed: {0}")]
    Connect(String),

    /// Authentication with the ephemeral keypair was rejected.
    #[error("SSH authentication failed: {0}")]
    Auth(String),

    /// The remote command exited with a non-zero status.
    #[error("Command execution failed (exit code {code}): {stderr}")]
    Exec { code: i32, stdout: String, stderr: String },

    /// The SSH session or its underlying TCP stream timed out.
    #[error("SSH operation timed out")]
    Timeout,

    /// The remote host key fingerprint does not match the stored TOFU value.
    #[error("Host key fingerprint mismatch for {ip}: expected {expected}, got {actual}")]
    HostKeyMismatch {
        ip: String,
        expected: String,
        actual: String,
    },
}

// ---------------------------------------------------------------------------
// ProvisionError
// ---------------------------------------------------------------------------

/// High-level errors that can occur during the full provision flow.
#[derive(Debug, thiserror::Error)]
pub enum ProvisionError {
    /// A cloud-provider operation failed.
    #[error("Cloud provider error: {0}")]
    Cloud(#[from] CloudError),

    /// An SSH session operation failed.
    #[error("SSH error: {0}")]
    Ssh(#[from] SshError),

    /// The overall provisioning operation timed out (5-minute limit).
    #[error("Provisioning timed out")]
    Timeout,

    /// The target VPS does not meet the OS version requirement.
    #[error("OS version mismatch: expected Ubuntu 24.04, got {0}")]
    OsMismatch(String),
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ── SshError Display ──────────────────────────────────────────────────

    #[test]
    fn ssh_error_connect_display() {
        let err = SshError::Connect("Connection refused".into());
        let msg = err.to_string();
        assert_eq!(msg, "SSH connection failed: Connection refused");
    }

    #[test]
    fn ssh_error_auth_display() {
        let err = SshError::Auth("No matching auth method".into());
        let msg = err.to_string();
        assert_eq!(msg, "SSH authentication failed: No matching auth method");
    }

    #[test]
    fn ssh_error_exec_display() {
        let err = SshError::Exec {
            code: 1,
            stdout: String::new(),
            stderr: "command not found".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("exit code 1"));
        assert!(msg.contains("command not found"));
    }

    #[test]
    fn ssh_error_timeout_display() {
        let err = SshError::Timeout;
        assert_eq!(err.to_string(), "SSH operation timed out");
    }

    #[test]
    fn ssh_error_host_key_mismatch_display() {
        let err = SshError::HostKeyMismatch {
            ip: "10.0.0.1".into(),
            expected: "abc123".into(),
            actual: "def456".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("10.0.0.1"));
        assert!(msg.contains("abc123"));
        assert!(msg.contains("def456"));
    }

    // ── ProvisionError Display ────────────────────────────────────────────

    #[test]
    fn provision_error_cloud_display() {
        let err = ProvisionError::Cloud(CloudError::Auth("bad key".into()));
        let msg = err.to_string();
        assert!(msg.contains("Cloud provider error"));
        assert!(msg.contains("bad key"));
    }

    #[test]
    fn provision_error_ssh_display() {
        let err = ProvisionError::Ssh(SshError::Timeout);
        let msg = err.to_string();
        assert!(msg.contains("SSH error"));
        assert!(msg.contains("SSH operation timed out"));
    }

    #[test]
    fn provision_error_timeout_display() {
        let err = ProvisionError::Timeout;
        assert_eq!(err.to_string(), "Provisioning timed out");
    }

    #[test]
    fn provision_error_os_mismatch_display() {
        let err = ProvisionError::OsMismatch("Ubuntu 22.04".into());
        let msg = err.to_string();
        assert!(msg.contains("OS version mismatch"));
        assert!(msg.contains("Ubuntu 22.04"));
    }

    // ── From impls (source chain) ─────────────────────────────────────────

    #[test]
    fn provision_error_from_cloud() {
        let cloud_err = CloudError::Quota("over limit".into());
        let err: ProvisionError = cloud_err.into();
        assert!(matches!(err, ProvisionError::Cloud(_)));
    }

    #[test]
    fn provision_error_from_ssh() {
        let ssh_err = SshError::Timeout;
        let err: ProvisionError = ssh_err.into();
        assert!(matches!(err, ProvisionError::Ssh(_)));
    }

    // ── SshError Debug ────────────────────────────────────────────────────

    #[test]
    fn ssh_error_debug_is_not_empty() {
        let err = SshError::Connect("oops".into());
        let debug = format!("{err:?}");
        assert!(!debug.is_empty());
        assert!(debug.contains("Connect"));
    }

    // ── ProvisionError Debug ──────────────────────────────────────────────

    #[test]
    fn provision_error_debug_is_not_empty() {
        let err = ProvisionError::Timeout;
        let debug = format!("{err:?}");
        assert!(!debug.is_empty());
    }
}
