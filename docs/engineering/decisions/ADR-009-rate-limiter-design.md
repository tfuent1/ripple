# ADR-009: Rendezvous Server Rate Limiter Design

## Status
Accepted

## Context
The rendezvous server is a publicly accessible store-and-forward relay. Without
rate limiting, a single client can flood the server with bundle submissions,
exhausting storage and degrading service for all other nodes. A rate limiting
mechanism was added in Milestone 1.8.

The question was what scope and implementation to use.

Options considered:

**Option A — Global rate limit via tower middleware**
`tower::limit::RateLimit` applies a single limit across all callers combined.
Simple to add — one middleware layer. Rejected because a single noisy or
malicious client can consume the entire global budget, blocking all other
nodes regardless of their individual behavior.

**Option B — Per-IP in-memory HashMap**
A `HashMap<IpAddr, (u32, Instant)>` behind a `Mutex` tracks submission count
and window start per source IP. Each request checks and increments the counter
for its IP. The window resets when the duration elapses.

Advantages: one noisy IP cannot starve others. Simple, no additional
dependencies, no infrastructure required.

Disadvantages: state is in-memory only — resets on server restart, not
shared across multiple server instances. Stale entries for IPs that go
silent accumulate without active eviction.

**Option C — Redis-backed distributed counter**
Per-IP counters stored in Redis with TTL-based expiry. Survives restarts,
shared across horizontal server instances.

Advantages: correct under all deployment topologies.

Disadvantages: adds a required Redis dependency. Operationally heavier —
Redis must be deployed, monitored, and kept available. Failure of Redis
takes down rate limiting entirely, requiring a fallback strategy. Not
justified at Phase 1 traffic volumes.

## Decision
Option B — per-IP in-memory HashMap — for Phase 1. Promote to Option C
(Redis or equivalent) in Phase 4 when the rendezvous server is hardened
for public deployment (Milestone 4.3).

**Current limits:**
- 60 bundle submissions per IP per 60-second rolling window
- Excess requests receive `429 Too Many Requests`
- The rate limiter and the DB connection are in separate `Mutex`es —
  a slow DB operation does not block rate limit checks for other requests

**Lazy eviction.** Stale entries are evicted on every 100th request from
an active IP rather than on a background timer. This bounds map growth
without requiring a separate task. Under sustained low-volume abuse from
many distinct IPs the map can grow slowly — a background eviction task
would fix this but is not warranted at Phase 1 scale.

**Known limitations documented and accepted for Phase 1:**
- State resets on server restart — an IP that was rate limited before a
  restart gets a fresh window afterward
- Not shared across multiple server instances — running two rendezvous
  servers behind a load balancer gives each IP double the effective limit
- No protection against bundle flooding where a single node submits
  bundles on behalf of many destination pubkeys to exhaust inbox storage
  for others — this requires per-pubkey or per-destination limits, not
  per-source-IP limits

## Consequences

**Positive:**
- No additional dependencies for Phase 1 deployment
- Per-IP isolation — one noisy client cannot starve others
- Separate Mutex from DB lock prevents rate limiting from becoming a
  bottleneck under DB load
- Lazy eviction bounds memory growth without a background task

**Negative:**
- In-memory state is lost on restart
- Not horizontally scalable without moving to a shared backing store
- Does not protect against storage exhaustion via many-destination flooding
  from a single IP within the rate limit

## Future Work
Phase 4 (Milestone 4.3) will replace this implementation with a
persistent, horizontally-scalable rate limiter as part of the broader
rendezvous server hardening milestone. At that point the per-destination
flooding gap should also be addressed with per-pubkey submission limits.
