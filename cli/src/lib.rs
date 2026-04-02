//! ripple-cli as a library.
//!
//! Exposes internal modules so integration tests in `cli/tests/` can reference
//! them as `ripple_cli::relay::...` rather than duplicating the relay logic.
//! The binary entry point remains in `src/main.rs`.

pub mod relay;
