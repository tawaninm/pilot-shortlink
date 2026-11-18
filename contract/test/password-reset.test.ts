// Forgot / reset password (requirements AC-R1..R7, test-plan section 8). Token expiry is in backend unit tests.
import { describe, expect, it } from "vitest";
import {
  anon, mailCountAfterPause, PASSWORD, problem, registerUnverified, registerUser, sessionCookie, tokenFrom, uniqueEmail,
  waitForMail, withCookie,
} from "../src/client";

const NEW_PASSWORD = "a-brand-new-passphrase";
const forgot = (email: string) => anon().POST("/api/auth/password/forgot", { body: { email } });
const reset = (token: string, password = NEW_PASSWORD) => anon().POST("/api/auth/password/reset", { body: { token, password } });
const login = (email: string, password: string) => anon().POST("/api/auth/login", { body: { email, password } });

/** Verified user who asked for a reset; returns the reset token from the 2nd mail (the 1st is the verify mail). */
async function userWithResetToken() {
  const user = await registerUser();
  expect((await forgot(user.email)).response.status).toBe(204);
  return { ...user, token: tokenFrom(await waitForMail(user.email, 2), "reset-password") };
}

describe("password reset", () => {
  it("AC-R1: forgot returns the same 204 for known and unknown emails and mails only the known one", async () => {
    const { email } = await registerUser();
    const unknown = uniqueEmail();
    const known = await forgot(email);
    const missing = await forgot(unknown);
    expect(known.response.status).toBe(204);
    expect(missing.response.status).toBe(204);
    expect(await known.response.clone().text()).toBe(await missing.response.clone().text());
    tokenFrom(await waitForMail(email, 2), "reset-password");
    expect(await mailCountAfterPause(unknown)).toBe(0);
  });

  it("AC-R2: a reset token sets the new password; the old one stops working", async () => {
    const { email, token } = await userWithResetToken();
    expect((await reset(token)).response.status).toBe(204);
    expect((await login(email, NEW_PASSWORD)).response.status).toBe(200);
    expect((await login(email, PASSWORD)).response.status).toBe(401);
  });

  it("AC-R3: a reset ends every existing session of the account", async () => {
    const { email, cookie, token } = await userWithResetToken();
    const second = sessionCookie((await login(email, PASSWORD)).response);
    if (!second) throw new Error("login: no session cookie");
    expect((await reset(token)).response.status).toBe(204);
    for (const c of [cookie, second]) expect((await withCookie(c).GET("/api/me")).response.status).toBe(401);
  });

  it("AC-R4: a used reset token returns 400 token_invalid", async () => {
    const { token } = await userWithResetToken();
    expect((await reset(token)).response.status).toBe(204);
    const again = await reset(token, "yet-another-passphrase");
    expect(again.response.status).toBe(400);
    expect((await problem(again.response)).code).toBe("token_invalid");
  });

  it("AC-R4: tokens do not cross purposes (verify token cannot reset, reset token cannot verify)", async () => {
    const unverified = await registerUnverified();
    const verifyToken = tokenFrom(await waitForMail(unverified.email, 1), "verify-email");
    const r = await reset(verifyToken);
    expect(r.response.status).toBe(400);
    expect((await problem(r.response)).code).toBe("token_invalid");

    const { token } = await userWithResetToken();
    const v = await anon().POST("/api/auth/verify-email", { body: { token } });
    expect(v.response.status).toBe(400);
    expect((await problem(v.response)).code).toBe("token_invalid");
  });

  it("AC-R4: asking again makes the earlier reset token unusable", async () => {
    const { email, token: first } = await userWithResetToken();
    expect((await forgot(email)).response.status).toBe(204);
    const second = tokenFrom(await waitForMail(email, 3), "reset-password");
    expect((await reset(first)).response.status).toBe(400);
    expect((await reset(second)).response.status).toBe(204);
  });

  it("AC-R5: the 4th forgot request for one email within an hour returns 429", async () => {
    const { email } = await registerUser();
    for (let i = 0; i < 3; i++) expect((await forgot(email)).response.status).toBe(204);
    const fourth = await forgot(email);
    expect(fourth.response.status).toBe(429);
    expect((await problem(fourth.response)).code).toBe("rate_limited");
  });

  it("AC-R6: resetting an unverified account marks the email verified", async () => {
    const { email } = await registerUnverified();
    expect((await forgot(email)).response.status).toBe(204);
    expect((await reset(tokenFrom(await waitForMail(email, 2), "reset-password"))).response.status).toBe(204);
    const cookie = sessionCookie((await login(email, NEW_PASSWORD)).response);
    if (!cookie) throw new Error("login: no session cookie");
    expect((await withCookie(cookie).GET("/api/me")).data?.emailVerified).toBe(true);
  });

  it("AC-R7: a short new password returns 422 weak_password and the token stays usable", async () => {
    const { token } = await userWithResetToken();
    const weak = await reset(token, "short");
    expect(weak.response.status).toBe(422);
    expect((await problem(weak.response)).code).toBe("weak_password");
    expect((await reset(token)).response.status).toBe(204);
  });

  it("concurrency: two resets with one token succeed exactly once", async () => {
    const { token } = await userWithResetToken();
    const statuses = (await Promise.all([reset(token), reset(token, "another-passphrase-2")])).map((r) => r.response.status).sort();
    expect(statuses).toEqual([204, 400]);
  });
});
