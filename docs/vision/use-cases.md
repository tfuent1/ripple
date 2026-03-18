# Use Cases

Ripple's use cases span a spectrum from emergency survival scenarios to everyday
convenience. This range is intentional — everyday utility drives the adoption that
makes emergency use viable.

## Tier 1 — Emergency and Disaster

The most critical use case and the one that justifies Ripple's existence. When
cellular infrastructure fails, Ripple becomes the communication layer for affected
communities.

**Scenarios:**
- Hurricane, earthquake, or wildfire destroys local cell towers
- Infrastructure is intact but overwhelmed by simultaneous emergency calls
- Deliberate disruption of communications during civil unrest
- Remote wilderness emergency where coverage never existed

**What Ripple enables:**
- SOS broadcasts that propagate through the mesh until they reach a responder
- Shared maps with pinned locations — shelters, medical posts, hazard zones,
  resource availability
- Family check-in and location sharing without cellular
- Coordination between community members and first responders on the same mesh
- Store-and-forward messaging that delivers when a path eventually exists, even
  with high latency

**Why this works:**
Mesh density in a disaster zone comes from the affected population itself. A
neighborhood of 500 households where even 10% have Ripple installed creates a
functional mesh across several city blocks. Messages hop device to device until
they reach someone with connectivity, who relays them to the internet or to
emergency services.

---

## Tier 2 — Indoor Dead Zones

Cellular signals don't reliably penetrate large buildings. This is an everyday
problem that affects millions of people in environments where communication matters.

### Hospitals

Patients and visitors frequently lose signal moving from lobbies into examination
rooms, procedure areas, and basement facilities. Staff communicating across floors
and wings face the same problem.

**What Ripple enables:**
- Patients messaging family from rooms with no signal — messages hop through
  nearby devices until reaching someone with connectivity
- Family members in waiting areas receiving updates without requiring staff to
  physically locate them
- Staff coordination that doesn't depend on the hospital's internal paging
  infrastructure
- A hospital-deployed private mesh (fixed relay nodes throughout the building)
  that functions as a resilient communication backbone independent of internet
  connectivity

**Institutional deployment:**
Hospitals can deploy fixed relay nodes (small devices plugged into ethernet
throughout the building) creating a permanent private mesh. Staff devices enroll
in the hospital's named mesh namespace. Communication continues even during an
internet outage. This is a realistic HIPAA-compliant replacement for outdated
paging systems.

### Universities

Large campuses combine the dead zone problem (basement labs, thick concrete
lecture halls) with the network saturation problem (thousands of students
simultaneously on WiFi and cellular).

**What Ripple enables:**
- Students and faculty communicating across campus regardless of signal
- Campus-wide alerts that reach people inside signal-dead lecture halls and labs
- Research building coordination — basement labs, shielded rooms, and server
  rooms are common dead zones
- Student safety infrastructure — emergency call points become mesh nodes,
  not isolated single-point devices

### Other Large Buildings

The same pattern applies to any large indoor environment: convention centers,
airports, shopping malls, office campuses, government buildings, courthouses,
and correctional facilities.

---

## Tier 3 — High Density Saturation

When too many people are in one place, cell towers become the bottleneck. This is
predictable and happens at the same events every year.

**Scenarios:**
- Sporting events and concerts (50,000+ people, one or two towers)
- Music festivals and outdoor gatherings
- Political rallies and protests
- New Year's Eve and other mass public events
- Large conferences and trade shows

**What Ripple enables:**
- Local group coordination that doesn't touch the cellular network at all
- Event-specific mesh namespaces — a festival creates a named mesh, attendees
  join it, staff coordinate over it
- Real-time shared maps of the venue — stages, exits, medical, lost and found
- Peer-to-peer file and media sharing without internet bandwidth

---

## Tier 4 — Underserved and Rural Areas

Large portions of the world have sparse or nonexistent cellular coverage as a
baseline condition, not an emergency.

**Scenarios:**
- Agricultural operations spread across large rural properties
- Remote construction and mining sites
- Developing regions where infrastructure investment hasn't reached
- Maritime — vessels at sea, fishing fleets, island communities

**What Ripple enables:**
- Farm worker coordination across properties larger than cellular range
- Site communication for construction and resource extraction operations
- Community mesh networks in areas where building cellular infrastructure
  is not economically viable
- Inter-vessel communication and coordination without satellite costs

---

## Tier 5 — Institutional Private Meshes

Organizations with a need for resilient, private, infrastructure-independent
communication can deploy Ripple as deliberate infrastructure rather than relying
on opportunistic consumer adoption.

**Deployment model:**
- Fixed relay nodes installed throughout a facility (ethernet-connected,
  WiFi and BLE mesh participation)
- Staff devices enrolled in a named, encrypted mesh namespace
- Communication fully isolated from public Ripple mesh
- Internet connectivity used opportunistically when available, not required

**Target institutions:**
- Hospitals and healthcare networks
- University campuses
- Military and law enforcement
- Emergency management agencies (FEMA, Red Cross, local OEM)
- Critical infrastructure operators (utilities, transit, ports)

---

## Tier 6 — Dense Mesh as Internet Alternative

At sufficient adoption density in urban environments, Ripple transitions from a
communication tool to something more fundamental: a decentralized, peer-to-peer
network layer that operates independently of traditional internet infrastructure.

This tier is not a day-one feature. It emerges from adoption of the tiers above.

**What becomes possible at high mesh density:**
- Sub-second message delivery across a dense urban mesh without any internet
  involvement
- Distributed local services — community bulletin boards, resource sharing,
  local commerce — hosted on the mesh itself with no servers
- Mesh-native naming and addressing — human-readable names resolved by the
  mesh without DNS servers
- Content-addressed data propagation — popular content caches itself across
  nearby nodes automatically
- IoT sensor networks — air quality, noise, traffic, environmental monitoring
  as community-owned open infrastructure

**The broader implication:**
A sufficiently dense Ripple mesh is communication infrastructure owned by no
single entity, with no choke point that can be shut down, no infrastructure
bill to pay, and no ISP between communities and their ability to communicate.
This is not a replacement for the internet in all cases — it is a complement
to it, and in some contexts a resilient alternative.

---

## Adoption Flywheel

These tiers are not independent. They reinforce each other:
```

Everyday utility (Tiers 2-3)
→ drives installation and retention
→ mesh gets denser
→ emergency use becomes reliable (Tier 1)
→ institutional deployment accelerates adoption (Tier 5)
→ mesh gets denser still
→ dense mesh services become viable (Tier 6)
→ more reasons to install and keep installed
→ mesh gets denser still
```

The messaging and maps features are not just product features — they are the
mechanism by which the network becomes reliable enough to matter when lives
depend on it.
