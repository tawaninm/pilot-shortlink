import { expect, type Page } from "@playwright/test";

export const PASSWORD = "correct-horse-battery";
export const uniqueEmail = () => `e2e${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}@example.test`;

/** Registers through the UI and waits for the links page, without verifying the email. */
export async function registerUnverified(page: Page, email = uniqueEmail()) {
  await page.goto("/register");
  await page.getByLabel("email").fill(email);
  await page.getByLabel("รหัสผ่าน", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "สร้างบัญชี" }).click();
  await expect(page.getByText("ยังไม่มีลิงก์")).toBeVisible();
  return email;
}

/**
 * Link from the newest captured mail to `email` that points at `pathname` (verify-email or reset-password).
 * Needs the backend to run with MAIL_MODE=capture (scripts/verify.py e2e-a / e2e-b), so the full suite
 * cannot run against production anymore; production gets the smoke checks in docs/deploy.md (AC-V8, AC-V12).
 */
export async function mailLink(page: Page, email: string, pathname: "/verify-email" | "/reset-password") {
  const deadline = Date.now() + 5_000;
  for (;;) {
    const res = await page.request.get(`/api/test/mail?to=${encodeURIComponent(email)}`);
    expect(res.status(), "GET /api/test/mail (MAIL_MODE=capture)").toBe(200);
    const { items } = (await res.json()) as { items: { text: string }[] };
    for (const mail of [...items].reverse()) {
      const m = mail.text.match(/https?:\/\/\S+?(\/(?:verify-email|reset-password)\?token=[A-Za-z0-9_-]{43})/);
      if (m && m[1].startsWith(pathname)) return m[1];
    }
    if (Date.now() > deadline) throw new Error(`no ${pathname} link for ${email}`);
    await page.waitForTimeout(100);
  }
}

/** Registers through the UI, verifies the email through the captured mail, and returns to the links page. */
export async function register(page: Page, email = uniqueEmail()) {
  await registerUnverified(page, email);
  await page.goto(await mailLink(page, email, "/verify-email"));
  await page.getByRole("button", { name: "ยืนยันอีเมล" }).click();
  await expect(page.getByRole("heading", { name: "ยืนยันอีเมลแล้ว" })).toBeVisible();
  await page.getByRole("link", { name: "ไปที่ลิงก์ของฉัน" }).click();
  await expect(page.getByText("ยังไม่มีลิงก์")).toBeVisible();
  return email;
}

/** Creates a link through the form and returns its code from the new row. */
export async function createLink(page: Page, url: string, alias?: string) {
  await page.getByLabel("URL ปลายทาง").fill(url);
  if (alias) await page.getByLabel(/alias/).fill(alias);
  await page.getByRole("button", { name: "ย่อลิงก์" }).click();
  const row = page.getByRole("button", { expanded: false }).filter({ hasText: alias ? `/${alias.toLowerCase()}` : /^\// });
  await expect(row.first()).toBeVisible();
  return ((await row.first().textContent()) ?? "").trim().replace(/^\//, "");
}
