use argon2::{
    password_hash::{phc::PasswordHash, PasswordHasher, PasswordVerifier},
    Algorithm, Argon2, Params, Version,
};
use serde::{Deserialize, Serialize};
use worker::*;

use crate::captcha::{CaptchaVerifier, EnvCaptchaVerifier};
use crate::http::{
    client_ip, json_response_with_headers, now_ms, now_sec, problem_field_response,
    problem_response, rate_limited_response, read_json_body,
};
use crate::mail::{spawn_purpose_mail, EnvMailer, TokenPurpose};
use crate::session::{
    authenticate_session, clear_session_cookie_header, create_session, delete_session,
    extract_session_cookie, generate_session_token, hash_session_token, session_cookie_header,
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

/// Non-PHC sentinel stored in `users.password_hash` for accounts created via OAuth (D2, I8, AC-O8).
pub const OAUTH_ONLY_PASSWORD_HASH: &str = "!oauth";

/// Fixed 15-minute window (in seconds) and failure threshold for login rate limiting (`rate_limits`).
pub const LOGIN_WINDOW_SECS: i64 = 15 * 60;
pub const LOGIN_MAX_FAILURES: i64 = 5;

/// Fixed 1-hour window (in seconds) and thresholds for email resend and forgot-password rate limiting.
pub const HOURLY_WINDOW_SECS: i64 = 60 * 60;
pub const RESEND_MAX_PER_HOUR: i64 = 3;
pub const FORGOT_MAX_PER_HOUR: i64 = 3;

/// Email token lifetime in seconds (30 minutes) and base64url character length (43 chars).
pub const EMAIL_TOKEN_EXPIRY_SECS: i64 = 30 * 60;
pub const EMAIL_TOKEN_LEN: usize = 43;

/// SQL statements shared with `tests/schema.rs` so SQLite tests exercise the exact queries handlers run.
pub const DELETE_UNUSED_EMAIL_TOKENS_SQL: &str =
    "DELETE FROM email_tokens WHERE user_id = ?1 AND purpose = ?2 AND used_at IS NULL";
pub const INSERT_EMAIL_TOKEN_SQL: &str =
    "INSERT INTO email_tokens (token_hash, user_id, purpose, expires_at) VALUES (?1, ?2, ?3, ?4)";
pub const CONSUME_EMAIL_TOKEN_SQL: &str = "UPDATE email_tokens \
    SET used_at = ?1 \
    WHERE token_hash = ?2 \
      AND purpose = ?3 \
      AND used_at IS NULL \
      AND expires_at > ?1 \
    RETURNING user_id";

/// Computes the Unix timestamp at which an email token issued at `now_sec` expires.
pub fn email_token_expires_at(now_sec: i64) -> i64 {
    now_sec + EMAIL_TOKEN_EXPIRY_SECS
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credentials {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterBody {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub captcha: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenBody {
    pub token: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForgotPasswordBody {
    pub email: String,
    #[serde(default)]
    pub captcha: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResetPasswordBody {
    pub token: String,
    pub password: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct MeResponse<'a> {
    pub email: &'a str,
    #[serde(rename = "emailVerified")]
    pub email_verified: bool,
}

#[derive(Debug, Deserialize)]
struct UserIdRow {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct ConsumedTokenRow {
    user_id: i64,
}

#[derive(Debug, Deserialize)]
struct UserAuthRow {
    id: i64,
    email: String,
    password_hash: String,
    email_verified_at: Option<String>,
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

/// Returns `true` if `stored_hash` is a syntactically valid PHC string (and not a sentinel like `"!oauth"`).
pub fn is_valid_phc_hash(stored_hash: &str) -> bool {
    PasswordHash::new(stored_hash).is_ok()
}

/// Selects the PHC hash to run Argon2id against for constant-time login checks.
/// Returns `(hash_to_verify, has_usable_password)`. When no user exists or `password_hash` is
/// not a valid PHC string (such as `"!oauth"`), falls back to `DUMMY_PASSWORD_HASH` with
/// `has_usable_password = false` so timing matches a real check and login always fails with 401 (AC-O8, I8).
pub fn select_verification_hash(stored_hash: Option<&str>) -> (&str, bool) {
    match stored_hash {
        Some(h) if is_valid_phc_hash(h) => (h, true),
        _ => (DUMMY_PASSWORD_HASH, false),
    }
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

/// Validates an email token against `^[A-Za-z0-9_-]{43}$` (`api/openapi.yaml` `#/components/schemas/EmailToken`).
pub fn is_valid_email_token(token: &str) -> bool {
    token.len() == EMAIL_TOKEN_LEN
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Generates a 43-character unpadded base64url email token from 32 CSPRNG bytes.
pub fn generate_email_token() -> std::result::Result<String, String> {
    generate_session_token()
}

/// Computes the 64-character lowercase hex SHA-256 digest of an email token.
pub fn hash_email_token(token: &str) -> String {
    hash_session_token(token)
}

/// Computes `(window_start, retry_after_secs)` for the fixed 15-minute login rate-limit window.
pub fn login_window(now_sec: i64) -> (i64, i64) {
    let window_start = (now_sec / LOGIN_WINDOW_SECS) * LOGIN_WINDOW_SECS;
    let retry_after = ((window_start + LOGIN_WINDOW_SECS) - now_sec).max(1);
    (window_start, retry_after)
}

/// Computes `(window_start, retry_after_secs)` for the fixed 1-hour email rate-limit window.
pub fn hourly_window(now_sec: i64) -> (i64, i64) {
    let window_start = (now_sec / HOURLY_WINDOW_SECS) * HOURLY_WINDOW_SECS;
    let retry_after = ((window_start + HOURLY_WINDOW_SECS) - now_sec).max(1);
    (window_start, retry_after)
}

pub(crate) async fn consume_rate_limit(
    db: &D1Database,
    key: &str,
    window_start: i64,
    now_sec: i64,
) -> Result<i64> {
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

    // Clean up rate_limits windows older than the longest window horizon (1 hour).
    let cleanup_cutoff = now_sec - HOURLY_WINDOW_SECS;
    db.prepare("DELETE FROM rate_limits WHERE window_start < ?1")
        .bind(&[wasm_bindgen::JsValue::from_f64(cleanup_cutoff as f64)])?
        .run()
        .await?;

    Ok(row.map_or(1, |r| r.count))
}

async fn clear_login_rate_limit(db: &D1Database, key: &str) -> Result<()> {
    db.prepare("DELETE FROM rate_limits WHERE key = ?1")
        .bind(&[key.into()])?
        .run()
        .await?;
    Ok(())
}

/// Issues a new single-use 30-minute email token for `(user_id, purpose)` in D1:
/// first deletes any unused tokens of the same user and purpose, then inserts the new SHA-256 hash.
/// Returns `(raw_token, token_hash)`.
pub async fn issue_email_token(
    db: &D1Database,
    user_id: i64,
    purpose: TokenPurpose,
    now_sec: i64,
) -> Result<(String, String)> {
    let token = generate_email_token().map_err(Error::RustError)?;
    let token_hash = hash_email_token(&token);
    let expires_at = email_token_expires_at(now_sec);

    let del_stmt = db.prepare(DELETE_UNUSED_EMAIL_TOKENS_SQL).bind(&[
        wasm_bindgen::JsValue::from_f64(user_id as f64),
        purpose.as_str().into(),
    ])?;

    let ins_stmt = db.prepare(INSERT_EMAIL_TOKEN_SQL).bind(&[
        token_hash.as_str().into(),
        wasm_bindgen::JsValue::from_f64(user_id as f64),
        purpose.as_str().into(),
        wasm_bindgen::JsValue::from_f64(expires_at as f64),
    ])?;

    db.batch(vec![del_stmt, ins_stmt]).await?;
    Ok((token, token_hash))
}

/// Atomically consumes a token for `purpose` in ONE statement:
/// `UPDATE email_tokens SET used_at = ? WHERE token_hash = ? AND purpose = ? AND used_at IS NULL AND expires_at > ? RETURNING user_id`.
/// Returns `Ok(Some(user_id))` if consumed, or `Ok(None)` if unknown, used, expired, or wrong purpose.
pub async fn consume_email_token(
    db: &D1Database,
    raw_token: &str,
    purpose: TokenPurpose,
    now_sec: i64,
) -> Result<Option<i64>> {
    let token_hash = hash_email_token(raw_token);
    let row = db
        .prepare(CONSUME_EMAIL_TOKEN_SQL)
        .bind(&[
            wasm_bindgen::JsValue::from_f64(now_sec as f64),
            token_hash.as_str().into(),
            purpose.as_str().into(),
        ])?
        .first::<ConsumedTokenRow>(None)
        .await?;

    Ok(row.map(|r| r.user_id))
}

/// `POST /api/auth/register`
pub async fn handle_register(
    req: &mut Request,
    db: &D1Database,
    app_origin: &str,
    mailer: EnvMailer,
    captcha: &EnvCaptchaVerifier,
    ctx: &Context,
) -> Result<(Response, Option<u64>)> {
    let raw_body = match read_json_body(req).await? {
        Ok(bytes) => bytes,
        Err(res) => return Ok((res, None)),
    };
    let Ok(creds) = serde_json::from_slice::<RegisterBody>(&raw_body) else {
        return Ok((
            problem_response(422, "Unprocessable Entity", "invalid_body")?,
            None,
        ));
    };

    let remote_ip = client_ip(req);
    if !captcha
        .verify(creds.captcha.as_deref(), remote_ip.as_deref())
        .await
    {
        return Ok((problem_response(403, "Forbidden", "captcha_failed")?, None));
    }

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

    let now = now_sec();
    let session_token = create_session(db, user.id, now).await?;
    let cookie = session_cookie_header(&session_token);
    let body = serde_json::to_string(&MeResponse {
        email: &email,
        email_verified: false,
    })?;

    match issue_email_token(db, user.id, TokenPurpose::Verify, now).await {
        Ok((verify_token, verify_token_hash)) => {
            spawn_purpose_mail(
                ctx,
                mailer,
                TokenPurpose::Verify,
                email,
                app_origin.to_string(),
                verify_token,
                verify_token_hash,
            );
        }
        Err(err) => {
            let err_line = crate::format_error_log(&err.to_string());
            console_error!("{err_line}");
        }
    }

    let res =
        json_response_with_headers(201, "application/json", body, &[("Set-Cookie", &cookie)])?;
    Ok((res, Some(hash_ms)))
}

/// `POST /api/auth/login`
pub async fn handle_login(req: &mut Request, db: &D1Database) -> Result<(Response, Option<u64>)> {
    let raw_body = match read_json_body(req).await? {
        Ok(bytes) => bytes,
        Err(res) => return Ok((res, None)),
    };
    let Ok(creds) = serde_json::from_slice::<Credentials>(&raw_body) else {
        return Ok((
            problem_response(422, "Unprocessable Entity", "invalid_body")?,
            None,
        ));
    };

    let email = normalize_email(&creds.email);
    if !is_valid_email(&email) {
        return Ok((
            problem_field_response(422, "Unprocessable Entity", "invalid_body", "email")?,
            None,
        ));
    }
    if !is_valid_password_len(&creds.password) {
        return Ok((
            problem_field_response(422, "Unprocessable Entity", "weak_password", "password")?,
            None,
        ));
    }

    let now = now_sec();
    let (window_start, retry_after) = login_window(now);
    let rate_key = format!("login:{email}");

    // Increment rate-limit bucket atomically FIRST; if count > 5, answer 429 without running Argon2.
    let attempt_count = consume_rate_limit(db, &rate_key, window_start, now).await?;
    if attempt_count > LOGIN_MAX_FAILURES {
        return Ok((rate_limited_response(retry_after)?, None));
    }

    let user = db
        .prepare("SELECT id, email, password_hash, email_verified_at FROM users WHERE email = ?1")
        .bind(&[email.as_str().into()])?
        .first::<UserAuthRow>(None)
        .await?;

    let (target_hash, has_usable_password) =
        select_verification_hash(user.as_ref().map(|u| u.password_hash.as_str()));
    let (password_ok, hash_ms) = verify_password_argon2id(&creds.password, target_hash);

    match (user, has_usable_password && password_ok) {
        (Some(u), true) => {
            clear_login_rate_limit(db, &rate_key).await?;
            let token = create_session(db, u.id, now).await?;
            let cookie = session_cookie_header(&token);
            let body = serde_json::to_string(&MeResponse {
                email: &u.email,
                email_verified: u.email_verified_at.is_some(),
            })?;
            let res = json_response_with_headers(
                200,
                "application/json",
                body,
                &[("Set-Cookie", &cookie)],
            )?;
            Ok((res, Some(hash_ms)))
        }
        _ => {
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
    let body = serde_json::to_string(&MeResponse {
        email: &user.email,
        email_verified: user.is_email_verified(),
    })?;
    crate::http::json_response(200, "application/json", body)
}

/// `POST /api/auth/verify-email`
pub async fn handle_verify_email(req: &mut Request, db: &D1Database) -> Result<Response> {
    let raw_body = match read_json_body(req).await? {
        Ok(bytes) => bytes,
        Err(res) => return Ok(res),
    };
    let Ok(payload) = serde_json::from_slice::<TokenBody>(&raw_body) else {
        return problem_response(422, "Unprocessable Entity", "invalid_body");
    };
    if !is_valid_email_token(&payload.token) {
        return problem_field_response(422, "Unprocessable Entity", "invalid_body", "token");
    }

    let now = now_sec();
    let Some(user_id) = consume_email_token(db, &payload.token, TokenPurpose::Verify, now).await?
    else {
        return problem_response(400, "Bad Request", "token_invalid");
    };

    db.prepare(
        "UPDATE users \
         SET email_verified_at = COALESCE(email_verified_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')) \
         WHERE id = ?1",
    )
    .bind(&[wasm_bindgen::JsValue::from_f64(user_id as f64)])?
    .run()
    .await?;

    Ok(Response::empty()?.with_status(204))
}

/// `POST /api/auth/verify-email/resend`
pub async fn handle_resend_verify_email(
    req: &Request,
    db: &D1Database,
    app_origin: &str,
    mailer: EnvMailer,
    ctx: &Context,
) -> Result<Response> {
    let now = now_sec();
    let Some(user) = authenticate_session(req, db, now).await? else {
        return problem_response(401, "Unauthorized", "unauthorized");
    };

    if user.is_email_verified() {
        return Ok(Response::empty()?.with_status(204));
    }

    let (window_start, retry_after) = hourly_window(now);
    let rate_key = format!("resend:{}", user.id);
    let count = consume_rate_limit(db, &rate_key, window_start, now).await?;
    if count > RESEND_MAX_PER_HOUR {
        return rate_limited_response(retry_after);
    }

    let (token, token_hash) = issue_email_token(db, user.id, TokenPurpose::Verify, now).await?;
    spawn_purpose_mail(
        ctx,
        mailer,
        TokenPurpose::Verify,
        user.email,
        app_origin.to_string(),
        token,
        token_hash,
    );

    Ok(Response::empty()?.with_status(204))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForgotPasswordDecision {
    InvalidBody,
    CaptchaFailed,
    InvalidEmail,
    OverQuota,
    Proceed,
}

/// Evaluates `POST /api/auth/password/forgot` gates in strict I9/AC-T4 order:
/// JSON shape (422) -> captcha (403 captcha_failed) -> email format (422) -> per-email counter (204).
pub async fn evaluate_forgot_password_gates<C, Q, Fut>(
    raw_body: &[u8],
    captcha: &C,
    remote_ip: Option<&str>,
    consume_quota: Q,
) -> Result<(ForgotPasswordDecision, Option<String>)>
where
    C: CaptchaVerifier,
    Q: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Result<i64>>,
{
    let Ok(payload) = serde_json::from_slice::<ForgotPasswordBody>(raw_body) else {
        return Ok((ForgotPasswordDecision::InvalidBody, None));
    };

    if !captcha.verify(payload.captcha.as_deref(), remote_ip).await {
        return Ok((ForgotPasswordDecision::CaptchaFailed, None));
    }

    let email = normalize_email(&payload.email);
    if !is_valid_email(&email) {
        return Ok((ForgotPasswordDecision::InvalidEmail, None));
    }

    let count = consume_quota(email.clone()).await?;
    if count > FORGOT_MAX_PER_HOUR {
        return Ok((ForgotPasswordDecision::OverQuota, None));
    }

    Ok((ForgotPasswordDecision::Proceed, Some(email)))
}

/// `POST /api/auth/password/forgot`
pub async fn handle_forgot_password(
    req: &mut Request,
    db: &D1Database,
    app_origin: &str,
    mailer: EnvMailer,
    captcha: &EnvCaptchaVerifier,
    ctx: &Context,
) -> Result<Response> {
    let raw_body = match read_json_body(req).await? {
        Ok(bytes) => bytes,
        Err(res) => return Ok(res),
    };
    let remote_ip = client_ip(req);
    let now = now_sec();
    let (window_start, _) = hourly_window(now);

    let (decision, maybe_email) = evaluate_forgot_password_gates(
        &raw_body,
        captcha,
        remote_ip.as_deref(),
        |email| async move {
            let rate_key = format!("forgot:{email}");
            consume_rate_limit(db, &rate_key, window_start, now).await
        },
    )
    .await?;

    let email = match decision {
        ForgotPasswordDecision::InvalidBody => {
            return problem_response(422, "Unprocessable Entity", "invalid_body");
        }
        ForgotPasswordDecision::CaptchaFailed => {
            return problem_response(403, "Forbidden", "captcha_failed");
        }
        ForgotPasswordDecision::InvalidEmail => {
            return problem_field_response(422, "Unprocessable Entity", "invalid_body", "email");
        }
        ForgotPasswordDecision::OverQuota => {
            return Ok(Response::empty()?.with_status(204));
        }
        ForgotPasswordDecision::Proceed => {
            maybe_email.ok_or_else(|| Error::RustError("missing email".to_string()))?
        }
    };

    let user = db
        .prepare("SELECT id FROM users WHERE email = ?1")
        .bind(&[email.as_str().into()])?
        .first::<UserIdRow>(None)
        .await?;

    if let Some(u) = user {
        match issue_email_token(db, u.id, TokenPurpose::Reset, now).await {
            Ok((token, token_hash)) => {
                spawn_purpose_mail(
                    ctx,
                    mailer,
                    TokenPurpose::Reset,
                    email,
                    app_origin.to_string(),
                    token,
                    token_hash,
                );
            }
            Err(err) => {
                let err_line = crate::format_error_log(&err.to_string());
                console_error!("{err_line}");
            }
        }
    }

    Ok(Response::empty()?.with_status(204))
}

/// `POST /api/auth/password/reset`
pub async fn handle_reset_password(
    req: &mut Request,
    db: &D1Database,
) -> Result<(Response, Option<u64>)> {
    let raw_body = match read_json_body(req).await? {
        Ok(bytes) => bytes,
        Err(res) => return Ok((res, None)),
    };
    let Ok(payload) = serde_json::from_slice::<ResetPasswordBody>(&raw_body) else {
        return Ok((
            problem_response(422, "Unprocessable Entity", "invalid_body")?,
            None,
        ));
    };

    // Validate password FIRST so a weak password returns 422 weak_password without consuming the token (AC-R7).
    if !is_valid_password_len(&payload.password) {
        return Ok((
            problem_field_response(422, "Unprocessable Entity", "weak_password", "password")?,
            None,
        ));
    }
    if !is_valid_email_token(&payload.token) {
        return Ok((
            problem_field_response(422, "Unprocessable Entity", "invalid_body", "token")?,
            None,
        ));
    }

    let now = now_sec();
    let Some(user_id) = consume_email_token(db, &payload.token, TokenPurpose::Reset, now).await?
    else {
        return Ok((problem_response(400, "Bad Request", "token_invalid")?, None));
    };

    let (password_hash, hash_ms) =
        hash_password_argon2id(&payload.password).map_err(Error::RustError)?;

    let update_user = db
        .prepare(
            "UPDATE users \
             SET password_hash = ?1, \
                 email_verified_at = COALESCE(email_verified_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')) \
             WHERE id = ?2",
        )
        .bind(&[
            password_hash.as_str().into(),
            wasm_bindgen::JsValue::from_f64(user_id as f64),
        ])?;

    let delete_sessions = db
        .prepare("DELETE FROM sessions WHERE user_id = ?1")
        .bind(&[wasm_bindgen::JsValue::from_f64(user_id as f64)])?;

    db.batch(vec![update_user, delete_sessions]).await?;

    Ok((Response::empty()?.with_status(204), Some(hash_ms)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::{send_purpose_mail, MailSendError, Mailer};
    use argon2::password_hash::phc::PasswordHash;
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context as TaskContext, Poll, RawWaker, RawWakerVTable, Waker};

    fn block_on_ready<F: Future>(mut fut: F) -> F::Output {
        fn noop_raw_waker() -> RawWaker {
            fn no_op(_: *const ()) {}
            fn clone(_: *const ()) -> RawWaker {
                noop_raw_waker()
            }
            static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, no_op, no_op, no_op);
            RawWaker::new(std::ptr::null(), &VTABLE)
        }
        let waker = unsafe { Waker::from_raw(noop_raw_waker()) };
        let mut ctx = TaskContext::from_waker(&waker);
        let pinned = unsafe { Pin::new_unchecked(&mut fut) };
        match pinned.poll(&mut ctx) {
            Poll::Ready(val) => val,
            Poll::Pending => panic!("test future must resolve immediately"),
        }
    }

    struct FailingTestMailer(MailSendError);

    impl Mailer for FailingTestMailer {
        async fn send(
            &self,
            _to: &str,
            _subject: &str,
            _text: &str,
            _html: &str,
            _idempotency_key: &str,
        ) -> std::result::Result<(), MailSendError> {
            Err(self.0)
        }
    }

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
    fn oauth_only_sentinel_never_verifies_and_uses_dummy_hash_for_equal_timing() {
        assert!(!is_valid_phc_hash(OAUTH_ONLY_PASSWORD_HASH));
        assert!(!is_valid_phc_hash(""));
        assert!(!is_valid_phc_hash("not-a-phc"));

        let (selected, usable) = select_verification_hash(Some(OAUTH_ONLY_PASSWORD_HASH));
        assert_eq!(selected, DUMMY_PASSWORD_HASH);
        assert!(!usable);

        let (selected_none, usable_none) = select_verification_hash(None);
        assert_eq!(selected_none, DUMMY_PASSWORD_HASH);
        assert!(!usable_none);

        for candidate in ["correct-horse-battery", "!oauth", ""] {
            let (ok_raw, _) = verify_password_argon2id(candidate, OAUTH_ONLY_PASSWORD_HASH);
            assert!(
                !ok_raw,
                "!oauth must never verify for password {candidate:?}"
            );
            let (ok_dummy, _) = verify_password_argon2id(candidate, selected);
            assert!(!ok_dummy);
        }

        let (real_phc, _) = hash_password_argon2id("correct-horse-battery").expect("hash");
        let (selected_real, usable_real) = select_verification_hash(Some(&real_phc));
        assert_eq!(selected_real, real_phc);
        assert!(usable_real);
    }

    #[test]
    fn credentials_and_token_bodies_reject_unknown_fields() {
        let raw = br#"{"email":"a@example.com","password":"correct-horse-battery","isAdmin":true}"#;
        assert!(serde_json::from_slice::<Credentials>(raw).is_err());

        let reg_extra = br#"{"email":"a@example.com","password":"correct-horse-battery","captcha":"fake-pass","isAdmin":true}"#;
        assert!(serde_json::from_slice::<RegisterBody>(reg_extra).is_err());

        let reg_wrong_type =
            br#"{"email":"a@example.com","password":"correct-horse-battery","captcha":123}"#;
        assert!(serde_json::from_slice::<RegisterBody>(reg_wrong_type).is_err());

        let reg_missing_captcha =
            br#"{"email":"a@example.com","password":"correct-horse-battery"}"#;
        let parsed_reg = serde_json::from_slice::<RegisterBody>(reg_missing_captcha)
            .expect("missing captcha parses");
        assert_eq!(parsed_reg.captcha, None);

        let extra_token = br#"{"token":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","extra":1}"#;
        assert!(serde_json::from_slice::<TokenBody>(extra_token).is_err());

        let extra_forgot = br#"{"email":"a@example.com","extra":1}"#;
        assert!(serde_json::from_slice::<ForgotPasswordBody>(extra_forgot).is_err());

        let forgot_missing_captcha = br#"{"email":"a@example.com"}"#;
        let parsed_forgot = serde_json::from_slice::<ForgotPasswordBody>(forgot_missing_captcha)
            .expect("missing captcha parses");
        assert_eq!(parsed_forgot.captcha, None);

        let extra_reset =
            br#"{"token":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","password":"correct-horse-battery","extra":1}"#;
        assert!(serde_json::from_slice::<ResetPasswordBody>(extra_reset).is_err());
    }

    #[test]
    fn password_email_and_token_validation() {
        assert!(!is_valid_password_len("short"));
        assert!(is_valid_password_len("1234567890"));
        assert!(is_valid_password_len(&"a".repeat(128)));
        assert!(!is_valid_password_len(&"a".repeat(129)));

        assert_eq!(normalize_email("  Alice@Example.COM "), "alice@example.com");
        assert!(is_valid_email("alice@example.com"));
        assert!(!is_valid_email("not-an-email"));
        assert!(!is_valid_email("@example.com"));
        assert!(!is_valid_email("alice@"));

        let valid_tok = generate_email_token().expect("generate email token");
        assert!(is_valid_email_token(&valid_tok));
        assert!(!is_valid_email_token("short"));
        assert!(!is_valid_email_token(&"A".repeat(42)));
        assert!(!is_valid_email_token(&"A".repeat(44)));
        assert!(!is_valid_email_token(&format!("{}+", "A".repeat(42))));
    }

    #[test]
    fn login_and_hourly_windows_calculate_fixed_buckets_and_retry_after() {
        let (start, retry) = login_window(1_800_005);
        assert_eq!(start, 1_800_000);
        assert_eq!(retry, 895);

        let (start_edge, retry_edge) = login_window(1_800_899);
        assert_eq!(start_edge, 1_800_000);
        assert_eq!(retry_edge, 1);

        let (h_start, h_retry) = hourly_window(3_600_100);
        assert_eq!(h_start, 3_600_000);
        assert_eq!(h_retry, 3_500);
    }

    #[test]
    fn email_token_expiry_is_thirty_minutes() {
        assert_eq!(EMAIL_TOKEN_EXPIRY_SECS, 1_800);
        assert_eq!(email_token_expires_at(1_700_000_000), 1_700_001_800);
    }

    #[test]
    fn failing_mailer_logs_without_leaking_and_register_payload_succeeds() {
        let email = "newuser@example.test";
        let session_tok = generate_session_token().expect("session token");
        let verify_tok = generate_email_token().expect("verify token");
        let verify_hash = hash_email_token(&verify_tok);

        let cookie = session_cookie_header(&session_tok);
        let body = serde_json::to_string(&MeResponse {
            email,
            email_verified: false,
        })
        .expect("serialize MeResponse");
        assert!(cookie.starts_with("__Host-session="));
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
        assert_eq!(parsed["email"], email);
        assert_eq!(parsed["emailVerified"], false);

        for err in [
            MailSendError::HttpStatus(500),
            MailSendError::HttpStatus(429),
            MailSendError::MissingApiKey,
            MailSendError::Transport,
        ] {
            let mailer = FailingTestMailer(err);
            let log_line = block_on_ready(send_purpose_mail(
                &mailer,
                TokenPurpose::Verify,
                email,
                "http://localhost:8787",
                &verify_tok,
                &verify_hash,
            ));

            assert!(!log_line.contains(&verify_tok));
            assert!(!log_line.contains(&verify_hash));
            assert!(!log_line.contains(email));
            assert!(log_line.contains(r#""result":"failed""#));
        }
    }

    #[test]
    fn forgot_password_gates_verify_captcha_before_counting_per_email_quota() {
        use crate::captcha::FakeCaptchaVerifier;
        use std::cell::Cell;

        let verifier = FakeCaptchaVerifier;

        // 1. Malformed JSON (unknown field) fails before captcha or quota counter.
        let quota_calls = Cell::new(0_i64);
        let res = block_on_ready(evaluate_forgot_password_gates(
            br#"{"email":"alice@example.test","captcha":"nope","extra":1}"#,
            &verifier,
            None,
            |_| {
                quota_calls.set(quota_calls.get() + 1);
                async { Ok(1) }
            },
        ))
        .expect("gate eval");
        assert_eq!(res, (ForgotPasswordDecision::InvalidBody, None));
        assert_eq!(quota_calls.get(), 0);

        // 2. Wrong or missing captcha fails with CaptchaFailed WITHOUT touching per-email quota (AC-T4, I9).
        for body in [
            br#"{"email":"alice@example.test"}"#.as_slice(),
            br#"{"email":"alice@example.test","captcha":"nope"}"#.as_slice(),
            br#"{"email":"not-an-email","captcha":"nope"}"#.as_slice(),
        ] {
            let calls = Cell::new(0_i64);
            let res = block_on_ready(evaluate_forgot_password_gates(
                body,
                &verifier,
                None,
                |_| {
                    calls.set(calls.get() + 1);
                    async { Ok(1) }
                },
            ))
            .expect("gate eval");
            assert_eq!(res, (ForgotPasswordDecision::CaptchaFailed, None));
            assert_eq!(
                calls.get(),
                0,
                "failed captcha must never increment per-email quota"
            );
        }

        // 3. Valid captcha + invalid email fails with InvalidEmail WITHOUT touching quota.
        let calls = Cell::new(0_i64);
        let res = block_on_ready(evaluate_forgot_password_gates(
            br#"{"email":"not-an-email","captcha":"fake-pass"}"#,
            &verifier,
            None,
            |_| {
                calls.set(calls.get() + 1);
                async { Ok(1) }
            },
        ))
        .expect("gate eval");
        assert_eq!(res, (ForgotPasswordDecision::InvalidEmail, None));
        assert_eq!(calls.get(), 0);

        // 4. Valid captcha + valid email over quota returns OverQuota (silent 204).
        let calls = Cell::new(0_i64);
        let res = block_on_ready(evaluate_forgot_password_gates(
            br#"{"email":"  Alice@Example.TEST ","captcha":"fake-pass"}"#,
            &verifier,
            None,
            |email| {
                assert_eq!(email, "alice@example.test");
                calls.set(calls.get() + 1);
                async { Ok(FORGOT_MAX_PER_HOUR + 1) }
            },
        ))
        .expect("gate eval");
        assert_eq!(res, (ForgotPasswordDecision::OverQuota, None));
        assert_eq!(calls.get(), 1);

        // 5. Valid captcha + valid email within quota proceeds with normalized email.
        let calls = Cell::new(0_i64);
        let res = block_on_ready(evaluate_forgot_password_gates(
            br#"{"email":"  Alice@Example.TEST ","captcha":"fake-pass"}"#,
            &verifier,
            None,
            |email| {
                assert_eq!(email, "alice@example.test");
                calls.set(calls.get() + 1);
                async { Ok(FORGOT_MAX_PER_HOUR) }
            },
        ))
        .expect("gate eval");
        assert_eq!(
            res,
            (
                ForgotPasswordDecision::Proceed,
                Some("alice@example.test".to_string())
            )
        );
        assert_eq!(calls.get(), 1);
    }
}
