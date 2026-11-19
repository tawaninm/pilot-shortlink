import { useEffect, useState } from "react";

const EMAIL_TOKEN_RE = /^[A-Za-z0-9_-]{43}$/;

/**
 * Reads the email token from the query string into component state and strips it from the address bar
 * with history.replaceState so browser history and Referer headers do not retain the token.
 */
export function useEmailToken(): { token: string | null; valid: boolean } {
  const [token] = useState<string | null>(() => {
    const raw = new URLSearchParams(window.location.search).get("token");
    return raw && EMAIL_TOKEN_RE.test(raw) ? raw : null;
  });

  useEffect(() => {
    const url = new URL(window.location.href);
    if (!url.searchParams.has("token")) return;
    url.searchParams.delete("token");
    window.history.replaceState(window.history.state, "", `${url.pathname}${url.search}${url.hash}`);
  }, []);

  return { token, valid: token !== null };
}
