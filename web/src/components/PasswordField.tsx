import { useState } from "react";
import { authCopy } from "../messages";

export function PasswordField(props: {
  id: string;
  label: string;
  autoComplete: "current-password" | "new-password";
  minLength?: number;
  value: string;
  onChange: (value: string) => void;
  error?: string;
  hint?: string;
}) {
  const [show, setShow] = useState(false);
  const hintId = `${props.id}-hint`;
  const hasMessage = Boolean(props.error || props.hint);

  return (
    <div className="flex flex-col gap-1">
      <label htmlFor={props.id}>{props.label}</label>
      <div className="flex gap-2">
        <input
          id={props.id}
          className="control min-w-0 flex-1"
          type={show ? "text" : "password"}
          autoComplete={props.autoComplete}
          required
          minLength={props.minLength}
          value={props.value}
          onChange={(e) => props.onChange(e.target.value)}
          aria-invalid={props.error ? true : undefined}
          aria-describedby={hasMessage ? hintId : undefined}
        />
        <button
          type="button"
          className="btn btn-quiet"
          aria-pressed={show}
          onClick={() => setShow((v) => !v)}
        >
          {show ? authCopy.hidePassword : authCopy.showPassword}
        </button>
      </div>
      <p id={hintId} className={`text-sm ${props.error ? "text-error" : "text-muted"}`}>
        {props.error ?? props.hint ?? ""}
      </p>
    </div>
  );
}
