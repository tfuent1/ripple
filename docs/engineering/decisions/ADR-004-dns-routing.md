# ADR-004: Custom Lightweight DTN Routing

## Status
Accepted

## Context
A mesh network where nodes are mobile, connections are intermittent, and end-to-end
paths may not exist at any given moment requires a routing protocol designed for
these conditions. Traditional internet routing protocols (OSPF, BGP) assume
continuous connectivity and fail entirely in a disconnected mesh. The question was
which delay-tolerant networking approach to adopt.

Options considered:

**Option A — Epidemic routing (flooding)**
Every node forwards every bundle to every peer it encounters. Maximum delivery
probability, minimal implementation complexity. Rejected due to catastrophic
resource consumption — bundle duplication grows exponentially, destroying battery
life and storage on every node in the mesh.

**Option B — Full Bundle Protocol (RFC 9171)**
The IETF-standardized DTN protocol, used in space communications and military
networks. Battle-tested and interoperable. Rejected due to excessive complexity
for this use case — the full spec includes administrative record types, class of
service hierarchies, and extension block mechanisms that add significant
implementation burden with no benefit for a civilian mesh application.

**Option C — Spray and Wait**
A controlled flooding approach. A bundle is given N copies at creation time.
Each copy is sprayed to a different peer encountered. Once all N copies are
distributed, nodes wait for direct delivery to the destination. Delivery
probability approaches epidemic routing at a fraction of the resource cost.
The spray count N is tunable per priority level.

**Option D — PRoPHET (Probabilistic Routing Protocol using History of Encounters)**
Routes bundles toward peers with a higher historical probability of encountering
the destination, based on logged encounter history. More efficient than Spray and
Wait in dense networks but requires maintaining and exchanging encounter probability
tables, adding complexity and per-peer state.

## Decision
Spray and Wait for v1, with a clear upgrade path to PRoPHET-informed routing in v2.

Spray and Wait is simple to implement correctly, has well-understood behavior, and
its primary tuning parameter (spray count N) maps cleanly to the priority system:

| Priority | Label | Spray Count | TTL |
|---|---|---|---|
| 0 | Normal | 6 | 24 hours |
| 1 | Urgent | 20 | 12 hours |
| 2 | SOS | Epidemic | Never expires |

SOS priority bundles revert to epidemic routing — delivery probability is
maximized regardless of resource cost because the message may be life-critical.

The routing layer operates in two modes that are selected automatically based
on observed network density:

- **DTN mode** (sparse network): store-and-forward with Spray and Wait
- **Interactive mode** (dense network): route like a packet-switched network
  along discovered paths

Transition between modes is driven by metrics collected passively: peer encounter
frequency, average bundle delivery latency, and hop count distributions.

Bundle format:
```rust
struct Bundle {
    id:          Uuid,
    origin:      [u8; 32],      // Ed25519 public key
    destination: Destination,   // Peer(pubkey) | Broadcast | ContentHash
    created_at:  i64,
    expires_at:  i64,
    hop_count:   u8,
    hop_limit:   u8,
    priority:    u8,
    payload:     Vec<u8>,       // encrypted if addressed, plaintext if broadcast
    signature:   [u8; 64],      // Ed25519 signature over all other fields
}
```

## Consequences

**Positive:**
- Simple enough to implement correctly and test thoroughly in v1
- Spray count tuning provides a direct knob for balancing delivery probability
  against resource consumption
- SOS epidemic fallback ensures maximum delivery probability for life-critical messages
- Dual DTN/interactive routing modes allow the protocol to scale gracefully from
  sparse emergency use to dense everyday use
- Clear upgrade path to PRoPHET in v2 without changing the bundle format

**Negative:**
- Spray and Wait delivery probability degrades in very sparse networks where
  N copies may not find N distinct carriers
- Optimal spray count N requires tuning based on real-world mesh density data
  we won't have until the network has users
- Dual routing modes add complexity to the routing state machine
- Interactive mode path discovery is a non-trivial problem not fully designed yet
