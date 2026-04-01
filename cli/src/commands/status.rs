//! `ripple status`

use ripple_core::crypto::Identity;
use ripple_core::routing::Router;

pub fn run(router: &Router, identity: &Identity) -> anyhow::Result<()> {
    let ed25519_hex  = hex::encode(identity.public_key());
    let x25519_hex   = hex::encode(identity.x25519_public_key());
    let unread       = router.store().unread_count()?;

    println!("Identity (Ed25519): {ed25519_hex}");
    println!("Inbox key (X25519): {x25519_hex}");
    println!("Unread messages:    {unread}");
    println!();
    println!("(give the X25519 key to others so they can send you messages)");
    Ok(())
}
