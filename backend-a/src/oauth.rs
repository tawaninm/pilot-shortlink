use base64ct::{Base64UrlUnpadded, Encoding};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use worker::*;

use crate::auth::{consume_rate_limit, is_valid_email, normalize_email, OAUTH_ONLY_PASSWORD_HASH};
use crate::http::{
    client_ip, encode_form_urlencoded, fetch_text_with_timeout, now_sec, read_env_string,
    OUTBOUND_FETCH_TIMEOUT,
};
use crate::mail::is_local_origin;
use crate::session::{
    create_session, generate_session_token, hash_session_token, session_cookie_header,
};

/// OAuth state lifetime in seconds (10 minutes, I3).
pub const OAUTH_STATE_TTL_SECS: i64 = 600;

/// Callback rate-limit window (10 minutes) and threshold per client IP.
pub const OAUTH_CALLBACK_WINDOW_SECS: i64 = 10 * 60;
pub const OAUTH_CALLBACK_MAX_PER_WINDOW: i64 = 30;

/// Cookie binding the OAuth state to the browser that started the flow (I3).
pub const OAUTH_COOKIE_NAME: &str = "__Host-oauth";

/// Fixed error destination for every OAuth failure (I4).
pub const OAUTH_ERROR_LOCATION: &str = "/login?error=oauth";

// Verified against https://accounts.google.com/.well-known/openid-configuration on 2026-11-19.
pub const GOOGLE_AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub const GOOGLE_USERINFO_URL: &str = "https://openidconnect.googleapis.com/v1/userinfo";
pub const GOOGLE_SCOPE: &str = "openid email profile";

pub const GITHUB_AUTHORIZE_URL: &str = "https://github.com/login/oauth/authorize";
pub const GITHUB_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
pub const GITHUB_USER_URL: &str = "https://api.github.com/user";
pub const GITHUB_EMAILS_URL: &str = "https://api.github.com/user/emails";
pub const GITHUB_SCOPE: &str = "read:user user:email";

pub const FAKE_CLIENT_ID: &str = "fake-client";
pub const FAKE_CLIENT_SECRET: &str = "fake-secret";

/// SQL statements shared with `tests/schema.rs` so SQLite tests exercise the exact queries handlers run.
pub const DELETE_EXPIRED_OAUTH_STATES_SQL: &str = "DELETE FROM oauth_states WHERE expires_at <= ?1";
pub const INSERT_OAUTH_STATE_SQL: &str =
    "INSERT INTO oauth_states (state_hash, provider, verifier, expires_at) VALUES (?1, ?2, ?3, ?4)";
pub const CONSUME_OAUTH_STATE_SQL: &str = "DELETE FROM oauth_states \
    WHERE state_hash = ?1 AND provider = ?2 AND expires_at > ?3 \
    RETURNING verifier";
pub const INSERT_OAUTH_ACCOUNT_SQL: &str =
    "INSERT INTO oauth_accounts (provider, subject, user_id, created_at) \
    VALUES (?1, ?2, ?3, ?4) \
    ON CONFLICT(provider, subject) DO NOTHING";
pub const SELECT_OAUTH_ACCOUNT_USER_SQL: &str =
    "SELECT user_id FROM oauth_accounts WHERE provider = ?1 AND subject = ?2";
pub const WIPE_UNVERIFIED_USER_SESSIONS_SQL: &str = "DELETE FROM sessions \
    WHERE user_id = ?1 \
      AND EXISTS (SELECT 1 FROM users WHERE id = ?1 AND email_verified_at IS NULL)";
pub const WIPE_UNVERIFIED_USER_EMAIL_TOKENS_SQL: &str = "DELETE FROM email_tokens \
    WHERE user_id = ?1 \
      AND used_at IS NULL \
      AND EXISTS (SELECT 1 FROM users WHERE id = ?1 AND email_verified_at IS NULL)";
pub const CLAIM_UNVERIFIED_USER_SQL: &str = "UPDATE users \
    SET password_hash = ?1, \
        email_verified_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
    WHERE id = ?2 AND email_verified_at IS NULL";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Google,
    Github,
}

impl Provider {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "google" => Some(Self::Google),
            "github" => Some(Self::Github),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::Github => "github",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthFailureReason {
    MissingCredentials,
    RateLimited,
    ProviderError,
    MissingParams,
    StateMismatch,
    InvalidState,
    TokenExchangeFailed,
    IdentityFetchFailed,
    EmailUnverified,
    AccountError,
}

impl OAuthFailureReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MissingCredentials => "missing_credentials",
            Self::RateLimited => "rate_limited",
            Self::ProviderError => "provider_error",
            Self::MissingParams => "missing_params",
            Self::StateMismatch => "state_mismatch",
            Self::InvalidState => "invalid_state",
            Self::TokenExchangeFailed => "token_exchange_failed",
            Self::IdentityFetchFailed => "identity_fetch_failed",
            Self::EmailUnverified => "email_unverified",
            Self::AccountError => "account_error",
        }
    }
}

#[derive(Serialize)]
struct OAuthFailureLog {
    event: &'static str,
    provider: &'static str,
    reason: &'static str,
}

/// Formats a single-line structured JSON log for an OAuth failure.
/// All fields are static enums so logs can never leak code, state, verifier, token, email, or response bodies (I7).
pub fn format_oauth_failure_log(provider: Provider, reason: OAuthFailureReason) -> String {
    serde_json::to_string(&OAuthFailureLog {
        event: "oauth_failed",
        provider: provider.as_str(),
        reason: reason.as_str(),
    })
    .expect("OAuthFailureLog serialization cannot fail")
}

fn emit_oauth_failure_log(provider: Provider, reason: OAuthFailureReason) {
    let line = format_oauth_failure_log(provider, reason);
    #[cfg(target_arch = "wasm32")]
    console_error!("{line}");
    #[cfg(not(target_arch = "wasm32"))]
    let _ = line;
}

/// Computes the Unix timestamp at which an OAuth state issued at `now_sec` expires.
pub fn oauth_state_expires_at(now_sec: i64) -> i64 {
    now_sec + OAUTH_STATE_TTL_SECS
}

/// Computes the fixed 10-minute callback rate-limit window start.
pub fn oauth_callback_window(now_sec: i64) -> i64 {
    (now_sec / OAUTH_CALLBACK_WINDOW_SECS) * OAUTH_CALLBACK_WINDOW_SECS
}

/// Computes the PKCE S256 code challenge: `BASE64URL-UNPADDED(SHA256(ASCII(verifier)))` (RFC 7636).
pub fn pkce_s256_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    Base64UrlUnpadded::encode_string(&digest)
}

/// Constant-time byte slice equality check for comparing state query parameter against cookie.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Formats the `Set-Cookie` header value that binds the OAuth state to this browser (10 minutes).
pub fn oauth_cookie_header(state: &str) -> String {
    format!(
        "{OAUTH_COOKIE_NAME}={state}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age={OAUTH_STATE_TTL_SECS}"
    )
}

/// Formats the `Set-Cookie` header value that clears `__Host-oauth` after callback.
pub fn clear_oauth_cookie_header() -> &'static str {
    "__Host-oauth=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0"
}

/// Extracts the `__Host-oauth` cookie value from a raw `Cookie` request header.
pub fn extract_oauth_cookie(cookie_header: Option<&str>) -> Option<&str> {
    let raw = cookie_header?;
    for part in raw.split(';') {
        let trimmed = part.trim();
        if let Some(val) = trimmed.strip_prefix("__Host-oauth=") {
            if !val.is_empty() {
                return Some(val);
            }
        }
    }
    None
}

/// Builds the fixed callback URL from `APP_ORIGIN` (never from `Host` or forwarded headers, I5).
pub fn build_redirect_uri(app_origin: &str, provider: Provider) -> String {
    format!(
        "{}/api/auth/oauth/{}/callback",
        app_origin.trim_end_matches('/'),
        provider.as_str()
    )
}

/// Returns `true` only when `OAUTH_MODE == "fake"` AND `APP_ORIGIN` is a local origin (I6, AC-O11).
pub fn is_fake_oauth_enabled(oauth_mode: Option<&str>, app_origin: &str) -> bool {
    oauth_mode == Some("fake") && is_local_origin(app_origin)
}

#[derive(Clone, PartialEq, Eq)]
pub struct ProviderConfig {
    pub provider: Provider,
    pub authorize_url: String,
    pub token_url: String,
    pub userinfo_url: String,
    pub emails_url: Option<String>,
    pub scope: &'static str,
    pub client_id: String,
    pub client_secret: String,
}

// Manual Debug so `{:?}` never prints OAuth client secrets into Workers logs (I7).
impl std::fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderConfig")
            .field("provider", &self.provider)
            .field("authorize_url", &self.authorize_url)
            .field("token_url", &self.token_url)
            .field("userinfo_url", &self.userinfo_url)
            .field("emails_url", &self.emails_url)
            .field("scope", &self.scope)
            .field("client_id", &self.client_id)
            .field("client_secret", &"<redacted>")
            .finish()
    }
}

impl ProviderConfig {
    /// Resolves provider endpoints and credentials from raw config strings.
    /// Fake mode is ignored unless `APP_ORIGIN` is a local origin (I6, AC-O11).
    pub fn resolve(
        provider: Provider,
        app_origin: &str,
        oauth_mode: Option<&str>,
        fake_base: Option<&str>,
        client_id: Option<&str>,
        client_secret: Option<&str>,
    ) -> Option<Self> {
        if is_fake_oauth_enabled(oauth_mode, app_origin) {
            let base = fake_base
                .map(str::trim)
                .filter(|s| !s.is_empty())?
                .trim_end_matches('/');
            return Some(match provider {
                Provider::Google => Self {
                    provider,
                    authorize_url: format!("{base}/google/authorize"),
                    token_url: format!("{base}/google/token"),
                    userinfo_url: format!("{base}/google/userinfo"),
                    emails_url: None,
                    scope: GOOGLE_SCOPE,
                    client_id: FAKE_CLIENT_ID.to_string(),
                    client_secret: FAKE_CLIENT_SECRET.to_string(),
                },
                Provider::Github => Self {
                    provider,
                    authorize_url: format!("{base}/github/login/oauth/authorize"),
                    token_url: format!("{base}/github/login/oauth/access_token"),
                    userinfo_url: format!("{base}/github/user"),
                    emails_url: Some(format!("{base}/github/user/emails")),
                    scope: GITHUB_SCOPE,
                    client_id: FAKE_CLIENT_ID.to_string(),
                    client_secret: FAKE_CLIENT_SECRET.to_string(),
                },
            });
        }

        let cid = client_id
            .map(str::trim)
            .filter(|s| !s.is_empty())?
            .to_string();
        let csec = client_secret
            .map(str::trim)
            .filter(|s| !s.is_empty())?
            .to_string();

        Some(match provider {
            Provider::Google => Self {
                provider,
                authorize_url: GOOGLE_AUTHORIZE_URL.to_string(),
                token_url: GOOGLE_TOKEN_URL.to_string(),
                userinfo_url: GOOGLE_USERINFO_URL.to_string(),
                emails_url: None,
                scope: GOOGLE_SCOPE,
                client_id: cid,
                client_secret: csec,
            },
            Provider::Github => Self {
                provider,
                authorize_url: GITHUB_AUTHORIZE_URL.to_string(),
                token_url: GITHUB_TOKEN_URL.to_string(),
                userinfo_url: GITHUB_USER_URL.to_string(),
                emails_url: Some(GITHUB_EMAILS_URL.to_string()),
                scope: GITHUB_SCOPE,
                client_id: cid,
                client_secret: csec,
            },
        })
    }

    pub fn from_env(provider: Provider, env: &Env, app_origin: &str) -> Option<Self> {
        let oauth_mode = read_env_string(env, "OAUTH_MODE");
        let fake_base = read_env_string(env, "OAUTH_FAKE_BASE");
        let (id_var, secret_var) = match provider {
            Provider::Google => ("GOOGLE_CLIENT_ID", "GOOGLE_CLIENT_SECRET"),
            Provider::Github => ("GITHUB_CLIENT_ID", "GITHUB_CLIENT_SECRET"),
        };
        let client_id = read_env_string(env, id_var);
        let client_secret = read_env_string(env, secret_var);

        Self::resolve(
            provider,
            app_origin,
            oauth_mode.as_deref(),
            fake_base.as_deref(),
            client_id.as_deref(),
            client_secret.as_deref(),
        )
    }

    /// Builds the provider authorization URL with PKCE S256 and fixed `redirect_uri` (AC-O6).
    pub fn build_authorize_url(
        &self,
        redirect_uri: &str,
        state: &str,
        challenge: &str,
    ) -> std::result::Result<String, String> {
        let mut url =
            Url::parse(&self.authorize_url).map_err(|e| format!("invalid authorize_url: {e}"))?;
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("scope", self.scope)
            .append_pair("state", state)
            .append_pair("code_challenge", challenge)
            .append_pair("code_challenge_method", "S256");
        Ok(url.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthIdentity {
    pub subject: String,
    pub email: Option<String>,
    pub email_verified: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GithubEmail {
    pub email: String,
    pub primary: bool,
    pub verified: bool,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleUserInfo {
    sub: Option<String>,
    email: Option<String>,
    email_verified: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct GithubUser {
    id: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ConsumedStateRow {
    verifier: String,
}

#[derive(Debug, Deserialize)]
struct OAuthUserIdRow {
    user_id: i64,
}

#[derive(Debug, Deserialize)]
struct UserIdRow {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct UserEmailRow {
    id: i64,
    email_verified_at: Option<String>,
}

/// Selects the sole usable GitHub email: `primary == true && verified == true` (AC-O2, I1).
/// Returns the trimmed, lower-cased email if valid, or `None` if no primary verified email exists.
pub fn select_github_primary_verified_email(emails: &[GithubEmail]) -> Option<String> {
    emails
        .iter()
        .find(|e| e.primary && e.verified)
        .map(|e| normalize_email(&e.email))
        .filter(|e| is_valid_email(e))
}

/// Parses Google's OIDC `/userinfo` JSON payload into an `OAuthIdentity`.
pub fn parse_google_userinfo(body: &str) -> Option<OAuthIdentity> {
    let info: GoogleUserInfo = serde_json::from_str(body).ok()?;
    let subject = info.sub?.trim().to_string();
    if subject.is_empty() {
        return None;
    }
    let email = info
        .email
        .map(|e| normalize_email(&e))
        .filter(|e| is_valid_email(e));
    let email_verified = info.email_verified == Some(true) && email.is_some();
    Some(OAuthIdentity {
        subject,
        email,
        email_verified,
    })
}

async fn exchange_code(
    config: &ProviderConfig,
    code: &str,
    redirect_uri: &str,
    verifier: &str,
) -> std::result::Result<String, OAuthFailureReason> {
    let body = encode_form_urlencoded(&[
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", &config.client_id),
        ("client_secret", &config.client_secret),
        ("code_verifier", verifier),
    ]);

    let headers = Headers::new();
    headers
        .set("Content-Type", "application/x-www-form-urlencoded")
        .map_err(|_| OAuthFailureReason::TokenExchangeFailed)?;
    headers
        .set("Accept", "application/json")
        .map_err(|_| OAuthFailureReason::TokenExchangeFailed)?;

    let mut init = RequestInit::new();
    init.with_method(Method::Post);
    init.with_headers(headers);
    init.with_body(Some(wasm_bindgen::JsValue::from_str(&body)));

    let req = Request::new_with_init(&config.token_url, &init)
        .map_err(|_| OAuthFailureReason::TokenExchangeFailed)?;
    let text = fetch_text_with_timeout(req, OUTBOUND_FETCH_TIMEOUT)
        .await
        .map_err(|_| OAuthFailureReason::TokenExchangeFailed)?;
    let parsed: TokenResponse =
        serde_json::from_str(&text).map_err(|_| OAuthFailureReason::TokenExchangeFailed)?;
    let token = parsed
        .access_token
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .ok_or(OAuthFailureReason::TokenExchangeFailed)?;
    Ok(token)
}

async fn fetch_json_bearer(
    url: &str,
    access_token: &str,
    accept: &str,
    include_user_agent: bool,
) -> std::result::Result<String, OAuthFailureReason> {
    let headers = Headers::new();
    headers
        .set("Authorization", &format!("Bearer {access_token}"))
        .map_err(|_| OAuthFailureReason::IdentityFetchFailed)?;
    headers
        .set("Accept", accept)
        .map_err(|_| OAuthFailureReason::IdentityFetchFailed)?;
    if include_user_agent {
        headers
            .set("User-Agent", "pilot-shortlink")
            .map_err(|_| OAuthFailureReason::IdentityFetchFailed)?;
    }

    let mut init = RequestInit::new();
    init.with_method(Method::Get);
    init.with_headers(headers);

    let req =
        Request::new_with_init(url, &init).map_err(|_| OAuthFailureReason::IdentityFetchFailed)?;
    fetch_text_with_timeout(req, OUTBOUND_FETCH_TIMEOUT)
        .await
        .map_err(|_| OAuthFailureReason::IdentityFetchFailed)
}

async fn fetch_identity(
    config: &ProviderConfig,
    access_token: &str,
) -> std::result::Result<OAuthIdentity, OAuthFailureReason> {
    match config.provider {
        Provider::Google => {
            let text = fetch_json_bearer(
                &config.userinfo_url,
                access_token,
                "application/json",
                false,
            )
            .await?;
            parse_google_userinfo(&text).ok_or(OAuthFailureReason::IdentityFetchFailed)
        }
        Provider::Github => {
            let user_text = fetch_json_bearer(
                &config.userinfo_url,
                access_token,
                "application/vnd.github+json",
                true,
            )
            .await?;
            let user: GithubUser = serde_json::from_str(&user_text)
                .map_err(|_| OAuthFailureReason::IdentityFetchFailed)?;
            let id = user.id.ok_or(OAuthFailureReason::IdentityFetchFailed)?;
            let subject = id.to_string();

            let emails_url = config
                .emails_url
                .as_deref()
                .ok_or(OAuthFailureReason::IdentityFetchFailed)?;
            let emails_text = fetch_json_bearer(
                emails_url,
                access_token,
                "application/vnd.github+json",
                true,
            )
            .await?;
            let emails: Vec<GithubEmail> = serde_json::from_str(&emails_text)
                .map_err(|_| OAuthFailureReason::IdentityFetchFailed)?;
            // Identity is (provider, subject): a missing primary+verified email must not stop an already
            // linked account from signing in (I2, AC-O9). resolve_or_create_user refuses to link or create
            // when email_verified is false.
            let email = select_github_primary_verified_email(&emails);

            Ok(OAuthIdentity {
                subject,
                email_verified: email.is_some(),
                email,
            })
        }
    }
}

async fn link_existing_user(
    db: &D1Database,
    provider: Provider,
    subject: &str,
    user: &UserEmailRow,
    now: i64,
) -> std::result::Result<(), OAuthFailureReason> {
    let link_stmt = db
        .prepare(INSERT_OAUTH_ACCOUNT_SQL)
        .bind(&[
            provider.as_str().into(),
            subject.into(),
            wasm_bindgen::JsValue::from_f64(user.id as f64),
            wasm_bindgen::JsValue::from_f64(now as f64),
        ])
        .map_err(|_| OAuthFailureReason::AccountError)?;

    if user.email_verified_at.is_none() {
        // Pre-hijack defense (I10, AC-O12): when linking to an unverified password account,
        // wipe its password, active sessions, and unused email tokens in the same batch.
        let wipe_sessions = db
            .prepare(WIPE_UNVERIFIED_USER_SESSIONS_SQL)
            .bind(&[wasm_bindgen::JsValue::from_f64(user.id as f64)])
            .map_err(|_| OAuthFailureReason::AccountError)?;
        let wipe_tokens = db
            .prepare(WIPE_UNVERIFIED_USER_EMAIL_TOKENS_SQL)
            .bind(&[wasm_bindgen::JsValue::from_f64(user.id as f64)])
            .map_err(|_| OAuthFailureReason::AccountError)?;
        let claim_user = db
            .prepare(CLAIM_UNVERIFIED_USER_SQL)
            .bind(&[
                OAUTH_ONLY_PASSWORD_HASH.into(),
                wasm_bindgen::JsValue::from_f64(user.id as f64),
            ])
            .map_err(|_| OAuthFailureReason::AccountError)?;

        db.batch(vec![link_stmt, wipe_sessions, wipe_tokens, claim_user])
            .await
            .map_err(|_| OAuthFailureReason::AccountError)?;
    } else {
        // Already-verified account: link without modifying password, sessions, or tokens (AC-O12).
        link_stmt
            .run()
            .await
            .map_err(|_| OAuthFailureReason::AccountError)?;
    }

    Ok(())
}

async fn resolve_or_create_user(
    db: &D1Database,
    provider: Provider,
    identity: &OAuthIdentity,
    now: i64,
) -> std::result::Result<i64, OAuthFailureReason> {
    // a) Existing (provider, subject) link -> always return that user (AC-O9, I2).
    let linked = db
        .prepare(SELECT_OAUTH_ACCOUNT_USER_SQL)
        .bind(&[provider.as_str().into(), identity.subject.as_str().into()])
        .map_err(|_| OAuthFailureReason::AccountError)?
        .first::<OAuthUserIdRow>(None)
        .await
        .map_err(|_| OAuthFailureReason::AccountError)?;

    if let Some(row) = linked {
        return Ok(row.user_id);
    }

    // b) Unverified email -> refuse to link or create (AC-O4, I1).
    if !identity.email_verified {
        return Err(OAuthFailureReason::EmailUnverified);
    }
    let email = identity
        .email
        .as_deref()
        .filter(|e| is_valid_email(e))
        .ok_or(OAuthFailureReason::EmailUnverified)?;

    // c) Existing users row with matching verified email -> link (and wipe unverified pre-hijack state, AC-O3/O12).
    let existing = db
        .prepare("SELECT id, email_verified_at FROM users WHERE email = ?1")
        .bind(&[email.into()])
        .map_err(|_| OAuthFailureReason::AccountError)?
        .first::<UserEmailRow>(None)
        .await
        .map_err(|_| OAuthFailureReason::AccountError)?;

    if let Some(u) = existing {
        link_existing_user(db, provider, &identity.subject, &u, now).await?;
    } else {
        // d) Create a new OAuth-only user (password_hash = '!oauth', email_verified_at = now).
        let inserted = db
            .prepare(
                "INSERT INTO users (email, password_hash, email_verified_at) \
                 VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')) \
                 ON CONFLICT(email) DO NOTHING \
                 RETURNING id",
            )
            .bind(&[email.into(), OAUTH_ONLY_PASSWORD_HASH.into()])
            .map_err(|_| OAuthFailureReason::AccountError)?
            .first::<UserIdRow>(None)
            .await
            .map_err(|_| OAuthFailureReason::AccountError)?;

        if let Some(row) = inserted {
            db.prepare(INSERT_OAUTH_ACCOUNT_SQL)
                .bind(&[
                    provider.as_str().into(),
                    identity.subject.as_str().into(),
                    wasm_bindgen::JsValue::from_f64(row.id as f64),
                    wasm_bindgen::JsValue::from_f64(now as f64),
                ])
                .map_err(|_| OAuthFailureReason::AccountError)?
                .run()
                .await
                .map_err(|_| OAuthFailureReason::AccountError)?;
        } else {
            // Concurrent creation race: re-read the existing user and apply the same linking rule (F1).
            let raced = db
                .prepare("SELECT id, email_verified_at FROM users WHERE email = ?1")
                .bind(&[email.into()])
                .map_err(|_| OAuthFailureReason::AccountError)?
                .first::<UserEmailRow>(None)
                .await
                .map_err(|_| OAuthFailureReason::AccountError)?
                .ok_or(OAuthFailureReason::AccountError)?;

            link_existing_user(db, provider, &identity.subject, &raced, now).await?;
        }
    }

    // F3: always re-read the committed (provider, subject) owner after INSERT ... ON CONFLICT DO NOTHING.
    let resolved = db
        .prepare(SELECT_OAUTH_ACCOUNT_USER_SQL)
        .bind(&[provider.as_str().into(), identity.subject.as_str().into()])
        .map_err(|_| OAuthFailureReason::AccountError)?
        .first::<OAuthUserIdRow>(None)
        .await
        .map_err(|_| OAuthFailureReason::AccountError)?
        .ok_or(OAuthFailureReason::AccountError)?;

    Ok(resolved.user_id)
}

fn oauth_start_error_redirect(provider: Provider, reason: OAuthFailureReason) -> Result<Response> {
    emit_oauth_failure_log(provider, reason);
    let headers = Headers::new();
    headers.set("Location", OAUTH_ERROR_LOCATION)?;
    headers.set("Cache-Control", "no-store")?;
    Ok(Response::empty()?.with_status(302).with_headers(headers))
}

fn oauth_callback_error_redirect(
    provider: Provider,
    reason: OAuthFailureReason,
) -> Result<Response> {
    emit_oauth_failure_log(provider, reason);
    let headers = Headers::new();
    headers.set("Location", OAUTH_ERROR_LOCATION)?;
    headers.set("Cache-Control", "no-store")?;
    headers.append("Set-Cookie", clear_oauth_cookie_header())?;
    Ok(Response::empty()?.with_status(302).with_headers(headers))
}

fn oauth_callback_success_redirect(session_cookie: &str) -> Result<Response> {
    let headers = Headers::new();
    headers.set("Location", "/")?;
    headers.set("Cache-Control", "no-store")?;
    headers.append("Set-Cookie", session_cookie)?;
    headers.append("Set-Cookie", clear_oauth_cookie_header())?;
    Ok(Response::empty()?.with_status(302).with_headers(headers))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthStartStep {
    RateLimitDb,
    RateLimitExceeded,
    Config,
    TokenGen,
    StateBindOrBatch,
    AuthorizeUrl,
    ResponseHeaders,
}

/// Maps every failure mode in `GET /api/auth/oauth/{provider}/start` to an `OAuthFailureReason`
/// so no D1 bind/run error or header error can surface as a 500 (F6).
pub fn map_oauth_start_error(step: OAuthStartStep) -> OAuthFailureReason {
    match step {
        OAuthStartStep::RateLimitDb => OAuthFailureReason::AccountError,
        OAuthStartStep::RateLimitExceeded => OAuthFailureReason::RateLimited,
        OAuthStartStep::Config => OAuthFailureReason::MissingCredentials,
        OAuthStartStep::TokenGen => OAuthFailureReason::AccountError,
        OAuthStartStep::StateBindOrBatch => OAuthFailureReason::AccountError,
        OAuthStartStep::AuthorizeUrl => OAuthFailureReason::MissingCredentials,
        OAuthStartStep::ResponseHeaders => OAuthFailureReason::AccountError,
    }
}

/// Builds the fixed `/login?error=oauth` redirect descriptor and structured single-line log
/// for any `OAuthStartStep` failure (F6).
pub fn oauth_start_failure_spec(
    provider: Provider,
    step: OAuthStartStep,
) -> (u16, &'static str, OAuthFailureReason, String) {
    let reason = map_oauth_start_error(step);
    let log_line = format_oauth_failure_log(provider, reason);
    (302, OAUTH_ERROR_LOCATION, reason, log_line)
}

async fn run_oauth_start(
    req: &Request,
    env: &Env,
    db: &D1Database,
    app_origin: &str,
    provider: Provider,
) -> std::result::Result<(String, String), OAuthFailureReason> {
    let now = now_sec();

    // F4: rate-limit start requests per client IP (30 per 10 minutes, same window as callback)
    // before writing anything to oauth_states.
    let ip = client_ip(req).unwrap_or_else(|| "unknown".to_string());
    let rate_key = format!("oauth_start:{ip}");
    let window_start = oauth_callback_window(now);
    let count = consume_rate_limit(db, &rate_key, window_start, now)
        .await
        .map_err(|_| map_oauth_start_error(OAuthStartStep::RateLimitDb))?;
    if count > OAUTH_CALLBACK_MAX_PER_WINDOW {
        return Err(map_oauth_start_error(OAuthStartStep::RateLimitExceeded));
    }

    let config = ProviderConfig::from_env(provider, env, app_origin)
        .ok_or_else(|| map_oauth_start_error(OAuthStartStep::Config))?;

    let state =
        generate_session_token().map_err(|_| map_oauth_start_error(OAuthStartStep::TokenGen))?;
    let verifier =
        generate_session_token().map_err(|_| map_oauth_start_error(OAuthStartStep::TokenGen))?;
    let challenge = pkce_s256_challenge(&verifier);
    let state_hash = hash_session_token(&state);

    let expires_at = oauth_state_expires_at(now);

    let del_expired = db
        .prepare(DELETE_EXPIRED_OAUTH_STATES_SQL)
        .bind(&[wasm_bindgen::JsValue::from_f64(now as f64)])
        .map_err(|_| map_oauth_start_error(OAuthStartStep::StateBindOrBatch))?;
    let ins_state = db
        .prepare(INSERT_OAUTH_STATE_SQL)
        .bind(&[
            state_hash.as_str().into(),
            provider.as_str().into(),
            verifier.as_str().into(),
            wasm_bindgen::JsValue::from_f64(expires_at as f64),
        ])
        .map_err(|_| map_oauth_start_error(OAuthStartStep::StateBindOrBatch))?;

    db.batch(vec![del_expired, ins_state])
        .await
        .map_err(|_| map_oauth_start_error(OAuthStartStep::StateBindOrBatch))?;

    let redirect_uri = build_redirect_uri(app_origin, provider);
    let authorize_url = config
        .build_authorize_url(&redirect_uri, &state, &challenge)
        .map_err(|_| map_oauth_start_error(OAuthStartStep::AuthorizeUrl))?;

    Ok((authorize_url, state))
}

fn build_oauth_start_redirect(authorize_url: &str, state: &str) -> Result<Response> {
    let headers = Headers::new();
    headers.set("Location", authorize_url)?;
    headers.set("Cache-Control", "no-store")?;
    headers.append("Set-Cookie", &oauth_cookie_header(state))?;
    Ok(Response::empty()?.with_status(302).with_headers(headers))
}

/// `GET /api/auth/oauth/{provider}/start`
pub async fn handle_oauth_start(
    req: &Request,
    env: &Env,
    db: &D1Database,
    app_origin: &str,
    provider: Provider,
) -> Result<Response> {
    match run_oauth_start(req, env, db, app_origin, provider).await {
        Ok((authorize_url, state)) => match build_oauth_start_redirect(&authorize_url, &state) {
            Ok(res) => Ok(res),
            Err(_) => oauth_start_error_redirect(
                provider,
                map_oauth_start_error(OAuthStartStep::ResponseHeaders),
            ),
        },
        Err(reason) => oauth_start_error_redirect(provider, reason),
    }
}

async fn run_oauth_callback(
    req: &Request,
    env: &Env,
    db: &D1Database,
    app_origin: &str,
    provider: Provider,
) -> std::result::Result<String, OAuthFailureReason> {
    let now = now_sec();

    // Rate-limit callbacks: 30 per 10 minutes per client IP.
    let ip = client_ip(req).unwrap_or_else(|| "unknown".to_string());
    let rate_key = format!("oauth_cb:{ip}");
    let window_start = oauth_callback_window(now);
    let count = consume_rate_limit(db, &rate_key, window_start, now)
        .await
        .map_err(|_| OAuthFailureReason::AccountError)?;
    if count > OAUTH_CALLBACK_MAX_PER_WINDOW {
        return Err(OAuthFailureReason::RateLimited);
    }

    let url = req.url().map_err(|_| OAuthFailureReason::MissingParams)?;
    let mut code: Option<String> = None;
    let mut state: Option<String> = None;
    let mut error: Option<String> = None;
    for (k, v) in url.query_pairs() {
        match k.as_ref() {
            "code" if code.is_none() => code = Some(v.into_owned()),
            "state" if state.is_none() => state = Some(v.into_owned()),
            "error" if error.is_none() => error = Some(v.into_owned()),
            _ => {}
        }
    }

    if error.is_some() {
        return Err(OAuthFailureReason::ProviderError);
    }
    let code = code
        .filter(|c| !c.is_empty() && c.len() <= 2048)
        .ok_or(OAuthFailureReason::MissingParams)?;
    let state = state
        .filter(|s| !s.is_empty() && s.len() <= 128)
        .ok_or(OAuthFailureReason::MissingParams)?;

    let cookie_header = req
        .headers()
        .get("Cookie")
        .map_err(|_| OAuthFailureReason::StateMismatch)?;
    let cookie_state =
        extract_oauth_cookie(cookie_header.as_deref()).ok_or(OAuthFailureReason::StateMismatch)?;
    if !constant_time_eq(cookie_state.as_bytes(), state.as_bytes()) {
        return Err(OAuthFailureReason::StateMismatch);
    }

    let config = ProviderConfig::from_env(provider, env, app_origin)
        .ok_or(OAuthFailureReason::MissingCredentials)?;

    // Consume the state atomically in one statement so concurrent callbacks succeed at most once (I3).
    let state_hash = hash_session_token(&state);
    let consumed = db
        .prepare(CONSUME_OAUTH_STATE_SQL)
        .bind(&[
            state_hash.as_str().into(),
            provider.as_str().into(),
            wasm_bindgen::JsValue::from_f64(now as f64),
        ])
        .map_err(|_| OAuthFailureReason::AccountError)?
        .first::<ConsumedStateRow>(None)
        .await
        .map_err(|_| OAuthFailureReason::AccountError)?;

    let verifier = consumed
        .map(|r| r.verifier)
        .ok_or(OAuthFailureReason::InvalidState)?;

    let redirect_uri = build_redirect_uri(app_origin, provider);
    let access_token = exchange_code(&config, &code, &redirect_uri, &verifier).await?;
    let identity = fetch_identity(&config, &access_token).await?;
    let user_id = resolve_or_create_user(db, provider, &identity, now).await?;

    let session_token = create_session(db, user_id, now)
        .await
        .map_err(|_| OAuthFailureReason::AccountError)?;
    Ok(session_cookie_header(&session_token))
}

/// `GET /api/auth/oauth/{provider}/callback`
pub async fn handle_oauth_callback(
    req: &Request,
    env: &Env,
    db: &D1Database,
    app_origin: &str,
    provider: Provider,
) -> Result<Response> {
    match run_oauth_callback(req, env, db, app_origin, provider).await {
        Ok(session_cookie) => oauth_callback_success_redirect(&session_cookie),
        Err(reason) => oauth_callback_error_redirect(provider, reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_s256_matches_rfc7636_appendix_b_vector() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let expected = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert_eq!(pkce_s256_challenge(verifier), expected);
    }

    #[test]
    fn fake_mode_is_ignored_when_app_origin_is_not_local() {
        assert!(is_fake_oauth_enabled(Some("fake"), "http://localhost:8787"));
        assert!(is_fake_oauth_enabled(Some("fake"), "http://127.0.0.1:8787"));
        assert!(!is_fake_oauth_enabled(
            Some("fake"),
            "https://pilot-shortlink-a.workers.dev"
        ));
        assert!(!is_fake_oauth_enabled(
            Some("fake"),
            "http://localhost.evil.test"
        ));

        // On non-local origin, even with OAUTH_MODE=fake and OAUTH_FAKE_BASE set, real Google endpoints are used.
        let prod_google = ProviderConfig::resolve(
            Provider::Google,
            "https://pilot-shortlink-a.workers.dev",
            Some("fake"),
            Some("http://127.0.0.1:9411"),
            Some("real-google-id"),
            Some("real-google-secret"),
        )
        .expect("real config resolved");
        assert_eq!(prod_google.authorize_url, GOOGLE_AUTHORIZE_URL);
        assert_eq!(prod_google.token_url, GOOGLE_TOKEN_URL);
        assert_eq!(prod_google.userinfo_url, GOOGLE_USERINFO_URL);
        assert_eq!(prod_google.client_id, "real-google-id");

        // Without real credentials on non-local origin, fake mode cannot substitute fake-client.
        assert_eq!(
            ProviderConfig::resolve(
                Provider::Google,
                "https://pilot-shortlink-a.workers.dev",
                Some("fake"),
                Some("http://127.0.0.1:9411"),
                None,
                None,
            ),
            None
        );

        // On local origin with OAUTH_MODE=fake, fake endpoints and fake credentials are used.
        let local_github = ProviderConfig::resolve(
            Provider::Github,
            "http://localhost:8787",
            Some("fake"),
            Some("http://127.0.0.1:9411/"),
            None,
            None,
        )
        .expect("fake github config");
        assert_eq!(
            local_github.authorize_url,
            "http://127.0.0.1:9411/github/login/oauth/authorize"
        );
        assert_eq!(
            local_github.token_url,
            "http://127.0.0.1:9411/github/login/oauth/access_token"
        );
        assert_eq!(
            local_github.userinfo_url,
            "http://127.0.0.1:9411/github/user"
        );
        assert_eq!(
            local_github.emails_url.as_deref(),
            Some("http://127.0.0.1:9411/github/user/emails")
        );
        assert_eq!(local_github.client_id, FAKE_CLIENT_ID);

        // Debug redacts client_secret.
        let dbg = format!("{prod_google:?}");
        assert!(!dbg.contains("real-google-secret"));
        assert!(dbg.contains("<redacted>"));
    }

    #[test]
    fn github_email_selection_requires_both_primary_and_verified() {
        let emails = vec![
            GithubEmail {
                email: "secondary@example.test".to_string(),
                primary: false,
                verified: true,
            },
            GithubEmail {
                email: "unverified-primary@example.test".to_string(),
                primary: true,
                verified: false,
            },
        ];
        assert_eq!(select_github_primary_verified_email(&emails), None);
        assert_eq!(select_github_primary_verified_email(&[]), None);

        let with_primary = vec![
            GithubEmail {
                email: "secondary@example.test".to_string(),
                primary: false,
                verified: true,
            },
            GithubEmail {
                email: "  Primary@Example.TEST ".to_string(),
                primary: true,
                verified: true,
            },
        ];
        assert_eq!(
            select_github_primary_verified_email(&with_primary).as_deref(),
            Some("primary@example.test")
        );
    }

    #[test]
    fn google_userinfo_parsing_requires_boolean_true_email_verified() {
        let verified = parse_google_userinfo(
            r#"{"sub":"12345","email":"Alice@Example.TEST","email_verified":true}"#,
        )
        .expect("verified google user");
        assert_eq!(verified.subject, "12345");
        assert_eq!(verified.email.as_deref(), Some("alice@example.test"));
        assert!(verified.email_verified);

        let unverified = parse_google_userinfo(
            r#"{"sub":"12345","email":"alice@example.test","email_verified":false}"#,
        )
        .expect("unverified google user");
        assert!(!unverified.email_verified);

        let missing_flag = parse_google_userinfo(r#"{"sub":"12345","email":"alice@example.test"}"#)
            .expect("missing email_verified");
        assert!(!missing_flag.email_verified);

        assert_eq!(parse_google_userinfo(r#"{"email":"a@b.c"}"#), None);
    }

    #[test]
    fn authorize_url_and_cookie_headers_follow_spec() {
        let cfg = ProviderConfig::resolve(
            Provider::Google,
            "https://pilot-shortlink-a.workers.dev",
            None,
            None,
            Some("cid-123"),
            Some("csec-456"),
        )
        .expect("google config");
        let redirect_uri =
            build_redirect_uri("https://pilot-shortlink-a.workers.dev/", Provider::Google);
        assert_eq!(
            redirect_uri,
            "https://pilot-shortlink-a.workers.dev/api/auth/oauth/google/callback"
        );
        let challenge = pkce_s256_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
        let auth_url = cfg
            .build_authorize_url(&redirect_uri, "state_abc123", &challenge)
            .expect("authorize url");
        assert!(auth_url.starts_with(GOOGLE_AUTHORIZE_URL));
        let parsed = Url::parse(&auth_url).expect("parse authorize url");
        let params: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(
            params.get("response_type").map(String::as_str),
            Some("code")
        );
        assert_eq!(params.get("client_id").map(String::as_str), Some("cid-123"));
        assert_eq!(
            params.get("redirect_uri").map(String::as_str),
            Some(redirect_uri.as_str())
        );
        assert_eq!(params.get("scope").map(String::as_str), Some(GOOGLE_SCOPE));
        assert_eq!(
            params.get("state").map(String::as_str),
            Some("state_abc123")
        );
        assert_eq!(
            params.get("code_challenge").map(String::as_str),
            Some(challenge.as_str())
        );
        assert_eq!(
            params.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );

        let cookie = oauth_cookie_header("state_abc123");
        assert_eq!(
            cookie,
            "__Host-oauth=state_abc123; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=600"
        );
        assert_eq!(
            clear_oauth_cookie_header(),
            "__Host-oauth=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0"
        );
        assert_eq!(
            extract_oauth_cookie(Some("other=1; __Host-oauth=state_abc123")),
            Some("state_abc123")
        );
        assert_eq!(extract_oauth_cookie(Some("__Host-oauth=")), None);
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    #[test]
    fn oauth_failure_logs_contain_no_secrets_codes_states_or_emails() {
        let secret_code = "secret_oauth_code_12345";
        let secret_state = "secret_oauth_state_67890";
        let secret_token = "secret_access_token_abcde";
        let secret_email = "private.user@example.test";

        for provider in [Provider::Google, Provider::Github] {
            for reason in [
                OAuthFailureReason::MissingCredentials,
                OAuthFailureReason::RateLimited,
                OAuthFailureReason::ProviderError,
                OAuthFailureReason::MissingParams,
                OAuthFailureReason::StateMismatch,
                OAuthFailureReason::InvalidState,
                OAuthFailureReason::TokenExchangeFailed,
                OAuthFailureReason::IdentityFetchFailed,
                OAuthFailureReason::EmailUnverified,
                OAuthFailureReason::AccountError,
            ] {
                let line = format_oauth_failure_log(provider, reason);
                assert!(!line.contains(secret_code));
                assert!(!line.contains(secret_state));
                assert!(!line.contains(secret_token));
                assert!(!line.contains(secret_email));
                let parsed: serde_json::Value =
                    serde_json::from_str(&line).expect("valid JSON log");
                assert_eq!(parsed["event"], "oauth_failed");
                assert_eq!(parsed["provider"], provider.as_str());
                assert_eq!(parsed["reason"], reason.as_str());
            }
        }
    }

    #[test]
    fn oauth_state_expiry_is_ten_minutes() {
        assert_eq!(OAUTH_STATE_TTL_SECS, 600);
        assert_eq!(oauth_state_expires_at(1_700_000_000), 1_700_000_600);
    }

    #[test]
    fn oauth_start_errors_map_to_redirect_and_single_line_reason_log_without_500() {
        let cases = [
            (
                OAuthStartStep::RateLimitDb,
                OAuthFailureReason::AccountError,
            ),
            (
                OAuthStartStep::RateLimitExceeded,
                OAuthFailureReason::RateLimited,
            ),
            (
                OAuthStartStep::Config,
                OAuthFailureReason::MissingCredentials,
            ),
            (OAuthStartStep::TokenGen, OAuthFailureReason::AccountError),
            (
                OAuthStartStep::StateBindOrBatch,
                OAuthFailureReason::AccountError,
            ),
            (
                OAuthStartStep::AuthorizeUrl,
                OAuthFailureReason::MissingCredentials,
            ),
            (
                OAuthStartStep::ResponseHeaders,
                OAuthFailureReason::AccountError,
            ),
        ];

        for provider in [Provider::Google, Provider::Github] {
            for (step, expected_reason) in cases {
                let (status, location, reason, log_line) = oauth_start_failure_spec(provider, step);
                assert_eq!(status, 302);
                assert_eq!(location, OAUTH_ERROR_LOCATION);
                assert_eq!(reason, expected_reason);
                assert!(!log_line.contains('\n'));
                let parsed: serde_json::Value =
                    serde_json::from_str(&log_line).expect("valid JSON log");
                assert_eq!(parsed["event"], "oauth_failed");
                assert_eq!(parsed["provider"], provider.as_str());
                assert_eq!(parsed["reason"], expected_reason.as_str());
            }
        }
    }
}
