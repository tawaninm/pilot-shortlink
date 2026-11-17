// N5 confirm delete: native <dialog> (focus trap, Esc to close), "ยกเลิก" gets focus first.
import { useEffect, useRef, useState } from "react";

export function ConfirmDelete(props: { code: string; onCancel: () => void; onConfirm: () => Promise<void> }) {
  const ref = useRef<HTMLDialogElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    const dialog = ref.current;
    dialog?.showModal();
    cancelRef.current?.focus(); // the safe choice gets focus first (flows.md N5)
    return () => dialog?.close();
  }, []);

  return (
    <dialog
      ref={ref}
      aria-labelledby={`del-${props.code}`}
      className="m-auto max-w-md rounded-card border border-line-strong bg-raised p-6 text-fg backdrop:bg-fg/40"
      onCancel={props.onCancel}
    >
      <h2 id={`del-${props.code}`} className="text-lg font-semibold">
        ลบ /{props.code}?
      </h2>
      <p className="mt-2">ลิงก์จะใช้ไม่ได้ทันทีและสถิติจะหายด้วย ย้อนกลับไม่ได้</p>
      <div className="mt-6 flex justify-end gap-2">
        <button
          ref={cancelRef}
          type="button"
          className="btn btn-quiet border border-line-strong"
          onClick={props.onCancel}
        >
          ยกเลิก
        </button>
        <button
          type="button"
          className="btn btn-danger"
          disabled={busy}
          onClick={async () => {
            setBusy(true);
            await props.onConfirm();
          }}
        >
          {busy ? "กำลังลบ..." : "ลบ"}
        </button>
      </div>
    </dialog>
  );
}
