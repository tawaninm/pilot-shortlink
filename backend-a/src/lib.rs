pub mod auth;
pub mod captcha;
pub mod http;
pub mod links;
pub mod mail;
pub mod oauth;
pub mod session;

use serde::Serialize;
use worker::*;

pub use crate::http::problem_json_body;

/// Maps a raw request URL path to a safe route pattern so logs never contain
/// shortlink codes, emails, tokens, or arbitrary user-supplied segments (OWASP A09).
pub fn path_pattern(path: &str) -> &'static str {
    match path {
        "/" => "/",
        "/login" => "/login",
        "/register" => "/register",
        "/verify-email" => "/verify-email",
        "/forgot-password" => "/forgot-password",
        "/reset-password" => "/reset-password",
        "/health" => "/health",
        "/api/auth/register" => "/api/auth/register",
        "/api/auth/login" => "/api/auth/login",
        "/api/auth/logout" => "/api/auth/logout",
        "/api/auth/verify-email" => "/api/auth/verify-email",
        "/api/auth/verify-email/resend" => "/api/auth/verify-email/resend",
        "/api/auth/password/forgot" => "/api/auth/password/forgot",
        "/api/auth/password/reset" => "/api/auth/password/reset",
        "/api/config" => "/api/config",
        _ if path.starts_with("/api/auth/oauth/") => {
            let rest = &path["/api/auth/oauth/".len()..];
            match rest.split_once('/') {
                Some((provider, "start")) if !provider.is_empty() => {
                    "/api/auth/oauth/{provider}/start"
                }
                Some((provider, "callback")) if !provider.is_empty() => {
                    "/api/auth/oauth/{provider}/callback"
                }
                _ => "/api/*",
            }
        }
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
        "/assets" => "/assets/*",
        _ if path.starts_with("/assets/") => "/assets/*",
        _ => {
            if is_short_code_candidate(path).is_some() {
                "/{code}"
            } else {
                "/*"
            }
        }
    }
}

/// Returns `true` if `path` is an endpoint that requires or manages an active session
/// (`/api/me`, `/api/links`, `/api/links/{code}`, `/api/links/{code}/stats`, `/api/auth/logout`),
/// whose responses (including errors) must carry `Cache-Control: no-store`.
pub fn is_session_endpoint(path: &str) -> bool {
    matches!(
        path_pattern(path),
        "/api/me"
            | "/api/links"
            | "/api/links/{code}"
            | "/api/links/{code}/stats"
            | "/api/auth/logout"
            | "/api/auth/verify-email/resend"
    )
}

/// Returns `true` if `path` is one of the SPA client-side routes that should return `index.html`.
pub fn is_spa_route(path: &str) -> bool {
    matches!(
        path,
        "/" | "/login" | "/register" | "/verify-email" | "/forgot-password" | "/reset-password"
    )
}

/// Returns `Some(segment)` if `path` is a single-segment path that is NOT a reserved word
/// (`links::RESERVED_ALIASES`), so reserved words like `login`, `register`, `assets`, etc.
/// can never reach the redirect handler.
pub fn is_short_code_candidate(path: &str) -> Option<&str> {
    let trimmed = path.trim_start_matches('/');
    if trimmed.is_empty() || trimmed.contains('/') {
        return None;
    }
    if links::is_reserved_alias(&trimmed.to_ascii_lowercase()) {
        return None;
    }
    Some(trimmed)
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

async fn route_request(
    req: &mut Request,
    env: &Env,
    ctx: &Context,
) -> Result<(Response, Option<u64>)> {
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

        if method == Method::Get && path == "/api/test/mail" {
            if let Some(captured_res) = mail::handle_get_captured_mail(req, env)? {
                return Ok((captured_res, None));
            }
            return Ok((http::problem_response(404, "Not Found", "not_found")?, None));
        }

        if method == Method::Get && path == "/api/config" {
            return Ok((captcha::handle_config(env, &app_origin)?, None));
        }

        let db = env.d1("DB")?;
        let mailer = mail::EnvMailer::from_env(env);
        let captcha_verifier = captcha::EnvCaptchaVerifier::from_env(env, &app_origin);

        return match (method, path.as_str()) {
            (Method::Post, "/api/auth/register") => {
                auth::handle_register(req, &db, &app_origin, mailer, &captcha_verifier, ctx).await
            }
            (Method::Post, "/api/auth/login") => auth::handle_login(req, &db).await,
            (Method::Post, "/api/auth/logout") => Ok((auth::handle_logout(req, &db).await?, None)),
            (Method::Post, "/api/auth/verify-email") => {
                Ok((auth::handle_verify_email(req, &db).await?, None))
            }
            (Method::Post, "/api/auth/verify-email/resend") => Ok((
                auth::handle_resend_verify_email(req, &db, &app_origin, mailer, ctx).await?,
                None,
            )),
            (Method::Post, "/api/auth/password/forgot") => Ok((
                auth::handle_forgot_password(req, &db, &app_origin, mailer, &captcha_verifier, ctx)
                    .await?,
                None,
            )),
            (Method::Post, "/api/auth/password/reset") => {
                auth::handle_reset_password(req, &db).await
            }
            (Method::Get, p) if p.starts_with("/api/auth/oauth/") => {
                let rest = &p["/api/auth/oauth/".len()..];
                match rest.split_once('/') {
                    Some((raw_provider, "start")) => match oauth::Provider::parse(raw_provider) {
                        Some(provider) => Ok((
                            oauth::handle_oauth_start(req, env, &db, &app_origin, provider).await?,
                            None,
                        )),
                        None => Ok((http::problem_response(404, "Not Found", "not_found")?, None)),
                    },
                    Some((raw_provider, "callback")) => {
                        match oauth::Provider::parse(raw_provider) {
                            Some(provider) => Ok((
                                oauth::handle_oauth_callback(req, env, &db, &app_origin, provider)
                                    .await?,
                                None,
                            )),
                            None => {
                                Ok((http::problem_response(404, "Not Found", "not_found")?, None))
                            }
                        }
                    }
                    _ => Ok((http::problem_response(404, "Not Found", "not_found")?, None)),
                }
            }
            (Method::Get, "/api/me") => Ok((auth::handle_me(req, &db).await?, None)),
            (Method::Get, "/api/links") => Ok((links::handle_list_links(req, &db).await?, None)),
            (Method::Post, "/api/links") => Ok((
                links::handle_create_link(req, &db, &app_origin).await?,
                None,
            )),
            (m, p) if p.starts_with("/api/links/") => {
                let rest = &p["/api/links/".len()..];
                match (m, rest.split_once('/')) {
                    (Method::Patch, None) if !rest.is_empty() => Ok((
                        links::handle_patch_link(req, &db, &app_origin, rest).await?,
                        None,
                    )),
                    (Method::Delete, None) if !rest.is_empty() => {
                        Ok((links::handle_delete_link(req, &db, rest).await?, None))
                    }
                    (Method::Get, Some((code, "stats"))) if !code.is_empty() => {
                        Ok((links::handle_link_stats(req, &db, code).await?, None))
                    }
                    _ => Ok((http::problem_response(404, "Not Found", "not_found")?, None)),
                }
            }
            _ => Ok((http::problem_response(404, "Not Found", "not_found")?, None)),
        };
    }

    if method == Method::Get {
        if is_spa_route(&path) {
            let assets = env.assets("ASSETS")?;
            let mut index_url = req.url()?;
            index_url.set_path("/");
            index_url.set_query(None);
            return Ok((assets.fetch(index_url.to_string(), None).await?, None));
        }

        if path == "/assets" || path.starts_with("/assets/") {
            let assets = env.assets("ASSETS")?;
            return Ok((assets.fetch(req.url()?.to_string(), None).await?, None));
        }

        if let Some(code) = is_short_code_candidate(&path) {
            let db = env.d1("DB")?;
            return Ok((links::handle_redirect(req, &db, code).await?, None));
        }
    }

    Ok((links::html_not_found_response()?, None))
}

#[event(fetch)]
pub async fn fetch(mut req: Request, env: Env, ctx: Context) -> Result<Response> {
    let start_ms = http::now_ms();
    let method_str = req.method().to_string();
    let raw_path = req.path();

    let (raw_response, hash_ms) = match route_request(&mut req, &env, &ctx).await {
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

    let response = http::apply_security_headers(raw_response, is_session_endpoint(&raw_path))?;

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
        assert_eq!(path_pattern("/"), "/");
        assert_eq!(path_pattern("/login"), "/login");
        assert_eq!(path_pattern("/register"), "/register");
        assert_eq!(path_pattern("/verify-email"), "/verify-email");
        assert_eq!(path_pattern("/forgot-password"), "/forgot-password");
        assert_eq!(path_pattern("/reset-password"), "/reset-password");
        assert_eq!(
            path_pattern("/api/auth/verify-email"),
            "/api/auth/verify-email"
        );
        assert_eq!(
            path_pattern("/api/auth/verify-email/resend"),
            "/api/auth/verify-email/resend"
        );
        assert_eq!(
            path_pattern("/api/auth/password/forgot"),
            "/api/auth/password/forgot"
        );
        assert_eq!(
            path_pattern("/api/auth/password/reset"),
            "/api/auth/password/reset"
        );
        assert_eq!(path_pattern("/api/config"), "/api/config");
        assert_eq!(
            path_pattern("/api/auth/oauth/google/start"),
            "/api/auth/oauth/{provider}/start"
        );
        assert_eq!(
            path_pattern("/api/auth/oauth/github/callback"),
            "/api/auth/oauth/{provider}/callback"
        );
        assert_eq!(
            path_pattern("/api/auth/oauth/twitter/callback"),
            "/api/auth/oauth/{provider}/callback"
        );
        assert_eq!(path_pattern("/api/auth/oauth/google/other"), "/api/*");
        assert_eq!(path_pattern("/assets/index-IjLlgpqB.js"), "/assets/*");
        assert_eq!(path_pattern("/abc1234"), "/{code}");
        assert_eq!(path_pattern("/admin"), "/*");

        assert!(is_session_endpoint("/api/me"));
        assert!(is_session_endpoint("/api/links"));
        assert!(is_session_endpoint("/api/links/my-code"));
        assert!(is_session_endpoint("/api/links/my-code/stats"));
        assert!(is_session_endpoint("/api/auth/logout"));
        assert!(is_session_endpoint("/api/auth/verify-email/resend"));

        assert!(!is_session_endpoint("/api/auth/register"));
        assert!(!is_session_endpoint("/api/auth/login"));
        assert!(!is_session_endpoint("/api/auth/verify-email"));
        assert!(!is_session_endpoint("/api/auth/password/forgot"));
        assert!(!is_session_endpoint("/api/auth/password/reset"));
        assert!(!is_session_endpoint("/health"));
        assert!(!is_session_endpoint("/"));
        assert!(!is_session_endpoint("/abc1234"));
    }

    #[test]
    fn spa_routes_and_reserved_words_never_reach_redirect_candidate() {
        assert!(is_spa_route("/"));
        assert!(is_spa_route("/login"));
        assert!(is_spa_route("/register"));
        assert!(is_spa_route("/verify-email"));
        assert!(is_spa_route("/forgot-password"));
        assert!(is_spa_route("/reset-password"));
        assert!(!is_spa_route("/abc1234"));

        for reserved in links::RESERVED_ALIASES {
            let path = format!("/{reserved}");
            assert_eq!(
                is_short_code_candidate(&path),
                None,
                "reserved word {reserved} must not be a short code candidate"
            );
            let upper_path = format!("/{}", reserved.to_ascii_uppercase());
            assert_eq!(
                is_short_code_candidate(&upper_path),
                None,
                "uppercase reserved word {reserved} must not be a short code candidate"
            );
        }

        assert_eq!(is_short_code_candidate("/abc1234"), Some("abc1234"));
        assert_eq!(is_short_code_candidate("/assets/app.js"), None);
        assert_eq!(is_short_code_candidate("/"), None);
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

        let auth_log = format_request_log_with_hash("POST", "/api/auth/login", 200, 60, Some(54));
        assert_eq!(
            auth_log,
            r#"{"method":"POST","path":"/api/auth/login","status":200,"ms":60,"hash_ms":54}"#
        );

        let oauth_cb_log =
            format_request_log("GET", "/api/auth/oauth/secret-provider/callback", 302, 12);
        assert_eq!(
            oauth_cb_log,
            r#"{"method":"GET","path":"/api/auth/oauth/{provider}/callback","status":302,"ms":12}"#
        );
        assert!(!oauth_cb_log.contains("secret-provider"));
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
