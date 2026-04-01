//! `ripple peers`

use ripple_core::routing::Router;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn run(router: &Router) -> anyhow::Result<()> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_secs() as i64;

    let since = now - 24 * 3600; // last 24 hours
    let encounters = router.store().recent_encounters(since)?;

    if encounters.is_empty() {
        println!("no peers encountered in the last 24 hours");
        return Ok(());
    }

    for enc in &encounters {
        println!(
            "peer {} | transport {} | rssi {} | last seen {}s ago",
            hex::encode(&enc.peer_pubkey[..8]), // first 8 bytes = readable prefix
            enc.transport,
            enc.rssi,
            now - enc.seen_at,
        );
    }
    Ok(())
}
