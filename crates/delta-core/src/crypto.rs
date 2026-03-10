//! Symmetric encryption for secrets storage using BLAKE3 keyed hashing + XOR stream cipher.
//!
//! Uses BLAKE3 in keyed hash mode to derive a keystream, then XORs with plaintext.
//! This avoids adding heavy crypto dependencies while providing confidentiality at rest.
//! The nonce ensures each encryption produces unique ciphertext.

use crate::{DeltaError, Result};

/// Encrypt a plaintext value using the given key.
/// Returns a hex-encoded string of `nonce || ciphertext`.
pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> String {
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce).expect("RNG failure");

    let ciphertext = xor_stream(key, &nonce, plaintext);

    let mut out = Vec::with_capacity(16 + ciphertext.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    hex::encode(out)
}

/// Decrypt a hex-encoded `nonce || ciphertext` value using the given key.
pub fn decrypt(key: &[u8; 32], hex_input: &str) -> Result<String> {
    let raw = hex::decode(hex_input)
        .map_err(|e| DeltaError::Storage(format!("invalid secret encoding: {e}")))?;

    if raw.len() < 16 {
        return Err(DeltaError::Storage("secret data too short".into()));
    }

    let (nonce, ciphertext) = raw.split_at(16);
    let nonce: [u8; 16] = nonce
        .try_into()
        .map_err(|_| DeltaError::Storage("invalid nonce".into()))?;

    let plaintext = xor_stream(key, &nonce, ciphertext);

    String::from_utf8(plaintext)
        .map_err(|e| DeltaError::Storage(format!("decrypted secret is not valid UTF-8: {e}")))
}

/// Derive encryption key from a passphrase string using BLAKE3.
pub fn derive_key(passphrase: &str) -> [u8; 32] {
    blake3::derive_key("delta-secrets-v1", passphrase.as_bytes())
}

/// XOR stream cipher using BLAKE3 keyed hash as keystream generator.
fn xor_stream(key: &[u8; 32], nonce: &[u8; 16], data: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(data.len());

    for (counter, chunk) in data.chunks(32).enumerate() {
        // Build input: nonce || counter
        let mut block_input = Vec::with_capacity(24);
        block_input.extend_from_slice(nonce);
        block_input.extend_from_slice(&(counter as u64).to_le_bytes());

        let keystream = blake3::keyed_hash(key, &block_input);
        let keystream_bytes = keystream.as_bytes();

        for (i, &byte) in chunk.iter().enumerate() {
            result.push(byte ^ keystream_bytes[i]);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip() {
        let key = derive_key("test-secret-key");
        let plaintext = "my-super-secret-api-key-12345";
        let encrypted = encrypt(&key, plaintext.as_bytes());
        let decrypted = decrypt(&key, &encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_different_nonce_each_time() {
        let key = derive_key("test-key");
        let plaintext = b"same-value";
        let enc1 = encrypt(&key, plaintext);
        let enc2 = encrypt(&key, plaintext);
        assert_ne!(enc1, enc2); // Different nonces produce different ciphertext
    }

    #[test]
    fn test_wrong_key_fails() {
        let key1 = derive_key("key-one");
        let key2 = derive_key("key-two");
        let encrypted = encrypt(&key1, b"secret");
        let result = decrypt(&key2, &encrypted);
        // Decryption "succeeds" but produces garbage — that's expected for XOR cipher
        // The important thing is it doesn't produce the original plaintext
        assert_ne!(result.unwrap_or_default(), "secret");
    }

    #[test]
    fn test_empty_plaintext() {
        let key = derive_key("key");
        let encrypted = encrypt(&key, b"");
        let decrypted = decrypt(&key, &encrypted).unwrap();
        assert_eq!(decrypted, "");
    }

    #[test]
    fn test_long_plaintext() {
        let key = derive_key("key");
        let plaintext = "a]".repeat(500);
        let encrypted = encrypt(&key, plaintext.as_bytes());
        let decrypted = decrypt(&key, &encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }
}
