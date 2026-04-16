# Phase 2 — Mobile MVP

## Goal

Ship a working Android app that demonstrates real mesh communication over BLE,
WiFi Direct, and internet relay between two physical devices, with a functional
messaging UI and offline map. iOS follows Android, operating in a degraded but
functional mode due to background execution constraints.

A passing Phase 2 means: two Android phones with no internet connection can
discover each other via BLE, sync bundles over WiFi Direct, send and receive
encrypted direct messages, and view a shared map with pins — all without
touching a cell tower or WiFi access point. An iOS phone can join the same
mesh when the app is in the foreground.

## Success Criteria

- [ ] Android app builds and runs on a physical device
- [ ] iOS app builds and runs on a physical device
- [ ] Identity generated on first launch, persisted in Android Keystore / iOS Keychain
- [ ] QR code contact exchange with signed contact card format defined in `ripple-core`
- [ ] BLE peer discovery — devices discover each other within ~100m
- [ ] BLE small-bundle transfer (messages, SOS) over GATT
- [ ] WiFi Direct session negotiated after BLE discovery (Android)
- [ ] Bundle sync over WiFi Direct between two Android devices
- [ ] iOS Multipeer Connectivity bulk sync
- [ ] Internet relay transport integrated into Android app (reusing rendezvous server)
- [ ] Encrypted direct messaging UI — compose, send, receive, display
- [ ] Broadcast messaging — send to all nearby nodes
- [ ] SOS broadcast — priority 2, epidemic routing, distinct UI treatment
- [ ] Offline map with OpenStreetMap tiles, user-selected region pre-bundled
- [ ] Map pins — create, share via mesh, display on other devices
- [ ] Shared state sync (map pins) via CRDT over mesh transports
- [ ] Android foreground service — mesh participation persists when app backgrounded
- [ ] Battery optimization exemption prompt on first launch
- [ ] iOS background Bluetooth mode entitlement applied for with Apple
- [ ] Phase 2 audit complete — hidden panics, error handling, FFI boundary safety
- [ ] All Phase 1 CLI and rendezvous integration tests still pass
- [ ] `cargo test --workspace` passes clean

## Out of Scope

- Desktop app (Phase 3)
- Web client (Phase 4)
- Mesh namespaces (Phase 3)
- LoRa bridge (Phase 3)
- PRoPHET routing (Phase 3)
- iOS background mesh participation beyond what BGTaskScheduler + background
  Bluetooth modes support — iOS is explicitly degraded mode
- Institutional dashboard (Phase 5)
- Content-hash addressing (Phase 5)

## Prerequisites

Phase 1 must be complete and its audit series closed out. ADR-008 IDENTITY
singleton must be in place. The FFI error surface must be the expanded
eight-code set. All Phase 1 integration tests must still pass at the start of
Phase 2 — any Phase 1 regression blocks Phase 2 work.

---

## Milestones

### Milestone 2.1 — Contact Exchange Protocol (ripple-core)

Before any mobile code is written, define the contact card format in
`ripple-core` so Android and iOS both consume the same protocol. This is a
small milestone that unblocks all QR-related work later.

**Deliverables:**
- `core/src/contact.rs` — new module
- `ContactCard` struct — `ed25519_pubkey: [u8; 32]`, `x25519_pubkey: [u8; 32]`,
  `display_name: Option<String>`, `created_at: i64`, `signature: [u8; 64]`
- `ContactCard::new(identity, display_name, now)` — constructs and self-signs
- `ContactCard::verify() -> Result<(), ContactError>` — verifies signature
  against the carried Ed25519 pubkey
- `ContactCard::to_qr_bytes()` — MessagePack + base32 encoding suitable for QR
- `ContactCard::from_qr_bytes(&str)` — parse and return, but do NOT auto-verify
  (caller decides whether to trust)
- Unit tests — roundtrip, tampered pubkey detection, tampered signature
  detection, future-dated cards rejected
- FFI exposure — `mesh_contact_card_create`, `mesh_contact_card_parse`
- Integration test — a bundle can be encrypted to a peer whose keys were
  loaded from a `ContactCard::from_qr_bytes()` call
- ADR-010 — "Contact exchange via self-signed QR cards"

**Dependencies:** Phase 1 complete. No mobile work depends on this beyond
just the format being stable.

**Validation:** `cargo test -p ripple-core` adds 6+ contact card tests, all
passing.

---

### Milestone 2.2 — Android Project Scaffold

Create the Android Gradle project and its build infrastructure. No mesh
code yet — this milestone ends when a blank Android app launches on a
physical device and runs in CI.

**Deliverables:**
- `android/` directory at workspace root (parallel to `core/`, `cli/`, etc.)
- Gradle project (Kotlin DSL) — `build.gradle.kts`, `settings.gradle.kts`,
  `gradle.properties`
- Minimum SDK 26 (Android 8.0), target SDK 34
- App module: `android/app/` — single empty activity
- NDK configured — `ndk { abiFilters += listOf("arm64-v8a", "armeabi-v7a", "x86_64") }`
- `build_android.sh` at workspace root — cross-compiles `ripple-ffi` for all
  three ABIs using `cargo ndk`, copies `.so` files to
  `android/app/src/main/jniLibs/<abi>/libripple_ffi.so`
- `android/README.md` — local build instructions (Android Studio, JDK 17,
  NDK version pin, `cargo ndk` install)
- `.github/workflows/android.yml` — CI job that installs JDK + NDK + Rust,
  runs `build_android.sh`, then `./gradlew :app:assembleDebug`. Uploads APK
  as artifact.
- Cargo workspace unchanged — Android is not a Cargo member, it consumes
  `.so` files produced by the workspace
- `.gitignore` additions for Android build artifacts (`android/app/build/`,
  `android/.gradle/`, `local.properties`)

**Dependencies:** Milestone 2.1 for any eventual FFI calls, but technically
this milestone produces no FFI calls yet.

**Validation:** A blank Android app installs and launches on Tommy's physical
test device. CI passes on a PR. APK is downloadable from the CI run.

---

### Milestone 2.3 — Android / Rust FFI Bridge

Wire the Kotlin app to `ripple-ffi` via JNI. End state: a debug button in
the app calls `mesh_init` and prints the generated public key. No mesh
behavior yet, just the bridge.

**Deliverables:**
- `android/app/src/main/java/com/ripple/core/MeshCore.kt` — JNI wrapper for
  every `mesh_*` function currently in `ripple-ffi`
- `ffi/src/lib.rs` — `#[cfg(target_os = "android")]` JNI entry points that
  wrap the existing C FFI functions. The C FFI is the source of truth;
  JNI functions are thin adapters that unpack jbyteArray to `*const u8`,
  call the C FFI function, repack the output.
- Kotlin sealed class `MeshError` mapping each FFI error code
  (`ERR_NOT_INIT`, `ERR_ALREADY_INIT`, `ERR_BAD_INPUT`, `ERR_POISONED`,
  `ERR_STORE`, `ERR_CRYPTO`, `ERR_SERIALIZE`, `ERR_INTERNAL`) to a typed
  Kotlin error
- `MeshCore.init(dbPath: String, identityKey: ByteArray?): Result<Unit, MeshError>`
- Identity persistence — Android Keystore backs the 32-byte Ed25519 seed,
  wrapped in `KeystoreIdentityStore.kt`. On first launch: generate key in
  Keystore, pass seed to `mesh_init`. On subsequent launches: unwrap seed
  from Keystore, pass to `mesh_init`.
- Debug UI — single button labeled "Init mesh" that calls `MeshCore.init()`,
  then displays the public key hex in a TextView
- `mesh_free` called correctly after every FFI call that returns an output
  buffer — enforced by a `use` helper in Kotlin that mirrors
  `try-with-resources`
- Unit tests — `MeshCoreTest.kt` using Robolectric, covering init/reinit,
  error code mapping, identity Keystore roundtrip

**Dependencies:** 2.1 (contact card FFI), 2.2 (scaffold).

**Rust concepts covered (for Tommy's notes):**
- `#[no_mangle]` and `extern "C"` on JNI functions — same pattern as the
  existing C FFI, different calling convention
- `jni` crate's `JNIEnv` and how Rust manages Java references via `JObject`,
  `JString`, `JByteArray`
- Why JNI functions unpack input, call the C FFI, and repack output instead
  of reimplementing logic — single source of truth
- Keystore is a PHP analog of something like HashiCorp Vault — the app asks
  Keystore to generate and hold a key; the app never sees the raw bytes
  unless it explicitly unwraps

**Validation:** APK launches, button click shows a 64-hex-char pubkey.
Uninstalling and reinstalling the app generates a new identity. Force-stopping
and relaunching uses the same identity.

---

### Milestone 2.4 — Android Internet Relay Transport

The simplest transport to ship first because the rendezvous server already
exists from Phase 1. This milestone validates the full Rust core ↔ Kotlin
app ↔ rendezvous server path end-to-end before any radio work starts.

**Deliverables:**
- `android/app/src/main/java/com/ripple/transport/InternetRelayTransport.kt`
- OkHttp client configured for the rendezvous server
- Periodic poll loop (WorkManager, 15-minute interval in foreground,
  exponential backoff when offline)
- `submitBundle(bytes: ByteArray)` — POST to `/submit`
- `fetchInbox(pubkey: ByteArray): List<ByteArray>` — GET from
  `/inbox/<hex_pubkey>`
- Config screen — rendezvous server URL (default to Tommy's test server),
  on/off toggle
- Integration test — Android emulator + local rendezvous container,
  Android app submits a bundle, CLI daemon on host receives and decrypts it
- Two-device scenario — one Android device sends a bundle addressed to
  the other device's pubkey; the other device fetches it and decrypts
- Error handling — connection failure logs and retries, does not crash
- Network security config — HTTPS only in release builds, HTTP allowed for
  localhost rendezvous in debug builds

**Dependencies:** 2.3 (FFI bridge works).

**Validation:** Two physical Android devices with internet exchange an
encrypted direct message through the rendezvous server, displayed in a
temporary debug UI (full messaging UI comes later).

---

### Milestone 2.5 — Android Foreground Service + Lifecycle

The Android background execution story. This is arguably the hardest
engineering problem in the entire phase and deserves its own milestone
ahead of any BLE work — if the service model is wrong, every transport
built on top of it will inherit the flaw.

**Deliverables:**
- `android/app/src/main/java/com/ripple/service/MeshService.kt` — a bound
  foreground service with `startForeground()` and a persistent notification
- Service lifecycle — started on app launch, survives app backgrounding,
  continues running when the activity is destroyed
- Tick loop — calls `mesh_tick(now)` every 60 seconds, processes returned
  `Action`s. Uses `CoroutineScope(Dispatchers.IO)` + a cancellation-safe
  `while (isActive)` loop.
- `POST_NOTIFICATIONS` permission request (Android 13+)
- Battery optimization exemption — `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS`
  intent on first launch, with a rationale screen explaining why
- Wake lock strategy — partial wake lock only during `mesh_tick` execution
  and active bundle transfers, released between ticks
- Service binding from the activity — UI reads mesh state by binding to
  the service, not by calling FFI directly
- Notification UI — shows current mesh state ("Connected — 3 peers",
  "Searching for peers", "Offline")
- Handling of system-killed service — `START_STICKY`, plus a
  `JobScheduler`-based backup that re-launches the service if the system
  reaped it
- Boot receiver — optional, mesh service auto-starts on device boot if
  the user enabled it in settings
- Tests — service lifecycle tests under Robolectric, tick loop cancellation
  test

**Dependencies:** 2.4 (internet relay works, so there's something worth
keeping alive in the background).

**Rust concepts covered:**
- Nothing Rust-specific this milestone; this is entirely Kotlin and Android
  platform work. The Rust core is just called from the service's tick loop.

**Validation:** Two Android devices with the app backgrounded successfully
exchange a message via internet relay. Closing the app's activity (swiping
it from recents) does not stop the service; pulling the notification shade
shows it still running.

---

### Milestone 2.6 — Android BLE Transport (Discovery)

BLE peer discovery only — no bundle transfer yet. This milestone ends when
two devices can see each other and log a `mesh_peer_encountered` call with
each other's pubkeys.

**Deliverables:**
- `android/app/src/main/java/com/ripple/transport/BleTransport.kt`
- BLE advertiser — advertises a Ripple service UUID + truncated Ed25519
  pubkey in the advertisement payload (limited to 31 bytes total on BLE 4.x;
  extended advertising for 5.0+ optional)
- BLE scanner — continuous scan with `ScanSettings.SCAN_MODE_LOW_POWER` when
  in background, `SCAN_MODE_BALANCED` in foreground
- Permissions — `BLUETOOTH_SCAN`, `BLUETOOTH_ADVERTISE`, `BLUETOOTH_CONNECT`,
  `ACCESS_FINE_LOCATION` (required by Android for BLE scan until API 31)
- Runtime permission flow — clear rationale screen, graceful handling of
  permission denial
- Peer deduplication — same pubkey seen multiple times in a scan window is
  one encounter, not many
- Integration with `mesh_peer_encountered` — on each unique peer discovery,
  call the FFI with pubkey, `Transport::Ble (0)`, RSSI, current timestamp
- `SyncOffer` handling — the FFI call returns a `SyncOffer` listing bundle
  IDs the peer wants; this milestone just logs them (transfer comes in 2.7)
- Unit tests — BLE scan result parsing, advertisement payload encoding,
  permission state machine
- Manual test scenario — two devices in the same room log each other's
  pubkeys within 10 seconds

**Dependencies:** 2.3 (FFI), 2.5 (foreground service — BLE scanning must
run in the service, not the activity).

**Known Android quirks to handle:**
- Manufacturer BLE stack differences — Samsung advertises differently than
  Pixel; Xiaomi aggressively kills background scans; test on at least two
  manufacturers
- Android 12+ `BLUETOOTH_SCAN` permission with `neverForLocation` flag
- Scan filter limits — Android caps the number of active scan filters; use
  service UUID filtering to reduce callback volume

**Validation:** Two Android devices (preferably different manufacturers)
advertise and discover each other via BLE, logging each other's pubkeys
to the service log.

---

### Milestone 2.7 — Android BLE Transport (Bundle Transfer)

Extend BLE transport to actually transfer small bundles over GATT. This
closes the loop on offline messaging for short payloads.

**Deliverables:**
- GATT server implementation — a Ripple service with two characteristics:
  `BUNDLE_SUBMIT` (write) and `BUNDLE_POLL` (read + notify)
- GATT client — connects to discovered peer's GATT server, reads/writes
  characteristics
- Bundle fragmentation — BLE MTU is typically 20–512 bytes; bundles >500 bytes
  are chunked with a simple length-prefix framing protocol
- Flow — after BLE discovery (from 2.6), exchange `SyncOffer`s via a small
  handshake characteristic, then transfer needed bundles via `BUNDLE_SUBMIT`
- `mesh_bundle_received` called for every inbound bundle
- `mesh_bundle_forwarded` called after every successful outbound transfer
  (decrements spray count for Spray and Wait)
- Connection lifecycle — GATT connections are short-lived; connect, sync,
  disconnect within ~10 seconds to save battery
- Concurrency — only one GATT connection at a time per peer, serialized
  via a Kotlin `Mutex`
- Integration test — two emulators with a BLE bridge harness exchange a
  small bundle
- Physical device test — two phones in airplane mode (WiFi + cell off,
  Bluetooth on) exchange a direct message

**Dependencies:** 2.6 (discovery works).

**Throughput note:** BLE 4.x realistic throughput is 5–20 KB/s. A 200-byte
message transfers in well under a second; a 64KB bundle would take several
seconds and is a candidate for WiFi Direct handoff (Milestone 2.8).

**Validation:** Two physical devices in airplane mode exchange an encrypted
direct message. The receiving device decrypts and displays the plaintext
via a temporary debug UI.

---

### Milestone 2.8 — Android WiFi Direct Transport

Bulk bundle sync over WiFi Direct. Triggered after BLE discovery when two
peers have more data to exchange than BLE can carry efficiently.

**Deliverables:**
- `android/app/src/main/java/com/ripple/transport/WifiDirectTransport.kt`
- WiFi Direct peer discovery using `WifiP2pManager` — runs as a secondary
  discovery channel (BLE is primary)
- Handoff logic — after BLE discovery + initial `SyncOffer` exchange, if the
  offered bundle total exceeds 4KB, initiate WiFi Direct handshake
- Group owner negotiation — which device hosts the socket
- TCP socket on the group owner, client on the other — port 8988 per convention
- Bundle sync protocol — same `SyncOffer` / bundle transfer format as BLE,
  but over a raw socket without fragmentation
- Session timeout — disconnect after 30 seconds of idle or after sync completes
- `Transport::WifiDirect (1)` used in `mesh_peer_encountered` — lets the
  router record that this peer has WiFi Direct capability
- Fallback — if WiFi Direct negotiation fails, fall back to BLE chunked
  transfer without dropping the bundle
- Physical device test — two phones sync 100 bundles (each ~2KB) in under
  30 seconds via WiFi Direct

**Dependencies:** 2.7 (BLE transfer works as fallback).

**Known Android quirks:**
- WiFi Direct kills existing WiFi connection on some devices — document this
  in settings with a warning
- `WifiP2pManager.discoverPeers` is unreliable on some manufacturers; retry
  with backoff
- Group owner intent negotiation is racy — both devices call `createGroup`
  in parallel and one wins; handle the loser reconnecting as a client

**Validation:** Two Android devices sync a batch of 100 map pin bundles
within 30 seconds with WiFi off and cellular off — validates pure-mesh
bulk sync.

---

### Milestone 2.9 — Android Messaging UI

The first real user-facing milestone. All the transport work from 2.4–2.8
is now exposed through an interface users can actually use.

**Deliverables:**
- Compose Multiplatform UI (or Jetpack Compose — same thing for single-platform)
- Contact list screen — shows known peers by `display_name` (from ContactCard)
  or fallback to truncated pubkey
- Conversation view — sent and received messages with delivery status
  badges (queued, spraying, delivered, failed)
- Message compose — text input, send button, priority selector (Normal / Urgent)
- Broadcast compose — separate screen, sends to `Destination::Broadcast`
- SOS screen — single large button, confirms before sending, attaches current
  GPS coordinates as payload metadata, sends as `Priority::Sos`
- QR display screen — renders the current identity's `ContactCard` as a QR code
- QR scanner screen — scans a QR, parses `ContactCard::from_qr_bytes`,
  verifies signature, prompts user to confirm add
- Message status polling — UI observes bundle state changes (via a Kotlin
  `Flow` fed from the service) and updates status badges
- Unread count badge on the app icon and notification
- Accessibility — TalkBack labels, minimum touch target sizes, high-contrast
  SOS button

**Dependencies:** 2.7 for offline messaging, 2.3 for FFI, 2.1 for contact cards.

**Validation:** A user can: install the app, scan another user's QR, message
them, receive a reply, and see delivery status change as the bundle moves
through the routing state machine. SOS broadcast works with airplane mode on.

---

### Milestone 2.10 — Android Offline Maps

MapLibre integration with pre-bundled offline tiles and pin CRDT sync.

**Deliverables:**
- MapLibre GL Android SDK dependency (`org.maplibre.gl:android-sdk`)
- Tile pack download flow — user selects a region (Texas, Bay Area, NYC,
  custom), app downloads an MBTiles file from a tile server (MapTiler free
  tier initially; document tile license attribution)
- Tile storage — MBTiles file in app's internal storage, offline-first
  rendering
- Map screen — renders tiles, shows user's GPS location, renders pins
- Pin creation — long-press the map, select category (shelter, medical,
  hazard, resource), add optional label, save
- Pin storage — pins stored as entries in an `ORSet` within `SharedState`
  (existing CRDT from `core/src/crdt.rs`)
- Pin broadcast — new pin is serialized into a bundle with
  `Destination::Broadcast`, `Priority::Normal`, payload is the CRDT delta
- Pin receive — incoming shared-state bundles merged into local state via
  `SharedState::merge`, map re-renders
- GPS permission — `ACCESS_FINE_LOCATION` runtime request with rationale
- Pin list view — shows all pins sortable by distance, with filters per
  category
- Tile license attribution — required MapTiler / OpenStreetMap attribution
  visible on map

**Dependencies:** 2.9 (messaging UI shell), 2.8 (WiFi Direct for bulk pin
sync when a new peer joins a mesh with many pins).

**Validation:** Device A creates a pin while offline. Device B comes into
range and receives the pin via BLE or WiFi Direct within 30 seconds.
Device C, which joins the mesh later, receives the full pin set via WiFi
Direct bulk sync on first encounter with device B.

---

### Milestone 2.11 — iOS Project Scaffold + FFI

iOS equivalent of 2.2 + 2.3 combined. Xcode project, XCFramework build,
Swift FFI wrapper, identity in Keychain, debug button showing pubkey.

**Deliverables:**
- `ios/` directory at workspace root
- Xcode project (SwiftUI app template, iOS 16+ target)
- `build_ios.sh` at workspace root — compiles `ripple-ffi` for
  `aarch64-apple-ios` (device), `aarch64-apple-ios-sim` (Apple Silicon
  simulator), `x86_64-apple-ios` (Intel simulator), packages into an
  XCFramework with a Swift module map
- `ios/Ripple/MeshCore/MeshCore.swift` — Swift wrapper around all C FFI
  functions, with typed errors via a `MeshError` enum matching Kotlin's
- `Data` ↔ `[u8]` marshaling helpers — Swift `Data.withUnsafeBytes` patterns
  replacing Kotlin's `ByteArray`
- Keychain-backed identity — `KeychainIdentityStore.swift`, using
  `kSecClassKey` with `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`
- Debug UI — SwiftUI view with "Init mesh" button showing the generated
  pubkey
- `.github/workflows/ios.yml` — CI job on `macos-latest`, installs Rust
  iOS targets, runs `build_ios.sh`, then `xcodebuild` for the app. Does
  not archive or code-sign — debug build only.
- `ios/README.md` — local build instructions (Xcode version, Rust targets,
  simulator setup)

**Dependencies:** 2.1 (contact card FFI), 2.3 (establishes the FFI pattern
that Swift mirrors).

**Rust concepts covered:**
- XCFramework vs plain static library — why iOS needs the XCFramework
  format for multi-architecture binaries
- Differences between Android's JNI (VM-mediated) and iOS's C FFI (direct
  library calls) — iOS is closer to the CLI daemon's use of the core
- Swift's `Unmanaged` vs JNI's `JObject` — both are "here's a pointer to
  native memory" with different safety stories

**Validation:** iOS app launches on both simulator and physical device,
Init button displays the pubkey. CI passes.

---

### Milestone 2.12 — iOS BLE + Multipeer Transports

iOS transport stack — CoreBluetooth for BLE, Multipeer Connectivity for
bulk sync (iOS's Apple-approved replacement for WiFi Direct). Also
integrates the internet relay transport.

**Deliverables:**
- `ios/Ripple/Transport/BLETransport.swift` — CoreBluetooth `CBCentralManager`
  (scanning) + `CBPeripheralManager` (advertising)
- BLE service UUID matches Android's — iOS ↔ Android BLE interop is a
  hard requirement
- GATT characteristics mirror Android's — same `BUNDLE_SUBMIT` + `BUNDLE_POLL`
  UUIDs
- `ios/Ripple/Transport/MultipeerTransport.swift` — `MCSession`,
  `MCNearbyServiceAdvertiser`, `MCNearbyServiceBrowser`
- iOS-to-iOS bulk sync via Multipeer; iOS-to-Android bulk sync falls back
  to BLE (slower but works)
- `ios/Ripple/Transport/InternetRelayTransport.swift` — URLSession-based
  client for the rendezvous server, mirrors Android's OkHttp client
- Background modes — `bluetooth-central`, `bluetooth-peripheral`,
  `processing` in `Info.plist`
- BGTaskScheduler integration — periodic `mesh_tick` invocations while
  backgrounded (iOS gives no guarantees about frequency — documented as
  best-effort)
- Apple entitlement application — documented process to apply for
  extended background BLE capabilities
- Scanning limitations — iOS cannot advertise service UUIDs in the
  background unless foregrounded recently; documented as a known limitation
- Physical device test — iOS device discovers and exchanges a bundle with
  an Android device via BLE

**Dependencies:** 2.11 (iOS FFI + scaffold), 2.7 (Android BLE so there's
something to interop with).

**iOS degraded mode documentation:** a clear section in `docs/architecture/`
explaining what works vs. doesn't work on iOS compared to Android — no
surprises for users, no surprises for future contributors. Include:
- Background BLE advertising requires the app to have been foregrounded
  recently
- WiFi Direct is impossible on iOS
- BGAppRefreshTask gives ~30 seconds every few hours at the system's
  discretion

**Validation:** iOS device exchanges a message with an Android device via
BLE, both with apps in the foreground. iOS ↔ iOS message exchange works
via Multipeer. iOS device participates in the internet relay mesh
regardless of foreground/background state.

---

### Milestone 2.13 — iOS Messaging UI + Maps

iOS equivalents of 2.9 and 2.10 combined. Feature parity with Android
for everything that iOS can actually do.

**Deliverables:**
- SwiftUI messaging UI — contact list, conversation, compose, broadcast,
  SOS, matching Android's information architecture
- QR code display — uses `CIFilter.qrCodeGenerator`
- QR scanner — `AVCaptureSession` with a QR detection metadata output
- Contact card parsing via `MeshCore.parseContactCard` (FFI from 2.1)
- MapLibre GL iOS SDK (`MapLibre` via SPM)
- Tile pack download flow — identical region selection and tile sources
  as Android
- Pin CRUD with long-press gesture
- Pin sync via mesh transports (the transports from 2.12 handle bundle
  delivery; this milestone only adds the UI)
- Accessibility — VoiceOver labels, Dynamic Type support, high-contrast
  SOS button
- Tile license attribution visible on map

**Dependencies:** 2.12 (transports working), 2.10 (map pin protocol
defined and validated on Android — iOS just re-uses it).

**Validation:** An iPhone can install the app, scan another user's QR
(Android or iOS), send a direct message, receive a reply, create a map
pin, and see pins from other devices appear on the map. SOS broadcast
works with the app in the foreground.

---

### Milestone 2.14 — Phase 2 Audit

Parallel to the pre-Phase-2 audit series that closed out Phase 1. This
milestone is not additional features — it is a deliberate pause to harden
what was built in 2.1–2.13 before Phase 3.

**Deliverables:**
- **FFI boundary audit** — every C FFI function and every JNI function
  re-reviewed for: panic safety on malformed input, correct memory
  management (every `*mut u8` has a matching `mesh_free`), correct
  integer widths across the boundary (Rust `i64` ↔ Java `long`
  ↔ Swift `Int64`)
- **Android service resilience audit** — behavior under Doze mode, app
  standby, background-restricted state, forced-stop, system-killed service
- **iOS background behavior audit** — document what actually works vs. what
  is advertised; user-facing docs calibrated to match
- **BLE throughput and battery audit** — measured battery drain over 24 hours
  of idle mesh participation; must be <5% on a modern Android device
- **Cryptographic audit** — pubkey handling at every transport boundary.
  Specific check: every place Android or iOS code handles a raw 32-byte
  array, it is clearly labeled as Ed25519 or X25519 and never passed to a
  function expecting the other. This is the ADR-006 invariant extended to
  Kotlin and Swift.
- **Error propagation audit** — FFI error codes are surfaced to the UI
  when actionable (user needs to reconnect) and silently logged when not
  (transient mesh chatter)
- **Rust core regression test** — full Phase 1 test suite must still pass.
  If any Phase 2 work introduced a regression in `ripple-core`, it must
  be fixed before Phase 2 can close.
- **Audit notes committed to `docs/engineering/audits/phase-2-audit.md`**
  in the same format as the pre-Phase-2 audits

**Dependencies:** 2.13 (everything shipped).

**Validation:** Audit document merged, all critical and high-severity
findings resolved, medium-severity findings either resolved or documented
as "accepted risk for Phase 2" with a tracking issue.

---

## Testing Strategy

**Physical device testing is required from Phase 2 onward.** BLE and WiFi
Direct behavior cannot be meaningfully tested on emulators. Multipeer
Connectivity does not exist on the iOS simulator.

**Minimum test device matrix:**
- Two Android devices, different manufacturers (recommend one Pixel + one
  Samsung or Xiaomi — BLE stack differences are real)
- One iOS device (iPhone 12 or newer — older models have unreliable
  CoreBluetooth background behavior)
- Mixed Android + iOS pair for interop testing

**Test scenarios per milestone are specified in each milestone above.**
The full Phase 2 validation matrix — run at 2.14 audit — is:

| Scenario | Devices | Network | Expected result |
|---|---|---|---|
| Internet-only messaging | 2× Android | WiFi + cell on | Message delivered via rendezvous |
| Offline direct message | 2× Android | Airplane mode | Message delivered via BLE |
| Bulk pin sync | 2× Android | Airplane mode, 50 pins queued | Full sync in <60s via WiFi Direct |
| iOS foreground BLE | iOS + Android | Airplane mode | Message delivered via BLE |
| iOS background relay | iOS + Android | Internet | Message delivered via rendezvous regardless of iOS foreground state |
| Mixed mesh | 2× Android + 1× iOS | Airplane mode | All three devices exchange messages and pins |
| Battery drain | 1× Android | Idle, 24h | <5% battery drain from mesh service |
| SOS epidemic | 3× devices | Airplane mode | SOS broadcast reaches all devices within 2 minutes |
| Backgrounded delivery | 2× Android | Internet, apps backgrounded | Message delivered within 15 minutes |

---

## Definition of Done

Phase 2 is complete when:

1. Two Android devices with airplane mode on can exchange an encrypted
   direct message and sync a map pin
2. An iPhone can participate in the same mesh in foreground mode and via
   internet relay when backgrounded
3. The Android foreground service keeps the mesh active when the app is
   backgrounded, with <5% battery drain over 24 hours of idle operation
4. `cargo test --workspace` passes clean — zero Phase 1 regressions
5. Phase 2 audit (Milestone 2.14) is merged with no outstanding
   critical or high-severity findings
6. Both Android APK and iOS IPA build artifacts are produced by CI on
   every push to `main`
7. All physical-device test scenarios in the validation matrix pass
