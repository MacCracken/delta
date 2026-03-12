use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use delta_core::models::user::User;
use delta_core::{DeltaError, Result, db};
use sqlx::SqlitePool;

/// Hash a password with argon2.
pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| DeltaError::AuthFailed(e.to_string()))?;
    Ok(hash.to_string())
}

/// Verify a password against its hash.
pub fn verify_password(password: &str, hash: &str) -> Result<bool> {
    let parsed = match PasswordHash::new(hash) {
        Ok(h) => h,
        Err(_) => return Ok(false), // treat corrupted/invalid hash as non-match
    };
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

/// Generate a random API token. Returns (raw_token, hash).
pub fn generate_token() -> Result<(String, String)> {
    use base64::Engine;
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| DeltaError::Storage(format!("RNG failure: {}", e)))?;
    let raw = format!(
        "delta_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes),
    );
    let hash = blake3::hash(raw.as_bytes()).to_hex().to_string();
    Ok((raw, hash))
}

/// Hash a raw token for lookup.
pub fn hash_token(raw: &str) -> String {
    blake3::hash(raw.as_bytes()).to_hex().to_string()
}

/// Authenticate a user by token. Returns the user if valid.
pub async fn authenticate_token(pool: &SqlitePool, raw_token: &str) -> Result<User> {
    let token_hash = hash_token(raw_token);
    db::user::get_by_token_hash(pool, &token_hash)
        .await?
        .ok_or_else(|| DeltaError::AuthFailed("invalid or expired token".into()))
}

/// Register a new user. Returns the user.
pub async fn register(
    pool: &SqlitePool,
    username: &str,
    email: &str,
    password: &str,
    is_agent: bool,
) -> Result<User> {
    let pw_hash = hash_password(password)?;
    db::user::create(pool, username, email, &pw_hash, is_agent).await
}

/// Compute token expiry timestamp from config.
pub fn compute_expiry(expiry_secs: u64) -> Option<String> {
    if expiry_secs == 0 {
        return None;
    }
    let expires = chrono::Utc::now() + chrono::Duration::seconds(expiry_secs as i64);
    Some(expires.to_rfc3339())
}

/// Login with username/password. Returns (user, raw_token).
pub async fn login(
    pool: &SqlitePool,
    username: &str,
    password: &str,
    token_expiry_secs: u64,
) -> Result<(User, String)> {
    let (user_id, pw_hash) = db::user::get_password_hash(pool, username).await?;

    if !verify_password(password, &pw_hash)? {
        return Err(DeltaError::AuthFailed("invalid credentials".into()));
    }

    let (raw_token, token_hash) = generate_token()?;
    let expires_at = compute_expiry(token_expiry_secs);
    db::user::create_token(
        pool,
        &user_id,
        "login",
        &token_hash,
        "*",
        expires_at.as_deref(),
    )
    .await?;

    let user = db::user::get_by_id(pool, &user_id).await?;
    Ok((user, raw_token))
}
