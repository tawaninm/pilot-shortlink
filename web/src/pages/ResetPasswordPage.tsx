import { useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { type FormEvent, useId, useState } from "react";
import { api, call } from "../api";
import { PasswordField } from "../components/PasswordField";
import { authCopy, type FieldError, resetPasswordCopy, resetPasswordError } from "../messages";
import { useEmailToken } from "./token";

export function ResetPasswordPage() {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { token, valid } = useEmailToken();
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<(FieldError & { tokenInvalid?: boolean }) | null>(
    valid ? null : { field: "form", text: resetPasswordCopy.tokenInvalid, tokenInvalid: true },
  );
  const id = useId();

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!token || busy) return;
    setBusy(true);
    setError(null);
    try {
      await call(api.POST("/api/auth/password/reset", { body: { token, password } }));
      queryClient.clear();
      await navigate({ to: "/login", search: { email: undefined, expired: undefined, reset: true } });
    } catch (err) {
      setError(resetPasswordError(err));
    } finally {
      setBusy(false);
    }
  }

  const passErr = error?.field === "password" ? error : null;
  const formErr = error?.field === "form" ? error : null;

  return (
    <main className="mx-auto w-full max-w-100 px-4 py-12">
      <p className="font-semibold text-muted">ย่อลิงก์</p>
      <h1 className="mt-1 text-xl font-semibold">{resetPasswordCopy.title}</h1>

      {valid ? (
        <form className="mt-6 flex flex-col gap-4" onSubmit={submit} noValidate>
          <PasswordField
            id={`${id}-password`}
            label={resetPasswordCopy.passwordLabel}
            autoComplete="new-password"
            minLength={10}
            value={password}
            onChange={setPassword}
            error={passErr?.text}
            hint={authCopy.passwordHint}
          />

          {formErr && (
            <p role="alert" aria-live="polite" className="text-sm text-error">
              {formErr.text}{" "}
              {formErr.tokenInvalid && (
                <Link to="/forgot-password" search={{ email: undefined }} className="text-brand underline">
                  {resetPasswordCopy.requestNewLink}
                </Link>
              )}
            </p>
          )}

          <button type="submit" className="btn btn-primary w-full" disabled={busy}>
            {busy ? resetPasswordCopy.busy : resetPasswordCopy.submit}
          </button>
        </form>
      ) : (
        <div className="mt-6 flex flex-col gap-4">
          <p role="alert" aria-live="polite" className="text-sm text-error">
            {resetPasswordCopy.tokenInvalid}
          </p>
          <p className="text-sm">
            <Link to="/forgot-password" search={{ email: undefined }} className="text-brand underline">
              {resetPasswordCopy.requestNewLink}
            </Link>
          </p>
        </div>
      )}
    </main>
  );
}
