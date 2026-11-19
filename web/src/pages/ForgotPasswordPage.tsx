import { Link, useSearch } from "@tanstack/react-router";
import { type FormEvent, useId, useState } from "react";
import { api, call } from "../api";
import { authCopy, type FieldError, forgotPasswordCopy, forgotPasswordError } from "../messages";
import { useTurnstile } from "../useTurnstile";

export function ForgotPasswordPage() {
  const search = useSearch({ strict: false }) as { email?: string };
  const [email, setEmail] = useState(search.email ?? "");
  const [busy, setBusy] = useState(false);
  const [sentEmail, setSentEmail] = useState<string | null>(null);
  const [error, setError] = useState<FieldError | null>(null);
  const turnstile = useTurnstile(true);
  const id = useId();

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (busy || !turnstile.captcha) return;
    setBusy(true);
    setError(null);
    setSentEmail(null);
    const trimmed = email.trim();
    try {
      await call(
        api.POST("/api/auth/password/forgot", {
          body: { email: trimmed, captcha: turnstile.captcha },
        }),
      );
      setSentEmail(trimmed);
    } catch (err) {
      setError(forgotPasswordError(err));
    } finally {
      turnstile.reset();
      setBusy(false);
    }
  }

  const emailErr = error?.field === "email" ? error : null;
  const formErr = error?.field === "form" ? error : null;

  return (
    <main className="mx-auto w-full max-w-100 px-4 py-12">
      <p className="font-semibold text-muted">ย่อลิงก์</p>
      <h1 className="mt-1 text-xl font-semibold">{forgotPasswordCopy.title}</h1>

      <form className="mt-6 flex flex-col gap-4" onSubmit={submit} noValidate>
        <div className="flex flex-col gap-1">
          <label htmlFor={`${id}-email`}>email</label>
          <input
            id={`${id}-email`}
            className="control"
            type="email"
            autoComplete="email"
            required
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            aria-invalid={emailErr ? true : undefined}
            aria-describedby={emailErr ? `${id}-email-err` : undefined}
          />
          {emailErr && (
            <p id={`${id}-email-err`} className="text-sm text-error">
              {emailErr.text}
            </p>
          )}
        </div>

        {(formErr || turnstile.loadError) && (
          <p role="alert" aria-live="polite" className="text-sm text-error">
            {formErr?.text ?? authCopy.turnstileLoadError}
          </p>
        )}

        {sentEmail && (
          <p
            role="status"
            aria-live="polite"
            className="rounded-control border border-line-strong p-3 text-sm"
          >
            {forgotPasswordCopy.sent(sentEmail)}
          </p>
        )}

        {turnstile.active && <div ref={turnstile.ref} data-testid="turnstile" />}

        <button type="submit" className="btn btn-primary w-full" disabled={busy || !turnstile.ready}>
          {busy ? forgotPasswordCopy.busy : forgotPasswordCopy.submit}
        </button>
      </form>

      <p className="mt-4 text-sm">
        <Link
          to="/login"
          search={{ email: email.trim() || undefined, expired: undefined, reset: undefined }}
          className="text-brand underline"
        >
          {forgotPasswordCopy.backToLogin}
        </Link>
      </p>
    </main>
  );
}
