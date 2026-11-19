// AC-V11, password reset pages, AC-Q1, AC-Q2 through the browser (docs/test-plan.md section 8,
// texts from docs/design/pages/account-email.md). Needs MAIL_MODE=capture.
import { createRequire } from "node:module";
import { readFile } from "node:fs/promises";
import { type Page, expect, test } from "@playwright/test";
import { PASSWORD, createLink, mailLink, register, registerUnverified } from "./helpers";

const require = createRequire(import.meta.url);


// Below 640px the row actions sit inside the expanded stats row (docs/design/pages/links.md), same as app.spec.ts
async function openRowActions(page: Page, code: string) {
  if (await page.getByRole("button", { name: `ดาวน์โหลด QR ของ /${code}` }).isHidden()) {
    await page.locator(`button[aria-controls="stats-${code}"]`).click();
  }
}

test("AC-V11: unverified users see the banner and cannot create links until they verify", async ({ page }) => {
  const email = await registerUnverified(page);
  const banner = page.getByRole("status").filter({ hasText: "ยืนยันอีเมลก่อนสร้างลิงก์" });
  await expect(banner).toContainText(email);
  await expect(page.getByRole("button", { name: "ย่อลิงก์" })).toBeDisabled();

  await page.getByRole("button", { name: "ส่งอีเมลยืนยันอีกครั้ง" }).click();
  await expect(page.getByText(`ส่งแล้ว ตรวจกล่องจดหมายของ ${email}`)).toBeVisible();

  await page.goto(await mailLink(page, email, "/verify-email"));
  await page.getByRole("button", { name: "ยืนยันอีเมล" }).click();
  await expect(page.getByRole("heading", { name: "ยืนยันอีเมลแล้ว" })).toBeVisible();
  await page.getByRole("link", { name: "ไปที่ลิงก์ของฉัน" }).click();
  await expect(page.getByText("ยืนยันอีเมลก่อนสร้างลิงก์")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "ย่อลิงก์" })).toBeEnabled();
});

test("AC-V11: opening the verify page alone does not verify; a used link shows the error", async ({ page }) => {
  const email = await registerUnverified(page);
  const link = await mailLink(page, email, "/verify-email");
  await page.goto(link);
  await page.goto("/");
  await expect(page.getByText("ยืนยันอีเมลก่อนสร้างลิงก์")).toBeVisible(); // the visit alone changed nothing

  await page.goto(link);
  await page.getByRole("button", { name: "ยืนยันอีเมล" }).click();
  await expect(page.getByRole("heading", { name: "ยืนยันอีเมลแล้ว" })).toBeVisible();
  await page.goto(link);
  await page.getByRole("button", { name: "ยืนยันอีเมล" }).click();
  await expect(page.getByRole("alert")).toContainText("ลิงก์นี้ใช้ไม่ได้แล้ว");
});

test("AC-R web: forgot password from the login page, set a new password, log in with it", async ({ page }) => {
  const email = await register(page);
  await page.getByRole("button", { name: "ออกจากระบบ" }).click();
  await page.getByLabel("email").fill(email);
  await page.getByRole("link", { name: "ลืมรหัสผ่าน" }).click();
  await expect(page).toHaveURL(/\/forgot-password/);
  await expect(page.getByLabel("email")).toHaveValue(email);
  await page.getByRole("button", { name: "ส่งลิงก์ตั้งรหัสใหม่" }).click();
  await expect(page.getByRole("status")).toContainText(`ถ้ามีบัญชีของ ${email} เราส่งลิงก์ไปแล้ว`);

  await page.goto(await mailLink(page, email, "/reset-password"));
  await page.getByLabel("รหัสผ่านใหม่", { exact: true }).fill("short");
  await page.getByRole("button", { name: "ตั้งรหัสผ่านใหม่" }).click();
  await expect(page.getByLabel("รหัสผ่านใหม่", { exact: true })).toHaveAttribute("aria-invalid", "true");

  await page.getByLabel("รหัสผ่านใหม่", { exact: true }).fill("a-brand-new-passphrase");
  await page.getByRole("button", { name: "ตั้งรหัสผ่านใหม่" }).click();
  await expect(page).toHaveURL(/\/login/);
  await expect(page.getByRole("status")).toContainText("ตั้งรหัสผ่านใหม่แล้ว");

  await page.getByLabel("email").fill(email);
  await page.getByLabel("รหัสผ่าน", { exact: true }).fill("a-brand-new-passphrase");
  await page.getByRole("button", { name: "เข้าสู่ระบบ" }).click();
  await expect(page.getByRole("heading", { name: /ลิงก์ของฉัน \(\d+\)/ })).toBeVisible(); // the h2 with the count, not the sr-only h1
});

test("AC-R web: a used reset link shows the error and a way to ask again", async ({ page }) => {
  const email = await register(page);
  await page.request.post("/api/auth/password/forgot", { data: { email }, headers: { Origin: new URL(page.url()).origin } });
  const link = await mailLink(page, email, "/reset-password");
  await page.goto(link);
  await page.getByLabel("รหัสผ่านใหม่", { exact: true }).fill("a-brand-new-passphrase");
  await page.getByRole("button", { name: "ตั้งรหัสผ่านใหม่" }).click();
  await expect(page).toHaveURL(/\/login/);

  await page.goto(link);
  await page.getByLabel("รหัสผ่านใหม่", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "ตั้งรหัสผ่านใหม่" }).click();
  await expect(page.getByRole("alert")).toContainText("ลิงก์นี้ใช้ไม่ได้แล้ว");
  await page.getByRole("link", { name: "ขอลิงก์ใหม่" }).click();
  await expect(page).toHaveURL(/\/forgot-password/);
});

test("AC-Q1: the QR download is <code>.svg and decodes to the short URL", async ({ page }) => {
  await register(page);
  const code = await createLink(page, "https://example.com/qr", `qr-${Date.now().toString(36)}`);
  await openRowActions(page, code);
  const [download] = await Promise.all([
    page.waitForEvent("download"),
    page.getByRole("button", { name: `ดาวน์โหลด QR ของ /${code}` }).click(),
  ]);
  expect(download.suggestedFilename()).toBe(`${code}.svg`);
  const svg = await readFile(await download.path(), "utf8");
  expect(svg).toContain("<svg");

  // Decode for real: draw the SVG on a canvas and read it with jsQR. This runs in a separate about:blank tab
  // because the app's CSP (script-src 'self') rightly blocks the injected jsQR script on the app origin.
  const scratch = await page.context().newPage();
  await scratch.addScriptTag({ path: require.resolve("jsqr/dist/jsQR.js") });
  const decoded = await scratch.evaluate(async (markup) => {
    const img = new Image();
    img.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(markup)}`;
    await img.decode();
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = 400;
    const ctx = canvas.getContext("2d")!;
    ctx.fillStyle = "#fff";
    ctx.fillRect(0, 0, 400, 400);
    ctx.drawImage(img, 0, 0, 400, 400);
    const { data } = ctx.getImageData(0, 0, 400, 400);
    // @ts-expect-error jsQR is the global from the UMD bundle added above
    return jsQR(data, 400, 400)?.data ?? null;
  }, svg);
  await scratch.close();
  expect(decoded).toBe(`${new URL(page.url()).origin}/${code}`);
});

test("AC-Q2: the QR button is reachable by keyboard and has an accessible name", async ({ page }) => {
  await register(page);
  const code = await createLink(page, "https://example.com/qr-key", `qk-${Date.now().toString(36)}`);
  const label = `ดาวน์โหลด QR ของ /${code}`;
  const button = page.getByRole("button", { name: label });
  // Reach it with Tab only (a tabIndex=-1 button must fail this); on narrow screens open the row with Enter
  const active = () => page.evaluate(() => document.activeElement?.getAttribute("aria-label") ?? document.activeElement?.getAttribute("aria-controls") ?? "");
  await page.locator("body").press("Tab");
  for (let i = 0; i < 80 && (await active()) !== label; i++) {
    if ((await active()) === `stats-${code}` && (await button.isHidden())) await page.keyboard.press("Enter");
    await page.keyboard.press("Tab");
  }
  await expect(button).toBeFocused();
  const [download] = await Promise.all([page.waitForEvent("download"), page.keyboard.press("Enter")]);
  expect(download.suggestedFilename()).toBe(`${code}.svg`);
});
