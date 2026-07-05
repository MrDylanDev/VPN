//! Cloud provider abstraction for provisioning and destroying VPS instances.
//!
//! This module defines the `CloudProvider` trait and provider-specific
//! implementations for DigitalOcean, Hetzner, and Oracle Cloud.

/// Represents errors that can occur during cloud provider operations.
#[derive(Debug, thiserror::Error)]
pub enum CloudError {
    #[error("Authentication failed: {0}")]
    Auth(String),

    #[error("Rate limited: retry after {0}s")]
    RateLimit(u64),

    #[error("Quota exceeded: {0}")]
    Quota(String),

    #[error("Request timed out")]
    Timeout,

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Provider error: {0}")]
    Provider(String),
}

/// A VPS instance descriptor returned after successful provisioning.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VpsInstance {
    pub id: String,
    pub provider: String,
    pub region: String,
    pub ip: String,
    pub status: String,
}

/// Parameters for creating a new VPS.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProvisionParams {
    pub provider: String,
    pub region: String,
    pub size: String,
    pub image: String,
}

impl Default for ProvisionParams {
    fn default() -> Self {
        Self {
            provider: "digitalocean".to_string(),
            region: "fra1".to_string(),
            size: "s-1vcpu-1gb".to_string(),
            image: "ubuntu-24-04-x64".to_string(),
        }
    }
}

/// Common interface for all cloud providers.
pub trait CloudProvider {
    /// Validate an API token against the provider.
    fn validate_token(&self, token: &str) -> Result<bool, CloudError>;

    /// Create a new VPS instance.
    fn create_vps(&self, params: &ProvisionParams, token: &str) -> Result<VpsInstance, CloudError>;

    /// Destroy an existing VPS instance by ID.
    fn destroy_vps(&self, instance_id: &str, token: &str) -> Result<(), CloudError>;

    /// List all VPS instances for this provider account.
    fn list_vpss(&self, token: &str) -> Result<Vec<VpsInstance>, CloudError>;
}
