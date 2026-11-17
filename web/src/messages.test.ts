import { describe, expect, it } from "vitest";
import { ApiError } from "./api";
import { authError, linkError } from "./messages";

const problem = (status: number, code: string, retryAfter?: number) =>
  new ApiError(status, { type: "about:blank", title: "", status, code } as never, retryAfter);

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
    const now = new Date("2026-11-17T10:00:00+07:00");
    expect(authError(problem(429, "rate_limited", 600), "login", now).text).toMatch(/ลองใหม่ได้เวลา .*10.10/);
  });

  it("marks a taken email on the email field", () => {
    expect(authError(problem(409, "email_taken"), "register").field).toBe("email");
  });
});
