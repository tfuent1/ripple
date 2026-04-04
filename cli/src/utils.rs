//! Shared utilities for ripple-cli.

/// Current time as a Unix timestamp in seconds.
///
/// Extracted here so daemon.rs and integration tests don't duplicate it.
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}
