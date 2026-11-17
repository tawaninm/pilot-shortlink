// Typed API client. Types come from api/openapi.yaml (pnpm -F web types); never write them by hand.
import createClient from "openapi-fetch";
import type { components, paths } from "./schema";

export type Problem = components["schemas"]["Problem"];
export type LinkSummary = components["schemas"]["LinkSummary"];
export type Stats = components["schemas"]["Stats"];

// Same origin as the page (ADR-001): the browser sends the session cookie and Origin header itself.
export const api = createClient<paths>({ baseUrl: "/", credentials: "same-origin" });

/** Every failed call becomes an ApiError. status 0 means the request never reached the server. */
export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly problem?: Problem,
    readonly retryAfter?: number,
  ) {
    super(problem?.code ?? (status === 0 ? "network" : `http_${status}`));
  }
  get code(): string {
    return this.message;
  }
}

/** Unwraps an openapi-fetch call: returns data, or throws ApiError with the RFC 9457 body. */
export async function call<T>(
  request: Promise<{ data?: T; error?: unknown; response: Response }>,
): Promise<T> {
  let result: { data?: T; error?: unknown; response: Response };
  try {
    result = await request;
  } catch {
    throw new ApiError(0);
  }
  const { data, error, response } = result;
  if (response.ok) return data as T;
  const retry = Number(response.headers.get("Retry-After"));
  throw new ApiError(
    response.status,
    typeof error === "object" && error !== null && "code" in error ? (error as Problem) : undefined,
    Number.isFinite(retry) && retry > 0 ? retry : undefined,
  );
}

export const shortUrl = (code: string) => `${window.location.origin}/${code}`;
