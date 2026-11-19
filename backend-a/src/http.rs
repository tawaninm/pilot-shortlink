use serde::Serialize;
use worker::*;

/// Content-Security-Policy applied to every HTML response (OWASP A02).
pub const HTML_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' https://fonts.googleapis.com; font-src https://fonts.gstatic.com; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'";

/// Referrer-Policy applied to every HTML response (OWASP A02).
pub const HTML_REFERRER_POLICY: &str = "strict-origin-when-cross-origin";

/// Strict-Transport-Security applied to every HTML response (S7 review).
pub const HTML_HSTS: &str = "max-age=31536000; includeSubDomains";

/// Maximum allowed JSON request body size in bytes (16 KiB).
pub const MAX_JSON_BODY_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyReadError {
    TooLarge,
}

/// Checks an optional `Content-Length` header value against `max_bytes`.
/// Returns `Err(BodyReadError::TooLarge)` if `Content-Length` is a numeric value exceeding `max_bytes`.
pub fn check_content_length(
    content_length: Option<&str>,
    max_bytes: usize,
) -> std::result::Result<(), BodyReadError> {
    let Some(raw) = content_length.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(());
    };
    if let Ok(len) = raw.parse::<u64>() {
        if len > max_bytes as u64 {
            return Err(BodyReadError::TooLarge);
        }
    } else if raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err(BodyReadError::TooLarge);
    }
    Ok(())
}

/// Appends `chunk` to `buf` if the resulting length is at most `max_bytes`,
/// or returns `Err(BodyReadError::TooLarge)` without growing `buf` past `max_bytes`.
pub fn append_bounded_chunk(
    buf: &mut Vec<u8>,
    chunk: &[u8],
    max_bytes: usize,
) -> std::result::Result<(), BodyReadError> {
    if buf.len().saturating_add(chunk.len()) > max_bytes {
        return Err(BodyReadError::TooLarge);
    }
    buf.extend_from_slice(chunk);
    Ok(())
}

/// Validates `content_length` first and then accumulates `chunks` up to `max_bytes`.
pub fn collect_bounded_chunks<'a, I>(
    content_length: Option<&str>,
    chunks: I,
    max_bytes: usize,
) -> std::result::Result<Vec<u8>, BodyReadError>
where
    I: IntoIterator<Item = &'a [u8]>,
{
    check_content_length(content_length, max_bytes)?;
    let mut buf = Vec::new();
    for chunk in chunks {
        append_bounded_chunk(&mut buf, chunk, max_bytes)?;
    }
    Ok(buf)
}

/// Reads the request body up to `max_bytes`, checking `Content-Length` first and capping
/// the bytes actually read from the stream.
pub async fn read_bounded_body(
    req: &mut Request,
    max_bytes: usize,
) -> Result<std::result::Result<Vec<u8>, BodyReadError>> {
    let content_length = req.headers().get("Content-Length")?;
    if let Err(err) = check_content_length(content_length.as_deref(), max_bytes) {
        return Ok(Err(err));
    }

    let Some(stream) = req.inner().body() else {
        return Ok(Ok(Vec::new()));
    };

    let reader = web_sys::ReadableStreamDefaultReader::new(&stream)?;
    let mut buf = Vec::new();

    loop {
        let step = wasm_bindgen_futures::JsFuture::from(reader.read()).await?;
        let done = js_sys::Reflect::get(&step, &"done".into())?
            .as_bool()
            .unwrap_or(false);
        if done {
            break;
        }
        let value = js_sys::Reflect::get(&step, &"value".into())?;
        if !value.is_undefined() && !value.is_null() {
            let chunk = js_sys::Uint8Array::new(&value).to_vec();
            if let Err(err) = append_bounded_chunk(&mut buf, &chunk, max_bytes) {
                let _ = reader.cancel();
                return Ok(Err(err));
            }
        }
    }

    Ok(Ok(buf))
}

/// Reads a JSON request body capped at `MAX_JSON_BODY_BYTES` (16 KiB), returning a
/// `413 Payload Too Large` `application/problem+json` (`code: "invalid_body"`) response if exceeded.
pub async fn read_json_body(req: &mut Request) -> Result<std::result::Result<Vec<u8>, Response>> {
    match read_bounded_body(req, MAX_JSON_BODY_BYTES).await? {
        Ok(bytes) => Ok(Ok(bytes)),
        Err(BodyReadError::TooLarge) => Ok(Err(problem_response(
            413,
            "Payload Too Large",
            "invalid_body",
        )?)),
    }
}

/// Returns true if a `Content-Type` header value represents an HTML document (`text/html`).
pub fn is_html_content_type(content_type: &str) -> bool {
    let media_type = content_type.split(';').next().unwrap_or("").trim();
    media_type.eq_ignore_ascii_case("text/html")
}

/// Attaches security headers in one place for every outgoing response:
/// - `X-Content-Type-Options: nosniff` on every response (HTML, API, `/health`, `problem+json`, assets).
/// - `Content-Security-Policy`, `Referrer-Policy`, and `Strict-Transport-Security` on every `text/html` response.
/// - `Cache-Control: no-store` on every response from an endpoint that requires or manages a session.
pub fn apply_security_headers(response: Response, no_store: bool) -> Result<Response> {
    let headers = response.headers().clone();
    headers.set("X-Content-Type-Options", "nosniff")?;
    if no_store {
        headers.set("Cache-Control", "no-store")?;
    }
    let is_html = headers
        .get("Content-Type")?
        .is_some_and(|ct| is_html_content_type(&ct));
    if is_html {
        headers.set("Content-Security-Policy", HTML_CSP)?;
        headers.set("Referrer-Policy", HTML_REFERRER_POLICY)?;
        headers.set("Strict-Transport-Security", HTML_HSTS)?;
    }
    Ok(response.with_headers(headers))
}

/// RFC 9457 `application/problem+json` response payload (`api/openapi.yaml` `#/components/schemas/Problem`).
#[derive(Serialize)]
pub struct ProblemBody<'a> {
    #[serde(rename = "type")]
    pub kind: &'a str,
    pub title: &'a str,
    pub status: u16,
    pub code: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<&'a str>,
}

/// Serializes an RFC 9457 `application/problem+json` body using `serde_json` so titles,
/// codes, fields, and details are always JSON-escaped.
pub fn problem_json_body_ext(
    status: u16,
    title: &str,
    code: &str,
    field: Option<&str>,
    detail: Option<&str>,
) -> String {
    serde_json::to_string(&ProblemBody {
        kind: "about:blank",
        title,
        status,
        code,
        detail,
        field,
    })
    .expect("ProblemBody serialization cannot fail")
}

/// Formats an RFC 9457 `application/problem+json` response body without internal details or stack traces.
pub fn problem_json_body(status: u16, title: &str, code: &str) -> String {
    problem_json_body_ext(status, title, code, None, None)
}

pub fn json_response(status: u16, content_type: &str, body: String) -> Result<Response> {
    json_response_with_headers(status, content_type, body, &[])
}

pub fn json_response_with_headers(
    status: u16,
    content_type: &str,
    body: String,
    extra_headers: &[(&str, &str)],
) -> Result<Response> {
    let headers = Headers::new();
    headers.set("Content-Type", content_type)?;
    for &(k, v) in extra_headers {
        headers.set(k, v)?;
    }
    Ok(Response::from_body(ResponseBody::Body(body.into_bytes()))?
        .with_status(status)
        .with_headers(headers))
}

pub fn problem_response(status: u16, title: &str, code: &str) -> Result<Response> {
    json_response(
        status,
        "application/problem+json",
        problem_json_body(status, title, code),
    )
}

pub fn problem_field_response(
    status: u16,
    title: &str,
    code: &str,
    field: &str,
) -> Result<Response> {
    json_response(
        status,
        "application/problem+json",
        problem_json_body_ext(status, title, code, Some(field), None),
    )
}

pub fn rate_limited_response(retry_after_secs: i64) -> Result<Response> {
    let retry_str = retry_after_secs.max(1).to_string();
    json_response_with_headers(
        429,
        "application/problem+json",
        problem_json_body(429, "Too Many Requests", "rate_limited"),
        &[("Retry-After", &retry_str)],
    )
}

/// Returns true if `method` is a state-changing HTTP method under `/api/*` that requires
/// `Origin` header validation (POST, PATCH, DELETE).
pub fn requires_origin_check(method: &Method) -> bool {
    matches!(method, Method::Post | Method::Patch | Method::Delete)
}

/// Validates that `req`'s `Origin` header exactly matches `app_origin` for state-changing
/// `/api/*` requests (CSRF protection, OWASP A01).
pub fn check_origin(req: &Request, app_origin: &str) -> Result<Option<Response>> {
    if !requires_origin_check(&req.method()) {
        return Ok(None);
    }
    let origin = req.headers().get("Origin")?;
    if origin.as_deref() != Some(app_origin) {
        return Ok(Some(problem_response(
            403,
            "Forbidden",
            "forbidden_origin",
        )?));
    }
    Ok(None)
}

/// Current epoch timestamp in milliseconds (`worker::Date::now()` on wasm32, `SystemTime` on host tests).
#[cfg(target_arch = "wasm32")]
pub fn now_ms() -> u64 {
    Date::now().as_millis()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Current epoch timestamp in seconds.
pub fn now_sec() -> i64 {
    (now_ms() / 1000) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn problem_json_body_escapes_quotes_and_newlines() {
        let body = problem_json_body_ext(
            422,
            "Invalid \"Input\"\nLine2",
            "invalid_body",
            Some("email"),
            Some("bad \"quote\"\nand newline"),
        );
        assert!(!body.contains('\n'));
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
        assert_eq!(parsed["type"], "about:blank");
        assert_eq!(parsed["title"], "Invalid \"Input\"\nLine2");
        assert_eq!(parsed["status"], 422);
        assert_eq!(parsed["code"], "invalid_body");
        assert_eq!(parsed["field"], "email");
        assert_eq!(parsed["detail"], "bad \"quote\"\nand newline");
    }

    #[test]
    fn html_content_type_detection_and_csp_constants() {
        assert!(is_html_content_type("text/html"));
        assert!(is_html_content_type("text/html; charset=utf-8"));
        assert!(is_html_content_type("TEXT/HTML; charset=UTF-8"));
        assert!(!is_html_content_type("application/json"));
        assert!(!is_html_content_type("application/problem+json"));
        assert!(!is_html_content_type("text/javascript"));

        assert!(HTML_CSP.contains("default-src 'self'"));
        assert!(HTML_CSP.contains("frame-ancestors 'none'"));
        assert!(!HTML_CSP.contains("'unsafe-eval'"));
        assert_eq!(HTML_REFERRER_POLICY, "strict-origin-when-cross-origin");
        assert_eq!(HTML_HSTS, "max-age=31536000; includeSubDomains");
    }

    #[test]
    fn body_cap_checks_content_length_and_stream_bytes_at_16_kib() {
        assert_eq!(MAX_JSON_BODY_BYTES, 16 * 1024);

        // Missing or exact 16 KiB Content-Length passes
        assert_eq!(check_content_length(None, MAX_JSON_BODY_BYTES), Ok(()));
        assert_eq!(check_content_length(Some(""), MAX_JSON_BODY_BYTES), Ok(()));
        assert_eq!(
            check_content_length(Some("16384"), MAX_JSON_BODY_BYTES),
            Ok(())
        );

        // Over 16 KiB Content-Length is rejected before reading stream
        assert_eq!(
            check_content_length(Some("16385"), MAX_JSON_BODY_BYTES),
            Err(BodyReadError::TooLarge)
        );
        assert_eq!(
            check_content_length(Some(" 65536 "), MAX_JSON_BODY_BYTES),
            Err(BodyReadError::TooLarge)
        );
        assert_eq!(
            check_content_length(Some("999999999999999999999999999999"), MAX_JSON_BODY_BYTES),
            Err(BodyReadError::TooLarge)
        );

        // Content-Length over limit fails even if chunks are empty
        assert_eq!(
            collect_bounded_chunks(Some("16385"), [b"{}".as_slice()], MAX_JSON_BODY_BYTES),
            Err(BodyReadError::TooLarge)
        );

        // Stream cap allows up to 16 KiB across multiple chunks
        let chunk_8k = vec![b'a'; 8 * 1024];
        let exact_16k = collect_bounded_chunks(
            None,
            [chunk_8k.as_slice(), chunk_8k.as_slice()],
            MAX_JSON_BODY_BYTES,
        )
        .expect("16 KiB body must be allowed");
        assert_eq!(exact_16k.len(), MAX_JSON_BODY_BYTES);

        // Stream cap rejects when actual bytes exceed 16 KiB even if Content-Length is missing or lies
        assert_eq!(
            collect_bounded_chunks(
                Some("10"),
                [chunk_8k.as_slice(), chunk_8k.as_slice(), b"x".as_slice()],
                MAX_JSON_BODY_BYTES,
            ),
            Err(BodyReadError::TooLarge)
        );
    }
}
