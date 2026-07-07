# Tasks: Phase-3-cloud-providers

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~440 |
| 400-line budget risk | Medium |
| Chained PRs recommended | Yes |
| Suggested split | PR 1: Foundation + DO + Hetzner (~270 lines); PR 2: OCI + signing (~170 lines) |
| Delivery strategy | ask-on-risk |
| Chain strategy | stacked-to-main |

Decision needed before apply: Yes (resolved — PR 2)
Chained PRs recommended: Yes
Chain strategy: stacked-to-main (PR 1 foundation + DO/HZ → PR 2 OCI)
400-line budget risk: Medium

### Suggested Work Units

| Unit | Goal | Likely PR | Notes |
|------|------|-----------|-------|
| 1 | Foundation + DO + Hetzner providers | PR 1 | Cargo.toml, mod.rs, retry.rs, do.rs, hz.rs |
| 2 | Oracle OCI provider | PR 2 | oci.rs with RSA signing, depends on PR 1 base types |

## Phase 1: Foundation
- [x] 1.1 `src-tauri/Cargo.toml` — add reqwest `json` feature (no `blocking` — async client only), add `rsa = { version = "0.9", features = ["pem"] }`, add `wiremock = "0.6"` dev-dep
- [x] 1.2 `src-tauri/src/cloud/mod.rs` — add `pub mod do; pub mod hz; pub mod retry;` and `pub use` re-exports (OCI module deferred to PR 2)

## Phase 2: Retry Middleware
- [x] 2.1 `src-tauri/src/cloud/retry.rs` — `RetryCloudProvider<T: CloudProvider>` struct with `max_retries` (default 3), `base_delay_ms` (default 1000), ±20% jitter
- [x] 2.2 Implement `CloudProvider for RetryCloudProvider<T>` — exponential backoff on `RateLimit` + `Provider` (5xx); propagate `Auth`, `Quota`, `Timeout` immediately
- [x] 2.3 Tests: wiremock mock returning 429→200 (recovery), 503×3 (exhaustion), 401 immediate passthrough + unit tests for `is_retriable` and `jittered_delay_ms`

## Phase 3: Bearer Token Providers
- [x] 3.1 `src-tauri/src/cloud/do.rs` — `DigitalOceanProvider` with async `reqwest::Client`, implement all 4 async trait methods against DO Droplets API (`api.digitalocean.com/v2/`)
- [x] 3.2 `src-tauri/src/cloud/hz.rs` — `HetznerProvider` with async `reqwest::Client`, implement all 4 async trait methods against Hetzner Cloud API (`api.hetzner.cloud/v1/`)
- [x] 3.3 Tests: wiremock per DO endpoint (valid/invalid token, create/destroy/list droplets) + per Hetzner endpoint (valid token, create server, destroy missing) + error mapping table (401→Auth, 429→RateLimit, 503→Provider) across both providers

## Phase 4: Oracle OCI Provider
- [x] 4.1 `src-tauri/src/cloud/oci.rs` — `OracleCloudProvider` struct with `compartment_id: Option<String>` and `region: &str`, `new()` building async `reqwest::Client` with `iaas.{region}.oraclecloud.com` base URL
- [x] 4.2 Implement token JSON parsing (`user_ocid`, `tenancy_ocid`, `key_fingerprint`, `private_key_pem`) and RSA-SHA256 request signing per RFC 7235
- [x] 4.3 Implement all 4 trait methods against `iaas.{region}.oraclecloud.com/20160918/`; `validate_token` returns `Err(Auth)` when compartment_id is `None`
- [x] 4.4 Tests: known-answer signing test (fixed key + payload → expected signature), wiremock for 429 rate limit, error mapping
