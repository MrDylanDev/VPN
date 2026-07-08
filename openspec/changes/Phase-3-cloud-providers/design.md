# Design: Phase 3 — Cloud Provider Integrations

## Technical Approach

Implement the `CloudProvider` trait for DigitalOcean (Droplets API), Hetzner (Cloud API), and Oracle (OCI Compute API), plus a `RetryCloudProvider<T>` middleware. Each provider in its own file under `cloud/`, uses async `reqwest::Client`, receives tokens per-call from `SecretsManager`.

Key constraints: trait is async (Tokio runtime), tokens never stored on provider struct, OCI uses request signing (not Bearer), 800-word budget.

## Architecture Decisions

### Async trait + async HTTP vs sync blocking

| Option | Tradeoff | Decision |
|--------|----------|----------|
| Make trait async | Needs `async fn in trait` (RPITIT); Tauri 2 backend is Tokio-native; wiremock tests need async | **Chosen** |
| Keep sync + `reqwest::blocking` | `blocking::Client` spawns its own tokio runtime internally; dropping inside an existing runtime panics; wiremock tests need `spawn_blocking` wrappers | Rejected |

`reqwest::blocking::Client` internally calls `block_on`, which panics when dropped inside a Tokio runtime ("Cannot drop a runtime in a context where blocking is not allowed"). Since Tauri 2 runs on Tokio natively, async reqwest is the correct architecture. The `async fn in trait` (RPITIT) warnings are harmless for internal traits.

### Oracle compartmentId placement

| Option | Tradeoff | Decision |
|--------|----------|----------|
| `ProvisionParams.compartment_id` | Pollutes generic params with OCI-specific field | Rejected |
| `OracleCloudProvider.compartment_id: Option<String>` | OCI-specific, set once at construction | **Chosen** |

`validate_token` returns `Err(Auth(...))` if compartment_id is `None` when required. `create_vps` reads it from the struct.

### Oracle OCI auth mechanism

OCI Compute API requires RFC 7235 Signature request signing — NOT a Bearer token. Stored "token" is a JSON blob:

```json
{"user_ocid":"ocid1.user...","tenancy_ocid":"ocid1.tenancy...",
 "key_fingerprint":"20:3b:...","private_key_pem":"-----BEGIN RSA PRIVATE KEY-----..."}
```

Per-request signing (OCI REST API v20160918):
1. Build signing string: `(request-target): get /20160918/instances\nhost: iaas.{region}.oraclecloud.com\ndate: {RFC 2822}`
2. Sign with RSA-SHA256 using stored private key
3. Emit: `Authorization: Signature keyId="{tenancy}/{user}/{fingerprint}",algorithm="rsa-sha256",headers="(request-target) host date x-content-sha256",signature="{base64}"`

Requires `rsa` crate with `pem` + `sha2` features for key parsing and signing.

### Module file structure

Single-file modules (`cloud/do.rs`, `cloud/hz.rs`, `cloud/oci.rs`, `cloud/retry.rs`) — no sub-module directories. Single files reduce ceremony for ~200-line providers. Conversion to directories is a trivial refactor if providers grow.

### HTTP client per provider

Each provider constructs its own async `reqwest::Client`:
```rust
Client::builder()
    .timeout(Duration::from_secs(30))
    .connect_timeout(Duration::from_secs(10))
    .build()
```
Per-provider clients avoid auth-header cross-contamination. Connection pooling is per-client but negligible at this scale (3 providers).

### Token flow never stored

Tokens are `&str` parameters on every method call. Provider structs hold NO token reference. Caller retrieves from `SecretsManager::get_token("digitalocean")` at invocation time. Prevents leakage through serialization, logging, or cross-provider contamination.

## Data Flow

```
Tauri command (async)
  │
  ├── ProvisionManager::create_vps(provider_name, params)
  │     │
  │     ├── SecretsManager::get_token(provider_name)
  │     │     └─► OS keyring / encrypted file
  │     │
  │     ├── DigitalOceanProvider::new()
  │     │   └── RetryCloudProvider::new(do_provider)
  │     │       └── create_vps(params, token).await
  │     │           ├── POST /v2/droplets — Bearer {token}
  │     │           └── parse → VpsInstance
  │     │
  │     ├── HetznerProvider::new()
  │     │   └── create_vps(params, token).await
  │     │       ├── POST /v1/servers — Bearer {token}
  │     │
  │     └── OracleCloudProvider::new(compartment_id)
  │         └── create_vps(params, token).await
  │             ├── parse token JSON → key material
  │             ├── build Signature header
  │             ├── POST /20160918/instances — Signature {signed}
  │             └── map LaunchInstance response → VpsInstance
```

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `src-tauri/src/cloud/do.rs` | Create | `DigitalOceanProvider` — DO Droplets API impl |
| `src-tauri/src/cloud/hz.rs` | Create | `HetznerProvider` — Hetzner Cloud API impl |
| `src-tauri/src/cloud/oci.rs` | Create | `OracleCloudProvider` — OCI Compute with request signing |
| `src-tauri/src/cloud/retry.rs` | Create | `RetryCloudProvider<T>` — exponential backoff middleware |
| `src-tauri/src/cloud/mod.rs` | Modify | Add `pub mod do; pub mod hz; pub mod oci; pub mod retry;` + re-exports |
| `src-tauri/Cargo.toml` | Modify | Add `rsa` dep, add `wiremock` dev-dep (no `blocking` reqwest feature — async client only) |

## Interfaces / Contracts

```rust
// Constructors
impl DigitalOceanProvider {
    pub fn new() -> Result<Self, CloudError>;
}
impl HetznerProvider {
    pub fn new() -> Result<Self, CloudError>;
}
impl OracleCloudProvider {
    pub fn new(compartment_id: Option<String>) -> Result<Self, CloudError>;
}

// Retry wrapper
pub struct RetryCloudProvider<T: CloudProvider> {
    inner: T,
    max_retries: u32,     // default: 3
    base_delay_ms: u64,   // default: 1000
}
impl<T: CloudProvider> CloudProvider for RetryCloudProvider<T> { /* delegates */ }

// mod.rs re-exports
pub use do::DigitalOceanProvider;
pub use hz::HetznerProvider;
pub use oci::OracleCloudProvider;
pub use retry::RetryCloudProvider;
```

### Cargo.toml delta
```toml
[dependencies]
reqwest = { version = "0.12", features = ["json"] }
rsa = { version = "0.9", features = ["pem"] }
sha2 = "0.10"

[dev-dependencies]
wiremock = "0.6"
```

## Testing Strategy

| Layer | Coverage | Approach |
|-------|----------|---------|
| Provider unit | Each endpoint all 3 providers | `wiremock` mount per test, fixture JSON per endpoint |
| Retry | RateLimit recovery, 5xx exhaustion, auth bypass | Mock provider returning controlled errors; verify retry count |
| OCI signing | Auth header format | Known-answer test: fixed key + payload → expected Signature string |
| Error mapping | Table-driven HTTP→CloudError | Mock HTTP codes, assert variant + message |
| Token redaction | Logs never include token | Regex assertion on `Debug` output of error paths |

**Fixture pattern**: Each provider has `#[cfg(test)]` helpers creating a wiremock `MockServer` and a provider pointing at it. Response bodies use `serde_json::json!()` inline for readability.

## Migration / Rollout

No migration. Additive modules only. Existing `cloud/mod.rs` unchanged except adding module declarations and re-exports. Wiring into Tauri commands deferred to provision orchestration.

## Open Questions

- [ ] OCI token UX format: composite form (4 fields) vs single JSON blob blob. JSON is simpler to implement but worse UX. Needs product decision.
- [ ] `rsa` crate 0.9: verify `pem` + `sha2` feature flags resolve correctly with `reqwest` 0.12's existing `rustls`/`native-tls` stack.
- [ ] `wiremock` 0.6: confirmed compatible with reqwest 0.12 async client (both use hyper 1.x). ✅
- [ ] OCI `list_vpss` returns instances across compartments or only configured one? Design assumes the configured compartment only. Validate with real OCI account.
