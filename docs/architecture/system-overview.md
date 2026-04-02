# System Overview

Ripple is organized as a protocol core surrounded by platform-specific client
shells. The Rust core contains all mesh logic. Platform shells handle hardware
access, UI, and background lifecycle.

## High Level Architecture

```mermaid
graph TB
    subgraph Clients
        iOS[iOS App<br/>Swift]
        Android[Android App<br/>Kotlin]
        Desktop[Desktop App<br/>Tauri]
        CLI[CLI Daemon<br/>Rust]
        Web[Web Client<br/>WASM]
    end

    subgraph Core["ripple-core (Rust)"]
        Routing[DTN Routing]
        Bundles[Bundle Engine]
        Crypto[Cryptography]
        Store[SQLite Store]
        CRDT[CRDT Sync]
    end

    subgraph FFI["ripple-ffi"]
        CFFI[C FFI Surface]
        WASM[WASM Bindings]
    end

    subgraph Transports
        BLE[BLE]
        WifiDirect[WiFi Direct]
        Multipeer[Multipeer<br/>Connectivity]
        WifiAdhoc[WiFi Ad-hoc]
        Internet[Internet Relay]
        LoRa[LoRa Bridge]
    end

    subgraph Infrastructure
        Rendezvous[Rendezvous Server]
    end

    iOS -->|FFI| CFFI
    Android -->|JNI| CFFI
    Desktop -->|direct| Core
    CLI -->|direct| Core
    Web -->|WASM| WASM

    CFFI --> Core
    WASM --> Core

    iOS --- BLE
    iOS --- Multipeer
    iOS --- Internet
    Android --- BLE
    Android --- WifiDirect
    Android --- Internet
    Desktop --- Multipeer
    Desktop --- WifiAdhoc
    Desktop --- Internet
    CLI --- WifiAdhoc
    CLI --- Internet
    CLI --- LoRa
    Web --- Internet

    Internet --> Rendezvous
```

## Cargo Workspace Structure

```mermaid
graph LR
    Workspace[Cargo Workspace]
    Workspace --> core[ripple-core<br/>lib crate]
    Workspace --> ffi[ripple-ffi<br/>staticlib + cdylib]
    Workspace --> cli[ripple-cli<br/>lib + bin crate]
    Workspace --> rendezvous[ripple-rendezvous<br/>lib + bin crate]

    ffi -->|depends on| core
    cli -->|depends on| core
    rendezvous -->|depends on| core

    core --> bundle[bundle.rs]
    core --> crypto[crypto.rs]
    core --> crdt[crdt.rs]
    core --> peer[peer.rs]
    core --> routing[routing.rs]
    core --> store[store.rs]
```

## Platform and FFI Boundary

The Rust core is a pure logic library. It has no knowledge of BLE, UI, or platform
APIs. Native platform code observes physical events and delegates all decisions to
the core.

```mermaid
sequenceDiagram
    participant Native as Native Layer<br/>(Swift / Kotlin)
    participant Core as Ripple Core<br/>(Rust)
    participant DB as SQLite

    Native->>Core: mesh_peer_encountered(pubkey, transport, rssi)
    Core->>DB: log encounter
    Core-->>Native: SyncOffer { bundle_ids[] }

    Native->>Core: mesh_bundle_received(bytes, from_peer)
    Core->>DB: store bundle
    Core->>Core: routing decision
    Core-->>Native: Actions { ForwardToPeer, NotifyUser }

    Native->>Core: mesh_tick(current_time)
    Core->>DB: expire old bundles
    Core-->>Native: Actions { SendBundle, UpdateMap }

    Native->>Core: mesh_create_bundle(payload, priority)
    Core->>Core: sign + encrypt
    Core->>DB: store outbound bundle
    Core-->>Native: bundle_id
```

## Bundle Lifecycle

```mermaid
stateDiagram-v2
    [*] --> Created: mesh_create_bundle()

    Created --> Stored: written to SQLite
    Stored --> Spraying: peer encountered
    Spraying --> Stored: spray count not exhausted
    Spraying --> Waiting: spray count exhausted
    Waiting --> Delivered: direct peer encounter
    Stored --> Delivered: destination encountered directly
    Delivered --> [*]

    Stored --> Expired: TTL elapsed
    Waiting --> Expired: TTL elapsed
    Expired --> [*]

    note right of Spraying
        SOS priority bundles
        never transition to
        Waiting — epidemic
        routing until delivered
    end note
```

## Mesh Routing Modes

The routing layer operates in two modes selected automatically based on observed
network density:

```mermaid
graph LR
    subgraph DTN["DTN Mode (sparse)"]
        direction TB
        A1[Store bundle] --> B1[Wait for peer]
        B1 --> C1[Forward copy]
        C1 --> B1
    end

    subgraph Interactive["Interactive Mode (dense)"]
        direction TB
        A2[Discover path] --> B2[Route along path]
        B2 --> C2[Acknowledge delivery]
    end

    Sparse[Low encounter frequency\nHigh delivery latency] --> DTN
    Dense[High encounter frequency\nLow delivery latency] --> Interactive
```

## Transport Layer

Each transport implements the same `MeshTransport` trait. The routing layer
selects transports per peer based on availability and capability.

```mermaid
graph TB
    Router[Transport Router]

    Router -->|discovery + small bundles| BLE[BLE<br/>Always on<br/>~100m]
    Router -->|bulk sync| WD[WiFi Direct<br/>Android only<br/>~250m]
    Router -->|bulk sync| MP[Multipeer<br/>iOS + Mac only<br/>~250m]
    Router -->|infrastructure nodes| WA[WiFi Ad-hoc<br/>Desktop + CLI<br/>500m+]
    Router -->|opportunistic| IR[Internet Relay<br/>All platforms<br/>Unlimited]
    Router -->|extended range| LR[LoRa Bridge<br/>CLI + Desktop<br/>Miles]
```

## Mesh Namespace Model

Ripple supports multiple isolated mesh namespaces on the same physical network.
Devices can participate in multiple namespaces simultaneously.

```mermaid
graph TB
    subgraph Physical["Physical Mesh (BLE + WiFi)"]
        N1((Node 1))
        N2((Node 2))
        N3((Node 3))
        N4((Node 4))
        N5((Node 5))

        N1 --- N2
        N2 --- N3
        N3 --- N4
        N4 --- N5
        N1 --- N3
    end

    subgraph Public["Public Namespace"]
        P[Open to all\nDisaster / consumer use]
    end

    subgraph Private["Hospital Namespace"]
        H[Enrolled devices only\nEncrypted with shared key]
    end

    N1 & N2 & N3 & N4 & N5 --> Public
    N2 & N3 --> Private
```

## Rendezvous Server

The rendezvous server is optional infrastructure that improves delivery rates
when any mesh node has internet connectivity. It is intentionally simple and
has no visibility into message content.

```mermaid
sequenceDiagram
    participant NodeA as Node A<br/>(has internet)
    participant Server as Rendezvous Server
    participant NodeB as Node B<br/>(regains internet later)

    NodeA->>Server: POST /bundle (signed, encrypted)
    Server->>Server: store by destination pubkey + TTL

    Note over NodeA,NodeB: Time passes, Node B regains connectivity

    NodeB->>Server: GET /inbox/{my_pubkey}
    Server-->>NodeB: pending bundles[]
    NodeB->>Server: DELETE /bundle/{id} (acknowledge)
```

## Key Management

Every Ripple identity is an Ed25519 keypair generated locally on first launch.
Private keys never leave the device.

```mermaid
graph LR
    Launch[First Launch] --> Keygen[Generate Ed25519 keypair]
    Keygen --> Store[Store in secure enclave\niOS Keychain / Android Keystore]
    Store --> Identity[Public key = your mesh address]

    Identity --> Sign[Sign all outbound bundles]
    Identity --> Encrypt[Encrypt direct messages\nX25519 key exchange]
    Identity --> Verify[Verify inbound bundle signatures]
    Identity --> QR[Share via QR code\nfor contact exchange]
```
````

