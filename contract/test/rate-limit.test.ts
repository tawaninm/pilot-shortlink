// AC11 and OWASP A06 (docs/test-plan.md). Each test uses a fresh account or email, so buckets never overlap.
import { describe, expect, it } from "vitest";
import { anon, problem, registerUser, uniqueEmail } from "../src/client";

describe("rate limits", () => {
  it("AC11: the 31st link created in one minute returns 429 rate_limited with Retry-After", { timeout: 120_000 }, async () => {
    const { api } = await registerUser();
    // The limit is a fixed window per clock minute (docs/architecture.md section 5). 31 requests take up to
    // ~25 s on a slow backend, so start early in a minute or the window resets halfway and the 31st is allowed.
    const second = new Date().getUTCSeconds();
    if (second > 20) await new Promise((r) => setTimeout(r, (60 - second) * 1000 + 300));
    for (let i = 0; i < 30; i++) {
      expect((await api.POST("/api/links", { body: { url: `https://example.com/${i}` } })).response.status).toBe(201);
    }
    const { response } = await api.POST("/api/links", { body: { url: "https://example.com/31" } });
    expect(response.status).toBe(429);
    expect(Number(response.headers.get("retry-after"))).toBeGreaterThanOrEqual(1);
    expect((await problem(response)).code).toBe("rate_limited");
  });

  it("A06: the 6th wrong login for one email within 15 minutes returns 429, even with the right password", async () => {
    const { email } = await registerUser();
    for (let i = 0; i < 5; i++) {
      expect((await anon().POST("/api/auth/login", { body: { email, password: `wrong-password-${i}` } })).response.status).toBe(401);
    }
    const { response } = await anon().POST("/api/auth/login", { body: { email, password: "correct-horse-battery" } });
    expect(response.status).toBe(429);
    expect(Number(response.headers.get("retry-after"))).toBeGreaterThanOrEqual(1);
  });

  it("the login limit is per email: another email still gets 401, not 429", async () => {
    const { email } = await registerUser();
    for (let i = 0; i < 6; i++) await anon().POST("/api/auth/login", { body: { email, password: `wrong-password-${i}` } });
    const { response } = await anon().POST("/api/auth/login", { body: { email: uniqueEmail(), password: "wrong-password-x" } });
    expect(response.status).toBe(401);
  });

  it("A06 (S7 review): 10 parallel wrong logins cannot get more than 5 guesses", async () => {
    const { email } = await registerUser();
    const results = await Promise.all(
      Array.from({ length: 10 }, (_, i) => anon().POST("/api/auth/login", { body: { email, password: `wrong-password-${i}` } })),
    );
    const statuses = results.map((r) => r.response.status);
    expect(statuses.filter((s) => s === 401).length).toBeLessThanOrEqual(5);
    expect(statuses.every((s) => s === 401 || s === 429)).toBe(true);
  });
});
