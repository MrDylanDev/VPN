# Tasks: WireGuard VPN Client

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | 3000–4500 |
| 400-line budget risk | High |
| Chained PRs recommended | Yes |
| Suggested split | PR 1 → PR 2 → PR 3 → PR 4 → PR 5 |
| Delivery strategy | auto-forecast |
| Chain strategy | stacked-to-main |

Decision needed before apply: Yes
Chained PRs recommended: Yes
Chain strategy: stacked-to-main
400-line budget risk: High

### Suggested Work Units

| Unit | Goal | Likely PR | Base |
|------|------|-----------|------|
| 1 | Scaffold + Config + i18n | PR 1 (📍 current) | `feat/wireguard-vpn-client` → main |
| 2 | Cloud Providers (DO/Hetz/Ora) | PR 2 | main (after PR 1 merges) |
| 3 | Server Provisioning (SSH) | PR 3 | main (after PR 2 merges) |
| 4 | Tunnel Engine + State Machine | PR 4 | main (after PR 3 merges) |
| 5 | UI + Polish | PR 5 | main (after PR 4 merges) |

## Phase 1: Foundation

- [x] 1.1 Scaffold Tauri 2 + Svelte 5 + TypeScript project (CF-5)
- [x] 1.2 Create Rust module stubs: cloud/, provision/, tunnel/, vpn/, config/, i18n/
- [x] 1.3 Create Svelte stubs: StatusBar, ConnectButton, ServerList, ProviderForm, ProgressIndicator
- [x] 1.4 Configure Cargo.toml: reqwest, ssh2, serde, serde_json, tokio, keyring, tauri
- [x] 1.5 Configure npm: Svelte 5, Tauri 2 API, TypeScript, vite
- [x] 1.6 Verify cargo build + npm run build pass

## Phase 2: Config & i18n

- [x] 2.1 AppConfig struct + JSON persistence + corruption recovery (CF-1, CF-5, CF-6, CF-7)
- [x] 2.2 Secrets: keyring + AES-256-GCM file fallback (CPA-1, NF-3)
- [x] 2.3 i18n module: loader, t! macro, en.json, es.json (I18N-1, I18N-4, I18N-5)
- [x] 2.4 Wire config + i18n at app startup (CF-5, I18N-2)
- [x] 2.5 Test: save/load config, corruption recovery, token CRUD (CF-S1, CF-S2, CPA-S3)

## Phase 3: Cloud Providers

- [ ] 3.1 CloudProvider trait + CloudError enum (SP-1)
- [ ] 3.2 DigitalOcean: create/list/destroy droplets, regions, validate token (CPA-5, SP-1, SP-6)
- [ ] 3.3 Hetzner: create/list/destroy servers, validate token (CPA-5, SP-1, SP-6)
- [ ] 3.4 Oracle: create/list/destroy instances via OCI, validate token (CPA-5, SP-1, SP-6)
- [ ] 3.5 Exponential backoff retry on rate limits / 5xx (SP-4)
- [ ] 3.6 Test: token validation, create with recorded responses (CPA-S1, CPA-S2)

## Phase 4: Server Provisioning

- [ ] 4.1 SSH module: ephemeral keygen, connect, execute, host key verify (SP-2, SP-5)
- [ ] 4.2 WireGuard install script (bash, sent over SSH) (SP-2)
- [ ] 4.3 Firewall + sysctl + DNS config script (SP-2)
- [ ] 4.4 Full provision: create VPS → wait active → SSH → install → peer config (SP-1, SP-2, SP-3)
- [ ] 4.5 Rollback: auto-destroy VPS if SSH or install fails (SP-7)
- [ ] 4.6 Destroy: remove VPS + clean local state (SP-6)
- [ ] 4.7 Test: full provision, failure rollback, destroy (SP-S1, SP-S2, SP-S3)

## Phase 5: Tunnel Engine

- [ ] 5.1 System WG detection: which wg-quick / wg show (TE-1)
- [ ] 5.2 System engine: up/down/status via wg-quick (TE-3, TE-4, TE-6)
- [ ] 5.3 Embedded engine: wireguard-go subprocess wrapper (TE-2, TE-3, TE-4)
- [ ] 5.4 Engine selection: system preferred → embedded fallback (TE-1, TE-2)
- [ ] 5.5 Test: detection, up/down cycle, status reporting (TE-S1, TE-S2, TE-S3)

## Phase 6: VPN Connection

- [ ] 6.1 State machine: Disconnected ↔ Connecting ↔ Connected ↔ Disconnecting (VC-5)
- [ ] 6.2 Tauri commands: connect, disconnect, get_status (VC-1, VC-2)
- [ ] 6.3 Tauri events: vpn:state, vpn:stats, vpn:error (VC-4)
- [ ] 6.4 Wire cloud + provision + tunnel into state machine (VC-1)
- [ ] 6.5 Persist / restore last state on app start (VC-3)
- [ ] 6.6 Test: connect cycle, invalid transitions, event emission (VC-S1, VC-S2, VC-S3)

## Phase 7: User Interface

- [ ] 7.1 ProviderForm: token input, provider select, region/size, provision trigger (UI-3, UI-6)
- [ ] 7.2 ServerList: provisioned servers list + destroy action (UI-5)
- [ ] 7.3 ConnectButton: toggle with spinner, state-driven label (UI-1, UI-2)
- [ ] 7.4 StatusBar: connection indicator, traffic bytes, handshake time (UI-4)
- [ ] 7.5 ProgressIndicator: step-by-step provision progress (UI-S3)
- [ ] 7.6 Settings view: language switch, theme picker, saved tokens (CF-2, I18N-3)
- [ ] 7.7 SvelteKit routes: / (dashboard), /settings + main layout
- [ ] 7.8 Dark/light/system theme via CSS custom properties (CF-2)

## Phase 8: Polish & Integration

- [ ] 8.1 i18n integration in all UI components (I18N-1)
- [ ] 8.2 Error toast/notification display system (UI-7)
- [ ] 8.3 Loading states + transitions for all async operations (UI-6)
- [ ] 8.4 Tray icon with connection status indicator
- [ ] 8.5 Auto-start on login (optional)
- [ ] 8.6 Cross-platform build + test: Linux, macOS, Windows (NF-4)
