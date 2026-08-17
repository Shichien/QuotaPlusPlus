import { X } from "lucide-react";

interface ConfirmDialogProps {
  title: string;
  message: string;
  confirmLabel: string;
  destructive?: boolean;
  busy?: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}

export function ConfirmDialog({
  title,
  message,
  confirmLabel,
  destructive = false,
  busy = false,
  onCancel,
  onConfirm,
}: ConfirmDialogProps) {
  return (
    <div className="dialog-backdrop" role="presentation" onMouseDown={onCancel}>
      <section
        className="dialog-panel confirm-panel"
        role="dialog"
        aria-modal="true"
        aria-labelledby="confirm-title"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <div className="dialog-heading">
          <h2 id="confirm-title">{title}</h2>
          <button
            className="icon-button"
            type="button"
            aria-label="关闭"
            title="关闭"
            disabled={busy}
            onClick={onCancel}
          >
            <X size={18} />
          </button>
        </div>
        <p className="confirm-message">{message}</p>
        <div className="dialog-actions">
          <button className="secondary-button" type="button" disabled={busy} onClick={onCancel}>
            取消
          </button>
          <button
            className={destructive ? "danger-button" : "primary-button"}
            type="button"
            disabled={busy}
            onClick={onConfirm}
          >
            {busy ? "处理中" : confirmLabel}
          </button>
        </div>
      </section>
    </div>
  );
}
