pub mod auth;
pub mod http;
pub mod session;

use serde::Serialize;
use worker::*;

pub use crate::http::problem_json_body;

/// Maps a raw request URL path to a safe route pattern so logs never contain
/// shortlink codes, emails, tokens, or arbitrary user-supplied segments (OWASP A09).
pub fn path_pattern(path: &str) -> &'static str {
    match path {
        "/" => "/",
        "/health" => "/health",
        "/api/auth/register" => "/api/auth/register",
        "/api/auth/login" => "/api/auth/login",
        "/api/auth/logout" => "/api/auth/logout",
        "/api/me" => "/api/me",
        "/api/links" => "/api/links",
        _ if path.starts_with("/api/links/") => {
            let rest = &path["/api/links/".len()..];
            match rest.split_once('/') {
                None if !rest.is_empty() => "/api/links/{code}",
                Some((code, "stats")) if !code.is_empty() => "/api/links/{code}/stats",
                _ => "/api/*",
            }
        }
        "/api" => "/api/*",
        _ if path.starts_with("/api/") => "/api/*",
        _ => {
            let trimmed = path.trim_start_matches('/');
            if !trimmed.is_empty() && !trimmed.contains('/') {
                "/{code}"
            } else {
                "/*"
            }
        }
    }
}

#[derive(Serialize)]
struct RequestLog<'a> {
    method: &'a str,
    path: &'a str,
    status: u16,
    ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    hash_ms: Option<u64>,
}

#[derive(Serialize)]
struct InternalErrorLog<'a> {
    event: &'static str,
    error: &'a str,
}

/// Formats a single-line structured JSON request log entry (docs/architecture.md section 8).
/// Never logs cookies, passwords, tokens, emails, or raw shortlink codes.
pub fn format_request_log(method: &str, path: &str, status: u16, ms: u64) -> String {
    format_request_log_with_hash(method, path, status, ms, None)
}

/// Formats a single-line structured JSON request log entry, including optional Argon2 `hash_ms` (ADR-002).
pub fn format_request_log_with_hash(
    method: &str,
    path: &str,
    status: u16,
    ms: u64,
    hash_ms: Option<u64>,
) -> String {
    let pattern = path_pattern(path);
    serde_json::to_string(&RequestLog {
        method,
        path: pattern,
        status,
        ms,
        hash_ms,
    })
    .expect("RequestLog serialization cannot fail")
}

/// Formats an internal error log line using `serde_json` so quotes and newlines are escaped.
pub fn format_error_log(error: &str) -> String {
    serde_json::to_string(&InternalErrorLog {
        event: "internal_error",
        error,
    })
    .expect("InternalErrorLog serialization cannot fail")
}

async fn route_request(req: &mut Request, env: &Env) -> Result<(Response, Option<u64>)> {
    let method = req.method();
    let path = req.path();

    if method == Method::Get && path == "/health" {
        return Ok((
            http::json_response(200, "application/json", r#"{"status":"ok"}"#.to_string())?,
            None,
        ));
    }

    if path == "/api" || path.starts_with("/api/") {
        let app_origin = env.var("APP_ORIGIN")?.to_string();
        if let Some(forbidden) = http::check_origin(req, &app_origin)? {
            return Ok((forbidden, None));
        }

        let db = env.d1("DB")?;

        return match (method, path.as_str()) {
            (Method::Post, "/api/auth/register") => auth::handle_register(req, &db).await,
            (Method::Post, "/api/auth/login") => auth::handle_login(req, &db).await,
            (Method::Post, "/api/auth/logout") => Ok((auth::handle_logout(req, &db).await?, None)),
            (Method::Get, "/api/me") => Ok((auth::handle_me(req, &db).await?, None)),
            _ => Ok((http::problem_response(404, "Not Found", "not_found")?, None)),
        };
    }

    Ok((Response::error("Not Found", 404)?, None))
}

#[event(fetch)]
pub async fn fetch(mut req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let start_ms = http::now_ms();
    let method_str = req.method().to_string();
    let raw_path = req.path();

    let (response, hash_ms) = match route_request(&mut req, &env).await {
        Ok(pair) => pair,
        Err(err) => {
            let err_line = format_error_log(&err.to_string());
            console_error!("{err_line}");
            (
                http::problem_response(500, "Internal Server Error", "internal_error")?,
                None,
            )
        }
    };

    let elapsed_ms = http::now_ms().saturating_sub(start_ms);
    let log_line = format_request_log_with_hash(
        &method_str,
        &raw_path,
        response.status_code(),
        elapsed_ms,
        hash_ms,
    );
    console_log!("{log_line}");

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_pattern_scrubs_codes_and_unknown_api_segments() {
        assert_eq!(path_pattern("/health"), "/health");
        assert_eq!(path_pattern("/api/links"), "/api/links");
        assert_eq!(
            path_pattern("/api/links/my-secret-code"),
            "/api/links/{code}"
        );
        assert_eq!(
            path_pattern("/api/links/my-secret-code/stats"),
            "/api/links/{code}/stats"
        );
        assert_eq!(
            path_pattern("/api/does-not-exist/user@example.com"),
            "/api/*"
        );
        assert_eq!(path_pattern("/abc1234"), "/{code}");
    }

    #[test]
    fn request_log_omits_raw_code_and_email() {
        let line = format_request_log("GET", "/api/links/secret-code-123/stats", 404, 3);
        assert_eq!(
            line,
            r#"{"method":"GET","path":"/api/links/{code}/stats","status":404,"ms":3}"#
        );
        assert!(!line.contains("secret-code-123"));

        let unknown_api = format_request_log("POST", "/api/users/alice@example.com", 404, 1);
        assert!(!unknown_api.contains("alice@example.com"));
        assert_eq!(
            unknown_api,
            r#"{"method":"POST","path":"/api/*","status":404,"ms":1}"#
        );

        let auth_log =
            format_request_log_with_hash("POST", "/api/auth/login", 200, 60, Some(54));
        assert_eq!(
            auth_log,
            r#"{"method":"POST","path":"/api/auth/login","status":200,"ms":60,"hash_ms":54}"#
        );
    }

    #[test]
    fn error_log_escapes_quotes_and_newlines() {
        let msg = "D1 error: near \"syntax\":\nunexpected token";
        let line = format_error_log(msg);
        assert!(
            !line.contains('\n'),
            "log line must be a single line without raw newlines: {line}"
        );
        let parsed: serde_json::Value =
            serde_json::from_str(&line).expect("error log must be valid JSON");
        assert_eq!(parsed["event"], "internal_error");
        assert_eq!(parsed["error"], msg);
    }

    #[test]
    fn problem_body_has_required_rfc9457_fields_and_no_stack_trace() {
        let body = problem_json_body(404, "Not Found", "not_found");
        assert_eq!(
            body,
            r#"{"type":"about:blank","title":"Not Found","status":404,"code":"not_found"}"#
        );
        assert!(!body.contains(".rs:"));
        assert!(!body.contains("panicked"));
    }
}
