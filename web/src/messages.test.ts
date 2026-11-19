import { describe, expect, it } from "vitest";
import { ApiError } from "./api";
import {
  authError,
  forgotPasswordError,
  linkError,
  resendEmailError,
  resetPasswordError,
  verifyEmailError,
} from "./messages";

const problem = (status: number, code: string, retryAfter?: number, field?: string) =>
  new ApiError(status, { type: "about:blank", title: "", status, code, field } as never, retryAfter);

describe("linkError", () => {
  it("puts alias problems under the alias field with a suggestion", () => {
    expect(linkError(problem(409, "alias_taken"), "CV")).toEqual({
      field: "alias",
      text: "alias นี้มีคนใช้แล้ว ลองชื่ออื่น เช่น cv-2",
    });
    expect(linkError(problem(422, "reserved_alias"), "API").text).toBe("api เป็นคำสงวน ลองชื่ออื่น");
  });

  it("explains URL problems on the url field", () => {
    expect(linkError(problem(422, "self_link")).field).toBe("url");
    expect(linkError(problem(422, "invalid_url")).text).toMatch(/http:\/\//);
  });

  it("uses Retry-After for the rate limit and a network message when offline", () => {
    expect(linkError(problem(429, "rate_limited", 42)).text).toContain("42 วินาที");
    expect(linkError(new ApiError(0)).text).toMatch(/อินเทอร์เน็ต/);
    expect(linkError(new TypeError("x")).field).toBe("form");
  });
});

describe("authError", () => {
  it("does not reveal which login field was wrong", () => {
    expect(authError(problem(401, "unauthorized"), "login")).toEqual({
      field: "form",
      text: "email หรือรหัสผ่านไม่ถูกต้อง",
    });
  });

  it("tells the time when a locked login can retry", () => {
    // The message shows the viewer's local time, so the expected value is computed in the runner's timezone.
    const now = new Date("2026-11-17T10:00:00+07:00");
    const at = new Date(now.getTime() + 600_000).toLocaleTimeString("th-TH", {
      hour: "2-digit",
      minute: "2-digit",
    });
    expect(authError(problem(429, "rate_limited", 600), "login", now).text).toBe(
      `ลองผิดหลายครั้งเกินไป ลองใหม่ได้เวลา ${at} น.`,
    );
  });

  it("marks a taken email on the email field", () => {
    expect(authError(problem(409, "email_taken"), "register").field).toBe("email");
  });
});

describe("account email and password reset messages", () => {
  it("formats resend rate limits in whole minutes", () => {
    expect(resendEmailError(problem(429, "rate_limited", 90))).toBe("ขอส่งบ่อยเกินไป ลองใหม่ใน 2 นาที");
    expect(resendEmailError(new ApiError(0))).toMatch(/อินเทอร์เน็ต/);
  });

  it("returns the unified alert for invalid verify tokens and keeps retryable errors distinct", () => {
    expect(verifyEmailError(problem(400, "token_invalid"))).toEqual({
      text: "ลิงก์นี้ใช้ไม่ได้แล้ว อาจหมดอายุหรือถูกใช้ไปแล้ว",
      tokenInvalid: true,
    });
    expect(verifyEmailError(problem(422, "invalid_body"))).toEqual({
      text: "ลิงก์นี้ใช้ไม่ได้แล้ว อาจหมดอายุหรือถูกใช้ไปแล้ว",
      tokenInvalid: true,
    });
    expect(verifyEmailError(problem(503, "unavailable"))).toEqual({
      text: "ยืนยันไม่สำเร็จ ลองอีกครั้ง",
    });
    expect(verifyEmailError(new ApiError(0)).tokenInvalid).toBeUndefined();
  });

  it("handles forgot-password rate limit and invalid email", () => {
    expect(forgotPasswordError(problem(429, "rate_limited", 1800))).toEqual({
      field: "form",
      text: "ขอลิงก์บ่อยเกินไป ลองใหม่ใน 30 นาที",
    });
    expect(forgotPasswordError(problem(422, "invalid_body")).field).toBe("email");
  });

  it("distinguishes weak_password from token_invalid on reset-password", () => {
    expect(resetPasswordError(problem(422, "weak_password"))).toEqual({
      field: "password",
      text: "รหัสผ่านต้องยาว 10-128 ตัว",
    });
    expect(resetPasswordError(problem(400, "token_invalid"))).toEqual({
      field: "form",
      text: "ลิงก์นี้ใช้ไม่ได้แล้ว อาจหมดอายุหรือถูกใช้ไปแล้ว",
      tokenInvalid: true,
    });
  });
});
