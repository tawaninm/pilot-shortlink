// N4 stats under a row: loading, empty, error, 404 (deleted in another tab) and data (flows.md).
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { ApiError, api, call } from "../api";
import { number } from "../messages";
import { CopyButton } from "./LinkTable";

export function LinkStats({ code, onGone }: { code: string; onGone: () => void }) {
  const queryClient = useQueryClient();
  const stats = useQuery({
    queryKey: ["stats", code],
    queryFn: () => call(api.GET("/api/links/{code}/stats", { params: { path: { code } } })),
  });
  const gone = stats.error instanceof ApiError && stats.error.status === 404;

  useEffect(() => {
    if (!gone) return;
    const t = setTimeout(() => {
      onGone();
      void queryClient.invalidateQueries({ queryKey: ["links"] });
    }, 2000);
    return () => clearTimeout(t);
  }, [gone, onGone, queryClient]);

  if (stats.isPending) {
    return (
      <div role="status" className="skeleton mt-2 h-48">
        <span className="sr-only">กำลังโหลดสถิติ</span>
      </div>
    );
  }
  if (gone) {
    return (
      <p role="alert" className="mt-2">
        ลิงก์นี้ถูกลบแล้ว
      </p>
    );
  }
  if (stats.isError) {
    return (
      <div role="alert" className="mt-2 flex items-center gap-4">
        <p>โหลดสถิติไม่ได้</p>
        <button
          type="button"
          className="btn btn-quiet border border-line-strong"
          onClick={() => void stats.refetch()}
        >
          ลองอีกครั้ง
        </button>
      </div>
    );
  }

  const { clicksTotal, daily, referrers } = stats.data;
  if (clicksTotal === 0) {
    return (
      <div className="mt-2 flex items-center gap-2">
        <p>ยังไม่มีใครเปิดลิงก์นี้</p>
        <CopyButton code={code} />
      </div>
    );
  }

  const max = Math.max(1, ...daily.map((d) => d.clicks));
  const refMax = Math.max(1, ...referrers.map((r) => r.clicks));
  return (
    <div className="mt-2 grid gap-6 rounded-card bg-sunken p-4 sm:grid-cols-[2fr_1fr]">
      <section aria-label="คลิกรายวัน 30 วัน">
        <p>
          <span className="font-mono text-xl">{number.format(clicksTotal)}</span> คลิกทั้งหมด
        </p>
        <div className="mt-3 flex h-24 items-end gap-px" aria-hidden="true">
          {daily.map((d) => (
            <span
              key={d.date}
              className="flex-1 rounded-t-badge bg-brand"
              style={{ height: `${Math.max(2, (d.clicks / max) * 100)}%`, opacity: d.clicks ? 1 : 0.25 }}
            />
          ))}
        </div>
        <table className="sr-only">
          <caption>คลิกรายวัน</caption>
          <tbody>
            {daily.map((d) => (
              <tr key={d.date}>
                <th scope="row">{d.date}</th>
                <td>{d.clicks} คลิก</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>

      <section aria-labelledby={`ref-${code}`}>
        <h3 id={`ref-${code}`} className="text-sm font-semibold">
          มาจากที่ไหน
        </h3>
        <ul className="mt-2 flex flex-col gap-2">
          {referrers.map((r) => (
            <li key={r.host ?? "(unknown)"} className="text-sm">
              <div className="flex justify-between gap-2">
                <span className="wrap-anywhere">{r.host ?? "ไม่ทราบที่มา"}</span>
                <span className="font-mono">{number.format(r.clicks)}</span>
              </div>
              <div className="mt-1 h-1.5 rounded-badge bg-raised" aria-hidden="true">
                <div
                  className="h-full rounded-badge bg-brand"
                  style={{ width: `${(r.clicks / refMax) * 100}%` }}
                />
              </div>
            </li>
          ))}
        </ul>
      </section>
    </div>
  );
}
