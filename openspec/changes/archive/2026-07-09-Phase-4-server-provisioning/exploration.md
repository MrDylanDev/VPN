# Exploration: Phase 4 — Server Provisioning

## Current State

**provision/mod.rs** is a stub with only `PeerConfig` (endpoint, server_public_key, client_private_key, client_public_key, allowed_ips, dns) and a `Default` impl. No SSH logic, no provisioning orchestration — the file is 28 lines of pure data structs.

**Cargo.toml** already has `ssh2 = "0.9"` wired in. The ssh2 crate wraps libssh2 (C library, fully synchronous). No additional runtime deps needed — it's been ready since Phase 1.

**Cloud providers** return `VpsInstance { id, provider, region, ip, status }`. After `create_vps` returns a VPS with `status: "active"`, the `ip` field is populated and ready for SSH. The `RetryCloudProvider` middleware handles 429/5xx retries before provision begins — but post-create there's no built-in "wait for active" polling.

**SecretsManager** stores provider API tokens via OS keyring + AES-256-GCM fallback. SSH key material (ephemeral keypairs) are NOT secrets — they're session-local and don't need persistent storage.

**Integration points exist as trait/enum stubs**:
- `tunnel/mod.rs`: `TunnelEngine` trait with `up`, `down`, `status` → will consume `PeerConfig` to bring the tunnel up
- `vpn/mod.rs`: `VpnState` state machine → will drive the lifecycle that includes provisioning as a pre-connect step

## Affected Areas

### New files to create

| File | Purpose |
|------|---------|
| `src-tauri/src/provision/ssh.rs` | SSH session lifecycle: ephemeral keygen, connect, execute, host key verify |
| `src-tauri/src/provision/scripts.rs` | Embedded bash scripts (WireGuard install, firewall/sysctl/DNS) via `include_str!` |
| `src-tauri/src/provision/manager.rs` | Orchestration: create VPS → wait active → SSH → install → generate PeerConfig |
| `src-tauri/src/provision/error.rs` | `SshError` + `ProvisionError` enums (wraps `CloudError`) |

### Existing files to modify

| File | Change |
|------|--------|
| `src-tauri/src/provision/mod.rs` | Re-export new submodules (`pub mod ssh`, `pub mod scripts`, `pub mod manager`, `pub mod error`) |
| `src-tauri/src/lib.rs` | Add Tauri commands: `provision_server`, `destroy_server`, `get_provision_status` |
| `src-tauri/src/cloud/mod.rs` | Optional: add `wait_for_active` helper to `CloudProvider` trait or as a standalone function |

### Future integration (NOT modified now)

| File | Why |
|------|-----|
| `src-tauri/src/tunnel/mod.rs` | Will consume `PeerConfig` from provision (Phase 5) |
| `src-tauri/src/vpn/mod.rs` | Will call provision as part of connect flow (Phase 6) |

## Approaches

### 1. SSH Module Design — async wrapper around sync ssh2

| Approach | Pros | Cons | Effort |
|----------|------|------|--------|
| **SpawnBlocking wrapper** — wrap ssh2 calls in `tokio::task::spawn_blocking` | Zero new deps, established Rust pattern, `ssh2` already in deps, Tauri commands are already async | `spawn_blocking` adds small overhead per call; need to manage channel boundaries cleanly | **Low** |
| Dedicated SSH thread pool with message passing | Full control over SSH connection lifecycle, can keep connections warm | Much more complex, overkill for one-shot provisioning, adds `crossbeam` or `flume` dep | **High** |
| `async-ssh2-tokio` crate | True async I/O, no `spawn_blocking` needed | **New dependency**, less mature, adds another crate to audit, `ssh2` already in deps unused | **Medium** |

**Recommendation**: **SpawnBlocking wrapper**. The ssh2 crate is already in `Cargo.toml`. The pattern is straightforward: an `SshSession` struct owns the `ssh2::Session`, and every async method calls `spawn_blocking(move || { /* sync ssh2 calls */ })`. For provisioning (one SSH session, a handful of commands), this is correct and proven. Example API:

```rust
// provision/ssh.rs
pub struct SshSession {
    ip: String,
    port: u16,
    session: Option<ssh2::Session>,
}

impl SshSession {
    pub async fn connect(
        ip: &str,
        port: u16,
        username: &str,
        key_pair: &KeyPair,
    ) -> Result<Self, SshError> { /* spawn_blocking */ }

    pub async fn execute(&self, cmd: &str) -> Result<CommandOutput, SshError> { /* spawn_blocking */ }

    pub async fn disconnect(&self) -> Result<(), SshError> { /* spawn_blocking */ }
}
```

### 2. Script Delivery — how to get bash scripts onto the VPS

| Approach | Pros | Cons | Effort |
|----------|------|------|--------|
| **Embed with `include_str!()`** | Compile-time, zero runtime deps, always available offline, trivial to test | Scripts baked into binary — need rebuild to update; larger binary (negligible for <2KB) | **Low** |
| Download from CDN at runtime | Can update scripts without app updates | Requires internet (ironic for provisioning), adds HTTP error paths, breaks if CDN is down | **Medium** |
| Generate inline Rust strings | Fully dynamic, can conditionally build commands | Harder to read/maintain, escaping hell, no syntax highlighting, error-prone | **Medium** |

**Recommendation**: **Embed with `include_str!()`**. Scripts are small (~30-50 lines each). Store them as `.sh` files alongside the provision module:

```
src-tauri/src/provision/
├── mod.rs
├── ssh.rs
├── manager.rs
├── scripts.rs
├── error.rs
└── scripts/
    ├── 01-install-wireguard.sh
    ├── 02-configure-firewall.sh
    └── 03-configure-dns-sysctl.sh
```

`scripts.rs` holds constants:
```rust
pub(crate) const INSTALL_WIREGUARD: &str = include_str!("scripts/01-install-wireguard.sh");
pub(crate) const CONFIGURE_FIREWALL: &str = include_str!("scripts/02-configure-firewall.sh");
pub(crate) const CONFIGURE_DNS_SYSCTL: &str = include_str!("scripts/03-configure-dns-sysctl.sh");
```

Each script is sent over SSH via `SshSession::execute()` or piped through stdin. Idempotent scripts with `set -euo pipefail`.

### 3. Host Key Verification

| Approach | Pros | Cons | Effort |
|----------|------|------|--------|
| **TOFU with stored fingerprints** — accept key on first SSH to a new IP, store SHA256 fingerprint in app data dir, verify on reconnects | Simple, matches SSH UX, MITM risk is minimal since we just created the VPS | State management (fingerprint file), needs to handle IP reuse across destroys | **Low** |
| `known_hosts` file — use ssh2's `KnownHosts` API with `known_hosts` file in app data | Standard SSH pattern, compatible with system tools | More complex API, needs file I/O and parsing on every connect | **Medium** |
| Skip verification — accept any host key | Dead simple | Insecure — MITM during provisioning could leak server keys/peer config | **Low** |
| Accept first + warn — accept first key and surface fingerprint in UI for user confirmation | Best UX, lets user decide | Frontend work needed in Phase 7; not needed for backend | **Medium** |

**Recommendation**: **TOFU with stored fingerprints** for MVP. The VPS is ephemeral — the app created it seconds ago. Risk is minimal. Store a JSON map of `{ip: fingerprint}` in `data_dir/vpn_known_hosts.json`. On disconnect/destroy, clean the entry. ssh2's `Session::host_key_hash()` returns the SHA256 hash for verification. Upgrade to full `known_hosts` file if the project needs compatibility with system WireGuard tools.

```rust
// Key flow:
let fingerprint = session.host_key_hash(ssh2::HashType::Sha256)
    .ok_or(SshError::HostKeyVerify)?;
if !known_hosts.is_known(ip, &fingerprint) {
    known_hosts.accept(ip, &fingerprint)?; // stores for next time
    // Optionally: surface fingerprint hash to user for manual verification
}
```

### 4. Error Handling

| Approach | Pros | Cons | Effort |
|----------|------|------|--------|
| **Separate error enums** — `SshError` + `ProvisionError` wrapping both | Clean concerns, `ProvisionError::Cloud(CloudError)` and `ProvisionError::Ssh(SshError)`, matches `CloudError` pattern already established | More types to define | **Low** |
| One big `ProvisionError` — all variants in a single enum | Single error type to match | Mixes SSH and cloud concerns, violates single-responsibility | **Low** |
| `Box<dyn Error>` — return generic errors everywhere | No new types | Loses type info, callers can't match specific errors for retry/rollback | **Low** |

**Recommendation**: **Separate error enums** following the established `CloudError` pattern:

```rust
// error.rs
#[derive(Debug, thiserror::Error)]
pub enum SshError {
    #[error("SSH connection to {0}:{1} failed: {2}")]
    Connect(String, u16, String),          // ip, port, reason

    #[error("SSH authentication failed")]
    Auth(String),                           // reason

    #[error("Command execution failed (exit {0}): {1}")]
    Command(i32, String),                   // exit code, stderr

    #[error("Host key verification failed")]
    HostKeyVerify,

    #[error("SSH session timed out")]
    Timeout,
}

#[derive(Debug, thiserror::Error)]
pub enum ProvisionError {
    #[error("Cloud error: {0}")]
    Cloud(#[from] CloudError),

    #[error("SSH error: {0}")]
    Ssh(#[from] SshError),

    #[error("Provision cancelled at step '{0}': {1}")]
    Cancelled(&'static str, String),

    #[error("Timeout waiting for VPS to become active")]
    VpsTimeout,
}
```

This allows the orchestrator (manager.rs) to cleanly match on `ProvisionError::Cloud(CloudError::Auth(_))` vs `ProvisionError::Ssh(SshError::Connect(...))` for rollback decisions.

### 5. "Wait for Active" — VPS readiness polling

The cloud provider APIs return `VpsInstance` with `status` but don't guarantee the server is immediately reachable via SSH.

| Approach | Pros | Cons | Effort |
|----------|------|------|--------|
| **Poll with SSH connect retry** — try SSH every 5s, timeout after 2 min | No API dependency, naturally waits for SSH readiness | Blocks the async thread during polling (even with spawn_blocking) | **Low** |
| Add `wait_for_active` to `CloudProvider` trait — poll provider API for `status == "active"` | More accurate, can check before SSH attempt | Not all providers report active status the same way; adds trait method for all impls | **Medium** |
| Fixed sleep (e.g., 30s) — just sleep and hope | Simplest | Brittle, wastes time, fails on slow provisioners | **Low** |

**Recommendation**: **Hybrid — poll provider API for status, then SSH retry**. Add a standalone `wait_for_vps` function (NOT a trait method) in `manager.rs` that:
1. Polls `CloudProvider::list_vpss` or checks the returned VPS status every 5s (max 2 min) until `status == "active"`
2. Then polls SSH port 22 with a TCP connect every 5s (max 1 min) until reachable
3. THEN establishes the SSH session

This keeps the trait clean and handles the gap between "API says active" and "SSH daemon is actually listening".

## Recommendations Summary

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Async SSH pattern | `spawn_blocking` wrapper | ssh2 already in deps, proven Rust pattern, no new deps |
| Script delivery | `include_str!()` embedded .sh files | Compile-time, offline, testable, trivial |
| Host key verify | TOFU with stored fingerprints | VPS is ephemeral and app-created; minimal risk; fingerprint stored per IP |
| Error handling | Separate `SshError` + `ProvisionError` | Matches existing `CloudError` pattern, enables clean rollback matching |
| VPS readiness | Hybrid: poll API status then SSH port | No trait changes, handles real-world SSH delay gracefully |
| Ephemeral SSH keys | Generate ed25519 keypair in memory per provision | No key persistence needed; keys are session-local; drop after `PeerConfig` generated |
| Rollback strategy | `Drop`-based guard on `ProvisionManager` | If provision fails mid-flow, a `ProvisionGuard` struct holds the VPS id; on drop, calls `destroy_vps`. Only `commit()` suppresses destruction. |

## File Change Plan

```
CREATE src-tauri/src/provision/ssh.rs          ~90 lines  — SshSession (connect, execute, disconnect)
CREATE src-tauri/src/provision/scripts.rs       ~30 lines  — include_str! constants for bash scripts
CREATE src-tauri/src/provision/scripts/          ─
  ├── 01-install-wireguard.sh                   ~40 lines
  ├── 02-configure-firewall.sh                  ~30 lines
  └── 03-configure-dns-sysctl.sh                ~20 lines
CREATE src-tauri/src/provision/error.rs         ~60 lines  — SshError + ProvisionError enums
CREATE src-tauri/src/provision/manager.rs       ~150 lines — full provision orchestration + rollback
MODIFY src-tauri/src/provision/mod.rs           +10 lines  — re-export new submodules
MODIFY src-tauri/src/lib.rs                     +30 lines  — Tauri commands
```

Estimated total: **~430 lines** (excluding tests).
Tests (4.7): additional ~200-300 lines with mock SSH server or struct substitution.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| `spawn_blocking` pool exhaustion if SSH takes long | Low | Degraded UI responsiveness | `spawn_blocking` has a growable pool; one SSH session per provision is fine. If pool is full, Tokio queues the task. |
| SSH connection drops mid-provision | Medium | Partially configured VPS orphaned | `Drop` guard in `manager.rs` destroys VPS if `commit()` not called. Manager tracks last completed step for retry. |
| libssh2 blocking I/O inside `spawn_blocking` can't be cancelled easily | Medium | User can't cancel provisioning mid-SSH | ssh2 has no built-in timeout for `channel_exec`. Set `session.set_timeout(ms)` on the ssh2 session (maps to `libssh2_session_set_timeout`). Wrap in `tokio::time::timeout()` as second layer. |
| WireGuard install script fails due to OS differences | Medium | Provision success varies by distro | Script targets Ubuntu 24.04 (matching `ProvisionParams` default). Use `set -e` and check `/etc/os-release`. Return stderr on failure for debugging. |
| VPS IP changes between create and SSH | Low | SSH connects to wrong host | Store IP from `create_vps` response; always connects to that exact IP. DO/Hetzner IPs are static after creation. |
| `include_str!` path resolution errors | Low | Build breaks | Scripts are in a `scripts/` subdirectory under provision/. Use a build script or relative path from the source file: `include_str!("scripts/01-install-wireguard.sh")`. Verify with `cargo check`. |
| ssh2 crate 0.9 may not build on all targets | Low | CI failures on macOS/Windows | ssh2 0.9 requires `libssh2` system lib or `libssh2-sys` feature. Check `Cargo.lock` — `libssh2-sys` is pulled transitively. Windows may need `vcpkg install libssh2`. |

## Ready for Proposal

**Yes**. The codebase is well-prepared for Phase 4:

- `ssh2` dep is already in `Cargo.toml` — no upstream dependency changes needed
- Cloud provider abstraction is production-ready (`VpsInstance` has `ip`, retry middleware handles transient errors)
- Established patterns (single-file modules, `thiserror` enums, `#[cfg(test)]` wiremock helpers) are clear and documented
- `PeerConfig` struct in provision/mod.rs defines the output contract; tunnel engine in Phase 5 takes `PeerConfig` as input — clean seam
- `SecretsManager` handles provider tokens; SSH ephemeral keys need no persistence (generated in memory per provision, dropped after)
- Rollback strategy via `Drop` guard is a well-known Rust pattern

The orchestrator should proceed with the **Proposal** phase using change name `Phase-4-server-provisioning`.
