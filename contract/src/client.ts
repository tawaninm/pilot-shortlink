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

/** Client that sends a session cookie and the app Origin. */
export const withCookie = (cookie: string) =>
  createClient<paths>({ baseUrl: BASE_URL, headers: { Origin: ORIGIN, Cookie: cookie }, fetch: keepClone });

/** Registers a fresh user without verifying the email (AC-V*). */
export async function registerUnverified(email = uniqueEmail()) {
  const { response } = await anon().POST("/api/auth/register", { body: { email, password: PASSWORD } });
  if (response.status !== 201) throw new Error(`register ${email}: ${response.status}`);
  const cookie = sessionCookie(response);
  if (!cookie) throw new Error("register: no session cookie");
  return { email, cookie, api: withCookie(cookie) };
}

/** Registers a fresh user, verifies the email through the captured mail, and returns a client with its cookie. */
export async function registerUser(email = uniqueEmail()) {
  const user = await registerUnverified(email);
  const token = tokenFrom(await waitForMail(email, 1), "verify-email");
  const { response } = await anon().POST("/api/auth/verify-email", { body: { token } });
  if (response.status !== 204) throw new Error(`verify ${email}: ${response.status}`);
  return user;
}

// --- Captured mail (backend runs with MAIL_MODE=capture; the route is not part of api/openapi.yaml) ---
// GET /api/test/mail?to=<email> -> 200 { items: [{ to, subject, text, html }] } oldest first.
// Without MAIL_MODE=capture the route does not exist and returns 404 (AC-V8).
export type Mail = { to: string; subject: string; text: string; html: string };

export async function readMail(to: string): Promise<Mail[]> {
  const res = await fetch(`${BASE_URL}/api/test/mail?to=${encodeURIComponent(to)}`);
  if (res.status !== 200) throw new Error(`GET /api/test/mail: ${res.status} (is MAIL_MODE=capture set?)`);
  return ((await res.json()) as { items: Mail[] }).items;
}

/** Mail is sent in the background after the response, so poll until `count` mails arrived (or time out). */
export async function waitForMail(to: string, count: number, timeoutMs = 3_000): Promise<Mail[]> {
  const until = Date.now() + timeoutMs;
  for (;;) {
    const items = await readMail(to);
    if (items.length >= count) return items;
    if (Date.now() > until) throw new Error(`expected ${count} mail(s) to ${to}, got ${items.length}`);
    await new Promise((r) => setTimeout(r, 100));
  }
}

/** Waits a moment and returns how many mails arrived, for "no mail is sent" checks. */
export async function mailCountAfterPause(to: string, pauseMs = 800): Promise<number> {
  await new Promise((r) => setTimeout(r, pauseMs));
  return (await readMail(to)).length;
}

/** Token from the newest mail whose link points at `page`. Checks the link starts with the app origin (A05). */
export function tokenFrom(mails: Mail[], page: "verify-email" | "reset-password"): string {
  for (const mail of [...mails].reverse()) {
    const m = mail.text.match(/(\S+)\/(verify-email|reset-password)\?token=([A-Za-z0-9_-]{43})/);
    if (m && m[2] === page) {
      if (m[1] !== ORIGIN) throw new Error(`mail link origin ${m[1]} is not ${ORIGIN}`);
      return m[3];
    }
  }
  throw new Error(`no ${page} link in ${mails.length} mail(s)`);
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
