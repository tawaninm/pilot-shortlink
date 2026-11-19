use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use worker::*;

use crate::http::json_response;

pub const DEFAULT_MAIL_FROM: &str = "pilot-shortlink <onboarding@resend.dev>";
pub const RESEND_API_URL: &str = "https://api.resend.com/emails";

pub const VERIFY_EMAIL_SUBJECT: &str = "ยืนยันอีเมลสำหรับ pilot-shortlink";
pub const RESET_PASSWORD_SUBJECT: &str = "ตั้งรหัสผ่านใหม่สำหรับ pilot-shortlink";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenPurpose {
    Verify,
    Reset,
}

impl TokenPurpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Verify => "verify",
            Self::Reset => "reset",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapturedMail {
    pub to: String,
    pub subject: String,
    pub text: String,
    pub html: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapturedMailListResponse {
    pub items: Vec<CapturedMail>,
}

static CAPTURED_MAILS: Mutex<Vec<CapturedMail>> = Mutex::new(Vec::new());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailSendError {
    MissingApiKey,
    HttpStatus(u16),
    Transport,
}

#[allow(async_fn_in_trait)]
pub trait Mailer {
    async fn send(
        &self,
        to: &str,
        subject: &str,
        text: &str,
        html: &str,
        idempotency_key: &str,
    ) -> std::result::Result<(), MailSendError>;
}

#[derive(Debug, Clone, Default)]
pub struct CaptureMailer;

impl Mailer for CaptureMailer {
    async fn send(
        &self,
        to: &str,
        subject: &str,
        text: &str,
        html: &str,
        _idempotency_key: &str,
    ) -> std::result::Result<(), MailSendError> {
        let mut guard = CAPTURED_MAILS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.push(CapturedMail {
            to: to.to_string(),
            subject: subject.to_string(),
            text: text.to_string(),
            html: html.to_string(),
        });
        Ok(())
    }
}

#[derive(Clone)]
pub struct ResendMailer {
    pub api_key: Option<String>,
    pub from: String,
}

// Manual Debug so a `{:?}` of the mailer never prints the Resend key into Workers logs.
impl std::fmt::Debug for ResendMailer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResendMailer")
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("from", &self.from)
            .finish()
    }
}

#[derive(Serialize)]
struct ResendPayload<'a> {
    from: &'a str,
    to: [&'a str; 1],
    subject: &'a str,
    text: &'a str,
    html: &'a str,
}

impl Mailer for ResendMailer {
    async fn send(
        &self,
        to: &str,
        subject: &str,
        text: &str,
        html: &str,
        idempotency_key: &str,
    ) -> std::result::Result<(), MailSendError> {
        let Some(api_key) = self.api_key.as_deref().filter(|k| !k.trim().is_empty()) else {
            return Err(MailSendError::MissingApiKey);
        };

        let body_json = serde_json::to_string(&ResendPayload {
            from: &self.from,
            to: [to],
            subject,
            text,
            html,
        })
        .map_err(|_| MailSendError::Transport)?;

        let headers = Headers::new();
        headers
            .set("Authorization", &format!("Bearer {api_key}"))
            .map_err(|_| MailSendError::Transport)?;
        headers
            .set("Content-Type", "application/json")
            .map_err(|_| MailSendError::Transport)?;
        headers
            .set("Idempotency-Key", idempotency_key)
            .map_err(|_| MailSendError::Transport)?;

        let mut init = RequestInit::new();
        init.with_method(Method::Post);
        init.with_headers(headers);
        init.with_body(Some(wasm_bindgen::JsValue::from_str(&body_json)));

        let req =
            Request::new_with_init(RESEND_API_URL, &init).map_err(|_| MailSendError::Transport)?;
        let res = Fetch::Request(req)
            .send()
            .await
            .map_err(|_| MailSendError::Transport)?;

        let status = res.status_code();
        if (200..300).contains(&status) {
            Ok(())
        } else {
            Err(MailSendError::HttpStatus(status))
        }
    }
}

#[derive(Debug, Clone)]
pub enum EnvMailer {
    Capture(CaptureMailer),
    Resend(ResendMailer),
}

impl EnvMailer {
    pub fn from_env(env: &Env) -> Self {
        if is_capture_env(env) {
            Self::Capture(CaptureMailer)
        } else {
            let api_key = env
                .secret("RESEND_API_KEY")
                .map(|s| s.to_string())
                .ok()
                .filter(|k| !k.trim().is_empty());
            let from = env
                .var("MAIL_FROM")
                .map(|v| v.to_string())
                .ok()
                .filter(|f| !f.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_MAIL_FROM.to_string());
            Self::Resend(ResendMailer { api_key, from })
        }
    }
}

impl Mailer for EnvMailer {
    async fn send(
        &self,
        to: &str,
        subject: &str,
        text: &str,
        html: &str,
        idempotency_key: &str,
    ) -> std::result::Result<(), MailSendError> {
        match self {
            Self::Capture(c) => c.send(to, subject, text, html, idempotency_key).await,
            Self::Resend(r) => r.send(to, subject, text, html, idempotency_key).await,
        }
    }
}

/// Returns true only when `MAIL_MODE` is `"capture"`.
pub fn is_capture_mode(mail_mode: Option<&str>) -> bool {
    mail_mode == Some("capture")
}

/// Checks whether the worker `Env` has `MAIL_MODE=capture` AND a local `APP_ORIGIN`.
/// Capture exposes every mailed token, so a stray `MAIL_MODE` on a deployed worker must not enable it.
pub fn is_capture_env(env: &Env) -> bool {
    let mode = env.var("MAIL_MODE").map(|v| v.to_string()).ok();
    let origin = env
        .var("APP_ORIGIN")
        .map(|v| v.to_string())
        .unwrap_or_default();
    is_capture_mode(mode.as_deref()) && is_local_origin(&origin)
}

/// True for `http://localhost[:port]` and `http://127.0.0.1[:port]` only.
pub fn is_local_origin(origin: &str) -> bool {
    ["http://localhost", "http://127.0.0.1"].iter().any(|base| {
        origin
            .strip_prefix(base)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(':'))
    })
}

/// Escapes HTML special characters in a URL or text segment.
pub fn escape_html(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltMail {
    pub subject: &'static str,
    pub link: String,
    pub text: String,
    pub html: String,
}

/// Builds the email subject, text, and HTML for `purpose` using `app_origin` from config
/// (never from `Host`, `X-Forwarded-Host`, or `Forwarded`).
pub fn build_auth_mail(purpose: TokenPurpose, app_origin: &str, token: &str) -> BuiltMail {
    let base = app_origin.trim_end_matches('/');
    let (subject, path, action_th) = match purpose {
        TokenPurpose::Verify => (VERIFY_EMAIL_SUBJECT, "verify-email", "ยืนยันอีเมล"),
        TokenPurpose::Reset => (RESET_PASSWORD_SUBJECT, "reset-password", "ตั้งรหัสผ่านใหม่"),
    };
    let link = format!("{base}/{path}?token={token}");
    let escaped_link = escape_html(&link);
    let text = format!("{subject}\n\nเปิดลิงก์นี้เพื่อ{action_th} (ลิงก์ใช้ได้ 30 นาที):\n{link}\n");
    let html = format!(
        "<p>{subject}</p><p>เปิดลิงก์นี้เพื่อ{action_th} (ลิงก์ใช้ได้ 30 นาที): <a href=\"{escaped_link}\">{escaped_link}</a></p>"
    );
    BuiltMail {
        subject,
        link,
        text,
        html,
    }
}

/// Formats the Resend `Idempotency-Key` header value: `"<purpose>/<first 16 hex of token_hash>"`.
pub fn format_idempotency_key(purpose: TokenPurpose, token_hash: &str) -> String {
    let prefix_len = token_hash.len().min(16);
    format!("{}/{}", purpose.as_str(), &token_hash[..prefix_len])
}

#[derive(Serialize)]
struct MailLogLine<'a> {
    event: &'static str,
    purpose: &'a str,
    result: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'static str>,
}

/// Formats a single-line structured JSON mail log entry for a successful send.
/// Contains `purpose` only; never contains token, mail link, email address, or password.
pub fn format_mail_log_ok(purpose: TokenPurpose) -> String {
    serde_json::to_string(&MailLogLine {
        event: "mail_send",
        purpose: purpose.as_str(),
        result: "ok",
        status: None,
        error: None,
    })
    .expect("MailLogLine serialization cannot fail")
}

/// Formats a single-line structured JSON mail log entry for a failed send.
/// Contains `purpose` and status/reason only; never contains token, mail link, email address, or password.
pub fn format_mail_log_err(purpose: TokenPurpose, err: MailSendError) -> String {
    let (status, error) = match err {
        MailSendError::HttpStatus(code) => (Some(code), None),
        MailSendError::MissingApiKey => (None, Some("missing_api_key")),
        MailSendError::Transport => (None, Some("transport_error")),
    };
    serde_json::to_string(&MailLogLine {
        event: "mail_send",
        purpose: purpose.as_str(),
        result: "failed",
        status,
        error,
    })
    .expect("MailLogLine serialization cannot fail")
}

fn emit_mail_log(is_error: bool, line: &str) {
    #[cfg(target_arch = "wasm32")]
    if is_error {
        console_error!("{line}");
    } else {
        console_log!("{line}");
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (is_error, line);
}

/// Sends an auth email via `mailer` and logs one structured line with the purpose and result.
/// Never fails the caller on mailer errors.
pub async fn send_purpose_mail<M: Mailer>(
    mailer: &M,
    purpose: TokenPurpose,
    to: &str,
    app_origin: &str,
    token: &str,
    token_hash: &str,
) -> String {
    let built = build_auth_mail(purpose, app_origin, token);
    let idempotency_key = format_idempotency_key(purpose, token_hash);
    match mailer
        .send(
            to,
            built.subject,
            &built.text,
            &built.html,
            &idempotency_key,
        )
        .await
    {
        Ok(()) => {
            let line = format_mail_log_ok(purpose);
            emit_mail_log(false, &line);
            line
        }
        Err(err) => {
            let line = format_mail_log_err(purpose, err);
            emit_mail_log(true, &line);
            line
        }
    }
}

/// Dispatches `send_purpose_mail` in the background using `ctx.wait_until`.
pub fn spawn_purpose_mail(
    ctx: &Context,
    mailer: EnvMailer,
    purpose: TokenPurpose,
    to: String,
    app_origin: String,
    token: String,
    token_hash: String,
) {
    ctx.wait_until(async move {
        send_purpose_mail(&mailer, purpose, &to, &app_origin, &token, &token_hash).await;
    });
}

/// Returns captured mails filtered by recipient `to` (oldest first) ONLY when `mail_mode == Some("capture")`.
/// Returns `None` when `MAIL_MODE` is unset or not `"capture"` (AC-V8).
pub fn query_captured_mail_for_mode(
    mail_mode: Option<&str>,
    to: Option<&str>,
) -> Option<CapturedMailListResponse> {
    if !is_capture_mode(mail_mode) {
        return None;
    }
    let target = to.map(|s| s.trim().to_lowercase()).unwrap_or_default();
    let guard = CAPTURED_MAILS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let items = guard
        .iter()
        .filter(|m| !target.is_empty() && m.to.eq_ignore_ascii_case(&target))
        .cloned()
        .collect();
    Some(CapturedMailListResponse { items })
}

/// `GET /api/test/mail?to=<email>` (only reachable when `MAIL_MODE == "capture"`).
pub fn handle_get_captured_mail(req: &Request, env: &Env) -> Result<Option<Response>> {
    let mode = is_capture_env(env).then(|| "capture".to_string());
    let url = req.url()?;
    let to_param = url
        .query_pairs()
        .find(|(k, _)| k == "to")
        .map(|(_, v)| v.into_owned());

    let Some(list) = query_captured_mail_for_mode(mode.as_deref(), to_param.as_deref()) else {
        return Ok(None);
    };

    let body = serde_json::to_string(&list)?;
    Ok(Some(json_response(200, "application/json", body)?))
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
        // SAFETY: `fut` is pinned on stack and not moved before dropping.
        let pinned = unsafe { Pin::new_unchecked(&mut fut) };
        match pinned.poll(&mut ctx) {
            Poll::Ready(val) => val,
            Poll::Pending => panic!("test future must resolve immediately without async I/O"),
        }
    }

    #[derive(Clone, Copy)]
    pub struct FailingMailer(pub MailSendError);

    impl Mailer for FailingMailer {
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
    fn ac_v8_capture_route_is_absent_when_mail_mode_is_unset_or_not_capture() {
        assert!(!is_capture_mode(None));
        assert!(!is_capture_mode(Some("")));
        assert!(!is_capture_mode(Some("resend")));
        assert!(!is_capture_mode(Some("production")));
        assert!(!is_capture_mode(Some("CAPTURE")));
        assert!(is_capture_mode(Some("capture")));

        assert_eq!(
            query_captured_mail_for_mode(None, Some("alice@example.test")),
            None,
            "AC-V8: /api/test/mail must be absent when MAIL_MODE is unset"
        );
        assert_eq!(
            query_captured_mail_for_mode(Some(""), Some("alice@example.test")),
            None
        );
        assert_eq!(
            query_captured_mail_for_mode(Some("resend"), Some("alice@example.test")),
            None
        );
    }

    #[test]
    fn capture_mailer_stores_oldest_first_and_filters_by_recipient() {
        let email = "capture-unit-test@example.test";
        let token1 = "A".repeat(43);
        let hash1 = "1".repeat(64);
        let token2 = "B".repeat(43);
        let hash2 = "2".repeat(64);

        block_on_ready(send_purpose_mail(
            &CaptureMailer,
            TokenPurpose::Verify,
            email,
            "http://localhost:8787",
            &token1,
            &hash1,
        ));
        block_on_ready(send_purpose_mail(
            &CaptureMailer,
            TokenPurpose::Reset,
            email,
            "http://localhost:8787",
            &token2,
            &hash2,
        ));

        let captured = query_captured_mail_for_mode(Some("capture"), Some(email))
            .expect("capture mode enabled");
        assert_eq!(captured.items.len(), 2);
        assert_eq!(captured.items[0].subject, VERIFY_EMAIL_SUBJECT);
        assert!(captured.items[0].text.contains(&format!(
            "http://localhost:8787/verify-email?token={token1}"
        )));
        assert!(captured.items[0].text.contains("30 นาที"));
        assert_eq!(captured.items[1].subject, RESET_PASSWORD_SUBJECT);
        assert!(captured.items[1].text.contains(&format!(
            "http://localhost:8787/reset-password?token={token2}"
        )));
    }

    #[test]
    fn capture_requires_a_local_app_origin() {
        assert!(is_local_origin("http://localhost:8787"));
        assert!(is_local_origin("http://127.0.0.1"));
        assert!(!is_local_origin(
            "https://pilot-shortlink-a.example.workers.dev"
        ));
        assert!(!is_local_origin("http://localhost.evil.test"));
        assert!(!is_local_origin("http://localhostx:8787"));
    }

    #[test]
    fn mail_log_lines_contain_purpose_only_and_never_contain_token_link_or_email() {
        let secret_token = format!("secretToken_{}", "x".repeat(31));
        assert_eq!(secret_token.len(), 43);
        let token_hash = "deadbeefcafebabe0123456789abcdefdeadbeefcafebabe0123456789abcdef";
        let email = "private.user@example.test";
        let app_origin = "https://pilot-shortlink.example.test";

        let ok_line = block_on_ready(send_purpose_mail(
            &CaptureMailer,
            TokenPurpose::Verify,
            email,
            app_origin,
            &secret_token,
            token_hash,
        ));
        let err_500_line = block_on_ready(send_purpose_mail(
            &FailingMailer(MailSendError::HttpStatus(500)),
            TokenPurpose::Verify,
            email,
            app_origin,
            &secret_token,
            token_hash,
        ));
        let err_key_line = block_on_ready(send_purpose_mail(
            &FailingMailer(MailSendError::MissingApiKey),
            TokenPurpose::Reset,
            email,
            app_origin,
            &secret_token,
            token_hash,
        ));

        for line in [&ok_line, &err_500_line, &err_key_line] {
            assert!(
                !line.contains(&secret_token),
                "log line leaked raw token: {line}"
            );
            assert!(
                !line.contains(token_hash),
                "log line leaked token_hash: {line}"
            );
            assert!(
                !line.contains(&token_hash[..16]),
                "log line leaked idempotency key prefix: {line}"
            );
            assert!(!line.contains(email), "log line leaked email: {line}");
            assert!(
                !line.contains(app_origin),
                "log line leaked mail link origin: {line}"
            );
            assert!(
                !line.contains("verify-email?token="),
                "log line leaked mail link: {line}"
            );
        }

        assert_eq!(
            ok_line,
            r#"{"event":"mail_send","purpose":"verify","result":"ok"}"#
        );
        assert_eq!(
            err_500_line,
            r#"{"event":"mail_send","purpose":"verify","result":"failed","status":500}"#
        );
        assert_eq!(
            err_key_line,
            r#"{"event":"mail_send","purpose":"reset","result":"failed","error":"missing_api_key"}"#
        );
    }

    #[test]
    fn idempotency_key_and_html_escaping_follow_spec() {
        let hash = "0123456789abcdef0000111122223333444455556666777788889999aaaabbbb";
        assert_eq!(
            format_idempotency_key(TokenPurpose::Verify, hash),
            "verify/0123456789abcdef"
        );
        assert_eq!(
            format_idempotency_key(TokenPurpose::Reset, hash),
            "reset/0123456789abcdef"
        );

        let built = build_auth_mail(
            TokenPurpose::Verify,
            "http://localhost:8787/",
            "AbCdEfGhIjKlMnOpQrStUvWxYz0123456789-_AbCdE",
        );
        assert_eq!(
            built.link,
            "http://localhost:8787/verify-email?token=AbCdEfGhIjKlMnOpQrStUvWxYz0123456789-_AbCdE"
        );
        assert!(built.html.contains(&built.link));
        assert_eq!(
            escape_html("https://example.com/?a=1&b=\"2\"&c=<3>"),
            "https://example.com/?a=1&amp;b=&quot;2&quot;&amp;c=&lt;3&gt;"
        );
    }
}
