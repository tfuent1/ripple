//! Identity persistence — load or generate the node's long-term identity.
//!
//! The identity file lives at `~/.ripple/identity.key` and contains exactly
//! 32 raw bytes: the Ed25519 private key scalar. chmod 0600 on creation.

use ripple_core::crypto::Identity;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use thiserror::Error;

/// Errors that can occur loading or saving the identity.
#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("identity.key is not 32 bytes — delete it to regenerate")]
    InvalidKeyFile,
}

/// Return the path to the identity key file, creating the directory if needed.
pub fn identity_path() -> Result<PathBuf, IdentityError> {
    // `dirs` crate would be cleaner but adds a dep — home dir from env is fine.
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"));

    let dir = home.join(".ripple");
    fs::create_dir_all(&dir)?;
    Ok(dir.join("identity.key"))
}

/// Load the identity from disk, or generate and save a new one.
///
/// On first run: generates, writes 32 bytes, chmod 0600.
/// On subsequent runs: reads 32 bytes, reconstructs Identity.
pub fn load_or_create() -> Result<Identity, IdentityError> {
    let path = identity_path()?;

    if path.exists() {
        // Load existing key.
        let mut file = fs::File::open(&path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;

        if bytes.len() != 32 {
            return Err(IdentityError::InvalidKeyFile);
        }

        let arr: [u8; 32] = bytes.try_into().map_err(|_| IdentityError::InvalidKeyFile)?;
        Ok(Identity::from_bytes(&arr))
    } else {
        // Generate new identity and persist it.
        let identity = Identity::generate();
        let private_bytes = identity.to_private_bytes();

        // `OpenOptions` lets us set the mode (permissions) at creation time.
        // 0o600 = owner read+write only. This is equivalent to `chmod 600`.
        // The `.mode()` call is from the `OpenOptionsExt` Unix trait we imported.
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true) // fail if file already exists (race-safe)
            .mode(0o600)
            .open(&path)?;

        file.write_all(&private_bytes)?;

        tracing::info!("generated new identity, saved to {}", path.display());
        Ok(identity)
    }
}
