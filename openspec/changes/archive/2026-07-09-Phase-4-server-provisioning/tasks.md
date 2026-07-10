# Tasks: Phase 4 — Server Provisioning

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~850 |
| 400-line budget risk | **High** |
| Chained PRs recommended | **Yes** |
| Suggested split | PR 1 (Foundation) → PR 2 (SSH+Manager) → PR 3 (Tauri+Tests) |
| Delivery strategy | ask-on-risk |
| Chain strategy | size-exception (maintainer-approved) |

Decision needed before apply: No
Chained PRs recommended: Yes
Chain strategy: size-exception
400-line budget risk: High

### Suggested Work Units

| Unit | Goal | Likely PR | Notes |
|------|------|-----------|-------|
| 1 | Error types, TofuStore, scripts, mod.rs | PR 1 | Foundation — no SSH deps, testable standalone |
| 2 | SshSession, ProvisionManager, ProvisionGuard | PR 2 | Core logic — depends on types from PR 1 |
| 3 | Tauri commands, lib.rs wiring, all unit tests | PR 3 | Integration layer — depends on PR 1+2 |

## Phase 1: Foundation

- [x] 1.1 Add `ed25519-dalek` to `src-tauri/Cargo.toml`
- [x] 1.2 Create `src-tauri/src/provision/error.rs` — `SshError` + `ProvisionError` (thiserror, Display, source chain)
- [x] 1.3 Create `src-tauri/src/provision/tofu.rs` — `TofuStore` with `new`, `verify`, `store`, `remove` methods
- [x] 1.4 Create `src-tauri/src/provision/scripts/install-wireguard.sh` — apt install WG + `wg genkey`
- [x] 1.5 Create `src-tauri/src/provision/scripts/configure-firewall.sh` — ufw allow 51820/udp
- [x] 1.6 Create `src-tauri/src/provision/scripts/configure-sysctl.sh` — net.ipv4.ip_forward=1
- [x] 1.7 Create `src-tauri/src/provision/scripts/configure-dns.sh` — resolv.conf → 1.1.1.1, 1.0.0.1
- [x] 1.8 Create `src-tauri/src/provision/scripts.rs` — `include_str!()` constants for each script
- [x] 1.9 Update `src-tauri/src/provision/mod.rs` — add `pub mod` declarations + re-exports

## Phase 2: SSH & Manager

- [x] 2.1 Create `src-tauri/src/provision/ssh.rs` — `SshSession` with `with_keypair`, `connect`, `execute`, `disconnect` via `spawn_blocking`
- [x] 2.2 Implement TOFU host key verification flow in `SshSession::connect`
- [x] 2.3 Create `src-tauri/src/provision/manager.rs` — `ProvisionManager` with `new` + `run` orchestrating full flow
- [x] 2.4 Implement `ProvisionGuard` with `Drop` (auto-destroy VPS) + `commit()` (suppress destroy)
- [x] 2.5 Add retry-once logic for SSH/install failures in `ProvisionManager::run`
- [x] 2.6 Add hybrid VPS readiness: API poll → TCP port 22 scan before SSH connect

## Phase 3: Tauri Commands

- [x] 3.1 Add `provision_server` Tauri command in `src-tauri/src/lib.rs`
- [x] 3.2 Add `destroy_server` Tauri command in `src-tauri/src/lib.rs`
- [x] 3.3 Register both commands in `invoke_handler` and wire into `AppState`

## Phase 4: Tests

- [x] 4.1 Unit: `SshError` Display + source chain conversions
- [x] 4.2 Unit: `TofuStore` — first-connect stores, mismatch rejects, missing-file creates, corrupt-file handles
- [x] 4.3 Unit: `ProvisionGuard` — Drop calls destroy, `commit()` suppresses it
- [x] 4.4 Integration: `ProvisionManager` with mock `CloudProvider` + fake SSH (wiremock or trait)
- [x] 4.5 Verify `scripts.rs` constants are valid UTF-8 via `std::str::from_utf8`
