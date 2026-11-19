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

/** Serves a stand-in for Cloudflare's script and a site key, so the real widget code path runs without the network. */
async function stubTurnstile(page: Page) {
  await page.route("**/api/config", (route) => route.fulfill({ json: { turnstileSiteKey: "1x00000000000000000000AA" } }));
  await page.route("https://challenges.cloudflare.com/turnstile/v0/api.js**", (route) =>
    route.fulfill({
      contentType: "application/javascript",
      body: `window.turnstile = {
        render(el, opts) { el.setAttribute("data-rendered", "yes"); el.textContent = "stub widget"; setTimeout(() => opts.callback("fake-pass"), 50); return "w1"; },
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
  expect(directive("script-src")).toContain("https://challenges.cloudflare.com");
  expect(directive("frame-src")).toContain("https://challenges.cloudflare.com");
  expect(directive("script-src")).not.toMatch(/unsafe-inline|unsafe-eval|\*/);
});

test("a11y: login page with the OAuth links has no WCAG A/AA violations", async ({ page }) => {
  await page.goto("/login");
  await expect(page.getByRole("link", { name: "เข้าสู่ระบบด้วย Google" })).toBeVisible();
  const { violations } = await new AxeBuilder({ page }).withTags(WCAG).analyze();
  expect(violations.map((v) => `${v.id}: ${v.nodes.length}`)).toEqual([]);
});
