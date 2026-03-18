# Phase 5 — Dense Mesh Services

## Goal

Unlock the application layer that becomes viable at high mesh density.
Implement interactive routing for dense networks, content-addressed bundles,
mesh naming, and the institutional deployment tooling that enables hospitals,
universities, and enterprises to deploy Ripple as deliberate infrastructure.

Phase 5 has no fixed endpoint — it is an ongoing expansion of what the
protocol can do as adoption grows. The milestones below represent the first
meaningful set of dense-mesh capabilities.

## Success Criteria

- [ ] Interactive routing mode implemented and automatically selected at
      sufficient mesh density
- [ ] Density metrics collected and exposed via CLI and desktop UI
- [ ] Content-addressed bundle type implemented
- [ ] Content propagates and caches across nearby nodes automatically
- [ ] Mesh naming — human-readable names resolve to public keys via
      distributed DHT
- [ ] Mesh browser — fetch and display content served by mesh nodes
- [ ] Institutional deployment tooling — enrollment, management dashboard,
      coverage mapping
- [ ] Fixed relay node firmware image for dedicated hardware
- [ ] All previous phase tests still pass

## Out of Scope for Phase 5 Initial Release

- Mesh-native payments
- Video streaming
- Full internet protocol compatibility (not a goal — Ripple is a complement
  to the internet, not a replacement for HTTP)

## Milestones

### Milestone 5.1 — Interactive Routing

Implement the dense-network routing mode described in ADR-004. The routing
layer currently operates exclusively in DTN store-and-forward mode. At high
density, continuous paths exist and interactive routing dramatically reduces
latency.

**Deliverables:**
- Density metrics — encounter frequency, average delivery latency,
  hop count distribution, collected passively by routing layer
- Automatic mode selection — transition to interactive routing when density
  metrics exceed configurable thresholds
- Path discovery — lightweight flooding with path accumulation, similar to
  AODV (Ad hoc On-Demand Distance Vector)
- Path caching — discovered paths cached with TTL, invalidated on delivery
  failure
- Graceful fallback — if interactive routing fails (path breaks), bundle
  falls back to DTN mode automatically
- Metrics exposed via CLI `ripple status --density` and desktop topology view

---

### Milestone 5.2 — Content-Addressed Bundles

Implement the content addressing bundle destination type introduced in the
bundle schema (`BundleDestination::ContentHash`).

**Deliverables:**
- `ContentHash` destination type — bundle is addressed by BLAKE2b hash of
  its payload
- Request bundle type — "I want the content with this hash, please send it
  to me"
- Automatic caching — nodes that have fetched content keep a copy and
  serve it to future requesters
- Cache eviction policy — LRU with configurable size limit
- Popular content naturally replicates to nodes where it is frequently
  requested
- Use case validation — emergency alert published once, propagates to
  thousands of nodes automatically

---

### Milestone 5.3 — Mesh Naming

Implement a distributed naming layer that maps human-readable names to
public keys without a central DNS server.

**Deliverables:**
- `MeshName` struct — name string, public key, timestamp, self-signature
- Name registration — broadcast a signed name claim to the mesh
- Name resolution — query the mesh for a name, receive the registered
  public key
- Conflict resolution — earlier registration wins, verified by signature
  timestamp
- Name propagation via CRDT — name registrations replicate across nodes
  using the existing shared state infrastructure
- CLI: `ripple name register <name>`
- CLI: `ripple name resolve <name>`
- Name display in messaging UI — show registered names instead of truncated
  public keys

---

### Milestone 5.4 — Mesh Browser

A minimal content browser that fetches and displays content served by mesh
nodes, addressed by mesh name or content hash.

**Deliverables:**
- Content serving — CLI nodes can serve static content (HTML, text, JSON)
  addressed by their mesh name
- `ripple serve <path>` — serves files from a directory as mesh content
- Mesh browser UI in desktop app and web client — enter a mesh name,
  fetch and display the content
- Content types: plain text, markdown (rendered), simple HTML
- Use cases: community bulletin boards, local business listings, emergency
  information, resource availability posts

---

### Milestone 5.5 — Institutional Deployment Tooling

Everything needed for a hospital, university, or enterprise to deploy
Ripple as deliberate infrastructure.

**Deliverables:**
- Management dashboard — web UI for IT administrators
  - Enrolled device list and status
  - Mesh topology map — node positions, signal strength, coverage gaps
  - Bundle delivery statistics
  - Namespace management
  - Node health monitoring
- Enrollment flow — generate namespace join QR code, scan on staff device,
  device joins private namespace
- Fixed relay node image — Raspberry Pi OS image with `ripple daemon`
  pre-configured, plug-and-play deployment
- Coverage mapping — admin tool that uses logged encounter data to generate
  a heatmap of mesh coverage in a building or campus
- HIPAA compliance documentation for healthcare deployments
- Deployment guide for hospitals, universities, and enterprises

---

## The Longer Vision

Phase 5 tooling is the foundation for Ripple as a platform rather than
just an application. At this stage:

- The protocol is stable and documented
- Multiple client implementations exist across all major platforms
- Institutional deployments provide dense, reliable mesh backbone in
  key locations
- The open protocol enables third-party clients and integrations
- Community mesh networks in underserved areas become viable

The network effects compound: every institutional deployment adds permanent
infrastructure nodes that benefit all nearby consumer users. Every consumer
user adds mesh density that makes institutional deployments more reliable.
The flywheel from the use cases document becomes self-sustaining.

What comes after Phase 5 is determined by where the network actually grows
and what the community builds on top of the protocol. The architecture
decisions made in Phase 1 — content addressing, CRDT shared state,
namespaces, the open FFI surface — were made with this future in mind.
