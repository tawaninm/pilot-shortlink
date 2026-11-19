// N3 links page (docs/design/pages/links.md, account-email.md). Stats open under the chosen row; no page change.
import { SignOut } from "@phosphor-icons/react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useEffect, useId, useRef, useState } from "react";
import { ApiError, api, call } from "../api";
import { CreateLinkForm } from "../components/CreateLinkForm";
import { LinkTable } from "../components/LinkTable";
import { resendEmailError, verifyBannerCopy } from "../messages";

const unauthorized = (e: unknown) => e instanceof ApiError && e.status === 401;

export function LinksPage() {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [announce, setAnnounce] = useState("");
  const [fresh, setFresh] = useState<string | null>(null);
  const [resendState, setResendState] = useState<
    { kind: "idle" } | { kind: "busy" } | { kind: "sent"; email: string } | { kind: "error"; text: string }
  >({ kind: "idle" });
  const verifyBannerId = useId();
  const resendStatusRef = useRef<HTMLParagraphElement>(null);

  useEffect(() => {
    if (resendState.kind === "sent") resendStatusRef.current?.focus();
  }, [resendState.kind]);

  const me = useQuery({ queryKey: ["me"], queryFn: () => call(api.GET("/api/me")) });
  const links = useQuery({
    queryKey: ["links"],
    queryFn: () => call(api.GET("/api/links")),
    enabled: me.isSuccess,
  });

  // Not logged in on arrival: go to login. Logged out mid-session (N8): go to login with the expired note.
  useEffect(() => {
    if (unauthorized(me.error))
      void navigate({ to: "/login", search: { email: undefined, expired: undefined, reset: undefined } });
  }, [me.error, navigate]);
  useEffect(() => {
    if (unauthorized(links.error)) {
      queryClient.removeQueries({ queryKey: ["me"] });
      void navigate({ to: "/login", search: { email: undefined, expired: true, reset: undefined } });
    }
  }, [links.error, navigate, queryClient]);

  async function logout() {
    await call(api.POST("/api/auth/logout")).catch(() => undefined); // logout is idempotent; leave either way
    queryClient.clear();
    await navigate({ to: "/login", search: { email: undefined, expired: undefined, reset: undefined } });
  }

  function markUnverified() {
    queryClient.setQueryData(["me"], (old: { email: string; emailVerified: boolean } | undefined) =>
      old ? { ...old, emailVerified: false } : old,
    );
  }

  async function resendVerification() {
    if (!me.data || resendState.kind === "busy") return;
    setResendState({ kind: "busy" });
    try {
      await call(api.POST("/api/auth/verify-email/resend"));
      setResendState({ kind: "sent", email: me.data.email });
    } catch (err) {
      if (unauthorized(err)) {
        queryClient.removeQueries({ queryKey: ["me"] });
        await navigate({ to: "/login", search: { email: undefined, expired: true, reset: undefined } });
        return;
      }
      setResendState({ kind: "error", text: resendEmailError(err) });
    }
  }

  if (!me.isSuccess) {
    return (
      <main className="mx-auto max-w-(--size-content-max) px-4 py-12" aria-busy="true">
        <p className="text-muted">
          {me.isError && !unauthorized(me.error) ? "โหลดไม่สำเร็จ ลองรีเฟรชหน้า" : "กำลังโหลด..."}
        </p>
      </main>
    );
  }

  const unverified = !me.data.emailVerified;
  const items = links.data?.items ?? [];

  return (
    <div className="mx-auto max-w-(--size-content-max) px-4 sm:px-8">
      <header className="flex items-center justify-between gap-4 py-4">
        <p className="whitespace-nowrap font-semibold">ย่อลิงก์</p>
        <div className="flex min-w-0 items-center gap-2">
          <span className="truncate text-sm text-muted">{me.data.email}</span>
          <button type="button" className="btn btn-quiet" onClick={logout}>
            <SignOut size={20} aria-hidden="true" />
            <span className="max-sm:sr-only">ออกจากระบบ</span>
          </button>
        </div>
      </header>

      <main className="pb-12">
        <h1 className="sr-only">ลิงก์ของฉัน</h1>

        {unverified && (
          <div
            role="status"
            aria-live="polite"
            className="mb-6 flex flex-wrap items-center justify-between gap-3 rounded-card border border-line-strong bg-raised p-4"
          >
            <div className="flex flex-col gap-1">
              <p id={verifyBannerId}>{verifyBannerCopy.unverified(me.data.email)}</p>
              {resendState.kind === "sent" && (
                <p ref={resendStatusRef} tabIndex={-1} className="text-sm">
                  {verifyBannerCopy.sent(resendState.email)}
                </p>
              )}
              {resendState.kind === "error" && (
                <p role="alert" className="text-sm text-error">
                  {resendState.text}
                </p>
              )}
            </div>
            {resendState.kind !== "sent" && (
              <button
                type="button"
                className="btn btn-quiet border border-line-strong"
                disabled={resendState.kind === "busy"}
                onClick={resendVerification}
              >
                {resendState.kind === "busy" ? verifyBannerCopy.sending : verifyBannerCopy.resend}
              </button>
            )}
          </div>
        )}

        <CreateLinkForm
          unverified={unverified}
          unverifiedBannerId={verifyBannerId}
          onUnverified={markUnverified}
          onCreated={(code) => {
            setFresh(code);
            setAnnounce(`สร้างลิงก์ /${code} แล้ว`);
            void queryClient.invalidateQueries({ queryKey: ["links"] });
          }}
          onUnauthorized={() =>
            navigate({ to: "/login", search: { email: undefined, expired: true, reset: undefined } })
          }
        />

        <h2 className="mt-8 text-lg font-semibold">ลิงก์ของฉัน{links.isSuccess ? ` (${items.length})` : ""}</h2>
        <LinkTable
          state={
            links.isPending ? "loading" : links.isError ? "error" : items.length === 0 ? "empty" : "ready"
          }
          items={items}
          freshCode={fresh}
          unverified={unverified}
          unverifiedBannerId={verifyBannerId}
          onUnverified={markUnverified}
          onRetry={() => void links.refetch()}
          onAnnounce={setAnnounce}
        />
      </main>

      <p aria-live="polite" className="sr-only">
        {announce}
      </p>
    </div>
  );
}
