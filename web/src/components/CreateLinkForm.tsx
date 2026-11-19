// N3a create form: values survive every error, including a session timeout (kept in sessionStorage, N8).
import { type FormEvent, useEffect, useId, useRef, useState } from "react";
import { ApiError, api, call } from "../api";
import { type FieldError, linkError } from "../messages";

const DRAFT_KEY = "create-link-draft";

function readDraft(): { url: string; alias: string } {
  try {
    const saved = JSON.parse(sessionStorage.getItem(DRAFT_KEY) ?? "{}") as { url?: unknown; alias?: unknown };
    return {
      url: typeof saved.url === "string" ? saved.url : "",
      alias: typeof saved.alias === "string" ? saved.alias : "",
    };
  } catch {
    return { url: "", alias: "" };
  }
}

export function CreateLinkForm(props: {
  onCreated: (code: string) => void;
  onUnauthorized: () => void;
  onUnverified: () => void;
  unverified?: boolean;
  unverifiedBannerId?: string;
}) {
  const [draft] = useState(readDraft);
  const [url, setUrl] = useState(draft.url);
  const [alias, setAlias] = useState(draft.alias);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<FieldError | null>(null);
  const [waitUntil, setWaitUntil] = useState(0);
  const [now, setNow] = useState(Date.now());
  const urlRef = useRef<HTMLInputElement>(null);
  const aliasRef = useRef<HTMLInputElement>(null);
  const id = useId();

  useEffect(() => {
    try {
      sessionStorage.setItem(DRAFT_KEY, JSON.stringify({ url, alias }));
    } catch {
      /* storage blocked: the form still works, the draft just is not kept */
    }
  }, [url, alias]);

  // Count down the rate-limit wait on the button.
  useEffect(() => {
    if (waitUntil <= Date.now()) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [waitUntil]);
  const waitSeconds = Math.max(0, Math.ceil((waitUntil - now) / 1000));

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (busy || waitSeconds > 0 || props.unverified) return; // no double submit (flows.md N3a)
    setBusy(true);
    setError(null);
    try {
      const created = await call(
        api.POST("/api/links", {
          body: alias.trim() ? { url: url.trim(), alias: alias.trim() } : { url: url.trim() },
        }),
      );
      setUrl("");
      setAlias("");
      sessionStorage.removeItem(DRAFT_KEY);
      props.onCreated(created.code);
    } catch (err) {
      if (err instanceof ApiError && err.status === 401) {
        props.onUnauthorized(); // the draft stays in sessionStorage and comes back after login
        return;
      }
      if (err instanceof ApiError && err.code === "email_unverified") {
        props.onUnverified();
        return;
      }
      const fe = linkError(err, alias.trim());
      setError(fe);
      if (err instanceof ApiError && err.code === "rate_limited") {
        setWaitUntil(Date.now() + (err.retryAfter ?? 60) * 1000);
        setNow(Date.now());
      }
      (fe.field === "alias" ? aliasRef : urlRef).current?.focus();
    } finally {
      setBusy(false);
    }
  }

  const urlErr = error?.field === "url" ? error : null;
  const aliasErr = error?.field === "alias" ? error : null;
  const formErr = error?.field === "form" ? error : null;

  return (
    <form
      onSubmit={submit}
      noValidate
      className="flex flex-col gap-2 sm:flex-row sm:flex-wrap sm:items-start"
    >
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <label htmlFor={`${id}-url`}>URL ปลายทาง</label>
        <input
          ref={urlRef}
          id={`${id}-url`}
          className="control"
          type="url"
          inputMode="url"
          placeholder="https://"
          required
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          aria-invalid={urlErr ? true : undefined}
          aria-describedby={urlErr ? `${id}-url-err` : undefined}
        />
        {urlErr && (
          <p id={`${id}-url-err`} className="text-sm text-error">
            {urlErr.text}
          </p>
        )}
      </div>

      <div className="flex flex-col gap-1 sm:w-56">
        <label htmlFor={`${id}-alias`}>
          alias <span className="text-muted">(ไม่บังคับ)</span>
        </label>
        <input
          ref={aliasRef}
          id={`${id}-alias`}
          className="control font-mono"
          autoCapitalize="none"
          spellCheck={false}
          maxLength={32}
          value={alias}
          onChange={(e) => setAlias(e.target.value)}
          aria-invalid={aliasErr ? true : undefined}
          aria-describedby={aliasErr ? `${id}-alias-err` : undefined}
        />
        {aliasErr && (
          <p id={`${id}-alias-err`} className="text-sm text-error">
            {aliasErr.text}
          </p>
        )}
      </div>

      <div className="flex flex-col gap-1 sm:pt-7">
        <button
          type="submit"
          className="btn btn-primary w-full sm:w-auto"
          disabled={busy || waitSeconds > 0 || props.unverified}
          aria-describedby={props.unverified ? props.unverifiedBannerId : undefined}
        >
          {busy ? "กำลังย่อ..." : waitSeconds > 0 ? `รอ ${waitSeconds} วินาที` : "ย่อลิงก์"}
        </button>
      </div>

      <p role="alert" className="text-sm text-error sm:basis-full">
        {formErr?.text}
      </p>
    </form>
  );
}
