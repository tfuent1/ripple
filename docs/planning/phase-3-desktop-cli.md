# Phase 3 — Desktop and CLI

## Goal

Ship the Tauri desktop app and a production-ready CLI daemon. Establish the
homelab and infrastructure node story — a Raspberry Pi or small server running
the CLI daemon becomes a permanent, always-on mesh relay that dramatically
improves mesh reliability for nearby mobile nodes.

A passing Phase 3 means: a Raspberry Pi running `ripple daemon` acts as a
persistent mesh backbone node, relaying bundles between mobile nodes that
would otherwise miss each other, and the desktop app provides a full-featured
mesh client for Mac, Windows, and Linux.

## Success Criteria

- [ ] CLI daemon runs as a systemd service on Linux
- [ ] CLI daemon participates in mesh via WiFi ad-hoc mode
- [ ] CLI daemon bridges to internet relay when connectivity available
- [ ] CLI daemon exposes local HTTP API for scripting and integration
- [ ] CLI daemon supports LoRa bridge via Meshtastic USB/BT serial
- [ ] Tauri desktop app builds on macOS, Windows, and Linux
- [ ] Desktop app connects to local CLI daemon or runs embedded core
- [ ] Desktop app full messaging UI
- [ ] Desktop app offline maps (MapLibre GL in webview)
- [ ] Desktop app network topology visualization
- [ ] Desktop app acts as WiFi ad-hoc mesh node
- [ ] Desktop app on macOS uses Multipeer Connectivity
- [ ] Mesh namespace support — create, join, and manage named namespaces
- [ ] PRoPHET-informed routing implemented in core (replaces pure Spray and Wait)
- [ ] `ripple daemon` Docker image published
- [ ] All Phase 1 and Phase 2 tests still pass

## Out of Scope

- Web client (Phase 4)
- Institutional management dashboard (Phase 5)
- Dense mesh interactive routing (Phase 5)

## Milestones

### Milestone 3.1 — Production CLI Daemon

Harden the Phase 1 CLI into a production-ready daemon suitable for
unattended infrastructure deployment.

**Deliverables:**
- `systemd` service unit file
- `ripple daemon --config <path>` — config file support (TOML)
- Config options: db path, identity path, relay URL, API port, log level,
  transport enable/disable flags
- Local HTTP API — REST endpoints mirroring the FFI surface for scripting
- WiFi ad-hoc transport (Linux `nl80211` via `neli` crate)
- LoRa bridge — detect Meshtastic device via serial/BT, bridge LoRa ↔ mesh
- Graceful shutdown — flush pending bundles before exit
- `ripple daemon --docker` mode for containerized deployment
- Docker image with multi-arch builds (amd64, arm64 for Raspberry Pi)
- Prometheus metrics endpoint for homelab monitoring integration

---

### Milestone 3.2 — Tauri Desktop App

Build the desktop app using Tauri with the React frontend shared partially
with the future web client.

**Deliverables:**
- Tauri project scaffold with `ripple-core` as direct Rust dependency
- `src-tauri/src/main.rs` — Tauri commands wrapping core functions
- React frontend — messaging UI, map view, settings
- System tray icon with unread message count
- Native OS notifications for incoming messages
- MapLibre GL in webview with offline tile support
- Network topology visualization — live graph of known peers, signal
  strength, transport type
- macOS Multipeer Connectivity transport
- WiFi ad-hoc transport on Linux desktop
- Settings UI — identity management, namespace management, transport
  toggles, relay configuration

---

### Milestone 3.3 — Mesh Namespaces

Implement namespace support in `ripple-core` and expose it across all
platforms.

**Deliverables:**
- `Namespace` struct — id, name, access policy, optional shared encryption key
- `AccessPolicy` enum — `Public`, `InviteOnly(Vec<pubkey>)`, `SharedSecret`
- Namespace-tagged bundles — bundles carry namespace ID, only forwarded
  within namespace
- Namespace management in CLI, desktop, and mobile apps
- QR code for namespace join (encodes namespace ID + shared secret)
- Public namespace always active by default

---

### Milestone 3.4 — PRoPHET Routing

Upgrade the routing layer from pure Spray and Wait to PRoPHET-informed
forwarding decisions.

**Deliverables:**
- Encounter probability table per peer — updated on every encounter
- Transitivity — if I often see peer B, and B often sees C, I have
  indirect probability of reaching C through B
- Aging — encounter probabilities decay over time without reinforcement
- Routing decisions use encounter probability to prefer higher-probability
  carriers when selecting spray targets
- Spray and Wait retained as fallback when no encounter history exists
- Unit tests verifying probability updates, transitivity, and aging

---

## Testing Strategy

**Infrastructure node testing** — deploy CLI daemon on a Raspberry Pi or
homelab VM and run the following scenarios:

- Mobile node out of range of another mobile node — verify CLI relay
  node bridges the gap
- CLI daemon restart — verify bundles persisted in SQLite survive restart
  and are forwarded after reconnect
- Internet outage simulation — disconnect relay, verify mesh-only delivery
  still works for in-range nodes
- LoRa bridge — Meshtastic device connected to CLI node extends range,
  verify bundle delivery over LoRa

**Desktop testing:**
- macOS ↔ iOS message exchange via Multipeer Connectivity
- Linux desktop ↔ Android via BLE (adapter permitting)
- Windows desktop ↔ Android via internet relay

---

## Definition of Done

Phase 3 is complete when:

1. A Raspberry Pi running `ripple daemon` as a systemd service successfully
   relays bundles between two mobile nodes that are out of direct range
2. The Tauri desktop app runs on all three platforms and exchanges messages
   with mobile nodes
3. Namespaces work — a private namespace message is not visible to nodes
   not enrolled in that namespace
4. PRoPHET routing demonstrably improves delivery rates over pure Spray
   and Wait in a three-node test scenario
5. `cargo test` still passes clean
