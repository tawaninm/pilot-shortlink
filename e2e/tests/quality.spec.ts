// OWASP A02 headers, WCAG 2.2 AA via axe, and layout at 320px (docs/test-plan.md sections 2 and 5).
import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";
import { createLink, register, registerUnverified } from "./helpers";

const WCAG = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"];

test("A02: the app page has CSP, nosniff and a referrer policy", async ({ request }) => {
  const res = await request.get("/");
  const h = res.headers();
  expect(h["content-security-policy"]).toContain("frame-ancestors 'none'");
  expect(h["x-content-type-options"]).toBe("nosniff");
  expect(h["referrer-policy"]).toBe("strict-origin-when-cross-origin");
});

test("a11y: login page has no WCAG A/AA violations", async ({ page }) => {
  await page.goto("/login");
  await expect(page.getByRole("heading", { name: "เข้าสู่ระบบ" })).toBeVisible();
  const { violations } = await new AxeBuilder({ page }).withTags(WCAG).analyze();
  expect(violations.map((v) => `${v.id}: ${v.nodes.length}`)).toEqual([]);
});

test("a11y: links page with an open stats row has no WCAG A/AA violations", async ({ page, request }) => {
  await register(page);
  const code = await createLink(page, "https://example.com/a11y");
  await request.get(`/${code}`, { maxRedirects: 0 });
  await page.reload();
  await page.locator(`button[aria-controls="stats-${code}"]`).click();
  await expect(page.getByText("คลิกทั้งหมด")).toBeVisible();
  const { violations } = await new AxeBuilder({ page }).withTags(WCAG).analyze();
  expect(violations.map((v) => `${v.id}: ${v.nodes.length}`)).toEqual([]);
});

// pilot-addons pages (account-email.md); the token is well-formed but unknown, which is enough to render the page
const FAKE_TOKEN = "A".repeat(43);
for (const path of ["/forgot-password", `/verify-email?token=${FAKE_TOKEN}`, `/reset-password?token=${FAKE_TOKEN}`]) {
  test(`a11y: ${path.split("?")[0]} has no WCAG A/AA violations`, async ({ page }) => {
    await page.goto(path);
    await expect(page.getByRole("heading").first()).toBeVisible();
    const { violations } = await new AxeBuilder({ page }).withTags(WCAG).analyze();
    expect(violations.map((v) => `${v.id}: ${v.nodes.length}`)).toEqual([]);
  });
}

test("a11y: links page with the unverified banner has no WCAG A/AA violations", async ({ page }) => {
  await registerUnverified(page);
  await expect(page.getByText("ยืนยันอีเมลก่อนสร้างลิงก์")).toBeVisible();
  const { violations } = await new AxeBuilder({ page }).withTags(WCAG).analyze();
  expect(violations.map((v) => `${v.id}: ${v.nodes.length}`)).toEqual([]);
});

test("keyboard: Tab reaches the form and Enter submits it", async ({ page }) => {
  await register(page);
  await page.getByLabel("URL ปลายทาง").focus();
  await page.keyboard.type("https://example.com/keyboard");
  await page.keyboard.press("Tab"); // alias
  await page.keyboard.press("Tab"); // submit
  await expect(page.getByRole("button", { name: "ย่อลิงก์" })).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("heading", { name: /ลิงก์ของฉัน \(1\)/ })).toBeVisible();
});

test("no horizontal scroll at the current viewport", async ({ page }) => {
  await register(page);
  await createLink(page, `https://example.com/${"long-path-without-spaces-".repeat(8)}`);
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(overflow).toBeLessThanOrEqual(0);
});
