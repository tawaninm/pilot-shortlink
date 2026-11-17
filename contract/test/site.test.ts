// C8: the backend serves the web app on the same origin as the API (ADR-001), with security headers (OWASP A02).
import { describe, expect, it } from "vitest";
import { BASE_URL } from "../src/client";

const page = (path: string) => fetch(`${BASE_URL}${path}`, { redirect: "manual" });

describe("web app served by the backend", () => {
  it.each(["/", "/login", "/register"])("%s returns the app's index.html", async (path) => {
    const res = await page(path);
    expect(res.status).toBe(200);
    expect(res.headers.get("content-type")).toMatch(/^text\/html/);
    expect(await res.text()).toContain('<div id="root"></div>');
  });

  it("A02: HTML pages carry CSP, nosniff and a referrer policy", async () => {
    const res = await page("/");
    const csp = res.headers.get("content-security-policy") ?? "";
    expect(csp).toContain("default-src 'self'");
    expect(csp).toContain("frame-ancestors 'none'");
    expect(csp).not.toContain("'unsafe-eval'");
    expect(res.headers.get("x-content-type-options")).toBe("nosniff");
    expect(res.headers.get("referrer-policy")).toBe("strict-origin-when-cross-origin");
  });

  it("built assets are served with a JavaScript content type", async () => {
    const html = await (await page("/")).text();
    const script = html.match(/src="(\/assets\/[^"]+\.js)"/)?.[1];
    expect(script, "index.html should reference a built /assets/*.js file").toBeTruthy();
    const res = await page(script!);
    expect(res.status).toBe(200);
    expect(res.headers.get("content-type")).toMatch(/javascript/);
  });

  it("a missing asset is 404, not the app shell", async () => {
    const res = await page("/assets/does-not-exist.js");
    expect(res.status).toBe(404);
  });

  it("A02: API responses carry nosniff", async () => {
    const res = await page("/health");
    expect(res.headers.get("x-content-type-options")).toBe("nosniff");
  });
});
