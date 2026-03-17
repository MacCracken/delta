//! Symmetric encryption for secrets storage using BLAKE3 keyed hashing.
//!
//! Uses BLAKE3 in keyed hash mode to derive a keystream (CTR mode), then XORs
//! with plaintext. An encrypt-then-MAC tag (BLAKE3 keyed hash of nonce||ciphertext)
//! is appended to detect tampering.
//!
//! Wire format: `hex(nonce[16] || ciphertext[N] || tag[32])`

use crate::{DeltaError, Result};

/// Encrypt a plaintext value using the given key.
/// Returns a hex-encoded string of `nonce || ciphertext || tag`.
pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<String> {
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce).map_err(|e| DeltaError::Storage(format!("RNG failure: {e}")))?;

    let ciphertext = xor_stream(key, &nonce, plaintext);

    // Encrypt-then-MAC: compute tag over nonce || ciphertext
    let tag = compute_mac(key, &nonce, &ciphertext);

    let mut out = Vec::with_capacity(16 + ciphertext.len() + 32);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    out.extend_from_slice(&tag);
    Ok(hex::encode(out))
}

/// Decrypt a hex-encoded `nonce || ciphertext || tag` value using the given key.
/// Returns an error if the tag is invalid (tampered or wrong key).
pub fn decrypt(key: &[u8; 32], hex_input: &str) -> Result<String> {
    let raw = hex::decode(hex_input)
        .map_err(|e| DeltaError::Storage(format!("invalid secret encoding: {e}")))?;

    // Minimum: 16 (nonce) + 0 (ciphertext) + 32 (tag) = 48
    if raw.len() < 48 {
        // Legacy format (no tag): 16 (nonce) + ciphertext, minimum 16 bytes
        // Fall back to untagged decryption for backwards compatibility.
        // NOTE: Legacy secrets have no integrity protection. Re-encrypt to upgrade.
        if raw.len() >= 16 {
            tracing::warn!(
                "decrypting legacy secret without MAC — re-encrypt to upgrade integrity protection"
            );
            return decrypt_legacy(key, &raw);
        }
        return Err(DeltaError::Storage("secret data too short".into()));
    }

    let tag_start = raw.len() - 32;
    let nonce: [u8; 16] = raw[..16]
        .try_into()
        .map_err(|_| DeltaError::Storage("invalid nonce".into()))?;
    let ciphertext = &raw[16..tag_start];
    let tag = &raw[tag_start..];

    // Verify MAC before decrypting
    let expected_tag = compute_mac(key, &nonce, ciphertext);
    if !constant_time_eq(tag, &expected_tag) {
        return Err(DeltaError::Storage(
            "secret integrity check failed (wrong key or tampered data)".into(),
        ));
    }

    let plaintext = xor_stream(key, &nonce, ciphertext);

    String::from_utf8(plaintext)
        .map_err(|e| DeltaError::Storage(format!("decrypted secret is not valid UTF-8: {e}")))
}

/// Decrypt legacy format (no MAC tag) for backwards compatibility.
fn decrypt_legacy(key: &[u8; 32], raw: &[u8]) -> Result<String> {
    let (nonce_bytes, ciphertext) = raw.split_at(16);
    let nonce: [u8; 16] = nonce_bytes
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

/// Compute MAC over nonce || ciphertext using a derived MAC key.
fn compute_mac(key: &[u8; 32], nonce: &[u8; 16], ciphertext: &[u8]) -> [u8; 32] {
    // Derive a separate MAC key from the encryption key to avoid key reuse
    let mac_key: [u8; 32] = blake3::derive_key("delta-secrets-mac-v1", key);
    let mut hasher = blake3::Hasher::new_keyed(&mac_key);
    hasher.update(nonce);
    hasher.update(ciphertext);
    *hasher.finalize().as_bytes()
}

/// Constant-time byte comparison.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
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

/// Generate a random 32-byte key and return it as hex.
pub fn generate_repo_key() -> Result<String> {
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).map_err(|e| DeltaError::Storage(format!("RNG failure: {e}")))?;
    Ok(hex::encode(key))
}

/// Encrypt a repository encryption key for a specific user.
/// Uses the user's derived key (from their token/password) to wrap the repo key.
/// Returns hex-encoded ciphertext.
pub fn wrap_repo_key(user_key: &[u8; 32], repo_key_hex: &str) -> Result<String> {
    encrypt(user_key, repo_key_hex.as_bytes())
}

/// Decrypt a wrapped repository encryption key.
/// Returns the repo key as hex string.
pub fn unwrap_repo_key(user_key: &[u8; 32], wrapped_hex: &str) -> crate::Result<String> {
    decrypt(user_key, wrapped_hex)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip() {
        let key = derive_key("test-secret-key");
        let plaintext = "my-super-secret-api-key-12345";
        let encrypted = encrypt(&key, plaintext.as_bytes()).unwrap();
        let decrypted = decrypt(&key, &encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_different_nonce_each_time() {
        let key = derive_key("test-key");
        let plaintext = b"same-value";
        let enc1 = encrypt(&key, plaintext).unwrap();
        let enc2 = encrypt(&key, plaintext).unwrap();
        assert_ne!(enc1, enc2); // Different nonces produce different ciphertext
    }

    #[test]
    fn test_wrong_key_fails_with_error() {
        let key1 = derive_key("key-one");
        let key2 = derive_key("key-two");
        let encrypted = encrypt(&key1, b"secret").unwrap();
        let result = decrypt(&key2, &encrypted);
        // With MAC, wrong key should produce an integrity error
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("integrity check failed")
        );
    }

    #[test]
    fn test_tampered_ciphertext_fails() {
        let key = derive_key("key");
        let encrypted = encrypt(&key, b"secret").unwrap();
        let mut raw = hex::decode(&encrypted).unwrap();
        // Flip a byte in the ciphertext (after nonce, before tag)
        if raw.len() > 20 {
            raw[18] ^= 0xFF;
        }
        let tampered = hex::encode(raw);
        let result = decrypt(&key, &tampered);
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_plaintext() {
        let key = derive_key("key");
        let encrypted = encrypt(&key, b"").unwrap();
        let decrypted = decrypt(&key, &encrypted).unwrap();
        assert_eq!(decrypted, "");
    }

    #[test]
    fn test_long_plaintext() {
        let key = derive_key("key");
        let plaintext = "a]".repeat(500);
        let encrypted = encrypt(&key, plaintext.as_bytes()).unwrap();
        let decrypted = decrypt(&key, &encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_generate_repo_key() {
        let key = generate_repo_key().unwrap();
        assert_eq!(key.len(), 64); // 32 bytes as hex
        assert!(key.chars().all(|c| c.is_ascii_hexdigit()));

        let key2 = generate_repo_key().unwrap();
        assert_ne!(key, key2);
    }

    #[test]
    fn test_wrap_unwrap_repo_key() {
        let user_key = derive_key("user-password");
        let repo_key = generate_repo_key().unwrap();

        let wrapped = wrap_repo_key(&user_key, &repo_key).unwrap();
        assert_ne!(wrapped, repo_key);

        let unwrapped = unwrap_repo_key(&user_key, &wrapped).unwrap();
        assert_eq!(unwrapped, repo_key);
    }

    #[test]
    fn test_wrap_wrong_key_fails() {
        let key1 = derive_key("user1");
        let key2 = derive_key("user2");
        let repo_key = generate_repo_key().unwrap();

        let wrapped = wrap_repo_key(&key1, &repo_key).unwrap();
        let result = unwrap_repo_key(&key2, &wrapped);
        assert!(result.is_err()); // Now properly fails instead of silently decrypting to garbage
    }

    #[test]
    fn test_encrypt_returns_ok() {
        let key = derive_key("test-key");
        let result = encrypt(&key, b"hello");
        assert!(result.is_ok());
        // Output should be valid hex
        let hex_str = result.unwrap();
        assert!(hex::decode(&hex_str).is_ok());
    }

    #[test]
    fn test_legacy_format_detection() {
        // Legacy format: nonce(16) + ciphertext, no tag — total < 48 bytes
        let key = derive_key("legacy-key");

        // Build a legacy-format ciphertext: nonce || xor_stream(key, nonce, plaintext)
        let plaintext = b"legacy-secret";
        let nonce = [0u8; 16];
        let ciphertext = xor_stream(&key, &nonce, plaintext);

        let mut raw = Vec::new();
        raw.extend_from_slice(&nonce);
        raw.extend_from_slice(&ciphertext);
        // No tag appended — this is legacy format (< 48 bytes for short plaintext)
        assert!(raw.len() < 48);

        let hex_input = hex::encode(&raw);
        let decrypted = decrypt(&key, &hex_input).unwrap();
        assert_eq!(decrypted, "legacy-secret");
    }

    #[test]
    fn test_tampered_tag_detected() {
        let key = derive_key("tag-test");
        let encrypted = encrypt(&key, b"secret-data").unwrap();
        let mut raw = hex::decode(&encrypted).unwrap();

        // Tamper with the last byte (part of the tag)
        let last = raw.len() - 1;
        raw[last] ^= 0xFF;

        let tampered = hex::encode(&raw);
        let result = decrypt(&key, &tampered);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("integrity check failed")
        );
    }

    #[test]
    fn test_tampered_nonce_detected() {
        let key = derive_key("nonce-test");
        let encrypted = encrypt(&key, b"secret-data").unwrap();
        let mut raw = hex::decode(&encrypted).unwrap();

        // Tamper with the first byte (part of the nonce)
        raw[0] ^= 0xFF;

        let tampered = hex::encode(&raw);
        let result = decrypt(&key, &tampered);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("integrity check failed")
        );
    }

    #[test]
    fn test_compute_mac_independent_of_encryption_key() {
        // MAC key is derived from encryption key, but different enc keys
        // produce different MACs for same input.
        let key1 = derive_key("key-alpha");
        let key2 = derive_key("key-beta");
        let nonce = [0u8; 16];
        let data = b"some ciphertext bytes";

        let mac1 = compute_mac(&key1, &nonce, data);
        let mac2 = compute_mac(&key2, &nonce, data);
        assert_ne!(mac1, mac2);
    }

    #[test]
    fn test_empty_ciphertext_with_mac() {
        // Empty plaintext should produce: nonce(16) + empty ciphertext + tag(32) = 48 bytes
        let key = derive_key("empty-test");
        let encrypted = encrypt(&key, b"").unwrap();
        let raw = hex::decode(&encrypted).unwrap();
        assert_eq!(raw.len(), 48); // 16 + 0 + 32

        let decrypted = decrypt(&key, &encrypted).unwrap();
        assert_eq!(decrypted, "");
    }

    #[test]
    fn test_too_short_data_errors() {
        let key = derive_key("short-test");
        // Less than 16 bytes — not even a nonce
        let hex_input = hex::encode([0u8; 10]);
        let result = decrypt(&key, &hex_input);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too short"));
    }

    #[test]
    fn test_constant_time_eq_different_lengths() {
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"", b"a"));
    }

    #[test]
    fn test_constant_time_eq_same() {
        assert!(constant_time_eq(b"hello", b"hello"));
        assert!(constant_time_eq(b"", b""));
    }
}
