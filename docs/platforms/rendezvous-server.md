# Rendezvous Server

The Ripple rendezvous server (`ripple-rendezvous`) is a store-and-forward
relay. It stores bundles by destination pubkey and serves them to polling
clients. It has no visibility into message content — all payloads are
encrypted before reaching the server.

## API

### `POST /bundle`
Accept a raw MessagePack bundle and store it.

Returns `201 Created` on success, `400 Bad Request` if the body cannot
be parsed as a valid MessagePack bundle, `413 Payload Too Large` if the
body exceeds the configured size limit, `429 Too Many Requests` if the
source IP has exceeded the rate limit.

### `GET /inbox/:pubkey_hex`
Return all pending bundles for a destination pubkey.

`pubkey_hex` is the recipient's X25519 pubkey as a lowercase hex string.
Bundles past their TTL are expired before the response is assembled.
Returns a JSON object: `{ "bundles": ["<base64>", ...] }` where each
entry is the raw MessagePack bundle bytes, base64-encoded.

### `DELETE /bundle/:id`
Acknowledge delivery and remove a bundle by UUID string.

Called by the daemon after successfully processing an inbound bundle.

## Design

**Opaque storage.** The server stores the raw MessagePack bytes of each
bundle. It reads only the fields needed for routing (destination pubkey,
TTL) and otherwise treats the payload as opaque. The server cannot read
message content.

**TTL enforcement.** Bundles with a non-null `expires_at` are deleted
on the next inbox poll after their TTL elapses. SOS bundles (`expires_at`
is null) are never deleted by the server.

**Idempotent storage.** `INSERT OR IGNORE` on the bundle ID means
resubmitting the same bundle is safe. The daemon submits all undelivered
bundles on every relay cycle; duplicates are silently dropped.

## Hardening (Milestone 1.8)

- **Persistent SQLite** — bundles survive server restarts. DB path
  configurable via `--db`, default `~/.ripple/rendezvous.db`.
- **Bundle size limit** — requests over the configured max (default 64 KB)
  are rejected with `413 Payload Too Large` before the handler runs.
- **Per-IP rate limiting** — max 60 bundle submissions per IP per minute.
  Excess requests receive `429 Too Many Requests`.
- **Graceful shutdown** — SIGINT and SIGTERM drain in-flight requests
  before exit.
- **`base64` crate** — hand-rolled encoder/decoder replaced throughout.

## Known Gaps

- **No authentication** — any client can submit bundles for any destination.
  Addressed in Phase 4 (Milestone 4.3).
- **Rate limiting is in-memory** — resets on restart, not shared across
  multiple server instances. Sufficient for Phase 1.


## Hardening (Phase 1 post-milestone)

In addition to Milestone 1.8 changes above, the following was added during
Phase 1 hardening:

- **Bundle signature verification** — `Db::insert_bundle` now calls
  `bundle.verify()` before storing. Bundles with invalid Ed25519 signatures
  are rejected with `400 Bad Request`. Forged `origin` fields can no longer
  be stored.

## Deployment
```bash
# Run directly
cargo run -p ripple-rendezvous                         # defaults: port 8080, ~/.ripple/rendezvous.db
cargo run -p ripple-rendezvous -- --port 9090 --db /data/relay.db

# Docker (persistent volume)
docker build -f rendezvous/Dockerfile -t ripple-rendezvous .
docker run -p 8080:8080 -v ripple-data:/home/ripple/.ripple ripple-rendezvous
```
