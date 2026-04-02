# Ripple

A resilient, decentralized mesh communication platform that works without internet infrastructure.

> Full documentation in [`/docs`](./docs)

## What is Ripple?

Ripple is an open-source mesh networking protocol and application suite that enables
device-to-device communication without cellular or internet infrastructure. Messages
"ripple" outward through nearby devices, hopping from node to node until they reach
their destination or an internet-connected relay.

## Use Cases

- Emergency and disaster communication
- Indoor dead zone coverage (hospitals, universities, large buildings)
- High-density environments where cell towers saturate (stadiums, conventions)
- Infrastructure-independent community networks

## Architecture

Ripple is built around a shared Rust core library consumed by all platforms:

- **iOS** (Swift) and **Android** (Kotlin) mobile apps
- **Desktop** app (Tauri)
- **CLI** daemon for headless relay nodes
- **Web** client
- **Rendezvous server** for internet-assisted relay

See [`docs/architecture/system-overview.md`](./docs/architecture/system-overview.md) for details.

## Status

✅ Phase 1 complete — full CLI daemon with end-to-end encrypted messaging over the rendezvous relay.

🚧 Phase 2 in progress — Android and iOS mobile apps.

CI: ![CI](https://github.com/tfuent1/ripple/actions/workflows/ci.yml/badge.svg)

## License

MIT OR Apache-2.0
