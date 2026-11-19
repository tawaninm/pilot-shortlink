// User-facing Thai copy for every state in docs/design/flows.md and account-email.md. Keep wording here, not in components.
import { ApiError } from "./api";

export const number = new Intl.NumberFormat("th-TH");

export const timeHHMM = (secondsFromNow: number, now = new Date()) =>
  new Date(now.getTime() + secondsFromNow * 1000).toLocaleTimeString("th-TH", {
    hour: "2-digit",
    minute: "2-digit",
  });

const waitMinutes = (retryAfter?: number) => Math.max(1, Math.ceil((retryAfter ?? 60) / 60));

export type FieldError = { field: "url" | "alias" | "email" | "password" | "form"; text: string };

const NETWORK = "ส่งไม่สำเร็จ ตรวจอินเทอร์เน็ตแล้วลองอีกครั้ง";
const TOKEN_INVALID = "ลิงก์นี้ใช้ไม่ได้แล้ว อาจหมดอายุหรือถูกใช้ไปแล้ว";

export const authCopy = {
  expired: "หมดเวลาเข้าสู่ระบบ เข้าสู่ระบบอีกครั้งเพื่อทำต่อ",
  resetSuccess: "ตั้งรหัสผ่านใหม่แล้ว เข้าสู่ระบบด้วยรหัสใหม่",
  forgotPasswordLink: "ลืมรหัสผ่าน",
  passwordHint: "อย่างน้อย 10 ตัว",
  showPassword: "แสดงรหัส",
  hidePassword: "ซ่อนรหัส",
} as const;

export const verifyBannerCopy = {
  unverified: (email: string) => `ยืนยันอีเมลก่อนสร้างลิงก์ เราส่งลิงก์ไปที่ ${email} แล้ว`,
  resend: "ส่งอีเมลยืนยันอีกครั้ง",
  sending: "กำลังส่ง...",
  sent: (email: string) => `ส่งแล้ว ตรวจกล่องจดหมายของ ${email}`,
} as const;

export const verifyEmailCopy = {
  title: "ยืนยันอีเมล",
  submit: "ยืนยันอีเมล",
  busy: "กำลังยืนยัน...",
  successTitle: "ยืนยันอีเมลแล้ว",
  toLinks: "ไปที่ลิงก์ของฉัน",
  tokenInvalid: TOKEN_INVALID,
} as const;

export const forgotPasswordCopy = {
  title: "ลืมรหัสผ่าน",
  submit: "ส่งลิงก์ตั้งรหัสใหม่",
  busy: "กำลังส่ง...",
  sent: (email: string) => `ถ้ามีบัญชีของ ${email} เราส่งลิงก์ไปแล้ว ลิงก์ใช้ได้ 30 นาที`,
  backToLogin: "กลับไปเข้าสู่ระบบ",
} as const;

export const resetPasswordCopy = {
  title: "ตั้งรหัสผ่านใหม่",
  passwordLabel: "รหัสผ่านใหม่",
  submit: "ตั้งรหัสผ่านใหม่",
  busy: "กำลังบันทึก...",
  tokenInvalid: TOKEN_INVALID,
  requestNewLink: "ขอลิงก์ใหม่",
} as const;

export const qrDownloadLabel = (code: string) => `ดาวน์โหลด QR ของ /${code}`;

/** Message for a failed create or edit of a link. */
export function linkError(err: unknown, alias = ""): FieldError {
  if (!(err instanceof ApiError) || err.status === 0) return { field: "form", text: NETWORK };
  switch (err.code) {
    case "email_unverified":
      return { field: "form", text: "ยืนยันอีเมลก่อนสร้างลิงก์" };
    case "alias_taken":
      return { field: "alias", text: `alias นี้มีคนใช้แล้ว ลองชื่ออื่น เช่น ${alias.toLowerCase()}-2` };
    case "invalid_alias":
      return { field: "alias", text: "ใช้ได้แค่ a-z 0-9 และ - ยาว 3-32 ตัว ไม่ขึ้นต้นหรือลงท้ายด้วย -" };
    case "reserved_alias":
      return { field: "alias", text: `${alias.toLowerCase()} เป็นคำสงวน ลองชื่ออื่น` };
    case "self_link":
      return { field: "url", text: "ย่อลิงก์ของแอปนี้ซ้ำไม่ได้" };
    case "invalid_url":
      return { field: "url", text: "ต้องขึ้นต้นด้วย http:// หรือ https:// และยาวไม่เกิน 2048 ตัวอักษร" };
    case "rate_limited":
      return { field: "form", text: `สร้างลิงก์ถี่เกินไป ลองใหม่ในอีก ${err.retryAfter ?? 60} วินาที` };
    default:
      return { field: "form", text: "เกิดข้อผิดพลาด ลองอีกครั้ง" };
  }
}

/** Message for a failed login or registration. Login never says which field was wrong (no email probing). */
export function authError(err: unknown, mode: "login" | "register", now = new Date()): FieldError {
  if (!(err instanceof ApiError) || err.status === 0) return { field: "form", text: NETWORK };
  switch (err.code) {
    case "unauthorized":
      return { field: "form", text: "email หรือรหัสผ่านไม่ถูกต้อง" };
    case "rate_limited":
      return {
        field: "form",
        text: `ลองผิดหลายครั้งเกินไป ลองใหม่ได้เวลา ${timeHHMM(err.retryAfter ?? 900, now)} น.`,
      };
    case "email_taken":
      return { field: "email", text: "email นี้มีบัญชีแล้ว" };
    case "weak_password":
      return { field: "password", text: "รหัสผ่านต้องยาว 10-128 ตัว" };
    case "invalid_body":
      return { field: "email", text: "รูปแบบ email ไม่ถูกต้อง" };
    default:
      return {
        field: "form",
        text: mode === "login" ? "เข้าสู่ระบบไม่สำเร็จ ลองอีกครั้ง" : "สร้างบัญชีไม่สำเร็จ ลองอีกครั้ง",
      };
  }
}

/** Message for a failed verification email resend. */
export function resendEmailError(err: unknown): string {
  if (!(err instanceof ApiError) || err.status === 0) return NETWORK;
  if (err.code === "rate_limited") {
    return `ขอส่งบ่อยเกินไป ลองใหม่ใน ${waitMinutes(err.retryAfter)} นาที`;
  }
  return "ส่งไม่สำเร็จ ลองอีกครั้ง";
}

/** Message for a failed email verification attempt. */
export function verifyEmailError(err: unknown): string {
  if (!(err instanceof ApiError) || err.status === 0) return NETWORK;
  if (
    err.code === "token_invalid" ||
    err.code === "invalid_body" ||
    err.status === 400 ||
    err.status === 422
  ) {
    return TOKEN_INVALID;
  }
  return "ยืนยันไม่สำเร็จ ลองอีกครั้ง";
}

/** Message for a failed forgot-password request. */
export function forgotPasswordError(err: unknown): FieldError {
  if (!(err instanceof ApiError) || err.status === 0) return { field: "form", text: NETWORK };
  if (err.code === "rate_limited") {
    return { field: "form", text: `ขอลิงก์บ่อยเกินไป ลองใหม่ใน ${waitMinutes(err.retryAfter)} นาที` };
  }
  if (err.code === "invalid_body") {
    return { field: "email", text: "รูปแบบ email ไม่ถูกต้อง" };
  }
  return { field: "form", text: "ส่งไม่สำเร็จ ลองอีกครั้ง" };
}

/** Message for a failed password reset attempt. */
export function resetPasswordError(err: unknown): FieldError & { tokenInvalid?: boolean } {
  if (!(err instanceof ApiError) || err.status === 0) return { field: "form", text: NETWORK };
  if (err.code === "weak_password") {
    return { field: "password", text: "รหัสผ่านต้องยาว 10-128 ตัว" };
  }
  if (
    err.code === "token_invalid" ||
    err.status === 400 ||
    (err.code === "invalid_body" && err.problem?.field !== "password")
  ) {
    return { field: "form", text: TOKEN_INVALID, tokenInvalid: true };
  }
  if (err.code === "invalid_body" && err.problem?.field === "password") {
    return { field: "password", text: "รหัสผ่านต้องยาว 10-128 ตัว" };
  }
  return { field: "form", text: "ตั้งรหัสผ่านไม่สำเร็จ ลองอีกครั้ง" };
}
