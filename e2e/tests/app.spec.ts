// AC1, AC3, AC6, AC8, AC10 through the browser (docs/test-plan.md). Same file runs against A and B (AC12).
import { expect, test } from "@playwright/test";
import { PASSWORD, createLink, register } from "./helpers";

test("AC1: register lands on the empty links page; AC10: logout returns to login", async ({ page }) => {
  await register(page);
  await expect(page.getByText("วาง URL ด้านบนเพื่อสร้างลิงก์แรก")).toBeVisible();
  await page.getByRole("button", { name: "ออกจากระบบ" }).click();
  await expect(page).toHaveURL(/\/login/);
  await page.goto("/");
  await expect(page).toHaveURL(/\/login/); // the old session no longer works
});

test("wrong password keeps the email and does not say which field was wrong", async ({ page }) => {
  const email = await register(page);
  await page.getByRole("button", { name: "ออกจากระบบ" }).click();
  await page.getByLabel("email").fill(email);
  await page.getByLabel("รหัสผ่าน", { exact: true }).fill("wrong-password-x");
  await page.getByRole("button", { name: "เข้าสู่ระบบ" }).click();
  await expect(page.getByRole("alert")).toHaveText("email หรือรหัสผ่านไม่ถูกต้อง");
  await expect(page.getByLabel("email")).toHaveValue(email);
  await page.getByLabel("รหัสผ่าน", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "เข้าสู่ระบบ" }).click();
  await expect(page.getByRole("heading", { name: /ลิงก์ของฉัน \(0\)/ })).toBeVisible();
});

test("AC6: a javascript: URL shows the error under the field and keeps the value", async ({ page }) => {
  await register(page);
  const url = page.getByLabel("URL ปลายทาง");
  await url.fill("javascript:alert(1)");
  await page.getByRole("button", { name: "ย่อลิงก์" }).click();
  await expect(page.getByText("ต้องขึ้นต้นด้วย http:// หรือ https://")).toBeVisible();
  await expect(url).toHaveAttribute("aria-invalid", "true");
  await expect(url).toHaveValue("javascript:alert(1)");
});

test("AC3 + AC8: create a link, open it 3 times from facebook, see the stats", async ({ page, request }) => {
  await register(page);
  const code = await createLink(page, "https://example.com/portfolio");
  expect(code).toMatch(/^[a-z0-9]{7}$/);

  for (let i = 0; i < 3; i++) {
    const res = await request.get(`/${code}`, { maxRedirects: 0, headers: { Referer: "https://www.facebook.com/x" } });
    expect(res.status()).toBe(302);
    expect(res.headers().location).toBe("https://example.com/portfolio");
  }

  await page.reload();
  await page.locator(`button[aria-controls="stats-${code}"]`).click();
  await expect(page.getByText("คลิกทั้งหมด")).toBeVisible();
  const stats = page.locator(`#stats-${code}`);
  await expect(stats.getByText("www.facebook.com")).toBeVisible();
  await expect(stats.locator(".font-mono.text-xl")).toHaveText("3");
});

test("delete asks first, focuses Cancel, then removes the row", async ({ page }) => {
  await register(page);
  const code = await createLink(page, "https://example.com/delete-me", `del-${Date.now().toString(36)}`);
  await page.reload();
  const toggle = page.locator(`button[aria-controls="stats-${code}"]`);
  if (await page.getByRole("button", { name: `ลบ /${code}` }).isHidden()) await toggle.click(); // mobile: actions sit in the open row
  await page.getByRole("button", { name: `ลบ /${code}` }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole("button", { name: "ยกเลิก" })).toBeFocused();
  await dialog.getByRole("button", { name: "ลบ", exact: true }).click();
  await expect(page.getByText("ยังไม่มีลิงก์")).toBeVisible();
});
