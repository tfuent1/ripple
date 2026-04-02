use chacha20poly1305::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    ChaCha20Poly1305, Key, Nonce,
};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use hkdf::Hkdf;
use rand::rngs::OsRng as RandOsRng;
use sha2::Sha256;
use thiserror::Error;
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};
use zeroize::ZeroizeOnDrop;

// ── Error type ───────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("signature verification failed")]
    VerificationFailed,

    #[error("encryption failed")]
    EncryptionFailed,

    #[error("decryption failed — message may be tampered or key is wrong")]
    DecryptionFailed,

    #[error("invalid public key bytes")]
    InvalidPublicKey,

    #[error("ciphertext too short to contain nonce")]
    CiphertextTooShort,
}

// ── Identity keypair ─────────────────────────────────────────────────────────

/// A node's long-term identity. The signing key never leaves this struct.
/// ZeroizeOnDrop ensures the private key bytes are wiped from memory when
/// this is dropped — important for a security-focused app.
#[derive(ZeroizeOnDrop)]
pub struct Identity {
    signing_key: SigningKey,
}

impl Identity {
    /// Generate a brand new random identity.
    pub fn generate() -> Self {
        let signing_key = SigningKey::generate(&mut RandOsRng);
        Self { signing_key }
    }

    /// Load an identity from raw private key bytes (e.g. from secure storage).
    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        Self {
            signing_key: SigningKey::from_bytes(bytes),
        }
    }

    /// Export the raw private key bytes for secure storage.
    /// Returns a copy — caller is responsible for zeroizing if needed.
    pub fn to_private_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    /// The public key — safe to share freely. This is a node's network identity.
    pub fn public_key(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }

    /// Sign arbitrary bytes. Used when creating a Bundle.
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.signing_key.sign(message).to_bytes()
    }
    pub(crate) fn to_x25519_secret(&self) -> StaticSecret {
        // `to_scalar_bytes()` performs SHA-512 expansion of the Ed25519 seed
        // and returns the lower 32 bytes — the actual private scalar before clamping.
        // `StaticSecret::from()` clamps it (per RFC 7748) when constructing the key.
        //
        // Using `self.signing_key.to_bytes()` (the raw seed) directly would bypass
        // this expansion step, producing a key pair where the X25519 public key
        // doesn't match what peers would compute — a silent, hard-to-debug mismatch.
        //
        // Requires the `hazmat` feature of ed25519-dalek.
        StaticSecret::from(self.signing_key.to_scalar_bytes())
    }

    /// The X25519 public key derived from this identity.
    /// Use this — not `public_key()` — when encrypting messages to this node.
    pub fn x25519_public_key(&self) -> [u8; 32] {
        X25519PublicKey::from(&self.to_x25519_secret()).to_bytes()
    }
}

// ── Signature verification ────────────────────────────────────────────────────

/// Verify a bundle signature. Called when a bundle is received from a peer.
#[must_use = "signature verification result must be checked — discarding it means accepting potentially forged data"]
pub fn verify_signature(
    public_key_bytes: &[u8; 32],
    message: &[u8],
    signature_bytes: &[u8; 64],
) -> Result<(), CryptoError> {
    let verifying_key =
        VerifyingKey::from_bytes(public_key_bytes).map_err(|_| CryptoError::InvalidPublicKey)?;
    let signature = Signature::from_bytes(signature_bytes);
    verifying_key
        .verify(message, &signature)
        .map_err(|_| CryptoError::VerificationFailed)
}

/// Derive a symmetric key from a raw X25519 shared secret using HKDF-SHA256.
///
/// Raw Diffie-Hellman output is not a uniformly random value — it lies on
/// an elliptic curve and has mathematical structure. Passing it directly as
/// a cipher key violates the "uniform random key" assumption that AEAD
/// schemes require. HKDF extracts a proper uniform key from it.
///
/// The `info` label "ripple-v1-message" domain-separates this derivation.
/// If the scheme ever changes, bump the label so old and new keys never collide.
fn derive_key(shared_secret: &x25519_dalek::SharedSecret) -> Result<Key, CryptoError> {
    let hk = Hkdf::<Sha256>::new(None, shared_secret.as_bytes());
    let mut key_bytes = [0u8; 32];
    hk.expand(b"ripple-v1-message", &mut key_bytes)
        .map_err(|_| CryptoError::EncryptionFailed)?;
    Ok(*Key::from_slice(&key_bytes))
}

// ── Encryption / Decryption ───────────────────────────────────────────────────

/// Encrypt a message for a recipient identified by their public key.
///
/// Returns: nonce (12 bytes) + ciphertext, concatenated.
/// The nonce is randomly generated per message and must be sent with it.
#[must_use = "encryption result must be used — discarding it means the plaintext was never protected"]
pub fn encrypt(
    sender_identity: &Identity,
    recipient_public_key_bytes: &[u8; 32],
    plaintext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    // X25519 Diffie-Hellman: combine our private key with their public key
    // to derive a shared secret that only the two of us can compute.
    let recipient_x25519_pub = X25519PublicKey::from(*recipient_public_key_bytes);
    let sender_secret = sender_identity.to_x25519_secret();
    let shared_secret = sender_secret.diffie_hellman(&recipient_x25519_pub);

    let cipher = ChaCha20Poly1305::new(&derive_key(&shared_secret)?);

    // Random nonce — must never repeat for the same key.
    let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);

    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|_| CryptoError::EncryptionFailed)?;

    // Prepend the nonce so the receiver can use it for decryption.
    let mut output = nonce.to_vec(); // 12 bytes
    output.extend_from_slice(&ciphertext);
    Ok(output)
}

/// Decrypt a message. `sender_public_key_bytes` is the origin field of the Bundle.
/// `ciphertext` is the raw payload bytes (nonce prepended).
pub fn decrypt(
    recipient_identity: &Identity,
    sender_public_key_bytes: &[u8; 32],
    ciphertext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    const NONCE_LEN: usize = 12;
    if ciphertext.len() < NONCE_LEN {
        return Err(CryptoError::CiphertextTooShort);
    }

    let (nonce_bytes, ciphertext) = ciphertext.split_at(NONCE_LEN);
    let nonce = Nonce::from_slice(nonce_bytes);

    // Mirror the key derivation from the sender's side.
    let sender_x25519_pub = X25519PublicKey::from(*sender_public_key_bytes);
    let recipient_secret = recipient_identity.to_x25519_secret();
    let shared_secret = recipient_secret.diffie_hellman(&sender_x25519_pub);

    let cipher = ChaCha20Poly1305::new(
        &derive_key(&shared_secret).map_err(|_| CryptoError::DecryptionFailed)?,
    );

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| CryptoError::DecryptionFailed)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_and_verify() {
        let identity = Identity::generate();
        let message = b"hello ripple";
        let sig = identity.sign(message);
        let pubkey = identity.public_key();

        assert!(verify_signature(&pubkey, message, &sig).is_ok());
        assert!(verify_signature(&pubkey, b"tampered", &sig).is_err());
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let alice = Identity::generate();
        let bob = Identity::generate();
        let plaintext = b"store and forward";

        let ciphertext = encrypt(&alice, &bob.x25519_public_key(), plaintext).unwrap();
        let recovered = decrypt(&bob, &alice.x25519_public_key(), &ciphertext).unwrap();

        assert_eq!(recovered, plaintext);
    }

    #[test]
    fn test_wrong_key_fails_decryption() {
        let alice = Identity::generate();
        let bob = Identity::generate();
        let eve = Identity::generate();

        let ciphertext = encrypt(&alice, &bob.x25519_public_key(), b"secret").unwrap();
        assert!(decrypt(&eve, &alice.x25519_public_key(), &ciphertext).is_err());
    }

    #[test]
    fn test_identity_roundtrip() {
        let identity = Identity::generate();
        let private_bytes = identity.to_private_bytes();
        let restored = Identity::from_bytes(&private_bytes);

        assert_eq!(identity.public_key(), restored.public_key());
    }

    #[test]
    fn test_x25519_public_key_matches_montgomery_point() {
        // Verify that our X25519 public key derivation is consistent with
        // the Ed25519 verifying key's Montgomery form.
        //
        // `verifying_key().to_montgomery()` is the authoritative source for
        // what the X25519 public key *should* be for a given Ed25519 identity.
        // If our `to_x25519_secret` were wrong (e.g., using the raw seed instead
        // of the expanded scalar), this test would catch the mismatch.
        let identity = Identity::generate();
        let derived_x25519_pub = identity.x25519_public_key();
        let canonical_montgomery = identity
            .signing_key
            .verifying_key()
            .to_montgomery()
            .to_bytes();
        assert_eq!(
            derived_x25519_pub, canonical_montgomery,
            "X25519 public key must match the Ed25519 verifying key's Montgomery form"
        );
    }
}
