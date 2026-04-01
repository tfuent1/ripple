# CLI Daemon

The Ripple CLI is a binary crate (`ripple-cli`) that provides both a mesh
daemon and tooling commands. It imports `ripple-core` directly — no FFI.

## Commands
```
ripple daemon [--server <url>]         Start the mesh daemon
ripple send <message>                  Queue a broadcast bundle
ripple send --to <x25519_hex> <msg>    Queue an encrypted direct bundle
ripple status                          Show identity, keys, and unread count
ripple peers                           List recently encountered peers
```

## Identity

On first run, an Ed25519 keypair is generated and written to
`~/.ripple/identity.key` as 32 raw private key bytes, chmod 0600.
On subsequent runs the key is loaded from disk.

The daemon prints two keys on start:
- **Ed25519 pubkey** — your mesh identity, used to verify your signatures
- **X25519 pubkey (inbox key)** — share this with others so they can send
  you encrypted direct messages. This is the key to pass to `--to`.

These are derived from the same underlying private scalar (see ADR-006).
Only the Ed25519 private key is stored on disk.

## Quiet Mode

`ripple daemon --quiet` suppresses all `tracing` log output. The two identity
lines printed on startup (Ed25519 and X25519 pubkeys) and all decrypted message
lines are always shown regardless of this flag — they use `println!` directly.
Use `--quiet` for clean terminal sessions during testing or when running
interactively and you only care about incoming messages.

## Daemon Loop

The daemon runs two concurrent async tasks via tokio:

**Tick task** — fires every 30 seconds, calls `Router::mesh_tick` to expire
old bundles and handle any resulting Actions.

**Relay task** — fires every 30 seconds (and immediately on start):
1. Submits all undelivered local bundles to the rendezvous server via
   `POST /bundle`
2. Polls `GET /inbox/{x25519_pubkey_hex}` for inbound bundles
3. Passes each received bundle to `Router::on_bundle_received`
4. On `NotifyUser` — fetches the bundle from the store, decrypts the payload
   using `crypto::decrypt` with the node's own identity, and prints sender
   pubkey (first 8 bytes hex) and plaintext to stdout. Calls
   `Store::mark_displayed` on success. Logs a warning and continues on
   decryption failure — does not crash.
5. Calls `Store::mark_delivered` and `DELETE /bundle/{id}` to ack delivery

## Database

The daemon stores bundles and encounter history in `~/.ripple/mesh.db`
(SQLite, WAL mode). The database persists across restarts — spray counts
and undelivered bundles survive a daemon restart.

## Internet Transport

In Phase 1, the internet relay is the only active transport. BLE and WiFi
Direct are Phase 2. The relay strategy is simple: submit everything to the
rendezvous server and let it route by destination pubkey.

Phase 3 will add transport-aware filtering so bundles already synced over
BLE are not redundantly submitted to the relay.
