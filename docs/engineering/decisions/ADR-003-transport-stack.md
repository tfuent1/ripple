# ADR-003: Multi-Transport Mesh Stack

## Status
Accepted

## Context
Physical message delivery between mesh nodes can use multiple underlying radio
technologies, each with different range, bandwidth, power, and platform availability
characteristics. The question was whether to standardize on a single transport or
support multiple transports simultaneously.

Transport options evaluated:

| Transport | Range | Bandwidth | Power | iOS | Android | Desktop | CLI |
|---|---|---|---|---|---|---|---|
| BLE | ~100m | Low | Low | ✅ | ✅ | ⚠️ | ⚠️ |
| WiFi Direct | ~250m | High | Medium | ❌ | ✅ | ❌ | ❌ |
| WiFi Aware (NAN) | ~250m | High | Medium | ❌ | ✅ (8+) | ❌ | ❌ |
| Multipeer Connectivity | ~250m | High | Medium | ✅ | ❌ | ✅ (Mac) | ❌ |
| WiFi Ad-hoc (IBSS) | ~500m+ | High | Medium | ❌ | ❌ | ✅ | ✅ |
| Internet relay | Unlimited | High | Low | ✅ | ✅ | ✅ | ✅ |
| LoRa (Meshtastic bridge) | Miles | Very low | Very low | ⚠️ | ⚠️ | ✅ | ✅ |

## Decision
Support all viable transports simultaneously. The DTN routing layer treats all
transports as interchangeable pipes and selects the best available transport per
peer based on availability, estimated bandwidth, and power cost.

The transport abstraction is a simple Rust trait:
```rust
trait MeshTransport {
    fn start_discovery(&mut self);
    fn connect_to_peer(&mut self, peer: &PeerInfo);
    fn send_bundle(&mut self, peer: &PeerInfo, bundle: &[u8]);
    fn on_bundle_received(&self) -> Option<(PeerInfo, Vec<u8>)>;
    fn estimated_bandwidth(&self) -> u32;
    fn estimated_range(&self) -> u32;
}
```

Transport selection strategy:
- **BLE** runs continuously for peer discovery and small bundles (beacons, SOS)
- **WiFi Direct / Multipeer** negotiated on-demand for bulk sync when a peer
  is discovered via BLE
- **WiFi Aware** used where available as a faster discovery alternative to BLE
- **Internet relay** used opportunistically — if a node has connectivity, it
  pushes queued bundles to the rendezvous server
- **LoRa bridge** treated as another transport if a Meshtastic device is
  connected via Bluetooth serial

## Consequences

**Positive:**
- Maximizes mesh connectivity — more transport options means more paths between nodes
- Graceful degradation — losing one transport doesn't break the mesh
- Internet relay dramatically improves delivery rates without requiring end-to-end
  offline paths
- LoRa bridge integration gives access to Meshtastic's existing hardware ecosystem
  and extended range without requiring users to have the hardware

**Negative:**
- Significantly more implementation complexity than a single transport
- Transport availability varies dramatically by platform — iOS cannot use WiFi Direct,
  Android cannot use Multipeer Connectivity
- Testing the full transport matrix requires physical devices across platforms
- Power management becomes complex when multiple radios are active simultaneously
- BLE background behavior on iOS remains a fundamental constraint regardless of
  how many transports are supported
