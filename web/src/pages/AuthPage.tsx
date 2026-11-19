// N1 login and N2 register (docs/design/pages/auth.md, account-email.md, flows.md).
import { useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { type FormEvent, useId, useState } from "react";
import { api, call } from "../api";
import { PasswordField } from "../components/PasswordField";
import { authCopy, authError, type FieldError } from "../messages";

type Mode = "login" | "register";

const copy = {
  login: {
    title: "เข้าสู่ระบบ",
    submit: "เข้าสู่ระบบ",
    busy: "กำลังเข้าสู่ระบบ...",
    switchText: "ยังไม่มีบัญชี?",
    switchTo: "/register",
    switchLabel: "สร้างบัญชี",
  },
  register: {
    title: "สร้างบัญชี",
    submit: "สร้างบัญชี",
    busy: "กำลังสร้างบัญชี...",
    switchText: "มีบัญชีแล้ว?",
    switchTo: "/login",
    switchLabel: "เข้าสู่ระบบ",
  },
} as const;

export function AuthPage({ mode }: { mode: Mode }) {
  const search = useSearch({ strict: false }) as { email?: string; expired?: boolean; reset?: boolean };
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [email, setEmail] = useState(search.email ?? "");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<FieldError | null>(null);
  const id = useId();
  const t = copy[mode];

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      const body = { email, password };
      const me = await call(
        mode === "login" ? api.POST("/api/auth/login", { body }) : api.POST("/api/auth/register", { body }),
      );
      queryClient.setQueryData(["me"], me);
      await navigate({ to: "/" });
    } catch (err) {
      setError(authError(err, mode));
      setPassword(""); // keep the email, clear the password (flows.md N1)
    } finally {
      setBusy(false);
    }
  }

  const emailErr = error?.field === "email" ? error : null;
  const passErr = error?.field === "password" ? error : null;
  const formErr = error?.field === "form" ? error : null;

  return (
    <main className="mx-auto w-full max-w-100 px-4 py-12">
      <p className="font-semibold text-muted">ย่อลิงก์</p>
      <h1 className="mt-1 text-xl font-semibold">{t.title}</h1>

      {search.expired && mode === "login" && (
        <p className="mt-4 rounded-control border border-line-strong p-3 text-sm">{authCopy.expired}</p>
      )}

      {search.reset && mode === "login" && (
        <p
          role="status"
          aria-live="polite"
          className="mt-4 rounded-control border border-line-strong p-3 text-sm"
        >
          {authCopy.resetSuccess}
        </p>
      )}

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
              {emailErr.text}{" "}
              {mode === "register" && emailErr.text.includes("มีบัญชีแล้ว") && (
                <Link
                  to="/login"
                  search={{ email, expired: undefined, reset: undefined }}
                  className="underline"
                >
                  เข้าสู่ระบบ
                </Link>
              )}
            </p>
          )}
        </div>

        <PasswordField
          id={`${id}-password`}
          label="รหัสผ่าน"
          autoComplete={mode === "login" ? "current-password" : "new-password"}
          minLength={mode === "register" ? 10 : undefined}
          value={password}
          onChange={setPassword}
          error={passErr?.text}
          hint={mode === "register" ? authCopy.passwordHint : undefined}
        />

        {mode === "login" && (
          <p className="-mt-2 text-sm">
            <Link
              to="/forgot-password"
              search={{ email: email.trim() || undefined }}
              className="text-brand underline"
            >
              {authCopy.forgotPasswordLink}
            </Link>
          </p>
        )}

        <p role="alert" className="text-sm text-error">
          {formErr?.text}
        </p>

        <button type="submit" className="btn btn-primary w-full" disabled={busy}>
          {busy ? t.busy : t.submit}
        </button>
      </form>

      <p className="mt-4 text-sm">
        {t.switchText}{" "}
        <Link
          to={t.switchTo}
          search={{ email: email || undefined, expired: undefined, reset: undefined }}
          className="text-brand underline"
        >
          {t.switchLabel}
        </Link>
      </p>
    </main>
  );
}
