//! ed25519 artifact signature verification.

use ed25519_dalek::{Signature, VerifyingKey};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct VerificationResult {
    pub key_id: String,
    pub key_name: String,
    pub valid: bool,
}

/// Verify an ed25519 signature over a content hash.
///
/// - `public_key_hex`: 32-byte ed25519 public key as hex (64 chars)
/// - `message`: the content hash string being signed (the BLAKE3 hex hash)
/// - `signature_hex`: 64-byte ed25519 signature as hex (128 chars)
pub fn verify_signature(
    public_key_hex: &str,
    message: &str,
    signature_hex: &str,
) -> Result<bool, String> {
    let pk_bytes =
        hex::decode(public_key_hex).map_err(|e| format!("invalid public key hex: {}", e))?;
    let sig_bytes =
        hex::decode(signature_hex).map_err(|e| format!("invalid signature hex: {}", e))?;

    let pk_array: [u8; 32] = pk_bytes
        .try_into()
        .map_err(|_| "public key must be 32 bytes".to_string())?;
    let sig_array: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| "signature must be 64 bytes".to_string())?;

    let verifying_key =
        VerifyingKey::from_bytes(&pk_array).map_err(|e| format!("invalid public key: {}", e))?;
    let signature = Signature::from_bytes(&sig_array);

    use ed25519_dalek::Verifier;
    Ok(verifying_key.verify(message.as_bytes(), &signature).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    fn generate_test_keypair() -> (String, String) {
        use ed25519_dalek::Signer;
        // Use a fixed seed for deterministic tests
        let seed: [u8; 32] = [42u8; 32];
        let signing_key = SigningKey::from_bytes(&seed);
        let verifying_key = signing_key.verifying_key();
        let pk_hex = hex::encode(verifying_key.to_bytes());

        let message = "test-content-hash";
        let sig = signing_key.sign(message.as_bytes());
        let sig_hex = hex::encode(sig.to_bytes());

        (pk_hex, sig_hex)
    }

    #[test]
    fn test_verify_valid_signature() {
        let (pk_hex, sig_hex) = generate_test_keypair();
        let result = verify_signature(&pk_hex, "test-content-hash", &sig_hex);
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn test_verify_wrong_message() {
        let (pk_hex, sig_hex) = generate_test_keypair();
        let result = verify_signature(&pk_hex, "wrong-message", &sig_hex);
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn test_verify_invalid_public_key_hex() {
        let result = verify_signature("not-hex", "msg", "ab".repeat(64).as_str());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid public key hex"));
    }

    #[test]
    fn test_verify_invalid_signature_hex() {
        let pk_hex = "ab".repeat(32);
        let result = verify_signature(&pk_hex, "msg", "not-hex");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid signature hex"));
    }

    #[test]
    fn test_verify_wrong_key_length() {
        let result = verify_signature("abcd", "msg", &"ab".repeat(64));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("32 bytes"));
    }

    #[test]
    fn test_verify_wrong_signature_length() {
        let pk_hex = "ab".repeat(32);
        let result = verify_signature(&pk_hex, "msg", "abcd");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("64 bytes"));
    }
}
