// Login with Google/GitHub through the fake IdP and Turnstile on register/forgot (requirements AC-W1, AC-T3,
// texts from docs/design/pages/oauth.md). Needs OAUTH_MODE=fake and TURNSTILE_MODE=fake (scripts/verify.py e2e-a / e2e-b).
import AxeBuilder from "@axe-core/playwright";
import { expect, type Page, test } from "@playwright/test";
import { PASSWORD } from "./helpers";

const WCAG = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"];
const uniqueEmail = () => `o${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}@example.test`;

/** On the fake provider's consent page: type the account and continue. */
async function consent(page: Page, email: string, verified = true) {
  await expect(page.getByRole("heading", { name: /Fake (google|github) sign-in/ })).toBeVisible();
  await page.getByLabel("Email").fill(email);
  await page.getByLabel("Verified").selectOption(verified ? "yes" : "no");
  await page.getByRole("button", { name: "Continue" }).click();
}

const resets = (page: Page) => page.evaluate(() => (window as unknown as { __turnstileResets?: number }).__turnstileResets ?? 0);

/** Serves a stand-in for Cloudflare's script and a site key, so the real widget code path runs without the network.
 *  mode "token" gives a token after delayMs; mode "error" calls the widget's error-callback instead. */
async function stubTurnstile(page: Page, opts: { delayMs?: number; mode?: "token" | "error" } = {}) {
  const { delayMs = 50, mode = "token" } = opts;
  await page.route("**/api/config", (route) => route.fulfill({ json: { turnstileSiteKey: "1x00000000000000000000AA" } }));
  await page.route("https://challenges.cloudflare.com/turnstile/v0/api.js**", (route) =>
    route.fulfill({
      contentType: "application/javascript",
      body: `window.turnstile = {
        render(el, opts) { el.setAttribute("data-rendered", "yes"); el.textContent = "stub widget";
          setTimeout(() => ${mode === "error" ? 'opts["error-callback"]()' : 'opts.callback("fake-pass")'}, ${delayMs}); return "w1"; },
        reset() { window.__turnstileResets = (window.__turnstileResets || 0) + 1; },
        remove() {},
      };`,
    }),
  );
}

test("AC-W1: Google login from the login page lands on a verified account's links", async ({ page }) => {
  await page.goto("/login");
  await page.getByRole("link", { name: "เข้าสู่ระบบด้วย Google" }).click();
  await consent(page, uniqueEmail());
  await expect(page.getByText("ยังไม่มีลิงก์")).toBeVisible();
  await expect(page.getByText("ยืนยันอีเมลก่อนสร้างลิงก์")).toHaveCount(0);
});

test("AC-W1: GitHub login from the register page works too", async ({ page }) => {
  await page.goto("/register");
  await expect(page.getByRole("link", { name: "เข้าสู่ระบบด้วย GitHub" })).toHaveAttribute("href", "/api/auth/oauth/github/start");
  await page.getByRole("link", { name: "เข้าสู่ระบบด้วย GitHub" }).click();
  await consent(page, uniqueEmail());
  await expect(page.getByText("ยังไม่มีลิงก์")).toBeVisible();
});

test("AC-W1: a failed provider login shows the error once on the login page", async ({ page }) => {
  await page.goto("/login");
  await page.getByRole("link", { name: "เข้าสู่ระบบด้วย Google" }).click();
  await consent(page, uniqueEmail(), false);
  await expect(page).toHaveURL(/\/login/);
  await expect(page.getByRole("alert")).toContainText("เข้าสู่ระบบด้วย Google หรือ GitHub ไม่สำเร็จ");
  await expect(page).not.toHaveURL(/error=oauth/);
  await page.reload();
  await expect(page.getByRole("alert")).toHaveCount(0);
});

test("AC-T3: the register page renders the Turnstile widget and sends its token", async ({ page }) => {
  await stubTurnstile(page);
  await page.goto("/register");
  await expect(page.getByTestId("turnstile")).toHaveAttribute("data-rendered", "yes");
  await page.getByLabel("email").fill(uniqueEmail());
  await page.getByLabel("รหัสผ่าน", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "สร้างบัญชี" }).click();
  await expect(page.getByText("ยังไม่มีลิงก์")).toBeVisible();
});

test("AC-T3: the register page has no WCAG A/AA violations with the widget rendered", async ({ page }) => {
  await stubTurnstile(page);
  await page.goto("/register");
  await expect(page.getByTestId("turnstile")).toHaveAttribute("data-rendered", "yes");
  const { violations } = await new AxeBuilder({ page }).withTags(WCAG).analyze();
  expect(violations.map((v) => `${v.id}: ${v.nodes.length}`)).toEqual([]);
});

test("AC-T3: the submit button stays disabled until the widget gives a token", async ({ page }) => {
  await stubTurnstile(page, { delayMs: 800 });
  await page.goto("/register");
  await page.getByLabel("email").fill(uniqueEmail());
  await page.getByLabel("รหัสผ่าน", { exact: true }).fill(PASSWORD);
  const submit = page.getByRole("button", { name: "สร้างบัญชี" });
  await expect(submit).toBeDisabled();
  await expect(submit).toBeEnabled({ timeout: 5_000 });
});

test("AC-T3: a rejected captcha shows the message and resets the widget (register and forgot)", async ({ page }) => {
  await stubTurnstile(page);
  const problem = { type: "about:blank", title: "Forbidden", status: 403, code: "captcha_failed" };
  await page.route("**/api/auth/register", (route) => route.fulfill({ status: 403, contentType: "application/problem+json", body: JSON.stringify(problem) }));
  await page.route("**/api/auth/password/forgot", (route) => route.fulfill({ status: 403, contentType: "application/problem+json", body: JSON.stringify(problem) }));

  await page.goto("/register");
  await page.getByLabel("email").fill(uniqueEmail());
  await page.getByLabel("รหัสผ่าน", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "สร้างบัญชี" }).click();
  await expect(page.getByRole("alert")).toContainText("ยืนยันว่าไม่ใช่บอทไม่สำเร็จ");
  await expect.poll(() => resets(page)).toBeGreaterThanOrEqual(1);

  await page.goto("/forgot-password");
  await expect(page.getByTestId("turnstile")).toHaveAttribute("data-rendered", "yes");
  await page.getByLabel("email").fill(uniqueEmail());
  await page.getByRole("button", { name: "ส่งลิงก์ตั้งรหัสใหม่" }).click();
  await expect(page.getByRole("alert")).toContainText("ยืนยันว่าไม่ใช่บอทไม่สำเร็จ");
  await expect.poll(() => resets(page)).toBeGreaterThanOrEqual(1);
});

test("AC-T3: a widget that cannot start shows an error instead of a dead form", async ({ page }) => {
  // (a) the script loads but never defines window.turnstile (for example a blocker returns an empty file)
  await page.route("**/api/config", (route) => route.fulfill({ json: { turnstileSiteKey: "1x00000000000000000000AA" } }));
  await page.route("https://challenges.cloudflare.com/turnstile/v0/api.js**", (route) =>
    route.fulfill({ contentType: "application/javascript", body: "/* emptied by an extension */" }),
  );
  await page.goto("/register");
  await expect(page.getByRole("alert")).toContainText("โหลดตัวช่วยยืนยันไม่สำเร็จ");
});

test("AC-T3: the widget's own error callback shows an error", async ({ page }) => {
  await stubTurnstile(page, { mode: "error" });
  await page.goto("/register");
  await expect(page.getByRole("alert")).toContainText("โหลดตัวช่วยยืนยันไม่สำเร็จ");
});

test("AC-T3: the forgot page renders the widget and the page has no WCAG A/AA violations", async ({ page }) => {
  await stubTurnstile(page);
  await page.goto("/forgot-password");
  await expect(page.getByTestId("turnstile")).toHaveAttribute("data-rendered", "yes");
  const { violations } = await new AxeBuilder({ page }).withTags(WCAG).analyze();
  expect(violations.map((v) => `${v.id}: ${v.nodes.length}`)).toEqual([]);
});

test("AC-T3: CSP allows only challenges.cloudflare.com in addition to self for scripts and frames", async ({ request }) => {
  const csp = (await request.get("/register")).headers()["content-security-policy"] ?? "";
  const directive = (name: string) => csp.split(";").map((d) => d.trim()).find((d) => d.startsWith(`${name} `)) ?? "";
  expect(directive("script-src")).toBe("script-src 'self' https://challenges.cloudflare.com");
  expect(directive("frame-src")).toBe("frame-src https://challenges.cloudflare.com");
});

test("a11y: login page with the OAuth links has no WCAG A/AA violations", async ({ page }) => {
  await page.goto("/login");
  await expect(page.getByRole("link", { name: "เข้าสู่ระบบด้วย Google" })).toBeVisible();
  const { violations } = await new AxeBuilder({ page }).withTags(WCAG).analyze();
  expect(violations.map((v) => `${v.id}: ${v.nodes.length}`)).toEqual([]);
});
