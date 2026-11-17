// AC8 and the click-counting requirement (docs/test-plan.md)
import { describe, expect, it } from "vitest";
import { registerUser, visit } from "../src/client";

describe("stats", () => {
  it("AC8: 3 visits from facebook show as total 3, today 3, referrer www.facebook.com 3", async () => {
    const { api } = await registerUser();
    const { code } = (await api.POST("/api/links", { body: { url: "https://example.com/" } })).data!;
    for (let i = 0; i < 3; i++) expect((await visit(code, "https://www.facebook.com/x?fbclid=abc")).status).toBe(302);

    const { data, response } = await api.GET("/api/links/{code}/stats", { params: { path: { code } } });
    expect(response.status).toBe(200);
    expect(data?.clicksTotal).toBe(3);
    expect(data?.daily).toHaveLength(30);
    const today = new Date().toISOString().slice(0, 10);
    expect(data?.daily.at(-1)).toEqual({ date: today, clicks: 3 });
    expect(data?.referrers).toEqual([{ host: "www.facebook.com", clicks: 3 }]);
  });

  it("a visit without Referer is counted under host null", async () => {
    const { api } = await registerUser();
    const { code } = (await api.POST("/api/links", { body: { url: "https://example.com/" } })).data!;
    await visit(code);
    const { data } = await api.GET("/api/links/{code}/stats", { params: { path: { code } } });
    expect(data?.referrers).toEqual([{ host: null, clicks: 1 }]);
  });

  it("concurrency: 20 simultaneous visits count as exactly 20", async () => {
    const { api } = await registerUser();
    const { code } = (await api.POST("/api/links", { body: { url: "https://example.com/" } })).data!;
    await Promise.all(Array.from({ length: 20 }, () => visit(code)));
    const { data } = await api.GET("/api/links/{code}/stats", { params: { path: { code } } });
    expect(data?.clicksTotal).toBe(20);
  });

  it("the list shows the same total and today's count as stats", async () => {
    const { api } = await registerUser();
    const { code } = (await api.POST("/api/links", { body: { url: "https://example.com/" } })).data!;
    await visit(code);
    await visit(code);
    const item = (await api.GET("/api/links")).data!.items.find((l) => l.code === code)!;
    expect(item.clicksTotal).toBe(2);
    expect(item.last30Days.at(-1)).toBe(2);
  });
});
