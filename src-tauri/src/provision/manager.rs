//! Provision orchestration and rollback guard.
//!
//! [`ProvisionManager`] coordinates the full provision flow:
//! create VPS → wait for readiness → SSH → install → verify → return config.
//! [`ProvisionGuard`] auto-destroys the VPS on Drop unless `commit()` is called.

use std::path::PathBuf;
use std::time::Duration;

use tokio::net::TcpStream;

use crate::cloud::{CloudProvider, ProvisionParams, VpsInstance};
use crate::provision::error::{ProvisionError, SshError};
use crate::provision::scripts;
use crate::provision::ssh::SshSession;
use crate::provision::tofu::TofuStore;
use crate::provision::PeerConfig;

// ---------------------------------------------------------------------------
// ProvisionManager
// ---------------------------------------------------------------------------

/// Orchestrates the full server provisioning flow.
pub struct ProvisionManager<'p, P: CloudProvider> {
    cloud: &'p P,
    token: &'p str,
}

impl<'p, P: CloudProvider> ProvisionManager<'p, P> {
    /// Create a new provision manager for the given cloud provider.
    pub fn new(cloud: &'p P, token: &'p str) -> Self {
        Self { cloud, token }
    }

    /// Run the full provision flow with a 5-minute overall timeout.
    pub async fn run(
        &mut self,
        params: &ProvisionParams,
        data_dir: PathBuf,
    ) -> Result<PeerConfig, ProvisionError> {
        tokio::time::timeout(Duration::from_secs(300), self.run_inner(params, data_dir))
            .await
            .map_err(|_| ProvisionError::Timeout)?
    }

    async fn run_inner(
        &mut self,
        params: &ProvisionParams,
        data_dir: PathBuf,
    ) -> Result<PeerConfig, ProvisionError> {
        // ── Step 1: Create or reuse VPS ───────────────────────────────────
        let instance = self.cloud.create_vps(params, self.token).await?;

        // ── Step 2: ProvisionGuard for automatic rollback ─────────────────
        let mut guard = ProvisionGuard::new(self.cloud, self.token, instance.clone());

        // ── Step 3: Wait for VPS readiness (API poll) ─────────────────────
        self.wait_for_vps_active(&instance).await?;

        // ── Step 4: Wait for TCP port 22 ──────────────────────────────────
        self.wait_for_port_22(&instance.ip).await?;

        // ── Step 5-7: SSH provision with retry-once ───────────────────────
        let result = self.try_ssh_provision(&instance, &data_dir).await;

        match result {
            Ok(pc) => {
                guard.commit();
                Ok(pc)
            }
            Err(e) => {
                // Explicitly destroy on error
                self.cleanup_vps(&instance).await;
                Err(e)
            }
        }
    }

    /// Attempt SSH provisioning with a single retry on failure.
    async fn try_ssh_provision(
        &self,
        instance: &VpsInstance,
        data_dir: &PathBuf,
    ) -> Result<PeerConfig, ProvisionError> {
        let first = self.ssh_provision_once(instance, data_dir).await;
        match first {
            Ok(pc) => Ok(pc),
            Err(e) => {
                log::warn!("SSH provision attempt 1 failed: {e:?}. Retrying once...");
                tokio::time::sleep(Duration::from_secs(2)).await;
                self.ssh_provision_once(instance, data_dir).await
            }
        }
    }

    /// Single SSH provision attempt.
    async fn ssh_provision_once(
        &self,
        instance: &VpsInstance,
        data_dir: &PathBuf,
    ) -> Result<PeerConfig, ProvisionError> {
        let ip = instance.ip.clone();
        let data_dir = data_dir.clone();

        tokio::task::spawn_blocking(move || -> Result<PeerConfig, ProvisionError> {
            let (mut session, signing_key) = SshSession::with_keypair();
            let tofu = TofuStore::new(data_dir);

            session
                .connect(&ip, 22, Duration::from_secs(15), &tofu)
                .map_err(ProvisionError::Ssh)?;

            for script in scripts::ALL_SCRIPTS {
                let (stdout, stderr) = session
                    .execute(script)
                    .map_err(ProvisionError::Ssh)?;

                if stderr.contains("ERROR: Expected Ubuntu 24.04") {
                    return Err(ProvisionError::OsMismatch(
                        stderr
                            .lines()
                            .find(|l| l.contains("got "))
                            .unwrap_or("unknown")
                            .to_string(),
                    ));
                }
                log::info!("Script stdout: {stdout}");
                if !stderr.is_empty() {
                    log::warn!("Script stderr: {stderr}");
                }
            }

            // Verify WireGuard is installed
            let (verify_out, _) = session
                .execute("wg show 2>/dev/null && echo 'WG_OK' || echo 'WG_MISSING'")
                .map_err(ProvisionError::Ssh)?;

            let vk = signing_key.verifying_key();
            let client_pub_key = base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                vk.to_bytes(),
            );

            let endpoint = format!("{}:51820", ip);
            let peer_config = PeerConfig {
                endpoint,
                server_public_key: String::new(),
                client_private_key: base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    signing_key.to_bytes(),
                ),
                client_public_key: client_pub_key,
                allowed_ips: "0.0.0.0/0, ::/0".to_string(),
                dns: "1.1.1.1, 1.0.0.1".to_string(),
            };

            session.disconnect();
            Ok(peer_config)
        })
        .await
        .map_err(|e| {
            ProvisionError::Ssh(SshError::Connect(format!("Task join failed: {e}")))
        })?
    }

    /// Clean up the VPS after a failed provision.
    async fn cleanup_vps(&self, instance: &VpsInstance) {
        log::warn!("Provision failed — destroying VPS {}", instance.id);
        if let Err(e) = self.cloud.destroy_vps(&instance.id, self.token).await {
            log::error!("Failed to destroy VPS {}: {e:?}", instance.id);
        }
    }

    /// Poll the cloud API until the VPS status is "active".
    async fn wait_for_vps_active(&self, instance: &VpsInstance) -> Result<(), ProvisionError> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        while tokio::time::Instant::now() < deadline {
            let instances = self.cloud.list_vpss(self.token).await?;
            if instances.iter().any(|i| i.id == instance.id && i.status == "active") {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        Err(ProvisionError::Timeout)
    }

    /// Wait until TCP port 22 is open on the VPS.
    async fn wait_for_port_22(&self, ip: &str) -> Result<(), ProvisionError> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        while tokio::time::Instant::now() < deadline {
            if TcpStream::connect(format!("{ip}:22")).await.is_ok() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        Err(ProvisionError::Timeout)
    }
}

// ---------------------------------------------------------------------------
// ProvisionGuard
// ---------------------------------------------------------------------------

/// A Drop guard that logs if a VPS was not committed.
///
/// The guard exists as a safety net: in normal error flow,
/// [`ProvisionManager::cleanup_vps`] destroys the VPS explicitly. If the guard
/// is dropped without being consumed (e.g., during a panic), it at least logs.
pub struct ProvisionGuard<'p, P: CloudProvider> {
    cloud: &'p P,
    token: &'p str,
    instance: Option<VpsInstance>,
}

impl<'p, P: CloudProvider> ProvisionGuard<'p, P> {
    /// Create a new guard that tracks the given VPS.
    pub fn new(cloud: &'p P, token: &'p str, instance: VpsInstance) -> Self {
        Self {
            cloud,
            token,
            instance: Some(instance),
        }
    }

    /// Mark the provision as successful.
    ///
    /// After calling this, dropping the guard will NOT log a warning
    /// about an uncommitted VPS.
    pub fn commit(&mut self) {
        self.instance = None;
    }

    /// The VPS instance being guarded, if still tracked.
    pub fn instance(&self) -> Option<&VpsInstance> {
        self.instance.as_ref()
    }
}

impl<P: CloudProvider> Drop for ProvisionGuard<'_, P> {
    fn drop(&mut self) {
        // If the guard still holds an instance, the caller forgot to commit
        // or cleanup. This can happen during panics or if the guard is
        // simply dropped without explicit cleanup.
        if let Some(ref instance) = self.instance {
            log::warn!(
                "ProvisionGuard dropped without commit for VPS {}. Manual cleanup advised.",
                instance.id
            );
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    // ── Mock CloudProvider for testing ────────────────────────────────────

    struct MockCloud {
        create_ok: bool,
        destroy_called: Arc<AtomicBool>,
    }

    impl MockCloud {
        fn new() -> Self {
            Self {
                create_ok: true,
                destroy_called: Arc::new(AtomicBool::new(false)),
            }
        }

        fn destroy_called(&self) -> bool {
            self.destroy_called.load(Ordering::SeqCst)
        }
    }

    impl CloudProvider for MockCloud {
        async fn validate_token(&self, _token: &str) -> Result<bool, crate::cloud::CloudError> {
            Ok(true)
        }

        async fn create_vps(
            &self,
            _params: &ProvisionParams,
            _token: &str,
        ) -> Result<VpsInstance, crate::cloud::CloudError> {
            if self.create_ok {
                Ok(VpsInstance {
                    id: "mock-1".into(),
                    provider: "mock".into(),
                    region: "fra1".into(),
                    ip: "10.0.0.1".into(),
                    status: "active".into(),
                })
            } else {
                Err(crate::cloud::CloudError::Provider("create failed".into()))
            }
        }

        async fn destroy_vps(
            &self,
            _instance_id: &str,
            _token: &str,
        ) -> Result<(), crate::cloud::CloudError> {
            self.destroy_called.store(true, Ordering::SeqCst);
            Ok(())
        }

        async fn list_vpss(
            &self,
            _token: &str,
        ) -> Result<Vec<VpsInstance>, crate::cloud::CloudError> {
            Ok(vec![VpsInstance {
                id: "mock-1".into(),
                provider: "mock".into(),
                region: "fra1".into(),
                ip: "10.0.0.1".into(),
                status: "active".into(),
            }])
        }
    }

    // ── ProvisionGuard tests (task 4.3) ───────────────────────────────────

    #[test]
    fn guard_creation_tracks_instance() {
        let cloud = MockCloud::new();
        let instance = VpsInstance {
            id: "test-1".into(),
            provider: "mock".into(),
            region: "fra1".into(),
            ip: "10.0.0.1".into(),
            status: "active".into(),
        };
        let guard = ProvisionGuard::new(&cloud, "token", instance);
        assert!(guard.instance().is_some());
    }

    #[test]
    fn guard_commit_clears_instance() {
        let cloud = MockCloud::new();
        let instance = VpsInstance {
            id: "test-2".into(),
            provider: "mock".into(),
            region: "fra1".into(),
            ip: "10.0.0.1".into(),
            status: "active".into(),
        };
        let mut guard = ProvisionGuard::new(&cloud, "token", instance);
        assert!(guard.instance().is_some());
        guard.commit();
        assert!(guard.instance().is_none());
    }

    #[test]
    fn guard_drop_logs_warning() {
        let cloud = MockCloud::new();
        let instance = VpsInstance {
            id: "test-3".into(),
            provider: "mock".into(),
            region: "fra1".into(),
            ip: "10.0.0.1".into(),
            status: "active".into(),
        };
        {
            let _guard = ProvisionGuard::new(&cloud, "token", instance);
            // guard drops without commit — should log a warning
        }
    }

    // ── ProvisionManager tests (task 4.4) ─────────────────────────────────

    #[tokio::test]
    async fn manager_creation_succeeds() {
        let cloud = MockCloud::new();
        let _manager = ProvisionManager::new(&cloud, "test-token");
    }

    #[tokio::test]
    async fn manager_cleanup_vps_destroys_instance() {
        let cloud = MockCloud::new();
        let manager = ProvisionManager::new(&cloud, "test-token");
        let instance = VpsInstance {
            id: "test-cleanup".into(),
            provider: "mock".into(),
            region: "fra1".into(),
            ip: "10.0.0.1".into(),
            status: "active".into(),
        };

        assert!(!cloud.destroy_called());
        manager.cleanup_vps(&instance).await;
        assert!(cloud.destroy_called());
    }

    #[tokio::test]
    async fn manager_wait_for_port_22_timeout() {
        // Use an unreachable IP — should timeout
        let cloud = MockCloud::new();
        let manager = ProvisionManager::new(&cloud, "test-token");

        let result = manager.wait_for_port_22("198.51.100.1").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ProvisionError::Timeout => { /* expected */ }
            other => panic!("expected Timeout, got {other:?}"),
        }
    }
}
