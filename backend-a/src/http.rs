use serde::Serialize;
use worker::*;

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
}
