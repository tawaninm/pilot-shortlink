// AC3-AC7 and OWASP A01/A05 (docs/test-plan.md)
import { describe, expect, it } from "vitest";
import createClient from "openapi-fetch";
import type { paths } from "../src/schema";
import { BASE_URL, ORIGIN, anon, problem, registerUser, visit } from "../src/client";

const TARGET = "https://example.com/portfolio?ref=cv";
const uniqueAlias = () => `t-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;

describe("create link", () => {
  it("AC3: no alias gives a 7-char [a-z0-9] code, and opening it redirects 302 with no-store", async () => {
    const { api } = await registerUser();
    const { data, response } = await api.POST("/api/links", { body: { url: TARGET } });
    expect(response.status).toBe(201);
    expect(data?.code).toMatch(/^[a-z0-9]{7}$/);
    expect(data).toMatchObject({ url: TARGET, clicksTotal: 0 });
    expect(data?.last30Days).toHaveLength(30);

    const res = await visit(data!.code);
    expect(res.status).toBe(302);
    expect(res.headers.get("location")).toBe(TARGET);
    expect(res.headers.get("cache-control")).toBe("no-store");
  });

  it("an alias is case-insensitive and stored in lower case", async () => {
    const { api } = await registerUser();
    const alias = uniqueAlias();
    const { data } = await api.POST("/api/links", { body: { url: TARGET, alias: alias.toUpperCase() } });
    expect(data?.code).toBe(alias);
    expect((await visit(alias.toUpperCase())).status).toBe(302);
  });

  it("AC4: two users asking for the same free alias at once get one 201 and one 409 alias_taken", async () => {
    const [a, b] = await Promise.all([registerUser(), registerUser()]);
    const alias = uniqueAlias();
    const results = await Promise.all([a, b].map((u) => u.api.POST("/api/links", { body: { url: TARGET, alias } })));
    expect(results.map((r) => r.response.status).sort()).toEqual([201, 409]);
    const loser = results.find((r) => r.response.status === 409)!;
    expect((await problem(loser.response)).code).toBe("alias_taken");
  });

  it.each(["API", "admin", "Health", "openapi"])("AC5: reserved alias %s returns 422 reserved_alias", async (alias) => {
    const { api } = await registerUser();
    const { response } = await api.POST("/api/links", { body: { url: TARGET, alias } });
    expect(response.status).toBe(422);
    expect(await problem(response)).toMatchObject({ code: "reserved_alias", field: "alias" });
  });

  it.each(["-cv", "cv-", "ab", "a_b_c", "x".repeat(33)])("invalid alias %s returns 422 invalid_alias", async (alias) => {
    const { api } = await registerUser();
    const { response } = await api.POST("/api/links", { body: { url: TARGET, alias } });
    expect(response.status).toBe(422);
    expect(await problem(response)).toMatchObject({ code: "invalid_alias", field: "alias" });
  });

  it.each(["javascript:alert(1)", "data:text/html,<script>alert(1)</script>", "ftp://example.com/a", "not a url", `https://example.com/${"a".repeat(2048)}`])(
    "AC6 / A05: target %s returns 422 invalid_url",
    async (url) => {
      const { api } = await registerUser();
      const { response } = await api.POST("/api/links", { body: { url } });
      expect(response.status).toBe(422);
      expect(await problem(response)).toMatchObject({ code: "invalid_url", field: "url" });
    },
  );

  it("AC6: a target on the app's own host returns 422 self_link", async () => {
    const { api } = await registerUser();
    const { response } = await api.POST("/api/links", { body: { url: `${ORIGIN}/abc1234` } });
    expect(response.status).toBe(422);
    expect(await problem(response)).toMatchObject({ code: "self_link", field: "url" });
  });

  it("without a session returns 401", async () => {
    const { response } = await anon().POST("/api/links", { body: { url: TARGET } });
    expect(response.status).toBe(401);
  });

  it.each([["missing", undefined], ["foreign", "https://evil.example"]])("A01: %s Origin header returns 403 forbidden_origin", async (_, origin) => {
    const { cookie } = await registerUser();
    const headers: Record<string, string> = { "Content-Type": "application/json", Cookie: cookie };
    if (origin) headers.Origin = origin;
    const res = await fetch(`${BASE_URL}/api/links`, { method: "POST", headers, body: JSON.stringify({ url: TARGET }) });
    expect(res.status).toBe(403);
    expect((await problem(res)).code).toBe("forbidden_origin");
  });
});

describe("list, edit and delete", () => {
  it("lists only the caller's links, newest first", async () => {
    const [a, b] = await Promise.all([registerUser(), registerUser()]);
    const first = (await a.api.POST("/api/links", { body: { url: "https://example.com/1" } })).data!;
    const second = (await a.api.POST("/api/links", { body: { url: "https://example.com/2" } })).data!;
    await b.api.POST("/api/links", { body: { url: "https://example.com/b" } });
    const { data } = await a.api.GET("/api/links");
    expect(data?.items.map((l) => l.code)).toEqual([second.code, first.code]);
  });

  it("P1: PATCH changes the target and the redirect follows it", async () => {
    const { api } = await registerUser();
    const { code } = (await api.POST("/api/links", { body: { url: TARGET } })).data!;
    const { data, response } = await api.PATCH("/api/links/{code}", { params: { path: { code } }, body: { url: "https://example.org/new" } });
    expect(response.status).toBe(200);
    expect(data?.url).toBe("https://example.org/new");
    expect((await visit(code)).headers.get("location")).toBe("https://example.org/new");
  });

  it("DELETE returns 204, then the short link returns 404 HTML", async () => {
    const { api } = await registerUser();
    const { code } = (await api.POST("/api/links", { body: { url: TARGET } })).data!;
    expect((await api.DELETE("/api/links/{code}", { params: { path: { code } } })).response.status).toBe(204);
    const res = await visit(code);
    expect(res.status).toBe(404);
    expect(res.headers.get("content-type")).toMatch(/^text\/html/);
  });

  it("a code that never existed returns 404 HTML", async () => {
    const res = await visit("zz-never-created-zz");
    expect(res.status).toBe(404);
    expect(res.headers.get("content-type")).toMatch(/^text\/html/);
  });
});

describe("AC7 / A01: user B cannot touch user A's link", () => {
  it("view stats, edit and delete all return the same 404 as a link that does not exist", async () => {
    const [a, b] = await Promise.all([registerUser(), registerUser()]);
    const { code } = (await a.api.POST("/api/links", { body: { url: TARGET } })).data!;
    const missing = "zz-not-a-link-zz";
    const p = (c: string) => ({ params: { path: { code: c } } });

    for (const c of [code, missing]) {
      const calls = [
        b.api.GET("/api/links/{code}/stats", p(c)),
        b.api.PATCH("/api/links/{code}", { ...p(c), body: { url: "https://evil.example/" } }),
        b.api.DELETE("/api/links/{code}", p(c)),
      ];
      for (const { response } of await Promise.all(calls)) {
        expect(response.status).toBe(404);
        expect((await problem(response)).code).toBe("not_found");
      }
    }
    // A's link is untouched
    expect((await visit(code)).headers.get("location")).toBe(TARGET);
    expect((await b.api.GET("/api/links")).data?.items).toEqual([]);
  });
});

describe("misc", () => {
  it("GET /health returns status ok", async () => {
    const { data, response } = await createClient<paths>({ baseUrl: BASE_URL }).GET("/health");
    expect(response.status).toBe(200);
    expect(data).toEqual({ status: "ok" });
  });

  it("A10: an unknown /api route returns problem+json without a stack trace", async () => {
    const res = await fetch(`${BASE_URL}/api/does-not-exist`);
    expect(res.status).toBe(404);
    const body = JSON.stringify(await problem(res));
    expect(body).not.toMatch(/\.rs:|\.go:|panicked|goroutine|stack/i);
  });
});
