// N3 links page (docs/design/pages/links.md). Stats open under the chosen row; no page change.
import { SignOut } from "@phosphor-icons/react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { ApiError, api, call } from "../api";
import { CreateLinkForm } from "../components/CreateLinkForm";
import { LinkTable } from "../components/LinkTable";

const unauthorized = (e: unknown) => e instanceof ApiError && e.status === 401;

export function LinksPage() {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [announce, setAnnounce] = useState("");
  const [fresh, setFresh] = useState<string | null>(null);

  const me = useQuery({ queryKey: ["me"], queryFn: () => call(api.GET("/api/me")) });
  const links = useQuery({
    queryKey: ["links"],
    queryFn: () => call(api.GET("/api/links")),
    enabled: me.isSuccess,
  });

  // Not logged in on arrival: go to login. Logged out mid-session (N8): go to login with the expired note.
  useEffect(() => {
    if (unauthorized(me.error))
      void navigate({ to: "/login", search: { email: undefined, expired: undefined } });
  }, [me.error, navigate]);
  useEffect(() => {
    if (unauthorized(links.error)) {
      queryClient.removeQueries({ queryKey: ["me"] });
      void navigate({ to: "/login", search: { email: undefined, expired: true } });
    }
  }, [links.error, navigate, queryClient]);

  async function logout() {
    await call(api.POST("/api/auth/logout")).catch(() => undefined); // logout is idempotent; leave either way
    queryClient.clear();
    await navigate({ to: "/login", search: { email: undefined, expired: undefined } });
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
        <CreateLinkForm
          onCreated={(code) => {
            setFresh(code);
            setAnnounce(`สร้างลิงก์ /${code} แล้ว`);
            void queryClient.invalidateQueries({ queryKey: ["links"] });
          }}
          onUnauthorized={() => navigate({ to: "/login", search: { email: undefined, expired: true } })}
        />

        <h2 className="mt-8 text-lg font-semibold">ลิงก์ของฉัน{links.isSuccess ? ` (${items.length})` : ""}</h2>
        <LinkTable
          state={
            links.isPending ? "loading" : links.isError ? "error" : items.length === 0 ? "empty" : "ready"
          }
          items={items}
          freshCode={fresh}
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
