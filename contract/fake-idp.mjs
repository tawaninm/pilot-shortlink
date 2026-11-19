// Fake OAuth provider for contract and e2e tests (docs/test-plan.md section 9). Node built-ins only.
// Backends run with OAUTH_MODE=fake and OAUTH_FAKE_BASE=http://127.0.0.1:9411 and map:
//   Google authorize  /google/authorize              token  /google/token                 identity  /google/userinfo
//   GitHub authorize  /github/login/oauth/authorize   token  /github/login/oauth/access_token   identity  /github/user, /github/user/emails
// In fake mode the backends use client id "fake-client" and secret "fake-secret".
// The identity returned for a login comes from `fake_user` (base64url JSON) on the authorize URL, or from the HTML form
// this server shows when `fake_user` is missing (e2e fills it in). Shapes:
//   google: { sub, email, email_verified }        github: { id, emails: [{ email, primary, verified }] }
//   either: { error: "access_denied" } -> redirects back with ?error=   { token_error: true } -> token endpoint 500
// It enforces PKCE S256, the exact redirect_uri, single-use codes and the client secret, so a backend that skips any of
// them fails the login instead of passing silently.
import { createHash, randomBytes } from "node:crypto";
import { createServer } from "node:http";

const PORT = Number(process.env.FAKE_IDP_PORT ?? process.argv[2] ?? 9411);
const CLIENT_ID = "fake-client";
const CLIENT_KEY = "fake-secret";
const codes = new Map(); // code -> { provider, challenge, redirectUri, identity, used }
const tokens = new Map(); // access token -> { provider, identity }

const b64url = (buf) => buf.toString("base64url");
const s256 = (verifier) => b64url(createHash("sha256").update(verifier).digest());
const send = (res, status, body, headers = {}) => {
  res.writeHead(status, { "Content-Type": "application/json", ...headers });
  res.end(typeof body === "string" ? body : JSON.stringify(body));
};
const redirect = (res, url) => {
  res.writeHead(302, { Location: url });
  res.end();
};
const readBody = (req) =>
  new Promise((resolve) => {
    let raw = "";
    req.on("data", (c) => (raw += c));
    req.on("end", () => resolve(new URLSearchParams(raw)));
  });

function authorize(provider, url, res) {
  const q = url.searchParams;
  const redirectUri = q.get("redirect_uri");
  const problems = [];
  if (q.get("response_type") !== "code") problems.push("response_type must be code");
  if (q.get("client_id") !== CLIENT_ID) problems.push("unknown client_id");
  if (!redirectUri) problems.push("missing redirect_uri");
  if (q.get("code_challenge_method") !== "S256") problems.push("code_challenge_method must be S256");
  if (!/^[A-Za-z0-9_-]{43}$/.test(q.get("code_challenge") ?? "")) problems.push("code_challenge must be 43 base64url chars");
  if (!q.get("state")) problems.push("missing state");
  const scope = (q.get("scope") ?? "").split(/[ +]/);
  if (provider === "google" && !(scope.includes("openid") && scope.includes("email"))) problems.push("scope needs openid email");
  if (provider === "github" && !scope.includes("user:email")) problems.push("scope needs user:email");
  if (problems.length) return send(res, 400, { error: "invalid_request", problems });

  let identity;
  if (q.get("fake_user")) {
    identity = JSON.parse(Buffer.from(q.get("fake_user"), "base64url").toString("utf8"));
  } else if (q.get("fake_email")) {
    const email = q.get("fake_email");
    const verified = q.get("fake_verified") !== "no";
    identity = provider === "google"
      ? { sub: `g-${email}`, email, email_verified: verified }
      : { id: Number.parseInt(createHash("sha256").update(email).digest("hex").slice(0, 8), 16), emails: [{ email, primary: true, verified }] };
  } else {
    // Consent page for the browser (e2e): same URL plus fake_email
    const hidden = [...q].map(([k, v]) => `<input type="hidden" name="${k}" value="${v.replace(/"/g, "&quot;")}">`).join("");
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    return res.end(`<!doctype html><html lang="en"><title>Fake ${provider}</title><main><h1>Fake ${provider} sign-in</h1>
<form method="get">${hidden}<label>Email <input name="fake_email" type="email" required></label>
<label>Verified <select name="fake_verified"><option value="yes">yes</option><option value="no">no</option></select></label>
<button type="submit">Continue</button></form></main></html>`);
  }

  const back = new URL(redirectUri);
  back.searchParams.set("state", q.get("state"));
  if (identity.error) {
    back.searchParams.set("error", identity.error);
    return redirect(res, back.toString());
  }
  const code = b64url(randomBytes(24));
  codes.set(code, { provider, challenge: q.get("code_challenge"), redirectUri, identity, used: false });
  back.searchParams.set("code", code);
  return redirect(res, back.toString());
}

async function token(provider, req, res) {
  const form = await readBody(req);
  const entry = codes.get(form.get("code") ?? "");
  const fail = (why) => send(res, 400, { error: "invalid_grant", error_description: why });
  if (form.get("grant_type") !== "authorization_code") return fail("grant_type");
  if (form.get("client_id") !== CLIENT_ID || form.get("client_secret") !== CLIENT_KEY) return send(res, 401, { error: "invalid_client" });
  if (!entry || entry.provider !== provider) return fail("unknown code");
  if (entry.used) return fail("code already used");
  entry.used = true;
  if (form.get("redirect_uri") !== entry.redirectUri) return fail("redirect_uri mismatch");
  if (s256(form.get("code_verifier") ?? "") !== entry.challenge) return fail("code_verifier does not match code_challenge");
  if (entry.identity.token_error) return send(res, 500, { error: "server_error" });
  const accessToken = b64url(randomBytes(24));
  tokens.set(accessToken, { provider, identity: entry.identity });
  return send(res, 200, { access_token: accessToken, token_type: "Bearer", expires_in: 3600, scope: "openid email" });
}

function identityFor(provider, req, res) {
  const auth = req.headers.authorization ?? "";
  const t = tokens.get(auth.replace(/^(Bearer|token)\s+/i, ""));
  if (!t || t.provider !== provider) {
    send(res, 401, { error: "invalid_token" });
    return null;
  }
  return t.identity;
}

const server = createServer(async (req, res) => {
  const url = new URL(req.url, `http://127.0.0.1:${PORT}`);
  const path = url.pathname;
  try {
    if (path === "/__health" || path === "/health") return send(res, 200, { status: "ok" });
    if (req.method === "GET" && path === "/google/authorize") return authorize("google", url, res);
    if (req.method === "GET" && path === "/github/login/oauth/authorize") return authorize("github", url, res);
    if (req.method === "POST" && path === "/google/token") return await token("google", req, res);
    if (req.method === "POST" && path === "/github/login/oauth/access_token") return await token("github", req, res);
    if (req.method === "GET" && path === "/google/userinfo") {
      const id = identityFor("google", req, res);
      if (id) send(res, 200, { sub: id.sub, email: id.email, email_verified: id.email_verified });
      return;
    }
    if (req.method === "GET" && path === "/github/user") {
      const id = identityFor("github", req, res);
      if (id) send(res, 200, { id: id.id, login: `user${id.id}`, email: null });
      return;
    }
    if (req.method === "GET" && path === "/github/user/emails") {
      const id = identityFor("github", req, res);
      if (id) send(res, 200, (id.emails ?? []).map((e) => ({ visibility: null, ...e })));
      return;
    }
    send(res, 404, { error: "not_found", path });
  } catch (err) {
    send(res, 500, { error: "fake_idp_crash", message: String(err) });
  }
});

server.listen(PORT, "127.0.0.1", () => console.log(`fake-idp listening on http://127.0.0.1:${PORT}`));
