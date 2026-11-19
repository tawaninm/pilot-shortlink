// LinkTable / LinkRow / Sparkline / CopyButton / QrButton (DESIGN.md inventory, account-email.md).
// The code cell is a real <button> that opens the stats row, so keyboard and screen reader users get aria-expanded.
import { CaretDown, Check, Copy, QrCode, Trash } from "@phosphor-icons/react";
import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { ApiError, api, call, type LinkSummary, shortUrl } from "../api";
import { number, qrDownloadLabel } from "../messages";
import { ConfirmDelete } from "./ConfirmDelete";
import { LinkStats } from "./LinkStats";

type TableState = "loading" | "error" | "empty" | "ready";

export function LinkTable(props: {
  state: TableState;
  items: LinkSummary[];
  freshCode: string | null;
  unverified?: boolean;
  unverifiedBannerId?: string;
  onUnverified: () => void;
  onRetry: () => void;
  onAnnounce: (text: string) => void;
}) {
  const [open, setOpen] = useState<string | null>(null);

  if (props.state === "loading") {
    return (
      <ul className="mt-4" aria-busy="true" aria-label="กำลังโหลดรายการลิงก์">
        {[1, 2, 3, 4, 5].map((n) => (
          <li key={n} className="row flex items-center gap-4">
            <span className="skeleton h-4 w-24" />
            <span className="skeleton h-4 flex-1" />
          </li>
        ))}
      </ul>
    );
  }
  if (props.state === "error") {
    return (
      <div className="mt-4 flex items-center gap-4" role="alert">
        <p>โหลดรายการลิงก์ไม่ได้</p>
        <button type="button" className="btn btn-quiet border border-line-strong" onClick={props.onRetry}>
          ลองอีกครั้ง
        </button>
      </div>
    );
  }
  if (props.state === "empty") {
    return (
      <div className="mt-4">
        <p className="font-semibold">ยังไม่มีลิงก์</p>
        <p className="text-muted">วาง URL ด้านบนเพื่อสร้างลิงก์แรก แล้วดูได้ว่ามีคนเปิดกี่ครั้ง</p>
      </div>
    );
  }

  return (
    <table className="mt-4 w-full border-collapse text-left">
      <thead className="max-sm:sr-only">
        <tr className="text-sm text-muted">
          <th scope="col" className="py-2 font-medium">
            ลิงก์สั้น
          </th>
          <th scope="col" className="py-2 font-medium">
            ปลายทาง
          </th>
          <th scope="col" className="py-2 font-medium max-sm:hidden">
            30 วัน
          </th>
          <th scope="col" className="py-2 text-right font-medium">
            คลิก
          </th>
          <th scope="col" className="py-2 max-sm:hidden">
            <span className="sr-only">การกระทำ</span>
          </th>
        </tr>
      </thead>
      <tbody>
        {props.items.map((link) => (
          <LinkRow
            key={link.code}
            link={link}
            fresh={link.code === props.freshCode}
            open={open === link.code}
            unverified={props.unverified}
            unverifiedBannerId={props.unverifiedBannerId}
            onUnverified={props.onUnverified}
            onToggle={() => setOpen((c) => (c === link.code ? null : link.code))}
            onClose={() => setOpen(null)}
            onAnnounce={props.onAnnounce}
          />
        ))}
      </tbody>
    </table>
  );
}

function LinkRow(props: {
  link: LinkSummary;
  fresh: boolean;
  open: boolean;
  unverified?: boolean;
  unverifiedBannerId?: string;
  onUnverified: () => void;
  onToggle: () => void;
  onClose: () => void;
  onAnnounce: (text: string) => void;
}) {
  const { link } = props;
  const queryClient = useQueryClient();
  const [confirming, setConfirming] = useState(false);
  const [deleteError, setDeleteError] = useState("");
  const copyRef = useRef<HTMLButtonElement>(null);
  const statsId = `stats-${link.code}`;

  // A new link gets focus on its copy button (flows.md: "ปุ่มคัดลอกได้ focus").
  useEffect(() => {
    if (props.fresh) copyRef.current?.focus();
  }, [props.fresh]);

  async function remove() {
    try {
      await call(api.DELETE("/api/links/{code}", { params: { path: { code: link.code } } }));
      setConfirming(false);
      props.onAnnounce(`ลบ /${link.code} แล้ว`);
      await queryClient.invalidateQueries({ queryKey: ["links"] });
    } catch (err) {
      setConfirming(false);
      if (err instanceof ApiError && err.code === "email_unverified") {
        props.onUnverified();
        return;
      }
      setDeleteError("ลบไม่สำเร็จ ลิงก์ยังใช้งานได้ ลองอีกครั้ง");
    }
  }

  const actions = (
    <div className="flex justify-end gap-1">
      <CopyButton code={link.code} buttonRef={copyRef} />
      <QrButton code={link.code} />
      <button
        type="button"
        className="btn btn-quiet px-0"
        aria-label={`ลบ /${link.code}`}
        disabled={props.unverified}
        aria-describedby={props.unverified ? props.unverifiedBannerId : undefined}
        onClick={() => {
          setDeleteError("");
          setConfirming(true);
        }}
      >
        <Trash size={20} aria-hidden="true" />
      </button>
    </div>
  );

  return (
    <>
      <tr
        className={`row align-middle max-sm:grid max-sm:grid-cols-[1fr_auto] max-sm:items-center ${props.fresh ? "bg-sunken" : ""}`}
      >
        <td className="py-2 pr-4">
          <button
            type="button"
            className="btn btn-quiet whitespace-nowrap sm:-ml-4 font-mono"
            aria-expanded={props.open}
            aria-controls={statsId}
            onClick={props.onToggle}
            onKeyDown={(e) => e.key === "Escape" && props.onClose()}
          >
            /{link.code}
            <CaretDown size={16} aria-hidden="true" className={props.open ? "rotate-180" : ""} />
          </button>
        </td>
        <td className="wrap-anywhere py-2 pr-4 text-muted max-sm:order-3 max-sm:col-span-2 max-sm:pt-0 max-sm:pb-3 max-sm:text-sm">
          {link.url}
        </td>
        <td className="py-2 pr-4 max-sm:hidden">
          <Sparkline values={link.last30Days} />
        </td>
        <td className="py-2 text-right font-mono max-sm:order-2">
          {number.format(link.clicksTotal)} <span className="sr-only">คลิก</span>
        </td>
        <td className="py-2 max-sm:hidden">{actions}</td>
      </tr>
      {deleteError && (
        <tr>
          <td colSpan={5} role="alert" className="pb-2 text-sm text-error">
            {deleteError}
          </td>
        </tr>
      )}
      {props.open && (
        <tr id={statsId}>
          <td colSpan={5} className="pb-4">
            <div className="sm:hidden">{actions}</div>
            <LinkStats code={link.code} onGone={props.onClose} />
          </td>
        </tr>
      )}
      {confirming && (
        <ConfirmDelete code={link.code} onCancel={() => setConfirming(false)} onConfirm={remove} />
      )}
    </>
  );
}

/** 30 daily values as a line; all zeros draws a flat line (DESIGN.md). Decorative: the number column says the total. */
export function Sparkline({ values }: { values: number[] }) {
  const max = Math.max(1, ...values);
  const points = values.map((v, i) => `${(i / (values.length - 1)) * 100},${28 - (v / max) * 24}`).join(" ");
  return (
    <svg viewBox="0 0 100 30" className="h-6 w-24" aria-hidden="true" focusable="false">
      <polyline points={points} fill="none" stroke="var(--component-sparkline-stroke)" strokeWidth="2" />
    </svg>
  );
}

export function CopyButton({ code, buttonRef }: { code: string; buttonRef?: React.Ref<HTMLButtonElement> }) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const t = setTimeout(() => setCopied(false), 2000);
    return () => clearTimeout(t);
  }, [copied]);

  return (
    <button
      ref={buttonRef}
      type="button"
      className="btn btn-quiet px-0"
      aria-label={copied ? `คัดลอก /${code} แล้ว` : `คัดลอก /${code}`}
      onClick={async () => {
        await navigator.clipboard.writeText(shortUrl(code)).catch(() => undefined);
        setCopied(true);
      }}
    >
      {copied ? <Check size={20} aria-hidden="true" /> : <Copy size={20} aria-hidden="true" />}
    </button>
  );
}

function QrButton({ code }: { code: string }) {
  const [busy, setBusy] = useState(false);

  async function download() {
    if (busy) return;
    setBusy(true);
    try {
      const { default: qrcode } = await import("qrcode");
      const svg = await qrcode.toString(shortUrl(code), { type: "svg", margin: 2 });
      const blob = new Blob([svg], { type: "image/svg+xml;charset=utf-8" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `${code}.svg`;
      document.body.append(a);
      a.click();
      a.remove();
      setTimeout(() => URL.revokeObjectURL(url), 1000);
    } finally {
      setBusy(false);
    }
  }

  return (
    <button
      type="button"
      className="btn btn-quiet px-0"
      aria-label={qrDownloadLabel(code)}
      disabled={busy}
      onClick={download}
    >
      <QrCode size={20} aria-hidden="true" />
    </button>
  );
}
