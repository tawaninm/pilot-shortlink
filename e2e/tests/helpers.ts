import { expect, type Page } from "@playwright/test";

export const PASSWORD = "correct-horse-battery";
export const uniqueEmail = () => `e2e${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}@example.test`;

/** Registers through the UI and waits for the links page. */
export async function register(page: Page, email = uniqueEmail()) {
  await page.goto("/register");
  await page.getByLabel("email").fill(email);
  await page.getByLabel("รหัสผ่าน", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "สร้างบัญชี" }).click();
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
