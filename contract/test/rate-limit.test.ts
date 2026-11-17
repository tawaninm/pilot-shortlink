// AC11 and OWASP A06 (docs/test-plan.md). Each test uses a fresh account or email, so buckets never overlap.
import { describe, expect, it } from "vitest";
import { anon, problem, registerUser, uniqueEmail } from "../src/client";

describe("rate limits", () => {
  it("AC11: the 31st link created in one minute returns 429 rate_limited with Retry-After", async () => {
    const { api } = await registerUser();
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
});
