# Rendezvous Server

The Ripple rendezvous server (`ripple-rendezvous`) is a store-and-forward
relay. It stores bundles by destination pubkey and serves them to polling
clients. It has no visibility into message content — all payloads are
encrypted before reaching the server.

## API

### `POST /bundle`
Accept a raw MessagePack bundle and store it.

The server deserializes using `Bundle::from_bytes` to extract the
destination pubkey and TTL. Everything else is stored as opaque bytes.
Returns `201 Created` on success, `400 Bad Request` if the bundle can't
be parsed.

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

## Phase 1 Limitations

The current server is a Phase 1 stub. Known gaps to address in
Milestone 1.8:

- In-memory SQLite — all bundles lost on restart
- No rate limiting
- No bundle size limits
- No authentication
- Base64 implementation is hand-rolled (replace with `base64` crate)

## Deployment
```bash
cargo run -p ripple-rendezvous          # default: 0.0.0.0:8080
```

Docker support and persistent storage are Milestone 1.8 deliverables.
