import Icon from '../icons'

type MessageModalProps = {
  open: boolean
  tone: 'success' | 'error'
  title: string
  message: string
  showConfirm?: boolean
  confirmLabel?: string
  onConfirm: () => void
}

const MessageModal = ({
  open,
  tone,
  title,
  message,
  showConfirm = true,
  confirmLabel,
  onConfirm,
}: MessageModalProps) => {
  if (!open) {
    return null
  }

  const isError = tone === 'error'

  return (
    <div className="fixed inset-0 z-[90] flex items-center justify-center bg-slate-900/45 px-4 py-6 backdrop-blur-sm">
      <div role="dialog" aria-modal="true" className="w-full max-w-md rounded-3xl border border-[var(--cp-border)] bg-white p-6 shadow-2xl">
        <div className="flex items-start gap-3">
          <span
            className={`inline-flex size-10 shrink-0 items-center justify-center rounded-2xl ${
              isError ? 'bg-rose-50 text-rose-600' : 'bg-emerald-50 text-emerald-600'
            }`}
          >
            <Icon name={isError ? 'alert' : 'spark'} className="size-5" />
          </span>
          <div className="min-w-0 flex-1">
            <p className="text-base font-semibold text-[var(--cp-ink)]">{title}</p>
            <p className="mt-1 text-sm leading-relaxed text-[var(--cp-muted)]">{message}</p>
          </div>
        </div>

        {showConfirm ? (
          <div className="mt-5 flex justify-end">
            <button
              type="button"
              onClick={onConfirm}
              className={`inline-flex items-center rounded-xl px-4 py-2 text-sm font-semibold text-white transition ${
                isError ? 'bg-rose-600 hover:bg-rose-700' : 'bg-[var(--cp-primary)] hover:bg-[var(--cp-primary-strong)]'
              }`}
            >
              {confirmLabel ?? 'OK'}
            </button>
          </div>
        ) : null}
      </div>
    </div>
  )
}

export default MessageModal
