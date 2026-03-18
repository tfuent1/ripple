# Phase 2 — Mobile MVP

## Goal

Ship a working Android app that demonstrates real mesh communication over BLE
and WiFi Direct between two physical devices, with a functional messaging UI
and offline map. iOS follows Android, operating in a degraded but functional
mode due to background execution constraints.

A passing Phase 2 means: two Android phones with no internet connection can
discover each other via BLE, sync bundles over WiFi Direct, send and receive
encrypted direct messages, and view a shared map with pins — all without
touching a cell tower or WiFi access point.

## Success Criteria

- [ ] Android app builds and runs on a physical device
- [ ] Identity generated on first launch, persisted in Android Keystore
- [ ] BLE peer discovery — devices discover each other within ~100m
- [ ] WiFi Direct session negotiated after BLE discovery
- [ ] Bundle sync over WiFi Direct between two devices
- [ ] Encrypted direct messaging UI — compose, send, receive, display
- [ ] Broadcast messaging — send to all nearby nodes
- [ ] SOS broadcast — priority 2, epidemic routing, distinct UI treatment
- [ ] Offline map with OpenStreetMap tiles pre-bundled by region
- [ ] Map pins — create, share via mesh, display on other devices
- [ ] Shared state sync (map pins) via CRDT over WiFi Direct
- [ ] Android foreground service — mesh participation persists when app backgrounded
- [ ] Battery optimization exemption prompt on first launch
- [ ] iOS app builds and runs on a physical device
- [ ] iOS BLE peer discovery (foreground only initially)
- [ ] iOS Multipeer Connectivity for bundle sync
- [ ] iOS background Bluetooth modes applied for
- [ ] Contact exchange via QR code scan
- [ ] All Phase 1 CLI integration tests still pass

## Out of Scope

- Desktop app (Phase 3)
- Web client (Phase 4)
- Mesh namespaces (Phase 3)
- LoRa bridge (Phase 3)
- PRoPHET routing (deferred to Phase 3+)
- Institutional dashboard (Phase 5)

## Milestones

### Milestone 2.1 — Android Transport Layer

Implement BLE and WiFi Direct transports as native Kotlin modules that
conform to the `MeshTransport` interface defined by the FFI surface.

**Deliverables:**
- `BLETransport.kt` — scanning, advertising, peer discovery, small bundle
  transfer over GATT
- `WifiDirectTransport.kt` — peer connection negotiation, bulk bundle sync
  over WiFi Direct socket
- `TransportRouter.kt` — selects transport per peer, manages handoff from
  BLE discovery to WiFi Direct sync
- Android foreground service with persistent notification
- Battery optimization exemption prompt
- Unit tests with mocked BLE and WiFi Direct interfaces

---

### Milestone 2.2 — Android / Rust FFI Integration

Wire the Kotlin app to `ripple-ffi`. The Android app delegates all mesh
logic to the Rust core via JNI.

**Deliverables:**
- `MeshCore.kt` — JNI wrapper for all FFI functions
- `build_android.sh` — cross-compiles `ripple-ffi` for all Android ABIs
  (arm64-v8a, armeabi-v7a, x86_64)
- Compiled `.so` files committed to `android/app/src/main/jniLibs/`
- Memory management — correct `mesh_free` calls after every FFI round-trip
- Error code mapping from FFI `i32` returns to Kotlin sealed classes

---

### Milestone 2.3 — Android Messaging UI

Implement the core messaging interface. Deliberately simple — function
over form at this stage.

**Deliverables:**
- Contact list — known peers by public key, aliased by display name after
  QR exchange
- Conversation view — sent and received messages with delivery status
  (queued, spraying, delivered)
- Compose and send direct message
- Broadcast message — sends to all mesh participants
- SOS screen — single large button, sends epidemic priority broadcast with
  GPS coordinates
- Message status indicators reflecting bundle state from the routing layer
- QR code display and scanner for contact exchange

---

### Milestone 2.4 — Offline Maps

Integrate MapLibre GL with pre-bundled OpenStreetMap tiles.

**Deliverables:**
- MapLibre GL Android SDK integrated
- Region tile pack download on first launch (user selects their region)
- Tile storage and cache management
- Map pins — create a pin with label and category (shelter, medical,
  hazard, resource)
- Pin sharing — pins serialized as bundles, broadcast to mesh
- Incoming pin display — pins received from mesh rendered on map
- GPS location display
- Pin persistence via CRDT shared state

---

### Milestone 2.5 — iOS App

Implement the iOS counterpart. Shares no code with Android beyond the
Rust core via FFI.

**Deliverables:**
- `MeshCore.swift` — Swift FFI wrapper
- `build_ios.sh` — compiles `ripple-ffi` to XCFramework
- `BLETransport.swift` — CoreBluetooth scanning and advertising
- `MultipeerTransport.swift` — Multipeer Connectivity session management
- Background Bluetooth mode entitlement applied for with Apple
- SwiftUI messaging UI matching Android feature set
- MapLibre GL iOS SDK with offline tiles
- QR contact exchange

---

## Testing Strategy

**Physical device testing is required from this phase forward.** BLE and
WiFi Direct behavior cannot be meaningfully tested on emulators.

Minimum test matrix:
- Two Android devices (different manufacturers preferred — BLE behavior
  varies significantly between vendors)
- One iOS device
- Mixed Android/iOS pair

**Test scenarios:**
- Two Android devices, no internet, exchange direct message via BLE + WiFi Direct
- Android + iOS, no internet, exchange message via BLE
- All three devices, one with internet, verify rendezvous relay still works
- Background sync — app backgrounded on both devices, message still delivers
- SOS broadcast — verify epidemic propagation reaches all nodes
- Map pin sync — create pin on device A, verify appears on device B

---

## Definition of Done

Phase 2 is complete when:

1. Two Android devices with airplane mode on can exchange an encrypted direct
   message and sync a map pin
2. An iOS device can participate in the same mesh in foreground mode
3. The Android foreground service keeps mesh active when app is backgrounded
4. `cargo test` still passes clean
5. Physical device test scenarios above all pass
