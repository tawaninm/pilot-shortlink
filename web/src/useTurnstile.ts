// Explicit Turnstile loader and widget lifecycle (docs/design/pages/oauth.md).
import { useQuery } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import { api, call } from "./api";

const TURNSTILE_SCRIPT_URL = "https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit";

let scriptPromise: Promise<void> | null = null;

function loadTurnstileScript(): Promise<void> {
  if (window.turnstile) return Promise.resolve();
  if (scriptPromise) return scriptPromise;

  const existing = document.querySelector<HTMLScriptElement>(`script[src="${TURNSTILE_SCRIPT_URL}"]`);
  if (existing) {
    scriptPromise = new Promise<void>((resolve, reject) => {
      existing.addEventListener("load", () => resolve(), { once: true });
      existing.addEventListener(
        "error",
        () => {
          scriptPromise = null;
          reject(new Error("Turnstile script failed to load"));
        },
        { once: true },
      );
    });
    return scriptPromise;
  }

  scriptPromise = new Promise<void>((resolve, reject) => {
    const script = document.createElement("script");
    script.src = TURNSTILE_SCRIPT_URL;
    script.async = true;
    script.defer = true;
    script.onload = () => resolve();
    script.onerror = () => {
      scriptPromise = null;
      script.remove();
      reject(new Error("Turnstile script failed to load"));
    };
    document.head.appendChild(script);
  });
  return scriptPromise;
}

/** Manages GET /api/config, lazy script loading, widget rendering, token state, and single-use resets. */
export function useTurnstile(enabled: boolean) {
  const ref = useRef<HTMLDivElement>(null);
  const widgetIdRef = useRef<string | null>(null);
  const [token, setToken] = useState<string | null>(null);
  const [scriptError, setScriptError] = useState(false);

  const config = useQuery({
    queryKey: ["config"],
    queryFn: () => call(api.GET("/api/config")),
    staleTime: Number.POSITIVE_INFINITY,
    enabled,
  });

  const siteKey = config.data?.turnstileSiteKey ?? null;
  const active = enabled && typeof siteKey === "string" && siteKey.length > 0;

  useEffect(() => {
    if (!active || !siteKey) return;
    let cancelled = false;
    let renderedId: string | null = null;
    setScriptError(false);

    loadTurnstileScript()
      .then(() => {
        if (cancelled || !ref.current || !window.turnstile) return;
        renderedId = window.turnstile.render(ref.current, {
          sitekey: siteKey,
          callback: (nextToken) => {
            if (!cancelled) setToken(nextToken);
          },
          "expired-callback": () => {
            if (!cancelled) setToken(null);
          },
          "error-callback": () => {
            if (!cancelled) setToken(null);
          },
        });
        widgetIdRef.current = renderedId;
      })
      .catch(() => {
        if (!cancelled) setScriptError(true);
      });

    return () => {
      cancelled = true;
      if (renderedId !== null) {
        window.turnstile?.remove(renderedId);
        if (widgetIdRef.current === renderedId) widgetIdRef.current = null;
      }
      setToken(null);
    };
  }, [active, siteKey]);

  const reset = useCallback(() => {
    if (!active) return;
    setToken(null);
    if (widgetIdRef.current !== null) {
      window.turnstile?.reset(widgetIdRef.current);
    }
  }, [active]);

  const captcha = !enabled ? null : config.isPending || config.isError ? null : active ? token : "fake-pass";

  return {
    ref,
    active,
    captcha,
    ready: captcha !== null,
    loadError: enabled && (config.isError || scriptError),
    reset,
  };
}
