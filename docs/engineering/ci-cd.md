# CI/CD

Ripple uses GitHub Actions for continuous integration. The pipeline runs on
every push to `main` and on every pull request targeting `main`.

Configuration lives in `.github/workflows/ci.yml`. Supply chain policy lives
in `deny.toml` at the repo root.

## Pipeline Overview

```
push / PR to main
       │
       ▼
┌─────────────────┐
│  Format check   │  cargo fmt --all -- --check          ~5s
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│     Clippy      │  cargo clippy --all-targets           ~30s (cached)
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│     Tests       │  cargo test --all                     ~60s (cached)
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│   Deny check    │  cargo deny check                     ~10s
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Unsafe report  │  cargo geiger (non-blocking)          ~varies
└─────────────────┘
```

Steps are ordered cheapest-failure-first. A formatting error fails in seconds
without waiting for compilation. A test failure doesn't waste time running
the supply chain checks.

## Step Reference

### Format check

```bash
cargo fmt --all -- --check
```

Fails if any file in the workspace diverges from `rustfmt` defaults. Does not
modify files — CI is read-only. Fix locally with `cargo fmt --all` before
pushing.

### Clippy

```bash
cargo clippy --all-targets --all-features
```

Runs with `RUSTFLAGS="-D warnings"` set in the workflow environment, which
promotes every warning to a hard error in CI. Locally, warnings remain
warnings — you're not blocked from compiling while iterating.

Lint configuration lives in the `[workspace.lints]` section of the root
`Cargo.toml`. Current policy:

| Group | Level |
|---|---|
| `clippy::all` | warn |
| `clippy::pedantic` | warn |
| `clippy::module_name_repetitions` | allow (intentional — `BundleError`, `StoreError`, etc.) |
| `clippy::must_use_candidate` | allow (applied manually where it matters) |
| `rust::unused_imports` | warn |
| `rust::dead_code` | warn |

### Tests

```bash
cargo test --all
```

Runs unit tests across every crate in the workspace. SQLite tests use
`:memory:` — no filesystem side effects, no cleanup required.

### Deny check

```bash
cargo deny check
```

Runs four checks against the full dependency graph. Configuration is in
`deny.toml`.

**Advisories** — checks every crate against the [RustSec advisory database](https://rustsec.org/).
All vulnerability and unsound advisories are hard errors with no lint-level
override. Yanked crate versions also fail the build. Unmaintained and unsound
advisories are scoped to `"workspace"` — only direct workspace dependencies
fail, transitive ones warn.

**Licenses** — verifies every dependency carries a license explicitly allowed
in `deny.toml`. Any license not in the `allow` list is denied by default,
which means copyleft protection is automatic — no explicit `copyleft = "deny"`
needed. The current allow list is:

| License | Why |
|---|---|
| MIT | Most of the ecosystem |
| Apache-2.0 | Most of the ecosystem |
| ISC | Some crypto primitive crates |
| Unicode-3.0 | ICU4X crates pulled in via reqwest → url → idna |
| BSD-2-Clause | Occasional transitive dep |
| BSD-3-Clause | subtle, curve25519 internals |
| MPL-2.0 | webpki-roots via reqwest → rustls |

Workspace-internal crates (ripple-core, ripple-cli, ripple-ffi,
ripple-rendezvous) are exempt — they are our own code and are not published
to crates.io.

**Bans** — enforces that specific crates never appear in the dependency tree.
Currently banned:

| Crate | Reason |
|---|---|
| `openssl` | ADR-005: pure-Rust crypto only |
| `native-tls` | ADR-005: pulls in openssl on Linux |

`reqwest` is configured with `default-features = false, features = ["rustls-tls"]`
to avoid native-tls. If either banned crate appears after a dependency update,
the build fails immediately with the ban reason printed.

A set of known duplicate crate versions are listed in the `skip` array — these
are all caused by the reqwest 0.11 / axum 0.7 split on the hyper ecosystem and
will resolve when reqwest is upgraded to 0.12 in a future milestone.

**Sources** — verifies every crate comes from crates.io. Git dependencies and
unknown registries fail the build. The `allow-git` list is intentionally empty.

### Unsafe code report

```bash
cargo geiger --all-features 2>/dev/null || true
```

Prints a report of `unsafe` blocks across the workspace and its dependency
tree. Non-blocking (`|| true`) — it never fails the build. The output is
visible in the CI log for auditing purposes.

Ripple has intentional `unsafe` in `ffi/src/lib.rs` at the FFI boundary.
Every `unsafe` block there carries a `// SAFETY:` comment explaining the
invariant the caller must uphold. All other workspace crates contain no
`unsafe`.

## Caching

The workflow caches `~/.cargo/registry`, `~/.cargo/git`, and `target/`
keyed on a hash of `Cargo.lock`:

```yaml
key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}
```

On a warm cache (no dependency changes), a full pipeline run takes roughly
2-3 minutes. A cold cache (first run, or `Cargo.lock` changed) takes
7-9 minutes, mostly `rusqlite` compiling SQLite from source via the
`bundled` feature.

`Cargo.lock` is committed to version control (not in `.gitignore`) so the
cache key is stable and builds are reproducible.

## Running Checks Locally

Run the same checks CI runs before pushing:

```bash
# Format
cargo fmt --all

# Lint (warnings only locally, errors in CI)
cargo clippy --all-targets --all-features

# Tests
cargo test --all

# Supply chain
cargo deny check

# Unsafe report (optional)
cargo geiger --all-features 2>/dev/null
```

If Clippy is clean locally but fails in CI, the most common cause is a
warning that only appears with `--all-features` enabled or on a target
you don't compile locally. Run with `RUSTFLAGS="-D warnings"` to reproduce
CI behavior exactly:

```bash
RUSTFLAGS="-D warnings" cargo clippy --all-targets --all-features
```

## Suppressing Advisories

If `cargo deny check` fails on a CVE or unmaintained advisory that does not
affect Ripple (e.g. the vulnerable code path is never called, or the advisory
applies to a feature we don't enable), add an entry to the `ignore` list in
`deny.toml` with a mandatory reason:

```toml
ignore = [
    { id = "RUSTSEC-0000-0000", reason = "only affects the foo feature which we don't enable" },
]
```

Never add an ignore entry without a reason. The `unused-ignored-advisory = "warn"`
setting will flag stale ignore entries if the advisory is later removed from the
advisory database.

## Adding a Banned Crate

To prevent a crate from entering the dependency tree, add it to the `deny`
list in `deny.toml`:

```toml
deny = [
    { crate = "some-crate", reason = "explain why this must never be used" },
]
```

The ban fires on `cargo deny check` and prints the full dependency path showing
which of your dependencies pulled it in transitively, making it easy to find
and remove or replace the offending dep.

## Future Improvements

- Add `cargo geiger` hard gate on `unsafe` count once the FFI surface stabilises
  after Phase 2 mobile integration — new `unsafe` outside `ffi/src/lib.rs` should
  require explicit review
- Add `cargo outdated` as a scheduled monthly workflow (separate from CI) to
  surface available dependency updates without blocking every push
- Promote `bans.multiple-versions` from `"warn"` to `"deny"` and resolve all
  remaining duplicates when reqwest upgrades to 0.12 (hyper 1.x ecosystem)
