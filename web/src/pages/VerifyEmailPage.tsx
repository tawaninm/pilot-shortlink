import { useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useState } from "react";
import { api, call } from "../api";
import { verifyEmailCopy, verifyEmailError } from "../messages";
import { useEmailToken } from "./token";

export function VerifyEmailPage() {
  const queryClient = useQueryClient();
  const { token, valid } = useEmailToken();
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState(false);
  const [error, setError] = useState<{ text: string; tokenInvalid?: boolean } | null>(
    valid ? null : { text: verifyEmailCopy.tokenInvalid, tokenInvalid: true },
  );

  async function verify() {
    if (!token || busy) return;
    setBusy(true);
    setError(null);
    try {
      await call(api.POST("/api/auth/verify-email", { body: { token } }));
      setDone(true);
      void queryClient.invalidateQueries({ queryKey: ["me"] });
    } catch (err) {
      setError(verifyEmailError(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <main className="mx-auto w-full max-w-100 px-4 py-12">
      <p className="font-semibold text-muted">ย่อลิงก์</p>

      {done ? (
        <>
          <h1 className="mt-1 text-xl font-semibold">{verifyEmailCopy.successTitle}</h1>
          <p className="mt-6">
            <Link to="/" className="text-brand underline">
              {verifyEmailCopy.toLinks}
            </Link>
          </p>
        </>
      ) : (
        <>
          <h1 className="mt-1 text-xl font-semibold">{verifyEmailCopy.title}</h1>
          {error && (
            <p role="alert" aria-live="polite" className="mt-6 text-sm text-error">
              {error.text}
            </p>
          )}
          {valid && !error?.tokenInvalid ? (
            <div className="mt-6">
              <button type="button" className="btn btn-primary w-full" disabled={busy} onClick={verify}>
                {busy ? verifyEmailCopy.busy : verifyEmailCopy.submit}
              </button>
            </div>
          ) : (
            <p className="mt-4 text-sm">
              <Link to="/" className="text-brand underline">
                {verifyEmailCopy.toLinks}
              </Link>
            </p>
          )}
        </>
      )}
    </main>
  );
}
