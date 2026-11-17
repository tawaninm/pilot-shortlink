//! ADR-002 spike: the hash runs inside the Worker's own Wasm (built ahead of time), which is how backend A will run.
//! CPU time is read from `wrangler tail` (cpuTime) and Error 1102, because Workers freeze the clock during execution.
//!   GET /noop
//!   GET /argon2?m=19456&t=2     Argon2id, m in KiB (OWASP minimum: m=19456, t=2, p=1)
//!   GET /pbkdf2?it=100000       PBKDF2-HMAC-SHA256 in Rust (Wasm), for comparison with WebCrypto in the JS spike
use argon2::{Algorithm, Argon2, Params, Version};
use worker::*;

const PASSWORD: &[u8] = b"correct horse battery staple";
const SALT: &[u8] = b"fixed-spike-salt"; // spike only: a fixed salt does not change the CPU cost

fn param(url: &Url, key: &str, default: u32) -> u32 {
    url.query_pairs()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(default)
}

#[event(fetch)]
async fn fetch(req: Request, _env: Env, _ctx: Context) -> Result<Response> {
    let url = req.url()?;
    match url.path() {
        "/noop" => Response::ok("ok noop\n"),
        "/argon2" => {
            let (m, t) = (param(&url, "m", 19_456), param(&url, "t", 2));
            let params = Params::new(m, t, 1, Some(32)).map_err(|e| Error::RustError(e.to_string()))?;
            let mut out = [0u8; 32];
            Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
                .hash_password_into(PASSWORD, SALT, &mut out)
                .map_err(|e| Error::RustError(e.to_string()))?;
            Response::ok(format!("ok argon2id m={m} t={t}\n"))
        }
        "/pbkdf2" => {
            let it = param(&url, "it", 100_000);
            let mut out = [0u8; 32];
            pbkdf2::pbkdf2_hmac::<sha2::Sha256>(PASSWORD, SALT, it, &mut out);
            Response::ok(format!("ok pbkdf2-rust it={it}\n"))
        }
        _ => Response::error("not found\n", 404),
    }
}
