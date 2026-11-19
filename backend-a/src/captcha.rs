use serde::{Deserialize, Serialize};
use worker::*;

use crate::http::{encode_form_urlencoded, json_response, read_env_string};
use crate::mail::is_local_origin;

/// Cloudflare Turnstile server-side verification endpoint.
pub const TURNSTILE_SITEVERIFY_URL: &str =
    "https://challenges.cloudflare.com/turnstile/v0/siteverify";

/// Exact token accepted when `TURNSTILE_MODE=fake` on a local `APP_ORIGIN` (AC-T1..T2).
pub const FAKE_CAPTCHA_TOKEN: &str = "fake-pass";

/// Maximum accepted captcha token length (`api/openapi.yaml` `#/components/schemas/Captcha`).
pub const MAX_CAPTCHA_LEN: usize = 2048;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptchaFailureReason {
    MissingToken,
    MissingSecret,
    Transport,
    HttpStatus,
    InvalidResponse,
    Rejected,
}

impl CaptchaFailureReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MissingToken => "missing_token",
            Self::MissingSecret => "missing_secret",
            Self::Transport => "transport_error",
            Self::HttpStatus => "http_status",
            Self::InvalidResponse => "invalid_response",
            Self::Rejected => "rejected",
        }
    }
}

#[derive(Serialize)]
struct CaptchaFailureLog {
    event: &'static str,
    reason: &'static str,
}

/// Formats a single-line structured JSON log for a captcha verification failure.
/// Never logs the captcha token, secret, IP, or provider response body (I7).
pub fn format_captcha_failure_log(reason: CaptchaFailureReason) -> String {
    serde_json::to_string(&CaptchaFailureLog {
        event: "captcha_failed",
        reason: reason.as_str(),
    })
    .expect("CaptchaFailureLog serialization cannot fail")
}

fn emit_captcha_failure_log(reason: CaptchaFailureReason) {
    let line = format_captcha_failure_log(reason);
    #[cfg(target_arch = "wasm32")]
    console_error!("{line}");
    #[cfg(not(target_arch = "wasm32"))]
    let _ = line;
}

/// Returns `true` only when `TURNSTILE_MODE` is `"fake"` AND `APP_ORIGIN` is a local origin (I6).
pub fn is_fake_turnstile_enabled(turnstile_mode: Option<&str>, app_origin: &str) -> bool {
    turnstile_mode == Some("fake") && is_local_origin(app_origin)
}

/// Resolves the public `turnstileSiteKey` returned by `GET /api/config`.
/// Always returns `None` in fake mode; otherwise returns the trimmed non-empty `TURNSTILE_SITE_KEY`.
pub fn resolve_turnstile_site_key(
    turnstile_mode: Option<&str>,
    app_origin: &str,
    configured_site_key: Option<&str>,
) -> Option<String> {
    if is_fake_turnstile_enabled(turnstile_mode, app_origin) {
        return None;
    }
    configured_site_key
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigResponse {
    #[serde(rename = "turnstileSiteKey")]
    pub turnstile_site_key: Option<String>,
}

/// `GET /api/config`
pub fn handle_config(env: &Env, app_origin: &str) -> Result<Response> {
    let mode = read_env_string(env, "TURNSTILE_MODE");
    let site_key = read_env_string(env, "TURNSTILE_SITE_KEY");
    let resolved = resolve_turnstile_site_key(mode.as_deref(), app_origin, site_key.as_deref());
    let body = serde_json::to_string(&ConfigResponse {
        turnstile_site_key: resolved,
    })?;
    json_response(200, "application/json", body)
}

#[allow(async_fn_in_trait)]
pub trait CaptchaVerifier {
    async fn verify(&self, token: Option<&str>, remote_ip: Option<&str>) -> bool;
}

#[derive(Debug, Clone, Default)]
pub struct FakeCaptchaVerifier;

impl CaptchaVerifier for FakeCaptchaVerifier {
    async fn verify(&self, token: Option<&str>, _remote_ip: Option<&str>) -> bool {
        let Some(tok) = token.filter(|t| !t.is_empty() && t.len() <= MAX_CAPTCHA_LEN) else {
            emit_captcha_failure_log(CaptchaFailureReason::MissingToken);
            return false;
        };
        if tok == FAKE_CAPTCHA_TOKEN {
            true
        } else {
            emit_captcha_failure_log(CaptchaFailureReason::Rejected);
            false
        }
    }
}

#[derive(Clone)]
pub struct CloudflareCaptchaVerifier {
    pub secret: Option<String>,
}

// Manual Debug so `{:?}` never prints TURNSTILE_SECRET into Workers logs (I7).
impl std::fmt::Debug for CloudflareCaptchaVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudflareCaptchaVerifier")
            .field("secret", &self.secret.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

#[derive(Deserialize)]
struct SiteverifyResponse {
    #[serde(default)]
    success: bool,
}

impl CaptchaVerifier for CloudflareCaptchaVerifier {
    async fn verify(&self, token: Option<&str>, remote_ip: Option<&str>) -> bool {
        let Some(tok) = token.filter(|t| !t.is_empty() && t.len() <= MAX_CAPTCHA_LEN) else {
            emit_captcha_failure_log(CaptchaFailureReason::MissingToken);
            return false;
        };
        let Some(secret) = self.secret.as_deref().filter(|s| !s.trim().is_empty()) else {
            emit_captcha_failure_log(CaptchaFailureReason::MissingSecret);
            return false;
        };

        let mut form: Vec<(&str, &str)> = vec![("secret", secret), ("response", tok)];
        if let Some(ip) = remote_ip.map(str::trim).filter(|ip| !ip.is_empty()) {
            form.push(("remoteip", ip));
        }
        let body = encode_form_urlencoded(&form);

        let headers = Headers::new();
        if headers
            .set("Content-Type", "application/x-www-form-urlencoded")
            .is_err()
        {
            emit_captcha_failure_log(CaptchaFailureReason::Transport);
            return false;
        }

        let mut init = RequestInit::new();
        init.with_method(Method::Post);
        init.with_headers(headers);
        init.with_body(Some(wasm_bindgen::JsValue::from_str(&body)));

        let Ok(req) = Request::new_with_init(TURNSTILE_SITEVERIFY_URL, &init) else {
            emit_captcha_failure_log(CaptchaFailureReason::Transport);
            return false;
        };
        let Ok(mut res) = Fetch::Request(req).send().await else {
            emit_captcha_failure_log(CaptchaFailureReason::Transport);
            return false;
        };
        if !(200..300).contains(&res.status_code()) {
            emit_captcha_failure_log(CaptchaFailureReason::HttpStatus);
            return false;
        }
        let Ok(text) = res.text().await else {
            emit_captcha_failure_log(CaptchaFailureReason::InvalidResponse);
            return false;
        };
        let Ok(parsed) = serde_json::from_str::<SiteverifyResponse>(&text) else {
            emit_captcha_failure_log(CaptchaFailureReason::InvalidResponse);
            return false;
        };
        if parsed.success {
            true
        } else {
            emit_captcha_failure_log(CaptchaFailureReason::Rejected);
            false
        }
    }
}

#[derive(Debug, Clone)]
pub enum EnvCaptchaVerifier {
    Fake(FakeCaptchaVerifier),
    Cloudflare(CloudflareCaptchaVerifier),
}

impl EnvCaptchaVerifier {
    pub fn from_env(env: &Env, app_origin: &str) -> Self {
        let mode = read_env_string(env, "TURNSTILE_MODE");
        if is_fake_turnstile_enabled(mode.as_deref(), app_origin) {
            Self::Fake(FakeCaptchaVerifier)
        } else {
            let secret = read_env_string(env, "TURNSTILE_SECRET");
            Self::Cloudflare(CloudflareCaptchaVerifier { secret })
        }
    }
}

impl CaptchaVerifier for EnvCaptchaVerifier {
    async fn verify(&self, token: Option<&str>, remote_ip: Option<&str>) -> bool {
        match self {
            Self::Fake(f) => f.verify(token, remote_ip).await,
            Self::Cloudflare(c) => c.verify(token, remote_ip).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn fake_turnstile_requires_local_origin() {
        assert!(is_fake_turnstile_enabled(
            Some("fake"),
            "http://localhost:8787"
        ));
        assert!(is_fake_turnstile_enabled(
            Some("fake"),
            "http://127.0.0.1:8787"
        ));
        assert!(!is_fake_turnstile_enabled(
            Some("fake"),
            "https://pilot-shortlink-a.workers.dev"
        ));
        assert!(!is_fake_turnstile_enabled(
            Some("fake"),
            "http://localhost.evil.test"
        ));
        assert!(!is_fake_turnstile_enabled(None, "http://localhost:8787"));
    }

    #[test]
    fn config_site_key_is_null_in_fake_mode_and_returned_in_real_mode() {
        assert_eq!(
            resolve_turnstile_site_key(Some("fake"), "http://localhost:8787", Some("0x4AAA")),
            None
        );
        assert_eq!(
            resolve_turnstile_site_key(
                Some("fake"),
                "https://pilot-shortlink-a.workers.dev",
                Some("0x4AAA")
            ),
            Some("0x4AAA".to_string())
        );
        assert_eq!(
            resolve_turnstile_site_key(None, "https://pilot-shortlink-a.workers.dev", Some("")),
            None
        );
        assert_eq!(
            resolve_turnstile_site_key(None, "https://pilot-shortlink-a.workers.dev", None),
            None
        );
    }

    #[test]
    fn fake_verifier_accepts_only_exact_fake_pass_token() {
        let verifier = FakeCaptchaVerifier;
        assert!(block_on_ready(verifier.verify(Some("fake-pass"), None)));
        assert!(!block_on_ready(verifier.verify(None, None)));
        assert!(!block_on_ready(verifier.verify(Some(""), None)));
        assert!(!block_on_ready(verifier.verify(Some("nope"), None)));
        assert!(!block_on_ready(verifier.verify(Some("fake-pass "), None)));
    }

    #[test]
    fn cloudflare_verifier_fails_closed_when_secret_or_token_missing_and_redacts_debug() {
        let missing_secret = CloudflareCaptchaVerifier { secret: None };
        assert!(!block_on_ready(
            missing_secret.verify(Some("some-token"), Some("1.2.3.4"))
        ));

        let with_secret = CloudflareCaptchaVerifier {
            secret: Some("super-secret-turnstile-key".to_string()),
        };
        assert!(!block_on_ready(with_secret.verify(None, None)));
        assert!(!block_on_ready(with_secret.verify(Some(""), None)));
        let dbg = format!("{with_secret:?}");
        assert!(!dbg.contains("super-secret-turnstile-key"));
        assert!(dbg.contains("<redacted>"));
    }

    #[test]
    fn captcha_failure_log_contains_only_event_and_reason() {
        let line = format_captcha_failure_log(CaptchaFailureReason::Rejected);
        assert_eq!(line, r#"{"event":"captcha_failed","reason":"rejected"}"#);
    }
}
