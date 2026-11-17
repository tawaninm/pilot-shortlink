use argon2::{
    password_hash::{PasswordHasher, PasswordVerifier},
    Algorithm, Argon2, Params, Version,
};
use serde::{Deserialize, Serialize};
use worker::*;

use crate::http::{
    json_response_with_headers, now_ms, now_sec, problem_field_response, problem_response,
    rate_limited_response,
};
use crate::session::{
    authenticate_session, clear_session_cookie_header, create_session, delete_session,
    extract_session_cookie, session_cookie_header,
};

/// OWASP minimum Argon2id parameters (ADR-002): m=19456 KiB (19 MiB), t=2, p=1, 32-byte tag.
pub const ARGON2_M_COST: u32 = 19_456;
pub const ARGON2_T_COST: u32 = 2;
pub const ARGON2_P_COST: u32 = 1;
pub const ARGON2_OUTPUT_LEN: usize = 32;
pub const SALT_LEN: usize = 16;

/// Fixed dummy Argon2id PHC string with identical parameters (`m=19456,t=2,p=1`) used when
/// a login attempt references an unknown email so response timing matches a real user check.
pub const DUMMY_PASSWORD_HASH: &str =
    "$argon2id$v=19$m=19456,t=2,p=1$AAAAAAAAAAAAAAAAAAAAAA$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

/// Fixed 15-minute window (in seconds) and failure threshold for login rate limiting (`rate_limits`).
pub const LOGIN_WINDOW_SECS: i64 = 15 * 60;
pub const LOGIN_MAX_FAILURES: i64 = 5;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credentials {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct MeResponse<'a> {
    pub email: &'a str,
}

#[derive(Debug, Deserialize)]
struct UserIdRow {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct UserAuthRow {
    id: i64,
    email: String,
    password_hash: String,
}

#[derive(Debug, Deserialize)]
struct CountRow {
    count: i64,
}

fn argon2_instance() -> std::result::Result<Argon2<'static>, String> {
    let params = Params::new(
        ARGON2_M_COST,
        ARGON2_T_COST,
        ARGON2_P_COST,
        Some(ARGON2_OUTPUT_LEN),
    )
    .map_err(|e| format!("invalid Argon2 params: {e}"))?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

/// Hashes `password` with Argon2id (`m=19456, t=2, p=1`) and a random 16-byte CSPRNG salt.
/// Returns `(phc_string, elapsed_ms)`.
pub fn hash_password_argon2id(password: &str) -> std::result::Result<(String, u64), String> {
    let mut salt = [0u8; SALT_LEN];
    getrandom::fill(&mut salt).map_err(|e| format!("CSPRNG failure: {e}"))?;
    let hasher = argon2_instance()?;
    let start = now_ms();
    let hash = hasher
        .hash_password_with_salt(password.as_bytes(), &salt)
        .map_err(|e| format!("Argon2 hash failure: {e}"))?
        .to_string();
    let elapsed = now_ms().saturating_sub(start);
    Ok((hash, elapsed))
}

/// Verifies `password` against a PHC string `phc_hash` in constant time.
/// Returns `(is_valid, elapsed_ms)`.
pub fn verify_password_argon2id(password: &str, phc_hash: &str) -> (bool, u64) {
    let Ok(verifier) = argon2_instance() else {
        return (false, 0);
    };
    let start = now_ms();
    let ok = verifier
        .verify_password(password.as_bytes(), phc_hash)
        .is_ok();
    let elapsed = now_ms().saturating_sub(start);
    (ok, elapsed)
}

/// Normalizes an email address by trimming and lower-casing.
pub fn normalize_email(raw: &str) -> String {
    raw.trim().to_lowercase()
}

/// Validates basic email constraints from `api/openapi.yaml` (`maxLength: 254`, `format: email`).
pub fn is_valid_email(email: &str) -> bool {
    if email.is_empty() || email.len() > 254 {
        return false;
    }
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && !domain.contains('@')
        && !email.chars().any(char::is_whitespace)
}

/// Validates password length constraints (`10..=128` characters).
pub fn is_valid_password_len(password: &str) -> bool {
    let count = password.chars().count();
    (10..=128).contains(&count)
}

/// Computes `(window_start, retry_after_secs)` for the fixed 15-minute login rate-limit window.
pub fn login_window(now_sec: i64) -> (i64, i64) {
    let window_start = (now_sec / LOGIN_WINDOW_SECS) * LOGIN_WINDOW_SECS;
    let retry_after = ((window_start + LOGIN_WINDOW_SECS) - now_sec).max(1);
    (window_start, retry_after)
}

async fn is_login_rate_limited(db: &D1Database, key: &str, window_start: i64) -> Result<bool> {
    let row = db
        .prepare("SELECT count FROM rate_limits WHERE key = ?1 AND window_start = ?2")
        .bind(&[
            key.into(),
            wasm_bindgen::JsValue::from_f64(window_start as f64),
        ])?
        .first::<CountRow>(None)
        .await?;
    Ok(row.is_some_and(|r| r.count >= LOGIN_MAX_FAILURES))
}

async fn record_failed_login(db: &D1Database, key: &str, window_start: i64) -> Result<i64> {
    let row = db
        .prepare(
            "INSERT INTO rate_limits (key, window_start, count) VALUES (?1, ?2, 1) \
             ON CONFLICT(key, window_start) DO UPDATE SET count = count + 1 \
             RETURNING count",
        )
        .bind(&[
            key.into(),
            wasm_bindgen::JsValue::from_f64(window_start as f64),
        ])?
        .first::<CountRow>(None)
        .await?;

    // Clean up expired rate_limits windows on write (docs/architecture.md section 5).
    db.prepare("DELETE FROM rate_limits WHERE window_start < ?1")
        .bind(&[wasm_bindgen::JsValue::from_f64(window_start as f64)])?
        .run()
        .await?;

    Ok(row.map_or(1, |r| r.count))
}

/// `POST /api/auth/register`
pub async fn handle_register(
    req: &mut Request,
    db: &D1Database,
) -> Result<(Response, Option<u64>)> {
    let raw_body = req.bytes().await?;
    let Ok(creds) = serde_json::from_slice::<Credentials>(&raw_body) else {
        return Ok((
            problem_response(422, "Unprocessable Entity", "invalid_body")?,
            None,
        ));
    };

    if !is_valid_password_len(&creds.password) {
        return Ok((
            problem_field_response(422, "Unprocessable Entity", "weak_password", "password")?,
            None,
        ));
    }

    let email = normalize_email(&creds.email);
    if !is_valid_email(&email) {
        return Ok((
            problem_field_response(422, "Unprocessable Entity", "invalid_body", "email")?,
            None,
        ));
    }

    let (password_hash, hash_ms) =
        hash_password_argon2id(&creds.password).map_err(Error::RustError)?;

    let inserted = db
        .prepare(
            "INSERT INTO users (email, password_hash) VALUES (?1, ?2) \
             ON CONFLICT(email) DO NOTHING \
             RETURNING id",
        )
        .bind(&[email.as_str().into(), password_hash.as_str().into()])?
        .first::<UserIdRow>(None)
        .await?;

    let Some(user) = inserted else {
        return Ok((
            problem_field_response(409, "Conflict", "email_taken", "email")?,
            Some(hash_ms),
        ));
    };

    let token = create_session(db, user.id, now_sec()).await?;
    let cookie = session_cookie_header(&token);
    let body = serde_json::to_string(&MeResponse { email: &email })?;

    let res = json_response_with_headers(
        201,
        "application/json",
        body,
        &[("Set-Cookie", &cookie)],
    )?;
    Ok((res, Some(hash_ms)))
}

/// `POST /api/auth/login`
pub async fn handle_login(
    req: &mut Request,
    db: &D1Database,
) -> Result<(Response, Option<u64>)> {
    let raw_body = req.bytes().await?;
    let Ok(creds) = serde_json::from_slice::<Credentials>(&raw_body) else {
        return Ok((
            problem_response(422, "Unprocessable Entity", "invalid_body")?,
            None,
        ));
    };

    let email = normalize_email(&creds.email);
    let now = now_sec();
    let (window_start, retry_after) = login_window(now);
    let rate_key = format!("login:{email}");

    // Check rate limit BEFORE hashing the password (even if the password is right).
    if is_login_rate_limited(db, &rate_key, window_start).await? {
        return Ok((rate_limited_response(retry_after)?, None));
    }

    let user = db
        .prepare("SELECT id, email, password_hash FROM users WHERE email = ?1")
        .bind(&[email.as_str().into()])?
        .first::<UserAuthRow>(None)
        .await?;

    let target_hash = user
        .as_ref()
        .map_or(DUMMY_PASSWORD_HASH, |u| u.password_hash.as_str());
    let (password_ok, hash_ms) = verify_password_argon2id(&creds.password, target_hash);

    match (user, password_ok) {
        (Some(u), true) => {
            let token = create_session(db, u.id, now).await?;
            let cookie = session_cookie_header(&token);
            let body = serde_json::to_string(&MeResponse { email: &u.email })?;
            let res = json_response_with_headers(
                200,
                "application/json",
                body,
                &[("Set-Cookie", &cookie)],
            )?;
            Ok((res, Some(hash_ms)))
        }
        _ => {
            let _ = record_failed_login(db, &rate_key, window_start).await?;
            let res = problem_response(401, "Unauthorized", "unauthorized")?;
            Ok((res, Some(hash_ms)))
        }
    }
}

/// `POST /api/auth/logout`
pub async fn handle_logout(req: &Request, db: &D1Database) -> Result<Response> {
    let cookie_header = req.headers().get("Cookie")?;
    if let Some(token) = extract_session_cookie(cookie_header.as_deref()) {
        delete_session(db, token).await?;
    }

    let headers = Headers::new();
    headers.set("Set-Cookie", clear_session_cookie_header())?;
    Ok(Response::empty()?.with_status(204).with_headers(headers))
}

/// `GET /api/me`
pub async fn handle_me(req: &Request, db: &D1Database) -> Result<Response> {
    let Some(user) = authenticate_session(req, db, now_sec()).await? else {
        return problem_response(401, "Unauthorized", "unauthorized");
    };
    let body = serde_json::to_string(&MeResponse { email: &user.email })?;
    crate::http::json_response(200, "application/json", body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use argon2::password_hash::phc::PasswordHash;

    #[test]
    fn argon2id_phc_format_and_verification() {
        let (phc, _ms) = hash_password_argon2id("correct-horse-battery").expect("hash password");
        assert!(
            phc.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"),
            "unexpected PHC prefix: {phc}"
        );
        let parsed = PasswordHash::new(&phc).expect("parse PHC");
        assert_eq!(parsed.salt.expect("salt present").len(), SALT_LEN);

        let (ok, _) = verify_password_argon2id("correct-horse-battery", &phc);
        assert!(ok, "correct password must verify");

        let (bad, _) = verify_password_argon2id("wrong-password-123", &phc);
        assert!(!bad, "wrong password must fail verification");
    }

    #[test]
    fn dummy_hash_parses_and_verifies_with_full_argon2id_params() {
        let parsed = PasswordHash::new(DUMMY_PASSWORD_HASH).expect("valid dummy PHC hash");
        assert_eq!(parsed.algorithm.as_str(), "argon2id");
        assert_eq!(parsed.salt.expect("dummy salt").len(), SALT_LEN);
        let (ok, _) = verify_password_argon2id("correct-horse-battery", DUMMY_PASSWORD_HASH);
        assert!(!ok);
    }

    #[test]
    fn credentials_rejects_unknown_fields() {
        let raw = br#"{"email":"a@example.com","password":"correct-horse-battery","isAdmin":true}"#;
        assert!(serde_json::from_slice::<Credentials>(raw).is_err());
    }

    #[test]
    fn password_and_email_validation() {
        assert!(!is_valid_password_len("short"));
        assert!(is_valid_password_len("1234567890"));
        assert!(is_valid_password_len(&"a".repeat(128)));
        assert!(!is_valid_password_len(&"a".repeat(129)));

        assert_eq!(normalize_email("  Alice@Example.COM "), "alice@example.com");
        assert!(is_valid_email("alice@example.com"));
        assert!(!is_valid_email("not-an-email"));
        assert!(!is_valid_email("@example.com"));
        assert!(!is_valid_email("alice@"));
    }

    #[test]
    fn login_window_calculates_fixed_15_min_bucket_and_retry_after() {
        let (start, retry) = login_window(1_800_005);
        assert_eq!(start, 1_800_000);
        assert_eq!(retry, 895);

        let (start_edge, retry_edge) = login_window(1_800_899);
        assert_eq!(start_edge, 1_800_000);
        assert_eq!(retry_edge, 1);
    }
}
