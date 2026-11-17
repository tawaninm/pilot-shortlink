// User-facing Thai copy for every error state in docs/design/flows.md. Keep wording here, not in components.
import { ApiError } from "./api";

export const number = new Intl.NumberFormat("th-TH");

export const timeHHMM = (secondsFromNow: number, now = new Date()) =>
  new Date(now.getTime() + secondsFromNow * 1000).toLocaleTimeString("th-TH", {
    hour: "2-digit",
    minute: "2-digit",
  });

export type FieldError = { field: "url" | "alias" | "email" | "password" | "form"; text: string };

const NETWORK = "ส่งไม่สำเร็จ ตรวจอินเทอร์เน็ตแล้วลองอีกครั้ง";

/** Message for a failed create or edit of a link. */
export function linkError(err: unknown, alias = ""): FieldError {
  if (!(err instanceof ApiError) || err.status === 0) return { field: "form", text: NETWORK };
  switch (err.code) {
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
