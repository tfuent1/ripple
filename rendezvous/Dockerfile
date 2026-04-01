# ── Stage 1: build ────────────────────────────────────────────────────────────
# We use the official Rust image to compile a release binary.
# "builder" is just a name we give this stage so we can copy from it later.
FROM rust:1.87-slim AS builder

# Install C build tools. rusqlite's "bundled" feature compiles SQLite from
# source using cc — it needs a linker and C compiler to do that.
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# ── Dependency cache layer ────────────────────────────────────────────────────
# Copy only the Cargo manifests first and build a fake main so Cargo
# downloads and compiles all dependencies. This layer is cached as long as
# the Cargo.toml / Cargo.lock files don't change — which means rebuilding
# after a source-only change skips the slow dependency compile entirely.
COPY Cargo.toml Cargo.lock ./
COPY core/Cargo.toml        core/Cargo.toml
COPY ffi/Cargo.toml         ffi/Cargo.toml
COPY cli/Cargo.toml         cli/Cargo.toml
COPY rendezvous/Cargo.toml  rendezvous/Cargo.toml

# Create stub lib/main files for every workspace member so `cargo build`
# has something to compile without the real source.
RUN mkdir -p core/src ffi/src cli/src rendezvous/src && \
    echo 'pub fn main() {}' > core/src/lib.rs && \
    echo 'pub fn main() {}' > ffi/src/lib.rs  && \
    echo 'fn main() {}'     > cli/src/main.rs && \
    echo 'fn main() {}'     > rendezvous/src/main.rs

RUN cargo build --release -p ripple-rendezvous

# ── Real source compile ───────────────────────────────────────────────────────
# Now copy the actual source and touch the files so Cargo knows they changed
# (otherwise it sees the same timestamps as the stubs and skips recompilation).
COPY core/src       core/src
COPY ffi/src        ffi/src
COPY cli/src        cli/src
COPY rendezvous/src rendezvous/src

RUN touch core/src/lib.rs ffi/src/lib.rs cli/src/main.rs rendezvous/src/main.rs && \
    cargo build --release -p ripple-rendezvous

# ── Stage 2: runtime ──────────────────────────────────────────────────────────
# Start fresh from a minimal Debian image — no Rust toolchain, no build tools,
# just the binary and its runtime dependencies. This keeps the final image
# small (typically ~50MB vs ~1.5GB for the builder stage).
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Create a non-root user. Running as root inside a container is a bad habit —
# if something escapes the container, it has root on the host.
RUN useradd --create-home --shell /bin/bash ripple
USER ripple
WORKDIR /home/ripple

# Copy only the compiled binary from the builder stage.
COPY --from=builder /build/target/release/rendezvous /usr/local/bin/rendezvous

# The DB file lives here. Mount a volume to this path to persist data across
# container restarts: `docker run -v ripple-data:/data ...`
RUN mkdir -p /home/ripple/.ripple

EXPOSE 8080

# Default command — port and db path are overridable via flags.
# The DB path points into the volume mount directory.
CMD ["rendezvous", "--port", "8080", "--db", "/home/ripple/.ripple/rendezvous.db"]
