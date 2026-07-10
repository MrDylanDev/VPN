# Proposal: Phase 4 — Server Provisioning

## Intent

Automate WireGuard server setup on cloud VPS after Phase 3 creates them. Users get a ready-to-connect server without SSHing into cloud consoles. This closes the gap between "VPS exists" and "tunnel is ready."

## Scope

### In Scope
- SSH session lifecycle: ephemeral ed25519 keygen, connect, execute, disconnect via `spawn_blocking`
- Embedded bash scripts for WireGuard install, firewall, sysctl, DNS
- TOFU host key verification with persistent fingerprints
- Provision orchestration: wait VPS active → SSH → install → verify → return `PeerConfig`
- `Drop`-based rollback guard (destroy VPS if `commit()` not called)
- Tauri commands: `provision_server`, `destroy_server`
- Failure handling: retry SSH/install once before destroying VPS; verify connectivity before returning success

### Out of Scope
- Importing existing SSH keys (ephemeral only)
- Non-Ubuntu 24.04 distros
- Cloud-init / user-data scripts
- Batch provisioning (one VPS at a time)
- Config presets or saved provisioning profiles

## Capabilities

### New Capabilities
- `server-ssh-provisioning`: SSH-based provisioning flow — keygen, connect, execute commands, TOFU host key verification, disconnect
- `server-install-scripts`: Embedded bash scripts for WireGuard + firewall + DNS/sysctl installation on Ubuntu 24.04

### Modified Capabilities
- None — no existing specs are affected at the capability level

## Approach

Spawn-blocking wrapper around sync `ssh2` crate (already in deps). Bash scripts embedded via `include_str!()`. TOFU host key verification stores SHA256 fingerprints per IP in app data dir. Separate `SshError` + `ProvisionError` enums matching `CloudError` pattern. `Drop` guard on `ProvisionManager` auto-destroys VPS unless `commit()` called. Hybrid VPS readiness: poll provider API for active status, then TCP-connect SSH port before session.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `src-tauri/src/provision/ssh.rs` | New | SshSession: keygen, connect, execute, disconnect |
| `src-tauri/src/provision/scripts.rs` | New | `include_str!()` bash script constants |
| `src-tauri/src/provision/manager.rs` | New | Full orchestration + Drop rollback guard |
| `src-tauri/src/provision/error.rs` | New | SshError + ProvisionError enums |
| `src-tauri/src/provision/mod.rs` | Modified | Re-export submodules |
| `src-tauri/src/lib.rs` | Modified | Tauri commands |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| SSH drops mid-provision | Med | Drop guard destroys VPS if not committed |
| WireGuard script fails on OS mismatch | Med | Script checks `/etc/os-release`, targets Ubuntu 24.04 only |
| SSH blocking I/O can't cancel easily | Med | `session.set_timeout()` + `tokio::time::timeout()` double layer |

## Rollback Plan

`Drop` guard in `ProvisionManager`: holds VPS id from `create_vps`. If `commit()` is not called before guard drops, calls `destroy_vps` automatically. For partial installs, retry SSH/install once before rolling back.

## Dependencies

- Internal: `CloudProvider` trait (Phase 3), `ssh2 = "0.9"` (already in Cargo.toml)
- External: Ubuntu 24.04 VPS with SSH reachable

## Success Criteria

- [ ] `provision_server` returns valid `PeerConfig` for a reachable Ubuntu 24.04 VPS
- [ ] `destroy_server` cleans up the VPS after provision
- [ ] Rollback guard fires and destroys VPS when SSH fails
- [ ] Connectivity verified (ping/check) before returning success
- [ ] `cargo test` passes with mock SSH behaviors
