# Problem Statement

## The Problem

Modern communication infrastructure has a single point of failure: centralized towers,
ISPs, and internet backbones. When these fail — whether from natural disaster, physical
damage, network saturation, or deliberate disruption — communication fails entirely.

This is not a rare edge case. It happens regularly:

- Hurricanes, earthquakes, and wildfires routinely destroy cell tower infrastructure
  in the exact areas where coordination is most critical
- Stadiums, conventions, and large public events saturate cell towers, making
  communication unreliable precisely when crowds need it most
- Hospitals, universities, and large buildings have significant indoor dead zones
  where signals don't penetrate, cutting off people even when towers are functioning
- Rural and underserved areas have sparse or nonexistent coverage as a baseline

When communication infrastructure fails, emergency responders can't coordinate,
families can't locate each other, and communities can't self-organize. People die
from coordination failures that reliable communication would have prevented.

## Why Existing Solutions Fall Short

### Cellular Networks
Centralized by design. A single tower outage creates a coverage gap. Tower saturation
under high load is a known, unsolved problem. No cellular network functions without
its infrastructure intact.

### Satellite Messaging (Garmin inReach, SPOT)
Requires dedicated hardware costing hundreds of dollars. Not something the average
person has installed before a disaster. Solves the individual survival use case but
not community coordination at scale.

### Walkie Talkies / Ham Radio
Short range, requires dedicated hardware, not integrated into the devices people
already carry. Ham radio requires licensing. Neither scales to civilian mass adoption.

### Existing Mesh Apps (Meshtastic, Briar, GoTenna)
- **Meshtastic** requires separate LoRa hardware. Pure software solution on existing
  phones is not possible.
- **Briar** is technically sound but built for activists in censored environments —
  the UX is not designed for mass civilian adoption or emergency use.
- **GoTenna** is proprietary, requires hardware, and the company controls the network.
- None of these have achieved meaningful civilian adoption, which means the mesh is
  too sparse to be reliable when needed most.

### The Adoption Problem
Every mesh network faces a chicken-and-egg problem: the network is only useful when
enough people have it installed, but people won't install it until it's useful. All
existing solutions ask users to install infrastructure software with no day-one value.

## The Insight

The solution to the adoption problem is to lead with everyday utility. A mesh
communication app that is genuinely useful before any disaster — for messaging in
dead zones, coordinating at events, communicating across large buildings — gets
installed and stays installed. The mesh gets dense as a side effect of normal usage.
When a disaster strikes, the infrastructure is already there.

## Ripple's Approach

Ripple is a mesh communication platform built on three core principles:

**1. Software-only, hardware people already own**
Ripple runs on the phones, laptops, and computers people already carry. No dedicated
hardware purchase required. Every device is a potential mesh node.

**2. Useful before the disaster**
Ripple solves real everyday problems — dead zones in hospitals and universities,
saturated networks at events, local communication without internet. People install
it because it helps them today, not because they're preparing for a worst case.

**3. Open protocol, multiple clients**
Ripple is a protocol first and an application second. The mesh gets denser with
every platform that ships a client — iOS, Android, desktop, CLI, web. An open
protocol means anyone can build on it, and no single company controls the network.

## The Larger Vision

At low adoption, Ripple is an emergency communication tool. At medium adoption, it
fills dead zones and reduces dependence on cellular infrastructure for local
communication. At high adoption and mesh density, it becomes something more: a
decentralized, infrastructure-independent communication layer that complements and
in some contexts replaces the traditional internet — owned by no one, operated by
everyone, resilient by design.
