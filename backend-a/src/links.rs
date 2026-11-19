use serde::{Deserialize, Serialize};
use worker::*;

use crate::http::{
    json_response, json_response_with_headers, now_sec, problem_field_response, problem_response,
    rate_limited_response, read_json_body,
};
use crate::session::authenticate_session;

/// Fixed 1-minute window (in seconds) and link creation threshold per user (`rate_limits`).
pub const CREATE_WINDOW_SECS: i64 = 60;
pub const CREATE_MAX_LINKS: i64 = 30;

/// Retention horizon for cleaning up expired rate_limits buckets (covers 1-hour email windows).
const RATE_LIMIT_CLEANUP_RETENTION_SECS: i64 = 60 * 60;

/// Maximum allowed length of a target URL (`api/openapi.yaml` `#/components/schemas/TargetUrl`).
pub const MAX_URL_LEN: usize = 2048;

/// Alias length bounds (`api/openapi.yaml` `#/components/schemas/CreateLink`).
pub const ALIAS_MIN_LEN: usize = 3;
pub const ALIAS_MAX_LEN: usize = 32;

/// Random code length and collision retry limit (`docs/architecture.md` section 5).
pub const RANDOM_CODE_LEN: usize = 7;
pub const MAX_CODE_RETRIES: usize = 5;

/// Base36 alphabet for random 7-char codes (`[a-z0-9]`).
const BASE36_ALPHABET: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

/// Largest multiple of 36 in `0..=255` (`36 * 7 = 252`) for rejection sampling without modulo bias.
const UNBIASED_THRESHOLD: u8 = 252;

/// Reserved aliases that cannot be registered (`docs/requirements.md` section 4).
pub const RESERVED_ALIASES: &[&str] = &[
    "api",
    "app",
    "admin",
    "login",
    "logout",
    "register",
    "static",
    "assets",
    "health",
    "openapi",
    "verify-email",
    "forgot-password",
    "reset-password",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlValidationError {
    InvalidUrl,
    SelfLink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AliasValidationError {
    InvalidAlias,
    ReservedAlias,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateLinkRequest {
    pub url: String,
    pub alias: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchLinkRequest {
    pub url: String,
}

#[derive(Debug, Serialize)]
pub struct LinkSummary {
    pub code: String,
    pub url: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "clicksTotal")]
    pub clicks_total: i64,
    #[serde(rename = "last30Days")]
    pub last_30_days: Vec<i64>,
}

#[derive(Debug, Serialize)]
pub struct LinksListResponse {
    pub items: Vec<LinkSummary>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct DailyClicks {
    pub date: String,
    pub clicks: i64,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ReferrerClicks {
    pub host: Option<String>,
    pub clicks: i64,
}

#[derive(Debug, Serialize)]
pub struct StatsResponse {
    pub code: String,
    #[serde(rename = "clicksTotal")]
    pub clicks_total: i64,
    pub daily: Vec<DailyClicks>,
    pub referrers: Vec<ReferrerClicks>,
}

#[derive(Debug, Deserialize)]
struct CountRow {
    count: i64,
}

#[derive(Debug, Deserialize)]
struct InsertedLinkRow {
    id: i64,
    code: String,
    url: String,
    created_at: String,
}

#[derive(Debug, Deserialize)]
struct LinkListRow {
    id: i64,
    code: String,
    url: String,
    created_at: String,
    clicks_total: i64,
}

#[derive(Debug, Deserialize)]
struct LinkIdCodeRow {
    id: i64,
    code: String,
}

#[derive(Debug, Deserialize)]
struct LinkIdRow {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct RedirectTargetRow {
    id: i64,
    url: String,
}

#[derive(Debug, Deserialize)]
struct LinkDailyClicksRow {
    link_id: i64,
    day: String,
    clicks: i64,
}

#[derive(Debug, Deserialize)]
struct DailyClicksRow {
    day: String,
    clicks: i64,
}

#[derive(Debug, Deserialize)]
struct ReferrerRow {
    referrer_host: String,
    clicks: i64,
}

#[derive(Debug, Deserialize)]
struct TotalClicksRow {
    clicks_total: i64,
}

/// Trims at most one trailing `.` from a host string so FQDN notation
/// (e.g. `localhost.` vs `localhost`) is compared on an equal footing.
pub fn trim_trailing_dot(host: &str) -> &str {
    host.strip_suffix('.').unwrap_or(host)
}

/// Validates a target URL against `docs/requirements.md` section 4 and `api/openapi.yaml`:
/// - Non-empty, at most 2048 chars, no whitespace or control characters.
/// - Parses as an absolute `http` or `https` URL with a non-empty host.
/// - Host does not match `app_origin`'s host after trimming one trailing `.` (case-insensitive).
/// - Returns the parsed `Url` serialization (`as_str()`, percent-encoded ASCII) so `Location` is always ASCII.
pub fn validate_target_url(
    raw_url: &str,
    app_origin: &str,
) -> std::result::Result<String, UrlValidationError> {
    if raw_url.is_empty()
        || raw_url.len() > MAX_URL_LEN
        || raw_url.chars().count() > MAX_URL_LEN
        || raw_url.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(UrlValidationError::InvalidUrl);
    }

    let Ok(parsed) = Url::parse(raw_url) else {
        return Err(UrlValidationError::InvalidUrl);
    };

    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(UrlValidationError::InvalidUrl);
    }

    let Some(target_host) = parsed
        .host_str()
        .map(trim_trailing_dot)
        .filter(|h| !h.is_empty())
    else {
        return Err(UrlValidationError::InvalidUrl);
    };

    if let Ok(origin_url) = Url::parse(app_origin) {
        if let Some(origin_host) = origin_url.host_str().map(trim_trailing_dot) {
            if target_host.eq_ignore_ascii_case(origin_host) {
                return Err(UrlValidationError::SelfLink);
            }
        }
    }

    Ok(parsed.as_str().to_string())
}

/// Checks whether a lowercase alias is one of the reserved words (`docs/requirements.md` section 4).
pub fn is_reserved_alias(lower_alias: &str) -> bool {
    RESERVED_ALIASES.contains(&lower_alias)
}

/// Validates and normalizes an optional custom alias:
/// - 3..=32 ASCII `[A-Za-z0-9-]` chars, no leading or trailing `-`.
/// - Normalized to lowercase.
/// - Rejects reserved words (`RESERVED_ALIASES`) with `AliasValidationError::ReservedAlias`.
pub fn validate_alias(raw_alias: &str) -> std::result::Result<String, AliasValidationError> {
    let len = raw_alias.len();
    if !(ALIAS_MIN_LEN..=ALIAS_MAX_LEN).contains(&len)
        || raw_alias.starts_with('-')
        || raw_alias.ends_with('-')
        || !raw_alias
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(AliasValidationError::InvalidAlias);
    }

    let lower = raw_alias.to_ascii_lowercase();
    if is_reserved_alias(&lower) {
        return Err(AliasValidationError::ReservedAlias);
    }

    Ok(lower)
}

/// Generates a random 7-character `[a-z0-9]` code using `getrandom` and rejection sampling
/// (`b < 252`) so there is zero modulo bias. Never returns a reserved word.
pub fn generate_random_code() -> std::result::Result<String, String> {
    let mut out = String::with_capacity(RANDOM_CODE_LEN);
    let mut buf = [0u8; 16];

    loop {
        getrandom::fill(&mut buf).map_err(|e| format!("CSPRNG failure: {e}"))?;
        for &b in &buf {
            if b < UNBIASED_THRESHOLD {
                let ch = BASE36_ALPHABET[(b % 36) as usize] as char;
                out.push(ch);
                if out.len() == RANDOM_CODE_LEN {
                    if !is_reserved_alias(&out) {
                        return Ok(out);
                    }
                    out.clear();
                }
            }
        }
    }
}

/// Extracts the lower-case host from an HTTP `Referer` header, or returns `""` when
/// absent, empty, non-http(s), or unparseable (`docs/requirements.md` section 4).
pub fn extract_referrer_host(referer: Option<&str>) -> String {
    let Some(raw) = referer.map(str::trim).filter(|s| !s.is_empty()) else {
        return String::new();
    };
    let Ok(parsed) = Url::parse(raw) else {
        return String::new();
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return String::new();
    }
    parsed
        .host_str()
        .filter(|h| !h.is_empty())
        .map(|h| h.to_ascii_lowercase())
        .unwrap_or_default()
}

/// Converts a count of days since the Unix epoch (`1970-01-01` UTC) into a `YYYY-MM-DD` string
/// using Howard Hinnant's civil-from-days algorithm.
pub fn ymd_from_epoch_days(days_since_epoch: i64) -> String {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02}")
}

/// Returns today's UTC date string (`YYYY-MM-DD`) for the given Unix timestamp in seconds.
pub fn today_utc_ymd(now_sec: i64) -> String {
    ymd_from_epoch_days(now_sec.div_euclid(86_400))
}

/// Returns the 30 consecutive UTC date strings (`YYYY-MM-DD`), oldest first, ending at today UTC.
pub fn last_30_days_utc(now_sec: i64) -> Vec<String> {
    let today_days = now_sec.div_euclid(86_400);
    (0..30)
        .rev()
        .map(|offset| ymd_from_epoch_days(today_days - offset))
        .collect()
}

/// Computes `(window_start, retry_after_secs)` for the fixed 1-minute link-creation rate limit.
pub fn create_window(now_sec: i64) -> (i64, i64) {
    let window_start = (now_sec / CREATE_WINDOW_SECS) * CREATE_WINDOW_SECS;
    let retry_after = ((window_start + CREATE_WINDOW_SECS) - now_sec).max(1);
    (window_start, retry_after)
}

async fn consume_create_rate_limit(
    db: &D1Database,
    user_id: i64,
    now_sec: i64,
) -> Result<Option<i64>> {
    let (window_start, retry_after) = create_window(now_sec);
    let key = format!("create:{user_id}");

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

    let cleanup_cutoff = now_sec - RATE_LIMIT_CLEANUP_RETENTION_SECS;
    db.prepare("DELETE FROM rate_limits WHERE window_start < ?1")
        .bind(&[wasm_bindgen::JsValue::from_f64(cleanup_cutoff as f64)])?
        .run()
        .await?;

    let count = row.map_or(1, |r| r.count);
    if count > CREATE_MAX_LINKS {
        Ok(Some(retry_after))
    } else {
        Ok(None)
    }
}

async fn try_insert_link(
    db: &D1Database,
    user_id: i64,
    code: &str,
    url: &str,
) -> Result<Option<InsertedLinkRow>> {
    db.prepare(
        "INSERT INTO links (user_id, code, url) VALUES (?1, ?2, ?3) \
         ON CONFLICT(code) DO NOTHING \
         RETURNING id, code, url, created_at",
    )
    .bind(&[
        wasm_bindgen::JsValue::from_f64(user_id as f64),
        code.into(),
        url.into(),
    ])?
    .first::<InsertedLinkRow>(None)
    .await
}

fn url_error_response(err: UrlValidationError) -> Result<Response> {
    let code = match err {
        UrlValidationError::InvalidUrl => "invalid_url",
        UrlValidationError::SelfLink => "self_link",
    };
    problem_field_response(422, "Unprocessable Entity", code, "url")
}

fn alias_error_response(err: AliasValidationError) -> Result<Response> {
    let code = match err {
        AliasValidationError::InvalidAlias => "invalid_alias",
        AliasValidationError::ReservedAlias => "reserved_alias",
    };
    problem_field_response(422, "Unprocessable Entity", code, "alias")
}

/// Shared helper for mutating link endpoints (`POST /api/links`, `PATCH /api/links/{code}`,
/// `DELETE /api/links/{code}`): returns `Some(403 email_unverified)` when the authenticated user
/// has not verified their email address (AC-V4).
pub fn require_verified_user(user: &crate::session::SessionUser) -> Result<Option<Response>> {
    if user.is_email_verified() {
        Ok(None)
    } else {
        Ok(Some(problem_response(
            403,
            "Forbidden",
            "email_unverified",
        )?))
    }
}

/// `POST /api/links`
pub async fn handle_create_link(
    req: &mut Request,
    db: &D1Database,
    app_origin: &str,
) -> Result<Response> {
    let raw_body = match read_json_body(req).await? {
        Ok(bytes) => bytes,
        Err(res) => return Ok(res),
    };

    let now = now_sec();
    let Some(user) = authenticate_session(req, db, now).await? else {
        return problem_response(401, "Unauthorized", "unauthorized");
    };
    if let Some(unverified) = require_verified_user(&user)? {
        return Ok(unverified);
    }

    if let Some(retry_after) = consume_create_rate_limit(db, user.id, now).await? {
        return rate_limited_response(retry_after);
    }

    let Ok(payload) = serde_json::from_slice::<CreateLinkRequest>(&raw_body) else {
        return problem_response(422, "Unprocessable Entity", "invalid_body");
    };

    let normalized_url = match validate_target_url(&payload.url, app_origin) {
        Ok(u) => u,
        Err(err) => return url_error_response(err),
    };

    let inserted = if let Some(raw_alias) = payload.alias.as_deref() {
        let code = match validate_alias(raw_alias) {
            Ok(c) => c,
            Err(err) => return alias_error_response(err),
        };
        let Some(row) = try_insert_link(db, user.id, &code, &normalized_url).await? else {
            return problem_field_response(409, "Conflict", "alias_taken", "alias");
        };
        row
    } else {
        let mut created = None;
        for _ in 0..MAX_CODE_RETRIES {
            let code = generate_random_code().map_err(Error::RustError)?;
            if let Some(row) = try_insert_link(db, user.id, &code, &normalized_url).await? {
                created = Some(row);
                break;
            }
        }
        let Some(row) = created else {
            return Err(Error::RustError(
                "failed to generate a unique short code after 5 attempts".to_string(),
            ));
        };
        row
    };

    let _ = inserted.id;
    let summary = LinkSummary {
        code: inserted.code,
        url: inserted.url,
        created_at: inserted.created_at,
        clicks_total: 0,
        last_30_days: vec![0; 30],
    };
    let body = serde_json::to_string(&summary)?;
    json_response(201, "application/json", body)
}

/// `GET /api/links`
pub async fn handle_list_links(req: &Request, db: &D1Database) -> Result<Response> {
    let now = now_sec();
    let Some(user) = authenticate_session(req, db, now).await? else {
        return problem_response(401, "Unauthorized", "unauthorized");
    };

    let link_rows: Vec<LinkListRow> = db
        .prepare(
            "SELECT l.id AS id, l.code AS code, l.url AS url, l.created_at AS created_at, \
             COALESCE((SELECT SUM(c.clicks) FROM click_daily c WHERE c.link_id = l.id), 0) AS clicks_total \
             FROM links l \
             WHERE l.user_id = ?1 \
             ORDER BY l.created_at DESC, l.id DESC",
        )
        .bind(&[wasm_bindgen::JsValue::from_f64(user.id as f64)])?
        .all()
        .await?
        .results::<LinkListRow>()?;

    if link_rows.is_empty() {
        let body = serde_json::to_string(&LinksListResponse { items: Vec::new() })?;
        return json_response(200, "application/json", body);
    }

    let days = last_30_days_utc(now);
    let start_day = &days[0];
    let end_day = &days[29];

    let daily_rows: Vec<LinkDailyClicksRow> = db
        .prepare(
            "SELECT c.link_id AS link_id, c.day AS day, COALESCE(SUM(c.clicks), 0) AS clicks \
             FROM click_daily c \
             JOIN links l ON l.id = c.link_id \
             WHERE l.user_id = ?1 AND c.day >= ?2 AND c.day <= ?3 \
             GROUP BY c.link_id, c.day",
        )
        .bind(&[
            wasm_bindgen::JsValue::from_f64(user.id as f64),
            start_day.as_str().into(),
            end_day.as_str().into(),
        ])?
        .all()
        .await?
        .results::<LinkDailyClicksRow>()?;

    let mut items: Vec<LinkSummary> = link_rows
        .iter()
        .map(|r| LinkSummary {
            code: r.code.clone(),
            url: r.url.clone(),
            created_at: r.created_at.clone(),
            clicks_total: r.clicks_total,
            last_30_days: vec![0; 30],
        })
        .collect();

    for d in daily_rows {
        if let Ok(day_idx) = days.binary_search(&d.day) {
            if let Some(pos) = link_rows.iter().position(|l| l.id == d.link_id) {
                items[pos].last_30_days[day_idx] = d.clicks;
            }
        }
    }

    let body = serde_json::to_string(&LinksListResponse { items })?;
    json_response(200, "application/json", body)
}

async fn load_link_click_summary(
    db: &D1Database,
    link_id: i64,
    now_sec: i64,
) -> Result<(i64, Vec<i64>)> {
    let total_row = db
        .prepare(
            "SELECT COALESCE(SUM(clicks), 0) AS clicks_total FROM click_daily WHERE link_id = ?1",
        )
        .bind(&[wasm_bindgen::JsValue::from_f64(link_id as f64)])?
        .first::<TotalClicksRow>(None)
        .await?;
    let clicks_total = total_row.map_or(0, |r| r.clicks_total);

    let days = last_30_days_utc(now_sec);
    let start_day = &days[0];
    let end_day = &days[29];

    let daily_rows: Vec<DailyClicksRow> = db
        .prepare(
            "SELECT day, COALESCE(SUM(clicks), 0) AS clicks \
             FROM click_daily \
             WHERE link_id = ?1 AND day >= ?2 AND day <= ?3 \
             GROUP BY day",
        )
        .bind(&[
            wasm_bindgen::JsValue::from_f64(link_id as f64),
            start_day.as_str().into(),
            end_day.as_str().into(),
        ])?
        .all()
        .await?
        .results::<DailyClicksRow>()?;

    let mut last_30_days = vec![0; 30];
    for d in daily_rows {
        if let Ok(day_idx) = days.binary_search(&d.day) {
            last_30_days[day_idx] = d.clicks;
        }
    }

    Ok((clicks_total, last_30_days))
}

/// `PATCH /api/links/{code}`
pub async fn handle_patch_link(
    req: &mut Request,
    db: &D1Database,
    app_origin: &str,
    raw_code: &str,
) -> Result<Response> {
    let raw_body = match read_json_body(req).await? {
        Ok(bytes) => bytes,
        Err(res) => return Ok(res),
    };

    let now = now_sec();
    let Some(user) = authenticate_session(req, db, now).await? else {
        return problem_response(401, "Unauthorized", "unauthorized");
    };
    if let Some(unverified) = require_verified_user(&user)? {
        return Ok(unverified);
    }

    let Ok(payload) = serde_json::from_slice::<PatchLinkRequest>(&raw_body) else {
        return problem_response(422, "Unprocessable Entity", "invalid_body");
    };

    let normalized_url = match validate_target_url(&payload.url, app_origin) {
        Ok(u) => u,
        Err(err) => return url_error_response(err),
    };

    let lower_code = raw_code.to_ascii_lowercase();
    let updated = db
        .prepare(
            "UPDATE links SET url = ?1 \
             WHERE code = ?2 AND user_id = ?3 \
             RETURNING id, code, url, created_at",
        )
        .bind(&[
            normalized_url.as_str().into(),
            lower_code.as_str().into(),
            wasm_bindgen::JsValue::from_f64(user.id as f64),
        ])?
        .first::<InsertedLinkRow>(None)
        .await?;

    let Some(link) = updated else {
        return problem_response(404, "Not Found", "not_found");
    };

    let (clicks_total, last_30_days) = load_link_click_summary(db, link.id, now).await?;
    let summary = LinkSummary {
        code: link.code,
        url: link.url,
        created_at: link.created_at,
        clicks_total,
        last_30_days,
    };
    let body = serde_json::to_string(&summary)?;
    json_response(200, "application/json", body)
}

/// `DELETE /api/links/{code}`
pub async fn handle_delete_link(
    req: &Request,
    db: &D1Database,
    raw_code: &str,
) -> Result<Response> {
    let now = now_sec();
    let Some(user) = authenticate_session(req, db, now).await? else {
        return problem_response(401, "Unauthorized", "unauthorized");
    };
    if let Some(unverified) = require_verified_user(&user)? {
        return Ok(unverified);
    }

    let lower_code = raw_code.to_ascii_lowercase();
    let deleted = db
        .prepare("DELETE FROM links WHERE code = ?1 AND user_id = ?2 RETURNING id")
        .bind(&[
            lower_code.as_str().into(),
            wasm_bindgen::JsValue::from_f64(user.id as f64),
        ])?
        .first::<LinkIdRow>(None)
        .await?;

    let Some(link) = deleted else {
        return problem_response(404, "Not Found", "not_found");
    };

    // Ensure click_daily rows are removed even if SQLite foreign_keys pragma is ever off.
    db.prepare("DELETE FROM click_daily WHERE link_id = ?1")
        .bind(&[wasm_bindgen::JsValue::from_f64(link.id as f64)])?
        .run()
        .await?;

    Ok(Response::empty()?.with_status(204))
}

/// `GET /api/links/{code}/stats`
pub async fn handle_link_stats(req: &Request, db: &D1Database, raw_code: &str) -> Result<Response> {
    let now = now_sec();
    let Some(user) = authenticate_session(req, db, now).await? else {
        return problem_response(401, "Unauthorized", "unauthorized");
    };

    let lower_code = raw_code.to_ascii_lowercase();
    let link = db
        .prepare("SELECT id, code FROM links WHERE code = ?1 AND user_id = ?2")
        .bind(&[
            lower_code.as_str().into(),
            wasm_bindgen::JsValue::from_f64(user.id as f64),
        ])?
        .first::<LinkIdCodeRow>(None)
        .await?;

    let Some(link) = link else {
        return problem_response(404, "Not Found", "not_found");
    };

    let (clicks_total, last_30_days) = load_link_click_summary(db, link.id, now).await?;
    let days = last_30_days_utc(now);
    let daily: Vec<DailyClicks> = days
        .into_iter()
        .zip(last_30_days)
        .map(|(date, clicks)| DailyClicks { date, clicks })
        .collect();

    let referrer_rows: Vec<ReferrerRow> = db
        .prepare(
            "SELECT referrer_host, COALESCE(SUM(clicks), 0) AS clicks \
             FROM click_daily \
             WHERE link_id = ?1 \
             GROUP BY referrer_host \
             HAVING SUM(clicks) > 0 \
             ORDER BY clicks DESC, referrer_host ASC \
             LIMIT 10",
        )
        .bind(&[wasm_bindgen::JsValue::from_f64(link.id as f64)])?
        .all()
        .await?
        .results::<ReferrerRow>()?;

    let referrers: Vec<ReferrerClicks> = referrer_rows
        .into_iter()
        .map(|r| ReferrerClicks {
            host: if r.referrer_host.is_empty() {
                None
            } else {
                Some(r.referrer_host)
            },
            clicks: r.clicks,
        })
        .collect();

    let stats = StatsResponse {
        code: link.code,
        clicks_total,
        daily,
        referrers,
    };
    let body = serde_json::to_string(&stats)?;
    json_response(200, "application/json", body)
}

/// Builds the 404 `text/html` response in Thai ("ไม่พบลิงก์นี้") with `Cache-Control: no-store`.
pub fn html_not_found_response() -> Result<Response> {
    let body = "<!doctype html><html lang=\"th\"><head><meta charset=\"utf-8\"><title>ไม่พบลิงก์นี้</title></head><body><h1>ไม่พบลิงก์นี้</h1></body></html>".to_string();
    json_response_with_headers(
        404,
        "text/html; charset=utf-8",
        body,
        &[("Cache-Control", "no-store")],
    )
}

async fn record_click(db: &D1Database, link_id: i64, day: &str, referrer_host: &str) -> Result<()> {
    db.prepare(
        "INSERT INTO click_daily (link_id, day, referrer_host, clicks) \
         VALUES (?1, ?2, ?3, 1) \
         ON CONFLICT(link_id, day, referrer_host) DO UPDATE SET clicks = clicks + 1",
    )
    .bind(&[
        wasm_bindgen::JsValue::from_f64(link_id as f64),
        day.into(),
        referrer_host.into(),
    ])?
    .run()
    .await?;
    Ok(())
}

/// `GET /{code}`
pub async fn handle_redirect(req: &Request, db: &D1Database, raw_code: &str) -> Result<Response> {
    let lower_code = raw_code.to_ascii_lowercase();
    let link = db
        .prepare("SELECT id, url FROM links WHERE code = ?1")
        .bind(&[lower_code.as_str().into()])?
        .first::<RedirectTargetRow>(None)
        .await?;

    let Some(link) = link else {
        return html_not_found_response();
    };

    let referer = req.headers().get("Referer").ok().flatten();
    let referrer_host = extract_referrer_host(referer.as_deref());
    let today = today_utc_ymd(now_sec());

    if let Err(err) = record_click(db, link.id, &today, &referrer_host).await {
        let err_line = crate::format_error_log(&err.to_string());
        console_error!("{err_line}");
    }

    let headers = Headers::new();
    headers.set("Location", &link.url)?;
    headers.set("Cache-Control", "no-store")?;
    Ok(Response::empty()?.with_status(302).with_headers(headers))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_url_validation_rules() {
        let origin = "http://localhost:8787";
        assert_eq!(
            validate_target_url("https://example.com/portfolio?ref=cv", origin),
            Ok("https://example.com/portfolio?ref=cv".to_string())
        );
        assert_eq!(
            validate_target_url("http://example.org:8080/a/b", origin),
            Ok("http://example.org:8080/a/b".to_string())
        );

        // Non-ASCII target URL is percent-encoded into ASCII
        let encoded = validate_target_url("https://example.com/ไทย?q=ลิงก์", origin)
            .expect("Thai URL must be valid");
        assert!(
            encoded.is_ascii(),
            "serialized URL must be ASCII: {encoded}"
        );
        assert!(encoded.starts_with("https://example.com/%E0%B9%84"));

        // Schemes other than http/https and malformed URLs -> InvalidUrl
        for bad in [
            "",
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "ftp://example.com/a",
            "not a url",
            "https://example.com/has space",
            "https://example.com/has\r\ncrlf",
        ] {
            assert_eq!(
                validate_target_url(bad, origin),
                Err(UrlValidationError::InvalidUrl),
                "expected InvalidUrl for {bad:?}"
            );
        }

        // Max length 2048
        let max_ok = format!("https://example.com/{}", "a".repeat(2048 - 20));
        assert_eq!(max_ok.len(), 2048);
        assert_eq!(validate_target_url(&max_ok, origin), Ok(max_ok));

        let too_long = format!("https://example.com/{}", "a".repeat(2048));
        assert_eq!(
            validate_target_url(&too_long, origin),
            Err(UrlValidationError::InvalidUrl)
        );

        // Self-link detection (case-insensitive host match against APP_ORIGIN)
        assert_eq!(
            validate_target_url("http://localhost:8787/abc1234", origin),
            Err(UrlValidationError::SelfLink)
        );
        assert_eq!(
            validate_target_url("https://LOCALHOST/abc1234", origin),
            Err(UrlValidationError::SelfLink)
        );
    }

    #[test]
    fn host_trailing_dot_trim_detects_self_links() {
        assert_eq!(trim_trailing_dot("localhost."), "localhost");
        assert_eq!(trim_trailing_dot("localhost"), "localhost");
        assert_eq!(trim_trailing_dot("example.com.."), "example.com.");

        let origin = "http://localhost:8787";
        assert_eq!(
            validate_target_url("http://localhost.:8787/abc1234", origin),
            Err(UrlValidationError::SelfLink)
        );
        assert_eq!(
            validate_target_url("https://LOCALHOST./abc1234", origin),
            Err(UrlValidationError::SelfLink)
        );
        assert_eq!(
            validate_target_url("http://localhost/abc1234", "http://localhost.:8787"),
            Err(UrlValidationError::SelfLink)
        );
    }

    #[test]
    fn alias_validation_and_reserved_words() {
        assert_eq!(validate_alias("tawan-cv"), Ok("tawan-cv".to_string()));
        assert_eq!(validate_alias("My-CV-2026"), Ok("my-cv-2026".to_string()));
        assert_eq!(validate_alias("abc"), Ok("abc".to_string()));
        assert_eq!(validate_alias(&"x".repeat(32)), Ok("x".repeat(32)));

        for reserved in [
            "API",
            "api",
            "app",
            "admin",
            "Health",
            "openapi",
            "login",
            "logout",
            "register",
            "static",
            "assets",
            "verify-email",
            "Reset-Password",
            "forgot-password",
        ] {
            assert_eq!(
                validate_alias(reserved),
                Err(AliasValidationError::ReservedAlias),
                "expected ReservedAlias for {reserved}"
            );
        }

        for invalid in ["-cv", "cv-", "ab", "a_b_c", &("x".repeat(33)), "a b", ""] {
            assert_eq!(
                validate_alias(invalid),
                Err(AliasValidationError::InvalidAlias),
                "expected InvalidAlias for {invalid:?}"
            );
        }
    }

    #[test]
    fn random_code_is_7_char_lowercase_base36_and_not_reserved() {
        for _ in 0..50 {
            let code = generate_random_code().expect("generate random code");
            assert_eq!(code.len(), 7);
            assert!(code
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
            assert!(!is_reserved_alias(&code));
        }
    }

    #[test]
    fn referrer_host_extraction() {
        assert_eq!(extract_referrer_host(None), "");
        assert_eq!(extract_referrer_host(Some("")), "");
        assert_eq!(extract_referrer_host(Some("not-a-url")), "");
        assert_eq!(extract_referrer_host(Some("javascript:alert(1)")), "");
        assert_eq!(
            extract_referrer_host(Some("https://www.Facebook.com/x?fbclid=abc")),
            "www.facebook.com"
        );
        assert_eq!(
            extract_referrer_host(Some("http://Example.ORG:8080/page")),
            "example.org"
        );
    }

    #[test]
    fn utc_date_and_30_day_window_math() {
        // 1970-01-01T00:00:00Z
        assert_eq!(today_utc_ymd(0), "1970-01-01");
        // 2024-02-29 (leap day): 19782 days * 86400 = 1_709_164_800
        assert_eq!(today_utc_ymd(1_709_164_800), "2024-02-29");
        // 2026-11-17T00:00:00Z: 20774 days * 86400 = 1_794_873_600
        let nov_17_2026 = 20_774 * 86_400;
        assert_eq!(today_utc_ymd(nov_17_2026), "2026-11-17");

        let days = last_30_days_utc(nov_17_2026);
        assert_eq!(days.len(), 30);
        assert_eq!(days.first().unwrap(), "2026-10-19");
        assert_eq!(days.last().unwrap(), "2026-11-17");
    }

    #[test]
    fn create_window_calculates_fixed_1_min_bucket_and_retry_after() {
        let (start, retry) = create_window(1_800_015);
        assert_eq!(start, 1_800_000);
        assert_eq!(retry, 45);

        let (start_edge, retry_edge) = create_window(1_800_059);
        assert_eq!(start_edge, 1_800_000);
        assert_eq!(retry_edge, 1);
    }
}
