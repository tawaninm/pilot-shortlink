// OAuth login with Google and GitHub through the fake IdP (requirements AC-O1..O11, test-plan section 9).
// State expiry (10 min) is covered by each backend's unit tests because a contract test cannot wait that long.
import { describe, expect, it } from "vitest";
import {
  anon, BASE_URL, CAPTCHA, fakeAuthorize, githubUser, googleUser, oauthCallback, oauthLogin, oauthStart, ORIGIN, PASSWORD,
  registerUnverified, registerUser, tokenFrom, uniqueEmail, waitForMail, withCookie,
} from "../src/client";

const ERROR = "/login?error=oauth";
const me = async (session?: string) => {
  if (!session) throw new Error("no session cookie");
  return (await withCookie(session).GET("/api/me")).data;
};

describe("oauth", () => {
  it("AC-O1: first Google login with a new email creates a verified account and a session", async () => {
    const user = googleUser();
    const { res, location, session } = await oauthLogin("google", user);
    expect(res.status).toBe(302);
    expect(location).toBe("/");
    expect(await me(session)).toEqual({ email: user.email, emailVerified: true });
  });

  it("AC-O2: GitHub uses only the primary verified email", async () => {
    const primary = uniqueEmail();
    const user = {
      id: Date.now(),
      emails: [
        { email: uniqueEmail(), primary: false, verified: true },
        { email: primary, primary: true, verified: true },
        { email: uniqueEmail(), primary: false, verified: false },
      ],
    };
    const { location, session } = await oauthLogin("github", user);
    expect(location).toBe("/");
    expect((await me(session))?.email).toBe(primary);
  });

  it("AC-O3: a verified provider email that matches a password account logs into that account", async () => {
    const owner = await registerUser();
    const created = await owner.api.POST("/api/links", { body: { url: "https://example.com/oauth-link" } });
    expect(created.response.status).toBe(201);
    for (const provider of ["google", "github"] as const) {
      const user = provider === "google" ? googleUser(owner.email) : githubUser(owner.email);
      const { location, session } = await oauthLogin(provider, user);
      expect(location).toBe("/");
      if (!session) throw new Error("no session");
      const links = (await withCookie(session).GET("/api/links")).data;
      expect(links?.items.map((l) => l.code)).toContain(created.data?.code);
    }
  });

  it("AC-O12: linking to an unverified password account wipes the squatter's password and sessions", async () => {
    const email = uniqueEmail();
    const squatter = await registerUnverified(email); // someone else registered our email first
    const { location, session } = await oauthLogin("google", googleUser(email));
    expect(location).toBe("/");
    expect((await me(session))?.email).toBe(email);
    expect((await squatter.api.GET("/api/me")).response.status).toBe(401);
    const login = await anon().POST("/api/auth/login", { body: { email, password: PASSWORD } });
    expect(login.response.status).toBe(401);
  });

  it("AC-O12: linking to a verified password account keeps its password", async () => {
    const owner = await registerUser();
    expect((await oauthLogin("github", githubUser(owner.email))).location).toBe("/");
    const login = await anon().POST("/api/auth/login", { body: { email: owner.email, password: PASSWORD } });
    expect(login.response.status).toBe(200);
  });

  it("AC-O4: an unverified provider email neither links nor creates an account", async () => {
    const owner = await registerUser();
    const google = await oauthLogin("google", googleUser(owner.email, false));
    expect(google.location).toBe(ERROR);
    expect(google.session).toBeUndefined();

    const fresh = uniqueEmail();
    const github = await oauthLogin("github", { id: Date.now(), emails: [{ email: fresh, primary: true, verified: false }] });
    expect(github.location).toBe(ERROR);
    expect(github.session).toBeUndefined();
    // no account was created for `fresh`, so it can still register
    const reg = await anon().POST("/api/auth/register", { body: { email: fresh, password: PASSWORD, captcha: CAPTCHA } });
    expect(reg.response.status).toBe(201);
  });

  it("AC-O5: a wrong, missing or reused state is rejected", async () => {
    const a = await oauthStart("google");
    const b = await oauthStart("google");
    const callbackA = await fakeAuthorize(a.authorizeUrl, googleUser());

    const wrongCookie = await oauthCallback(callbackA, b.stateCookie);
    expect(wrongCookie.location).toBe(ERROR);
    expect(wrongCookie.session).toBeUndefined();

    const callbackB = await fakeAuthorize(b.authorizeUrl, googleUser());
    const noCookie = await oauthCallback(callbackB);
    expect(noCookie.location).toBe(ERROR);
    expect(noCookie.session).toBeUndefined();

    const c = await oauthStart("google");
    const callbackC = await fakeAuthorize(c.authorizeUrl, googleUser());
    expect((await oauthCallback(callbackC, c.stateCookie)).location).toBe("/");
    const replay = await oauthCallback(callbackC, c.stateCookie);
    expect(replay.location).toBe(ERROR);
    expect(replay.session).toBeUndefined();
  });

  it("AC-O6: the authorize URL uses PKCE S256, the exact redirect_uri and the state cookie", async () => {
    for (const provider of ["google", "github"] as const) {
      const { res, authorizeUrl, stateCookie } = await oauthStart(provider);
      expect(res.status).toBe(302);
      const q = new URL(authorizeUrl).searchParams;
      expect(q.get("response_type")).toBe("code");
      expect(q.get("code_challenge_method")).toBe("S256");
      expect(q.get("code_challenge")).toMatch(/^[A-Za-z0-9_-]{43}$/);
      expect(q.get("redirect_uri")).toBe(`${ORIGIN}/api/auth/oauth/${provider}/callback`);
      expect(q.get("state")?.length).toBeGreaterThanOrEqual(32);
      expect(stateCookie).toBe(`__Host-oauth=${q.get("state")}`);
      const raw = res.headers.getSetCookie().find((c) => c.startsWith("__Host-oauth="));
      expect(raw).toMatch(/HttpOnly/i);
      expect(raw).toMatch(/Secure/i);
      expect(raw).toMatch(/SameSite=Lax/i);
      expect(raw).toMatch(/Path=\//);
    }
    // The fake IdP refuses the token request unless code_verifier matches code_challenge, so a login that works
    // is the proof that the verifier is sent correctly.
    expect((await oauthLogin("google", googleUser())).location).toBe("/");
  });

  it("AC-O7: the callback never follows a destination from the query", async () => {
    const start = await oauthStart("google");
    const callback = new URL(await fakeAuthorize(start.authorizeUrl, googleUser()));
    callback.searchParams.set("return_to", "https://evil.example/");
    callback.searchParams.set("next", "//evil.example");
    const { location, session } = await oauthCallback(callback.toString(), start.stateCookie);
    expect(location).toBe("/");
    expect(session).toBeDefined();
  });

  it("AC-O8: an OAuth-only account has no usable password until it sets one through reset", async () => {
    const user = googleUser();
    expect((await oauthLogin("google", user)).location).toBe("/");
    for (const password of [PASSWORD, "!oauth", ""]) {
      const res = await fetch(`${BASE_URL}/api/auth/login`, {
        method: "POST", headers: { "Content-Type": "application/json", Origin: ORIGIN }, body: JSON.stringify({ email: user.email, password }),
      });
      expect([401, 422]).toContain(res.status);
    }
    const loginWith = (password: string) => anon().POST("/api/auth/login", { body: { email: user.email, password } });
    expect((await loginWith(PASSWORD)).response.status).toBe(401);

    expect((await anon().POST("/api/auth/password/forgot", { body: { email: user.email, captcha: CAPTCHA } })).response.status).toBe(204);
    const token = tokenFrom(await waitForMail(user.email, 1), "reset-password");
    expect((await anon().POST("/api/auth/password/reset", { body: { token, password: "a-brand-new-passphrase" } })).response.status).toBe(204);
    expect((await loginWith("a-brand-new-passphrase")).response.status).toBe(200);
  });

  it("AC-O9: the same provider subject reaches the same account even if its email changes", async () => {
    const first = googleUser();
    const one = await oauthLogin("google", first);
    const two = await oauthLogin("google", { ...first, email: uniqueEmail() });
    expect(two.location).toBe("/");
    expect((await me(two.session))?.email).toBe(first.email);
    expect((await me(one.session))?.email).toBe(first.email);
  });

  it("AC-O10: a cancelled login or a failing provider ends at the error page, not a 500", async () => {
    const cancelled = await oauthLogin("google", { error: "access_denied" });
    expect(cancelled.res.status).toBe(302);
    expect(cancelled.location).toBe(ERROR);
    const broken = await oauthLogin("github", { token_error: true });
    expect(broken.res.status).toBe(302);
    expect(broken.location).toBe(ERROR);
    expect(broken.session).toBeUndefined();
  });

  it("AC-O11: an unknown provider returns 404", async () => {
    for (const path of ["/api/auth/oauth/twitter/start", "/api/auth/oauth/twitter/callback?code=x&state=y"]) {
      expect((await fetch(`${BASE_URL}${path}`, { redirect: "manual" })).status).toBe(404);
    }
  });

  it("concurrency: one state, two callbacks at once, exactly one session", async () => {
    const start = await oauthStart("github");
    const callback = await fakeAuthorize(start.authorizeUrl, githubUser());
    const results = await Promise.all([oauthCallback(callback, start.stateCookie), oauthCallback(callback, start.stateCookie)]);
    expect(results.filter((r) => r.location === "/" && r.session).length).toBe(1);
    expect(results.filter((r) => r.location === ERROR).length).toBe(1);
  });

  it("A01: two Google users never land in each other's account", async () => {
    const a = googleUser();
    const b = googleUser();
    const loginA = await oauthLogin("google", a);
    const loginB = await oauthLogin("google", b);
    expect((await me(loginA.session))?.email).toBe(a.email);
    expect((await me(loginB.session))?.email).toBe(b.email);
  });
});
