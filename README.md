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

![CI](https://github.com/tfuent1/ripple/actions/workflows/ci.yml/badge.svg)

## A Note on AI Assistance

This project was built with Claude (Anthropic) as a pair-programming and teaching tool. I want to be transparent about what that means in practice, because "AI-assisted" covers a wide spectrum.

**What AI was used for:**
- Explaining Rust ownership, borrow checker concepts, and idioms as they came up in real code — I'm a PHP/Laravel developer learning Rust through this project
- Reviewing and critiquing architectural decisions before they were committed
- Catching bugs and explaining *why* they're bugs, not just patching them
- Drafting code that I then read, understood, and integrated (or rejected)

**What AI was not used for:**
- The architecture is mine. The DTN routing approach, two-key identity model, relay design, and phased roadmap came from research and deliberate decision-making documented in `docs/engineering/decisions/`
- No code was blindly copy-pasted. 
- AI does not have commit access, review PRs, or make project decisions

**Why I'm disclosing this:**
The surge in vibe-coded AI projects has made this a fair thing to be skeptical about. My goal is to build something real that solves a real problem. Using AI as a learning accelerator while I grow into Rust felt honest — hiding it would not.

If you read the code and find something you don't understand or disagree with, open an issue. I'll either explain my reasoning or admit I got it wrong.

## License

MIT OR Apache-2.0
