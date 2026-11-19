// Turnstile on register and forgot password (requirements AC-T1..T2, test-plan section 9).
// Backends run with TURNSTILE_MODE=fake: only "fake-pass" passes. Order of checks on both endpoints:
// JSON shape (422 invalid_body) -> captcha (403 captcha_failed) -> field rules (422) -> work.
import { describe, expect, it } from "vitest";
import { anon, BASE_URL, CAPTCHA, mailCountAfterPause, ORIGIN, PASSWORD, problem, registerUser, uniqueEmail } from "../src/client";

const post = (path: string, body: unknown) =>
  fetch(`${BASE_URL}${path}`, { method: "POST", headers: { "Content-Type": "application/json", Origin: ORIGIN }, body: JSON.stringify(body) });

describe("captcha", () => {
  it("AC-T1: register without a captcha or with a wrong one returns 403 captcha_failed and creates no account", async () => {
    const email = uniqueEmail();
    for (const body of [{ email, password: PASSWORD }, { email, password: PASSWORD, captcha: "" }, { email, password: PASSWORD, captcha: "nope" }]) {
      const res = await post("/api/auth/register", body);
      expect(res.status).toBe(403);
      expect((await problem(res)).code).toBe("captcha_failed");
    }
    expect(await mailCountAfterPause(email)).toBe(0);
    const login = await anon().POST("/api/auth/login", { body: { email, password: PASSWORD } });
    expect(login.response.status).toBe(401);
  });

  it("AC-T1: forgot password without a valid captcha returns 403 and sends no mail", async () => {
    const { email } = await registerUser(); // 1 verify mail
    for (const body of [{ email }, { email, captcha: "nope" }]) {
      const res = await post("/api/auth/password/forgot", body);
      expect(res.status).toBe(403);
      expect((await problem(res)).code).toBe("captcha_failed");
    }
    expect(await mailCountAfterPause(email)).toBe(1);
  });

  it("AC-T4: failed captchas on forgot password do not use up the owner's hourly quota", async () => {
    const { email } = await registerUser(); // 1 verify mail
    for (let i = 0; i < 4; i++) expect((await post("/api/auth/password/forgot", { email, captcha: "nope" })).status).toBe(403);
    expect((await anon().POST("/api/auth/password/forgot", { body: { email, captcha: CAPTCHA } })).response.status).toBe(204);
    expect(await mailCountAfterPause(email)).toBe(2);
  });

  it("AC-T2: a valid captcha keeps register and forgot working", async () => {
    const email = uniqueEmail();
    const reg = await anon().POST("/api/auth/register", { body: { email, password: PASSWORD, captcha: CAPTCHA } });
    expect(reg.response.status).toBe(201);
    const forgot = await anon().POST("/api/auth/password/forgot", { body: { email, captcha: CAPTCHA } });
    expect(forgot.response.status).toBe(204);
  });

  it("order: a malformed body is 422 before the captcha is checked", async () => {
    const res = await post("/api/auth/register", { email: uniqueEmail(), password: PASSWORD, captcha: "nope", isAdmin: true });
    expect(res.status).toBe(422);
    expect((await problem(res)).code).toBe("invalid_body");
  });

  it("login does not need a captcha (D6)", async () => {
    const { email } = await registerUser();
    expect((await anon().POST("/api/auth/login", { body: { email, password: PASSWORD } })).response.status).toBe(200);
  });

  it("GET /api/config returns no site key in fake mode", async () => {
    const { data, response } = await anon().GET("/api/config");
    expect(response.status).toBe(200);
    expect(data).toEqual({ turnstileSiteKey: null });
  });
});
