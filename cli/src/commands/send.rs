//! `ripple send [--to <pubkey_hex>] <message>`

use ripple_core::bundle::{BundleBuilder, Destination, Priority};
use ripple_core::crypto::Identity;
use ripple_core::routing::Router;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn run(
    router: &mut Router,
    identity: &Identity,
    message: &str,
    to_pubkey_hex: Option<&str>,
) -> anyhow::Result<()> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;

    let destination = match to_pubkey_hex {
        Some(hex_str) => {
            let bytes = hex::decode(hex_str).map_err(|_| anyhow::anyhow!("invalid pubkey hex"))?;
            let arr: [u8; 32] = bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("pubkey must be 32 bytes"))?;
            // ADR-006: Destination::Peer holds X25519, not Ed25519.
            Destination::Peer(arr)
        }
        None => Destination::Broadcast,
    };

    let bundle = BundleBuilder::new(destination, Priority::Normal)
        .payload(message.as_bytes().to_vec())
        .build(identity, now)?;

    router.store().insert_bundle(&bundle)?;

    println!("queued bundle {}", bundle.id);
    Ok(())
}
