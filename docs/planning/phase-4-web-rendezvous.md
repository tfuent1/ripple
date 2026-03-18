# Phase 4 — Web Client and Rendezvous Infrastructure

## Goal

Ship the web client and harden the rendezvous server into production
infrastructure. The web client removes the last barrier to zero-install mesh
participation — anyone with a browser can join the mesh through a local CLI
node or internet relay without downloading anything.

A passing Phase 4 means: a user with no Ripple installation can open a browser,
navigate to a Ripple web client, connect to either a local CLI node on their
network or the public rendezvous server, and participate in the mesh.

## Success Criteria

- [ ] WASM build of `ripple-core` compiles and runs in browser
- [ ] Web client connects to local CLI node HTTP API
- [ ] Web client connects to rendezvous server via WebSocket
- [ ] Web client messaging UI — send and receive messages
- [ ] Web client map view with OpenStreetMap tiles
- [ ] Client-side crypto — private keys generated and stored in browser,
      never sent to server
- [ ] Web client installable as PWA
- [ ] Rendezvous server handles concurrent connections without degradation
- [ ] Rendezvous server bundle expiry runs reliably
- [ ] Rendezvous server rate limiting — prevent bundle spam
- [ ] Rendezvous server deployable via Docker Compose
- [ ] Rendezvous server metrics and health endpoint
- [ ] Public rendezvous server instance deployed
- [ ] End-to-end test: browser client ↔ Android device via rendezvous relay
- [ ] All previous phase tests still pass

## Out of Scope

- Dense mesh interactive routing (Phase 5)
- Institutional management dashboard (Phase 5)
- Content-addressed bundles (Phase 5)
- Mesh DNS (Phase 5)

## Milestones

### Milestone 4.1 — WASM Core

Compile `ripple-core` to WebAssembly and expose a clean JavaScript API.

**Deliverables:**
- `web/mesh-core-wasm/` crate with `wasm-bindgen` bindings
- `wasm_create_bundle(payload, priority, keypair_bytes) -> Uint8Array`
- `wasm_verify_bundle(bundle_bytes) -> bool`
- `wasm_decrypt_bundle(bundle_bytes, keypair_bytes) -> Uint8Array`
- `wasm_generate_identity() -> Uint8Array` — returns 64-byte keypair
- `wasm_merge_shared_state(state_a, state_b) -> Uint8Array` — CRDT merge
- `wasm-pack` build pipeline producing npm-compatible package
- Browser performance benchmark — bundle creation and verification latency

**Crates to add:**
```toml
wasm-bindgen = "0.2"
js-sys = "0.3"
web-sys = { version = "0.3", features = ["Window", "Storage"] }
```

---

### Milestone 4.2 — Web Client

Build the React web application. Shares UI components and patterns with
the Tauri desktop frontend.

**Deliverables:**
- Vite + React + TypeScript project scaffold
- Identity management — generate keypair on first visit, store in
  IndexedDB (private key never leaves browser)
- Connection modes:
  - Local mode — connect to CLI node HTTP API on LAN
  - Relay mode — connect to rendezvous server via WebSocket
- Messaging UI — conversations, compose, send, receive
- Map view — MapLibre GL with OSM tiles, pins, incoming pin display
- QR code display for identity and contact sharing
- PWA manifest and service worker for installability
- Responsive design — usable on mobile browser as fallback

---

### Milestone 4.3 — Rendezvous Server Hardening

Harden the Phase 1 rendezvous server stub into production infrastructure.

**Deliverables:**
- WebSocket support for real-time bundle push to connected clients
- Rate limiting — max bundles per pubkey per hour
- Bundle size limits
- Spam detection — duplicate bundle ID rejection
- Automatic TTL expiry with configurable cleanup interval
- Metrics endpoint (Prometheus format)
- Health check endpoint
- Structured logging
- Docker Compose file with optional nginx reverse proxy
- Deployment documentation
- Load test — verify handles 1000 concurrent connections

---

### Milestone 4.4 — Public Infrastructure

Deploy and operate the first public Ripple rendezvous server.

**Deliverables:**
- VPS deployment (Hetzner or equivalent — low cost, EU and US regions)
- Domain and TLS via Let's Encrypt
- Monitoring and alerting
- Uptime SLA target defined
- Privacy policy — document that server stores only encrypted opaque bytes,
  retains nothing after TTL, logs no IP addresses beyond rate limiting
- Status page

---

## Testing Strategy

**Browser compatibility matrix:**
- Chrome / Chromium (primary — best WASM and WebBluetooth support)
- Firefox
- Safari (most restricted — no WebBluetooth, WASM only)
- Mobile Chrome on Android
- Mobile Safari on iOS

**End-to-end scenarios:**
- Browser client sends message, Android receives via rendezvous relay
- Browser client on same LAN as CLI node — connects via local HTTP API,
  receives message from nearby mobile node
- PWA installed on Android — verify works offline when connected to local
  CLI node
- Two browser clients exchange messages via rendezvous server

**Load testing:**
- 1000 concurrent WebSocket connections to rendezvous server
- 10,000 bundle submissions in 60 seconds — verify rate limiting triggers
  correctly
- Bundle expiry under load — verify TTL cleanup doesn't block message handling

---

## Definition of Done

Phase 4 is complete when:

1. A browser with no Ripple installation exchanges an encrypted message with
   an Android device via the public rendezvous server
2. The rendezvous server passes the load test
3. The PWA installs and works on Android Chrome
4. The public rendezvous server is deployed and monitored
5. `cargo test` still passes clean
