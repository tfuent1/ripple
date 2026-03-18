# ADR-001: Shared Rust Core Library

## Status
Accepted

## Context
Ripple targets six platforms: iOS, Android, desktop (Tauri), CLI, web (WASM), and
a rendezvous server. The core logic — DTN routing, bundle serialization, cryptography,
CRDT merge logic, and SQLite storage — is identical across all of them. The question
was how to implement this logic without duplicating it across six codebases in
potentially six different languages.

Options considered:

**Option A — Reimplement per platform**
Each platform implements its own version of the core logic in its native language
(Swift, Kotlin, TypeScript, Rust). Simple to start, catastrophic to maintain. A
bug in the routing logic requires six fixes. Behavioral differences between
implementations are nearly guaranteed over time.

**Option B — Node.js / React Native shared logic**
A JavaScript core shared via React Native on mobile, Node.js on server/CLI, and
native web. Familiar ecosystem, one language. Unacceptable for this use case:
JS runtime overhead on mobile, poor background execution characteristics, weak
memory safety guarantees for crypto-adjacent code, and React Native's bridge
adds indirection to the platform APIs we need deepest access to (BLE, background
services).

**Option C — Rust core library with platform bindings**
A single Rust crate implementing all core logic, compiled to a static library for
iOS (via Swift Package Manager), a shared library for Android (via JNI), a native
binary for CLI and server, and WebAssembly for the web client. Each platform's
native layer handles platform-specific concerns (BLE, UI, background lifecycle)
and delegates all logic to the Rust core via a C-compatible FFI boundary.

## Decision
Option C — a shared Rust core library (`ripple-core`) with platform-specific
bindings per target.

The C FFI boundary is the lingua franca between Rust and all other languages.
All data crossing the boundary is serialized as MessagePack byte slices, keeping
the FFI surface minimal and avoiding complex type mapping. The core exposes a
`mesh_tick` function that native platforms call periodically — the core returns
a list of actions for native to execute, keeping the core purely functional and
avoiding the need for callbacks across the FFI boundary.

For the CLI and desktop (Tauri) targets, the Rust core is imported directly as
a library crate with no FFI overhead.

## Consequences

**Positive:**
- Single implementation of all core logic — bugs are fixed once, behavior is
  identical across all platforms
- Rust's memory safety guarantees are particularly valuable for cryptographic
  and networking code
- The Rust core can be extensively unit tested in isolation from any platform
- CLI and desktop targets get zero-overhead direct access to core logic
- WASM compilation enables client-side crypto in the browser with no server
  involvement
- Establishes a clean architectural boundary between platform concerns and
  protocol concerns

**Negative:**
- Rust is a significant learning investment for developers coming from other
  languages
- FFI boundary requires careful memory management — whoever allocates must free
- Cross-compilation setup for Android (multiple ABIs) and iOS adds build
  complexity
- Async Rust interacts poorly with FFI — the core must remain synchronous,
  with async confined to the application layer
- Two native codebases (Swift + Kotlin) still required for mobile platform concerns
