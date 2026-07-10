# Design: Phase 4 — Server Provisioning

## Technical Approach

Spawn-blocking wrapper around sync `ssh2` for SSH lifecycle; bash scripts (`include_str!()`) for WireGuard install; hybrid VPS readiness (cloud API poll → TCP port scan); `Drop` guard auto-destroys VPS unless `commit()` called. Matches proposal §4 approach and spec §1–§5 requirements.

## Architecture Decisions

| # | Decision | Options | Tradeoff | Chosen |
|---|----------|---------|----------|--------|
| 1 | SSH module design | spawn_blocking wrapper / async SSH crate | New dep (async-ssh2-tokio) vs zero dep cost + proven sync lib | **spawn_blocking** — ssh2 already in deps, no new Cargo risk |
| 2 | Script delivery | include_str!() / read at runtime / cloud-init | Runtime reads need relative paths + install step; include_str!() compile-time verified | **include_str!()** — zero runtime failure path |
| 3 | Host key verification | TOFU file / known_hosts parsing / skip | ssh2 known_hosts API is stateful; TOFU as JSON file is trivially testable | **TOFU per-IP JSON** — simplest correct impl, testable without SSH |
| 4 | Error model | Single ProvisionError / layered (Ssh + Provision) | Single enum mixes concerns; layered matches CloudError pattern | **Two enums** — SshError (Connect, Auth, Exec, Timeout, HostKeyMismatch) + ProvisionError(Cloud, Ssh, Timeout, OsMismatch) |
| 5 | Rollback mechanism | Drop guard / explicit try-finally / state machine | Explicit is error-prone; Drop guard is composable, zero-cost when commit() called | **ProvisionGuard** — holds VpsInstance, Drop calls destroy_vps |
| 6 | VPS readiness | Poll cloud API only / poll + TCP scan | API alone: status "active" but sshd not listening yet → SSH retry waste | **Both** — poll API for active, then TCP connect port 22 before SSH |

## Data Flow

```
User clicks "Provision"
    │
    ▼
Tauri command: provision_server(provider, params)
    │
    ▼
ProvisionManager::run(params, token, app_data_dir)
    │
    ├── 1. CloudProvider::create_vps() ───────────────► VpsInstance { id, ip }
    │
    ├── 2. Poll cloud API until status = "active"
    │      └── TCP connect 10.0.0.1:22 (max 30s, 2s interval)
    │
    ├── 3. SshSession::connect(ip, ephemeral_key)
    │      └── Read / store / verify TOFU fingerprint
    │
    ├── 4. Execute scripts (install-wg, firewall, sysctl, dns)
    │      └── Retry on failure once
    │
    ├── 5. Verify connectivity (ping check via SSH)
    │
    ├── 6. Generate PeerConfig ← ephemeral client key + server pubkey
    │
    ├── 7. ProvisionGuard::commit() ───► skip Drop destroy
    │
    └── Return Ok(PeerConfig)
```

On any error before commit: `ProvisionGuard::drop()` calls `destroy_vps`.

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `src-tauri/src/provision/mod.rs` | Modify | Re-export ssh, scripts, error, manager; keep PeerConfig |
| `src-tauri/src/provision/ssh.rs` | Create | SshSession: keygen, connect, exec, disconnect + TofuStore |
| `src-tauri/src/provision/error.rs` | Create | SshError + ProvisionError enums (thiserror, matches CloudError pattern) |
| `src-tauri/src/provision/manager.rs` | Create | ProvisionManager (orchestrate) + ProvisionGuard (Drop guard) |
| `src-tauri/src/provision/scripts.rs` | Create | `include_str!("scripts/*.sh")` constants |
| `src-tauri/src/provision/scripts/install-wireguard.sh` | Create | apt install + wg keygen |
| `src-tauri/src/provision/scripts/configure-firewall.sh` | Create | ufw allow 51820/udp |
| `src-tauri/src/provision/scripts/configure-sysctl.sh` | Create | net.ipv4.ip_forward + IPv6 |
| `src-tauri/src/provision/scripts/configure-dns.sh` | Create | resolv.conf → 1.1.1.1, 1.0.0.1 |
| `src-tauri/src/lib.rs` | Modify | Register provision_server + destroy_server commands; add AppState fields |
| `src-tauri/Cargo.toml` | Modify | Add `sha2` for fingerprint hashing (already present) |

## Interfaces / Contracts

```rust
// ── SshSession (sync, behind spawn_blocking) ──
pub struct SshSession { /* ephemeral keypair, ssh2::Session */ }

impl SshSession {
    pub fn with_keypair() -> (Self, ed25519::Keypair);
    pub fn connect(&mut self, addr: &str, port: u16, timeout: Duration) -> Result<(), SshError>;
    pub fn execute(&mut self, script: &str) -> Result<(String /* stdout */, String /* stderr */), SshError>;
    pub fn disconnect(&mut self);
}

// ── TofuStore ──
pub struct TofuStore { path: PathBuf }

impl TofuStore {
    pub fn new(data_dir: PathBuf) -> Self;
    pub fn verify(&self, ip: &str, fingerprint: &str) -> Result<(), SshError>;
    pub fn store(&self, ip: &str, fingerprint: &str);
    pub fn remove(&self, ip: &str);
}

// ── ProvisionManager ──
pub struct ProvisionManager<'p> {
    cloud: &'p dyn CloudProvider,
    token: &'p str,
    guard: Option<ProvisionGuard>,
}

impl ProvisionManager<'_> {
    pub fn new(cloud: &dyn CloudProvider, token: &str) -> Self;
    pub async fn run(&mut self, params: &ProvisionParams, data_dir: PathBuf) -> Result<PeerConfig, ProvisionError>;
}

// ── ProvisionGuard (Drop impl) ──
pub struct ProvisionGuard<'p> {
    cloud: &'p dyn CloudProvider,
    instance: Option<VpsInstance>,
    token: &'p str,
    committed: bool,
}
// Drop: if !committed && instance.is_some() → cloud.destroy_vps()
```

Error enums follow `thiserror` + `#[error("...")]` pattern matching cloud/do.rs.

## Testing Strategy

| Layer | What | Approach |
|-------|------|----------|
| Unit | SshError mapping + ProvisionError::from | Test error conversions and Display impls |
| Unit | TofuStore read/write/verify/remove | Temp dir + JSON file, vectors: first-connect, mismatch, missing-file, corrupt-file |
| Unit | ProvisionGuard drop behavior | Assert destroy_vps called on drop, NOT called after commit |
| Unit | Scripts parse check | `assert!(std::str::from_utf8(SCRIPT).is_ok())` for each embedded script |
| Integration | ProvisionManager with mock cloud | wiremock for create/list/destroy; SshSession replaced via trait or mock SSH server |
| Integration | Full provision flow | wiremock (cloud) + docker SSH container (if available in CI) |

## Open Questions

None — all decisions confirmed in the orchestration prompt.
