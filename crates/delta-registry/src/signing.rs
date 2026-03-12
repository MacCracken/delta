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
    let pk_bytes = hex::decode(public_key_hex)
        .map_err(|e| format!("invalid public key hex: {}", e))?;
    let sig_bytes = hex::decode(signature_hex)
        .map_err(|e| format!("invalid signature hex: {}", e))?;

    let pk_array: [u8; 32] = pk_bytes
        .try_into()
        .map_err(|_| "public key must be 32 bytes".to_string())?;
    let sig_array: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| "signature must be 64 bytes".to_string())?;

    let verifying_key = VerifyingKey::from_bytes(&pk_array)
        .map_err(|e| format!("invalid public key: {}", e))?;
    let signature = Signature::from_bytes(&sig_array);

    use ed25519_dalek::Verifier;
    Ok(verifying_key.verify(message.as_bytes(), &signature).is_ok())
}
