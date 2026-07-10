# Server Provisioning Specification

Automate WireGuard server setup on cloud VPS (Ubuntu 24.04) via SSH: ephemeral keys → connect → install WG + firewall + DNS/sysctl → verify → return `PeerConfig`.

## Requirements

### Requirement: SSH Session Lifecycle

The system MUST manage ephemeral SSH sessions: generate an in-memory ed25519 keypair per provision, connect to the VPS, execute remote commands, and disconnect. Total provision timeout: 5 minutes.

| Operation | Behavior |
|-----------|----------|
| Keygen | Ephemeral ed25519 in memory; never written to disk |
| Connect | TCP port 22, TOFU host key verification |
| Execute | Send bash script via stdin, collect stdout/stderr |
| Disconnect | Close session, discard ephemeral private key |

#### Scenario: Successful SSH round-trip
- GIVEN a reachable Ubuntu 24.04 VPS at `10.0.0.1`
- WHEN the system keys, connects, runs `uname -a`, and disconnects
- THEN stdout contains "Linux"
- AND the ephemeral key is discarded after disconnect

#### Scenario: SSH connection timeout
- GIVEN a VPS unresponsive on port 22 within the timeout
- WHEN the system attempts to connect
- THEN returns `Err(SshError::Timeout)`

### Requirement: Server Installation Scripts

The system MUST embed bash scripts (`include_str!`) that install WireGuard, configure `ufw`, enable `sysctl` IP forwarding, and set DNS on Ubuntu 24.04. Scripts MUST check `/etc/os-release` and reject other distros.

| Script | Action |
|--------|--------|
| `install-wireguard.sh` | `apt` install wireguard + generate server keys |
| `configure-firewall.sh` | `ufw allow 51820/udp` + `ufw --force enable` |
| `configure-sysctl.sh` | Enable `net.ipv4.ip_forward` + IPv6 forwarding |
| `configure-dns.sh` | Set `/etc/resolv.conf` to `1.1.1.1, 1.0.0.1` |

#### Scenario: Successful installation
- GIVEN an Ubuntu 24.04 VPS
- WHEN the system runs all install scripts
- THEN `wg show` exits 0, ufw has port 51820/udp, `net.ipv4.ip_forward=1`

#### Scenario: OS version mismatch
- GIVEN a VPS running Ubuntu 22.04
- WHEN the install script checks `/etc/os-release`
- THEN it exits with error
- AND returns `Err(ProvisionError::OsMismatch)`

### Requirement: Provision Orchestration

The system MUST orchestrate: wait for VPS active → SSH → install WG + firewall + sysctl + DNS → verify → return `PeerConfig`. On SSH or install failure, retry once before rolling back (destroy VPS via `Drop` guard).

#### Scenario: Full provision succeeds
- GIVEN a `VpsInstance` with `status: "active"`
- WHEN `provision_server` is invoked
- THEN WireGuard is installed, connectivity verified, and returns `Ok(PeerConfig)`

#### Scenario: SSH fails, retry then rollback
- GIVEN a VPS where first SSH connection fails
- WHEN the retry also fails
- THEN the VPS is destroyed
- AND returns `Err(ProvisionError::Ssh(...))`

### Requirement: TOFU Host Key Verification

The system MUST store SSH host key SHA256 fingerprints per IP. First connection stores it; subsequent connections MUST verify it matches.

#### Scenario: First connection stores fingerprint
- GIVEN no stored fingerprint for `10.0.0.1`
- WHEN the system connects via SSH
- THEN the fingerprint is persisted

#### Scenario: Fingerprint mismatch blocks connection
- GIVEN a stored fingerprint for `10.0.0.1`
- WHEN the remote host key differs
- THEN connection rejected with `Err(SshError::HostKeyMismatch)`

### Requirement: Destroy Server

The system MUST destroy a provisioned VPS and remove its stored fingerprint.

#### Scenario: Successful destroy
- GIVEN a provisioned VPS with a stored fingerprint
- WHEN `destroy_server` is called
- THEN `CloudProvider::destroy_vps` is invoked
- AND the fingerprint for that IP is deleted

### Requirement: Tauri Commands

The system MUST expose `provision_server` and `destroy_server` Tauri commands.

| Command | Input | Output |
|---------|-------|--------|
| `provision_server` | `provider, params` | `Result<PeerConfig, String>` |
| `destroy_server` | `provider, instance_id, region` | `Result<(), String>` |

#### Scenario: provision_server returns PeerConfig
- GIVEN valid provider and params
- WHEN the frontend invokes `provision_server`
- THEN the full flow runs
- AND a `PeerConfig` is returned

#### Scenario: destroy_server cleans up
- GIVEN a provisioned VPS
- WHEN the frontend invokes `destroy_server`
- THEN the VPS is destroyed and fingerprint cleaned


