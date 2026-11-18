// Email verification (requirements AC-V1..V10, test-plan section 8). Token expiry (30 min) is covered by
// each backend's unit tests because a contract test cannot wait that long.
import { describe, expect, it } from "vitest";
import {
  anon, BASE_URL, mailCountAfterPause, ORIGIN, problem, registerUnverified, registerUser, tokenFrom, uniqueEmail, waitForMail,
} from "../src/client";

const TARGET = "https://example.com/target";
const verify = (token: string) => anon().POST("/api/auth/verify-email", { body: { token } });
const emailVerified = async (api: Awaited<ReturnType<typeof registerUnverified>>["api"]) => (await api.GET("/api/me")).data?.emailVerified;

describe("verify email", () => {
  it("AC-V1: register leaves the account unverified and sends one mail with a link on the app origin", async () => {
    const { email, api } = await registerUnverified();
    expect(await emailVerified(api)).toBe(false);
    const mails = await waitForMail(email, 1);
    expect(mails).toHaveLength(1);
    expect(mails[0].to).toBe(email);
    expect(mails[0].text).toContain(`${ORIGIN}/verify-email?token=`);
    expect(await mailCountAfterPause(email)).toBe(1);
  });

  it("AC-V2: a valid token returns 204 and marks the account verified", async () => {
    const { email, api } = await registerUnverified();
    const { response } = await verify(tokenFrom(await waitForMail(email, 1), "verify-email"));
    expect(response.status).toBe(204);
    expect(await emailVerified(api)).toBe(true);
  });

  it("AC-V3: a used token returns 400 token_invalid", async () => {
    const { email } = await registerUnverified();
    const token = tokenFrom(await waitForMail(email, 1), "verify-email");
    expect((await verify(token)).response.status).toBe(204);
    const again = await verify(token);
    expect(again.response.status).toBe(400);
    expect((await problem(again.response)).code).toBe("token_invalid");
  });

  it("AC-V3: an unknown well-formed token returns the same 400 token_invalid", async () => {
    const { response } = await verify("A".repeat(43));
    expect(response.status).toBe(400);
    expect((await problem(response)).code).toBe("token_invalid");
  });

  it("AC-V3: a malformed token returns 422 invalid_body", async () => {
    const res = await fetch(`${BASE_URL}/api/auth/verify-email`, {
      method: "POST", headers: { "Content-Type": "application/json", Origin: ORIGIN }, body: JSON.stringify({ token: "short" }),
    });
    expect(res.status).toBe(422);
    expect((await problem(res)).code).toBe("invalid_body");
  });

  it("AC-V4: an unverified user can read links but cannot create, edit or delete them (403 email_unverified)", async () => {
    const { api } = await registerUnverified();
    expect((await api.GET("/api/links")).response.status).toBe(200);
    const created = await api.POST("/api/links", { body: { url: TARGET } });
    const patched = await api.PATCH("/api/links/{code}", { params: { path: { code: "nope123" } }, body: { url: TARGET } });
    const deleted = await api.DELETE("/api/links/{code}", { params: { path: { code: "nope123" } } });
    for (const { response } of [created, patched, deleted]) {
      expect(response.status).toBe(403);
      expect((await problem(response)).code).toBe("email_unverified");
    }
  });

  it("AC-V5: the 4th resend within an hour returns 429 with Retry-After", async () => {
    const { api } = await registerUnverified();
    for (let i = 0; i < 3; i++) expect((await api.POST("/api/auth/verify-email/resend")).response.status).toBe(204);
    const fourth = await api.POST("/api/auth/verify-email/resend");
    expect(fourth.response.status).toBe(429);
    expect(Number(fourth.response.headers.get("retry-after"))).toBeGreaterThan(0);
    expect((await problem(fourth.response)).code).toBe("rate_limited");
  });

  it("AC-V6: a resend makes the earlier token unusable and the new one works", async () => {
    const { email, api } = await registerUnverified();
    const first = tokenFrom(await waitForMail(email, 1), "verify-email");
    expect((await api.POST("/api/auth/verify-email/resend")).response.status).toBe(204);
    const second = tokenFrom(await waitForMail(email, 2), "verify-email");
    expect(second).not.toBe(first);
    expect((await verify(first)).response.status).toBe(400);
    expect((await verify(second)).response.status).toBe(204);
  });

  it("AC-V7 / concurrency: two simultaneous verifies with one token succeed exactly once", async () => {
    const { email } = await registerUnverified();
    const token = tokenFrom(await waitForMail(email, 1), "verify-email");
    const statuses = (await Promise.all([verify(token), verify(token)])).map((r) => r.response.status).sort();
    expect(statuses).toEqual([204, 400]);
  });

  it("AC-V10: the shared registerUser helper returns a verified account that can create links", async () => {
    const { api } = await registerUser();
    expect(await emailVerified(api)).toBe(true);
    expect((await api.POST("/api/links", { body: { url: TARGET } })).response.status).toBe(201);
  });

  it("A01: a token only verifies its own account, not the session that sends it", async () => {
    const a = await registerUnverified();
    const b = await registerUnverified();
    const token = tokenFrom(await waitForMail(a.email, 1), "verify-email");
    expect((await b.api.POST("/api/auth/verify-email", { body: { token } })).response.status).toBe(204);
    expect(await emailVerified(a.api)).toBe(true);
    expect(await emailVerified(b.api)).toBe(false);
  });

  it("A01: POST verify-email without the app Origin returns 403 forbidden_origin", async () => {
    const { email } = await registerUnverified();
    const token = tokenFrom(await waitForMail(email, 1), "verify-email");
    const res = await fetch(`${BASE_URL}/api/auth/verify-email`, {
      method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ token }),
    });
    expect(res.status).toBe(403);
    expect((await problem(res)).code).toBe("forbidden_origin");
  });

  it("A06: GET on verify-email (a mail scanner opening the link) does not verify", async () => {
    const { email, api } = await registerUnverified();
    const token = tokenFrom(await waitForMail(email, 1), "verify-email");
    const res = await fetch(`${BASE_URL}/api/auth/verify-email?token=${token}`);
    expect(res.status).toBeGreaterThanOrEqual(400);
    expect(await emailVerified(api)).toBe(false);
    expect((await verify(token)).response.status).toBe(204);
  });

  it("A05: mail links use APP_ORIGIN even when the request claims another host", async () => {
    const email = uniqueEmail();
    const res = await fetch(`${BASE_URL}/api/auth/register`, {
      method: "POST",
      headers: { "Content-Type": "application/json", Origin: ORIGIN, "X-Forwarded-Host": "evil.example", Forwarded: "host=evil.example" },
      body: JSON.stringify({ email, password: "correct-horse-battery" }),
    });
    expect(res.status).toBe(201);
    const [mail] = await waitForMail(email, 1);
    expect(mail.text).not.toContain("evil.example");
    expect(mail.html).not.toContain("evil.example");
    tokenFrom([mail], "verify-email"); // throws unless the link starts with ORIGIN
  });

  it("idempotency: resend after verifying returns 204 and sends nothing", async () => {
    const { email, api } = await registerUser();
    expect((await api.POST("/api/auth/verify-email/resend")).response.status).toBe(204);
    expect(await mailCountAfterPause(email)).toBe(1);
  });

  it("resend without a session returns 401", async () => {
    const { response } = await anon().POST("/api/auth/verify-email/resend");
    expect(response.status).toBe(401);
  });
});
