//! At-rest encryption for connection credentials (§17).
//!
//! Connection credentials (platform tokens, API keys) are encrypted with
//! AES-256-GCM before storage. Key material is held in a wrapper with a
//! manual `Debug` impl that redacts the bytes — a common real-world leak
//! vector closed by design (§17). In hosted deployments the key comes from a
//! KMS; self-hosted operators supply a key file.

use aes_gcm::aead::rand_core::{OsRng, RngCore};
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use secrecy::SecretString;

/// AES-256-GCM nonce size (96 bits, the GCM standard).
pub const NONCE_SIZE: usize = 12;

/// A 256-bit encryption key. `Debug` is manually implemented to redact the
/// bytes so the key is never accidentally logged (§17).
#[derive(Clone)]
pub struct EncryptionKey {
    key: [u8; 32],
}

impl EncryptionKey {
    /// Wrap an existing 32-byte key.
    pub fn from_bytes(key: [u8; 32]) -> Self {
        Self { key }
    }

    /// Generate a random key. Intended for tests and local dev; production
    /// should source the key from a KMS or operator-supplied key file (§17).
    pub fn generate() -> Self {
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        Self { key }
    }

    /// The raw key bytes.
    fn as_bytes(&self) -> &[u8; 32] {
        &self.key
    }
}

impl std::fmt::Debug for EncryptionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EncryptionKey(<redacted>)")
    }
}

/// Encrypt a plaintext credential string. Returns the ciphertext and the
/// 12-byte nonce, both of which must be stored — decryption needs both.
pub fn encrypt(
    plaintext: &str,
    key: &EncryptionKey,
) -> Result<(Vec<u8>, [u8; NONCE_SIZE]), CryptoError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key.as_bytes()));
    let mut nonce_bytes = [0u8; NONCE_SIZE];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| CryptoError::Aead(e.to_string()))?;
    Ok((ciphertext, nonce_bytes))
}

/// Decrypt ciphertext with the stored nonce. Returns a `SecretString` so the
/// plaintext credential is not accidentally logged (§17).
pub fn decrypt(
    ciphertext: &[u8],
    nonce: &[u8; NONCE_SIZE],
    key: &EncryptionKey,
) -> Result<SecretString, CryptoError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key.as_bytes()));
    let nonce = Nonce::from_slice(nonce);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| CryptoError::Aead(e.to_string()))?;
    let s = String::from_utf8(plaintext).map_err(CryptoError::Utf8)?;
    Ok(s.into())
}

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("AEAD encryption/decryption failed: {0}")]
    Aead(String),
    #[error("decrypted bytes were not valid UTF-8")]
    Utf8(#[from] std::string::FromUtf8Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    #[test]
    fn encrypt_decrypt_round_trip() {
        let key = EncryptionKey::generate();
        let plaintext = "foundry-token-deadbeef-cafe";
        let (ciphertext, nonce) = encrypt(plaintext, &key).unwrap();
        let decrypted = decrypt(&ciphertext, &nonce, &key).unwrap();
        assert_eq!(decrypted.expose_secret(), plaintext);
    }

    #[test]
    fn wrong_key_fails() {
        let key_a = EncryptionKey::generate();
        let key_b = EncryptionKey::generate();
        let (ciphertext, nonce) = encrypt("secret", &key_a).unwrap();
        assert!(decrypt(&ciphertext, &nonce, &key_b).is_err());
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let key = EncryptionKey::generate();
        let (mut ciphertext, nonce) = encrypt("secret", &key).unwrap();
        ciphertext[0] ^= 0xff;
        assert!(decrypt(&ciphertext, &nonce, &key).is_err());
    }

    #[test]
    fn key_debug_does_not_leak() {
        let key = EncryptionKey::from_bytes([0xAB; 32]);
        let debug = format!("{key:?}");
        assert!(!debug.contains("ab"), "Debug leaked key bytes: {debug}");
    }
}
