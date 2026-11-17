use base64ct::{Base64UrlUnpadded, Encoding};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use worker::*;

/// Session lifetime in seconds (7 days).
pub const SESSION_MAX_AGE_SECS: i64 = 604_800;

/// Raw random byte length of a session token (encodes to 43 base64url chars without padding).
pub const SESSION_TOKEN_BYTES: usize = 32;

#[derive(Debug, Clone, Deserialize)]
pub struct SessionUser {
    pub id: i64,
    pub email: String,
}

/// Generates a 43-character unpadded base64url token from 32 CSPRNG bytes.
pub fn generate_session_token() -> std::result::Result<String, String> {
    let mut bytes = [0u8; SESSION_TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|e| format!("CSPRNG failure: {e}"))?;
    Ok(Base64UrlUnpadded::encode_string(&bytes))
}

/// Computes the lowercase hex-encoded SHA-256 digest of `token` (stored in `sessions.token_hash`).
pub fn hash_session_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    hex
}

/// Formats the `Set-Cookie` header value for a newly issued 7-day session.
pub fn session_cookie_header(token: &str) -> String {
    format!("session={token}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age={SESSION_MAX_AGE_SECS}")
}

/// Formats the `Set-Cookie` header value that clears the session cookie on logout.
pub fn clear_session_cookie_header() -> &'static str {
    "session=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0"
}

/// Extracts the `session` cookie value from a raw `Cookie` request header.
pub fn extract_session_cookie(cookie_header: Option<&str>) -> Option<&str> {
    let raw = cookie_header?;
    for part in raw.split(';') {
        let trimmed = part.trim();
        if let Some(val) = trimmed.strip_prefix("session=") {
            if !val.is_empty() {
                return Some(val);
            }
        }
    }
    None
}

/// Creates a new session row in D1 (`sessions.token_hash` stores SHA-256 hex of the token)
/// and returns the raw 43-char base64url token to set in the response cookie.
pub async fn create_session(db: &D1Database, user_id: i64, now_sec: i64) -> Result<String> {
    let token = generate_session_token().map_err(Error::RustError)?;
    let token_hash = hash_session_token(&token);
    let expires_at = now_sec + SESSION_MAX_AGE_SECS;

    db.prepare("INSERT INTO sessions (token_hash, user_id, expires_at) VALUES (?1, ?2, ?3)")
        .bind(&[
            token_hash.into(),
            wasm_bindgen::JsValue::from_f64(user_id as f64),
            wasm_bindgen::JsValue::from_f64(expires_at as f64),
        ])?
        .run()
        .await?;

    Ok(token)
}

/// Deletes the session matching `token` from D1 if it exists.
pub async fn delete_session(db: &D1Database, token: &str) -> Result<()> {
    let token_hash = hash_session_token(token);
    db.prepare("DELETE FROM sessions WHERE token_hash = ?1")
        .bind(&[token_hash.into()])?
        .run()
        .await?;
    Ok(())
}

/// Looks up the authenticated user for a request's `session` cookie if present and not expired.
pub async fn authenticate_session(
    req: &Request,
    db: &D1Database,
    now_sec: i64,
) -> Result<Option<SessionUser>> {
    let cookie_header = req.headers().get("Cookie")?;
    let Some(token) = extract_session_cookie(cookie_header.as_deref()) else {
        return Ok(None);
    };
    let token_hash = hash_session_token(token);

    let user = db
        .prepare(
            "SELECT u.id AS id, u.email AS email \
             FROM sessions s \
             JOIN users u ON u.id = s.user_id \
             WHERE s.token_hash = ?1 AND s.expires_at > ?2",
        )
        .bind(&[
            token_hash.into(),
            wasm_bindgen::JsValue::from_f64(now_sec as f64),
        ])?
        .first::<SessionUser>(None)
        .await?;

    Ok(user)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_token_is_43_char_base64url_and_unique() {
        let t1 = generate_session_token().expect("generate token 1");
        let t2 = generate_session_token().expect("generate token 2");
        assert_eq!(t1.len(), 43);
        assert_eq!(t2.len(), 43);
        assert_ne!(t1, t2);
        assert!(t1
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn session_token_hash_is_64_char_lowercase_sha256_hex() {
        let token = "A".repeat(43);
        let hash = hash_session_token(&token);
        assert_eq!(hash.len(), 64);
        assert!(hash
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(hash, token);
    }

    #[test]
    fn session_cookie_format_matches_openapi_spec() {
        let token = "A".repeat(43);
        let header = session_cookie_header(&token);
        assert_eq!(
            header,
            format!("session={token}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=604800")
        );
        assert_eq!(
            clear_session_cookie_header(),
            "session=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0"
        );
    }

    #[test]
    fn extract_session_cookie_parses_cookie_header() {
        assert_eq!(extract_session_cookie(None), None);
        assert_eq!(extract_session_cookie(Some("")), None);
        assert_eq!(extract_session_cookie(Some("session=")), None);
        assert_eq!(
            extract_session_cookie(Some("other=1; session=abc_123-xyz; theme=dark")),
            Some("abc_123-xyz")
        );
    }
}
