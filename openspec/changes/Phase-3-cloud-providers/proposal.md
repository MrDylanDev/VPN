# Proposal: Phase 3 — Cloud Provider Integrations

## Intent

Automate VPS provisioning across DigitalOcean, Hetzner, and Oracle Cloud. Users spin up WireGuard servers from the desktop app instead of using cloud consoles. This replaces manual workflows with API-driven provisioning.

## Scope

### In Scope
- `CloudProvider` trait impls for DigitalOcean (Droplets API), Hetzner (Cloud API), Oracle (OCI API token auth)
- Exponential backoff retry middleware for rate limits / 5xx
- Token validation per provider (via `validate_token`)
- Test suite using mocked/recorded HTTP responses

### Out of Scope
- Oracle OCI CLI-based auth (API token only)
- Cloud-init / post-provision scripting (Phase 4)
- Multi-region discovery, IPv6, VPC customization
- Provider credential rotation UI
- Pricing or live-cost display

## Capabilities

### New Capabilities
- `cloud-provider-digitalocean`: DigitalOcean Droplets CRUD + token validation
- `cloud-provider-hetzner`: Hetzner Cloud Server CRUD + token validation
- `cloud-provider-oracle`: Oracle Cloud OCI instance CRUD + token validation
- `cloud-provider-retry`: Exponential backoff retry layer for rate limits / 5xx

### Modified Capabilities
- None — no existing specs change at this level

## Approach

Trait-based pattern stubbed in `cloud/mod.rs`. Each provider in its own module under `cloud/`:

- **Modules**: `cloud/do/`, `cloud/hz/`, `cloud/oci/` — each implements `CloudProvider`
- **HTTP**: Reqwest with per-provider base URL and `Authorization: Bearer` header
- **Retry**: `RetryCloudProvider<T>` wrapper applying 1s/2s/4s backoff (max 3 retries) on `RateLimit` / `Http(5xx)`
- **Tokens**: Stored via existing `SecretsManager` (keyring + AES-256-GCM fallback)
- **Testing**: `wiremock` for recorded HTTP response tests per endpoint

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `src-tauri/src/cloud/` | Modified | Add `do/`, `hz/`, `oci/` submodules, retry wrapper |
| `src-tauri/src/cloud/mod.rs` | Modified | Export new submodules, add retry module |
| `src-tauri/Cargo.toml` | Modified | Add `wiremock` dev-dependency |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Oracle OCI token auth differs from DO/Hetzner | Medium | Dedicated HTTP header format in own module |
| API rate limits cause UX failures | Low | Backoff retry handles transparently |
| Provider API breaking changes | Low | Mocked test fixtures pinned to known API versions |

## Rollback Plan

Delete `cloud/do/`, `cloud/hz/`, `cloud/oci/` modules, revert `cloud/mod.rs` to stub-only state, remove `wiremock` from `Cargo.toml`.

## Dependencies

- External: DO, Hetzner, Oracle Cloud public HTTP APIs
- Internal: `SecretsManager` (Phase 2) for token CRUD; `reqwest` (existing dep)

## Success Criteria

- [ ] All 3 providers implement `validate_token` with correct accept/reject on mock responses
- [ ] `create_vps` returns a `VpsInstance` for each provider under mocked conditions
- [ ] Retry wrapper fires on simulated rate limit and recovers within budget
- [ ] `cargo test` passes — all provider tests green
