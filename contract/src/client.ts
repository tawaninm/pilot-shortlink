// Test helpers for the API contract (spec: api/openapi.yaml). Runs against any backend at API_URL
// (not BASE_URL: Vitest sets BASE_URL to "/" from Vite's import.meta.env).
import createClient from "openapi-fetch";
import type { paths } from "./schema";

export const BASE_URL = (process.env.API_URL ?? "http://localhost:8787").replace(/\/$/, "");
export const ORIGIN = new URL(BASE_URL).origin;
export const PASSWORD = "correct-horse-battery";

let seq = 0;
/** Unique email per call so tests never share accounts or rate-limit buckets. */
export const uniqueEmail = () => `t${Date.now().toString(36)}${(seq++).toString(36)}${Math.random().toString(36).slice(2, 6)}@example.test`;

// openapi-fetch reads the body of every response itself, so problem() could not read it afterwards.
// Keep an unread clone of each response, keyed by the response object openapi-fetch hands back.
const unread = new WeakMap<Response, Response>();
const keepClone: typeof fetch = async (input, init) => {
  const res = await fetch(input, init);
  unread.set(res, res.clone());
  return res;
};

/** Client with no cookie. Mutating requests send the app Origin unless a test overrides it. */
export const anon = () => createClient<paths>({ baseUrl: BASE_URL, headers: { Origin: ORIGIN }, fetch: keepClone });

/** Reads `session=<token>` from a response's Set-Cookie headers. */
export function sessionCookie(res: Response): string | undefined {
  const raw = res.headers.getSetCookie().find((c) => c.startsWith("__Host-session="));
  return raw?.split(";")[0];
}

export type User = Awaited<ReturnType<typeof registerUser>>;

/** Registers a fresh user and returns a client that sends its session cookie. */
export async function registerUser(email = uniqueEmail()) {
  const { response } = await anon().POST("/api/auth/register", { body: { email, password: PASSWORD } });
  if (response.status !== 201) throw new Error(`register ${email}: ${response.status}`);
  const cookie = sessionCookie(response);
  if (!cookie) throw new Error("register: no session cookie");
  const api = createClient<paths>({ baseUrl: BASE_URL, headers: { Origin: ORIGIN, Cookie: cookie }, fetch: keepClone });
  return { email, cookie, api };
}

/** Opens a short link like a browser click would, without following the redirect. */
export const visit = (code: string, referer?: string) =>
  fetch(`${BASE_URL}/${code}`, { redirect: "manual", headers: referer ? { Referer: referer } : {} });

/** Body of an RFC 9457 problem response, after checking the content type. */
export async function problem(res: Response) {
  if (!res.headers.get("content-type")?.startsWith("application/problem+json")) {
    throw new Error(`expected application/problem+json, got ${res.headers.get("content-type")} (${res.status})`);
  }
  return (await (unread.get(res) ?? res.clone()).json()) as { type: string; title: string; status: number; code: string; field?: string; detail?: string };
}
