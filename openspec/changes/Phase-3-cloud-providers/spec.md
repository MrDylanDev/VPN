# Cloud Provider Integrations — Phase 3

## Purpose

Implement the `CloudProvider` trait for DigitalOcean, Hetzner, and Oracle Cloud to automate VPS provisioning from the desktop app, with an exponential backoff retry middleware for API resilience.

---

## ADDED Requirements

### Requirement: DigitalOcean Provider

The system MUST implement `CloudProvider` against the DO Droplets API (`https://api.digitalocean.com/v2/`). Auth uses `Authorization: Bearer {token}`.

| Operation | HTTP | Endpoint |
|-----------|------|----------|
| `validate_token` | GET | `/v2/account` |
| `create_vps` | POST | `/v2/droplets` |
| `destroy_vps` | DELETE | `/v2/droplets/{id}` |
| `list_vpss` | GET | `/v2/droplets` |

#### Scenarios

- **Valid token**: GIVEN a valid DO token WHEN `validate_token` is called THEN returns `Ok(true)`
- **Invalid token**: GIVEN an invalid DO token WHEN `validate_token` is called THEN returns `Err(CloudError::Auth(...))`
- **Create droplet**: GIVEN valid `ProvisionParams` WHEN `create_vps` is called THEN returns `VpsInstance` with `provider: "digitalocean"` and a non-empty `ip`
- **Destroy droplet**: GIVEN an existing droplet ID WHEN `destroy_vps` is called THEN returns `Ok(())`
- **List droplets**: GIVEN a valid token WHEN `list_vpss` is called THEN returns `Vec<VpsInstance>`

### Requirement: Hetzner Provider

The system MUST implement `CloudProvider` against the Hetzner Cloud API (`https://api.hetzner.cloud/v1/`). Auth uses `Authorization: Bearer {token}`.

| Operation | HTTP | Endpoint |
|-----------|------|----------|
| `validate_token` | GET | `/v1/datacenters` |
| `create_vps` | POST | `/v1/servers` |
| `destroy_vps` | DELETE | `/v1/servers/{id}` |
| `list_vpss` | GET | `/v1/servers` |

#### Scenarios

- **Valid token**: GIVEN a valid Hetzner token WHEN `validate_token` is called THEN returns `Ok(true)`
- **Create server**: GIVEN valid `ProvisionParams` WHEN `create_vps` is called THEN returns `VpsInstance` with `provider: "hetzner"` and `status: "running"`
- **Destroy missing server**: GIVEN a non-existent server ID WHEN `destroy_vps` is called THEN returns `Err(CloudError::Provider(...))`

### Requirement: Oracle Cloud Provider

The system MUST implement `CloudProvider` against the OCI API (`https://iaas.{region}.oraclecloud.com/20160918/`). Auth uses the Oracle-specific header format (different from DO/Hetzner Bearer scheme).

| Operation | HTTP | Endpoint |
|-----------|------|----------|
| `validate_token` | GET | `/20160918/instances` (with `compartmentId`) |
| `create_vps` | POST | `/20160918/instances` |
| `destroy_vps` | DELETE | `/20160918/instances/{id}` |
| `list_vpss` | GET | `/20160918/instances` |

`validate_token` requires a pre-configured compartment ID in `ProvisionParams` or provider config. `create_vps` MUST map the OCI launch response to `VpsInstance`.

#### Scenarios

- **Valid OCI token**: GIVEN a valid OCI token and compartment ID WHEN `validate_token` is called THEN returns `Ok(true)`
- **Rate limited**: GIVEN the OCI API returns HTTP 429 WHEN any operation is called THEN returns `Err(CloudError::RateLimit(retry_after))`

### Requirement: Retry Middleware

The system MUST provide `RetryCloudProvider<T: CloudProvider>` applying exponential backoff on retriable errors.

**Schedule**: 1s, 2s (max 3 total attempts = 1 initial + 2 retries; a 4s delay would require `max_retries=4`). Jitter: ±20%.

**Triggers**: `CloudError::RateLimit` and `Http(5xx)`.

**No retry**: `Auth`, `Quota`, `Timeout`, `Http(4xx)` — propagate immediately.

```rust
pub struct RetryCloudProvider<T: CloudProvider> {
    inner: T,
    max_retries: u32,       // default 3
    base_delay_ms: u64,     // default 1000
}
impl<T: CloudProvider> CloudProvider for RetryCloudProvider<T> { /* delegates with backoff */ }
```

#### Scenarios

- **Rate limit recovery**: GIVEN a provider returning 429 then 200 WHEN `create_vps` is called via retry wrapper THEN succeeds with the retry response
- **5xx exhausts retries**: GIVEN a provider returning 503 on all attempts WHEN any operation is called THEN returns the last error after 3 retries
- **Auth bypasses retry**: GIVEN a provider returning 401 WHEN any operation is called THEN propagates `Auth` immediately without retry

### Requirement: Error Mapping

All providers MUST map HTTP responses to `CloudError` consistently:

| HTTP | `CloudError` |
|------|-------------|
| 401 / 403 | `Auth(String)` |
| 429 | `RateLimit(seconds)` |
| 5xx | `Provider(String)` |
| Other 4xx | `Provider(String)` |
| Timeout | `Timeout` |

#### Scenarios

- **401 mapped**: GIVEN HTTP 401 response WHEN the provider parses it THEN returns `Err(CloudError::Auth(msg))`
- **503 mapped**: GIVEN HTTP 503 response WHEN the provider parses it THEN returns `Err(CloudError::Provider(msg))`

### Requirement: Security — Token Handling

Tokens MUST be read from `SecretsManager` at call time — never hardcoded, cached in plaintext, or printed. Error/log messages MUST redact token values.

#### Scenarios

- **No token in logs**: GIVEN an operation fails with token-related error WHEN the error is logged THEN the raw token value does not appear in the output
- **Token from SecretsManager**: GIVEN a saved token WHEN a provider operation is invoked THEN the token is retrieved via `SecretsManager::get_token(provider)`

---

## Interface Contract

### Existing (from `cloud/mod.rs`)

```rust
pub trait CloudProvider {
    async fn validate_token(&self, token: &str) -> Result<bool, CloudError>;
    async fn create_vps(&self, params: &ProvisionParams, token: &str) -> Result<VpsInstance, CloudError>;
    async fn destroy_vps(&self, instance_id: &str, token: &str) -> Result<(), CloudError>;
    async fn list_vpss(&self, token: &str) -> Result<Vec<VpsInstance>, CloudError>;
}
```

Provider modules: `cloud/do.rs`, `cloud/hz.rs`, `cloud/oci.rs` — each exports a struct implementing `CloudProvider`.

### New

```rust
// cloud/retry.rs
pub struct RetryCloudProvider<T: CloudProvider> {
    inner: T,
    max_retries: u32,     // default: 3
    base_delay_ms: u64,   // default: 1000
}

// Cargo.toml addition
[dependencies]
reqwest = { version = "0.12", features = ["json"] }
rsa = { version = "0.9", features = ["pem"] }
sha2 = "0.10"

[dev-dependencies]
wiremock = "0.6"
```

---

## Dependencies

| Change | Reason |
|--------|--------|
| `src-tauri/Cargo.toml` | Add `rsa` dep for OCI signing, `wiremock = "0.6"` under `[dev-dependencies]` |
| `src-tauri/src/cloud/mod.rs` | Add `pub mod r#do; pub mod hz; pub mod oci; pub mod retry;` |
