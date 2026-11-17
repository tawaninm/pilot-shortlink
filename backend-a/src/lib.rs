use worker::*;

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

/// Formats a single-line structured JSON request log entry (docs/architecture.md section 8).
/// Never logs cookies, passwords, tokens, emails, or raw shortlink codes.
pub fn format_request_log(method: &str, path: &str, status: u16, ms: u64) -> String {
    let pattern = path_pattern(path);
    format!(
        r#"{{"method":"{method}","path":"{pattern}","status":{status},"ms":{ms}}}"#
    )
}

/// Formats an RFC 9457 `application/problem+json` response body without internal details or stack traces.
pub fn problem_json_body(status: u16, title: &str, code: &str) -> String {
    format!(
        r#"{{"type":"about:blank","title":"{title}","status":{status},"code":"{code}"}}"#
    )
}

fn json_response(status: u16, content_type: &str, body: String) -> Result<Response> {
    let headers = Headers::new();
    headers.set("Content-Type", content_type)?;
    Ok(Response::from_body(ResponseBody::Body(body.into_bytes()))?
        .with_status(status)
        .with_headers(headers))
}

fn problem_response(status: u16, title: &str, code: &str) -> Result<Response> {
    json_response(
        status,
        "application/problem+json",
        problem_json_body(status, title, code),
    )
}

async fn route_request(req: &Request, _env: &Env) -> Result<Response> {
    let method = req.method();
    let path = req.path();

    if method == Method::Get && path == "/health" {
        return json_response(200, "application/json", r#"{"status":"ok"}"#.to_string());
    }

    if path == "/api" || path.starts_with("/api/") {
        return problem_response(404, "Not Found", "not_found");
    }

    Response::error("Not Found", 404)
}

#[event(fetch)]
pub async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let start_ms = Date::now().as_millis();
    let method_str = req.method().to_string();
    let raw_path = req.path();

    let response = match route_request(&req, &env).await {
        Ok(res) => res,
        Err(err) => {
            console_error!(r#"{{"event":"internal_error","error":"{err}"}}"#);
            problem_response(500, "Internal Server Error", "internal_error")?
        }
    };

    let elapsed_ms = Date::now().as_millis().saturating_sub(start_ms);
    let log_line = format_request_log(&method_str, &raw_path, response.status_code(), elapsed_ms);
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
        assert_eq!(path_pattern("/api/links/my-secret-code"), "/api/links/{code}");
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
        let line = format_request_log(
            "GET",
            "/api/links/secret-code-123/stats",
            404,
            3,
        );
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
