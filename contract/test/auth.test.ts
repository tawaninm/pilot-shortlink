// AC1, AC2, AC10 and OWASP A04/A07 (docs/test-plan.md)
import { describe, expect, it } from "vitest";
import { anon, BASE_URL, CAPTCHA, ORIGIN, PASSWORD, problem, registerUser, sessionCookie, uniqueEmail } from "../src/client";
import createClient from "openapi-fetch";
import type { paths } from "../src/schema";

describe("register", () => {
  it("AC1: new email + 10-char password returns 201, the user and a session cookie", async () => {
    const email = uniqueEmail();
    const { data, response } = await anon().POST("/api/auth/register", { body: { email, password: PASSWORD, captcha: CAPTCHA } });
    expect(response.status).toBe(201);
    expect(data?.email).toBe(email);
    expect(sessionCookie(response)).toMatch(/^__Host-session=[A-Za-z0-9_-]{43}$/); // 32 random bytes, base64url
  });

  it("A07: session cookie is HttpOnly, Secure, SameSite=Lax, Path=/, 7 days", async () => {
    const { response } = await anon().POST("/api/auth/register", { body: { email: uniqueEmail(), password: PASSWORD, captcha: CAPTCHA } });
    const raw = response.headers.getSetCookie().find((c) => c.startsWith("__Host-session="))!;
    const attrs = raw.split(";").map((a) => a.trim().toLowerCase());
    expect(attrs).toEqual(expect.arrayContaining(["httponly", "secure", "samesite=lax", "path=/", "max-age=604800"]));
  });

  it("AC2: an existing email returns 409 email_taken", async () => {
    const { email } = await registerUser();
    const { response } = await anon().POST("/api/auth/register", { body: { email, password: PASSWORD, captcha: CAPTCHA } });
    expect(response.status).toBe(409);
    expect((await problem(response)).code).toBe("email_taken");
  });

  it("email comparison ignores case", async () => {
    const { email } = await registerUser();
    const { response } = await anon().POST("/api/auth/register", { body: { email: email.toUpperCase(), password: PASSWORD, captcha: CAPTCHA } });
    expect(response.status).toBe(409);
  });

  it("a password under 10 characters returns 422 weak_password on field password", async () => {
    const { response } = await anon().POST("/api/auth/register", { body: { email: uniqueEmail(), password: "short", captcha: CAPTCHA } });
    expect(response.status).toBe(422);
    expect(await problem(response)).toMatchObject({ code: "weak_password", field: "password" });
  });

  it("unknown body fields return 422 invalid_body", async () => {
    const res = await fetch(`${BASE_URL}/api/auth/register`, {
      method: "POST",
      headers: { "Content-Type": "application/json", Origin: ORIGIN },
      body: JSON.stringify({ email: uniqueEmail(), password: PASSWORD, captcha: CAPTCHA, isAdmin: true }),
    });
    expect(res.status).toBe(422);
    expect((await problem(res)).code).toBe("invalid_body");
  });
});

describe("login", () => {
  it("correct credentials return 200 and a new session", async () => {
    const { email, cookie } = await registerUser();
    const { data, response } = await anon().POST("/api/auth/login", { body: { email, password: PASSWORD } });
    expect(response.status).toBe(200);
    expect(data?.email).toBe(email);
    expect(sessionCookie(response)).not.toBe(cookie);
  });

  it("wrong password and unknown email return the same 401 unauthorized", async () => {
    const { email } = await registerUser();
    const wrong = await anon().POST("/api/auth/login", { body: { email, password: "wrong-password-x" } });
    const unknown = await anon().POST("/api/auth/login", { body: { email: uniqueEmail(), password: PASSWORD } });
    expect(wrong.response.status).toBe(401);
    expect(unknown.response.status).toBe(401);
    const [a, b] = [await problem(wrong.response), await problem(unknown.response)];
    expect({ ...a, detail: undefined }).toEqual({ ...b, detail: undefined });
  });
});

describe("session", () => {
  it("GET /api/me without a cookie returns 401", async () => {
    const { response } = await anon().GET("/api/me");
    expect(response.status).toBe(401);
    expect((await problem(response)).code).toBe("unauthorized");
  });

  it("a made-up session token returns 401", async () => {
    const api = createClient<paths>({ baseUrl: BASE_URL, headers: { Cookie: `__Host-session=${"A".repeat(43)}` } });
    expect((await api.GET("/api/me")).response.status).toBe(401);
  });

  it("AC10: after logout the old cookie returns 401, and logout again is still 204", async () => {
    const { api } = await registerUser();
    expect((await api.GET("/api/me")).response.status).toBe(200);
    expect((await api.POST("/api/auth/logout")).response.status).toBe(204);
    expect((await api.GET("/api/me")).response.status).toBe(401);
    expect((await api.POST("/api/auth/logout")).response.status).toBe(204);
  });
});
