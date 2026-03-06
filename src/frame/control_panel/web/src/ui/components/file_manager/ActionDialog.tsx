import type { FormEventHandler, ReactNode } from 'react'
import { useEffect } from 'react'

type ActionDialogProps = {
  open: boolean
  title: string
  description?: ReactNode
  children?: ReactNode
  cancelLabel?: string
  confirmLabel: string
  confirmTone?: 'primary' | 'danger'
  confirmDisabled?: boolean
  busy?: boolean
  onCancel: () => void
  onConfirm?: () => void
  onSubmit?: FormEventHandler<HTMLFormElement>
}

const ActionDialog = ({
  open,
  title,
  description,
  children,
  cancelLabel = 'Cancel',
  confirmLabel,
  confirmTone = 'primary',
  confirmDisabled = false,
  busy = false,
  onCancel,
  onConfirm,
  onSubmit,
}: ActionDialogProps) => {
  useEffect(() => {
    if (!open || busy) {
      return
    }

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        onCancel()
      }
    }

    window.addEventListener('keydown', onKeyDown)
    return () => {
      window.removeEventListener('keydown', onKeyDown)
    }
  }, [busy, onCancel, open])

  if (!open) {
    return null
  }

  const confirmClassName =
    confirmTone === 'danger'
      ? 'rounded-xl bg-rose-600 px-4 py-2 text-sm font-semibold text-white transition hover:bg-rose-700 disabled:cursor-not-allowed disabled:opacity-60'
      : 'rounded-xl bg-primary px-4 py-2 text-sm font-semibold text-white transition hover:bg-teal-700 disabled:cursor-not-allowed disabled:opacity-60'

  const body = (
    <>
      <div className="space-y-3">
        <h3 className="text-lg font-semibold text-slate-900">{title}</h3>
        {description ? <div className="text-sm leading-relaxed text-slate-600">{description}</div> : null}
        {children}
      </div>

      <div className="mt-5 flex items-center justify-end gap-2">
        <button
          type="button"
          onClick={onCancel}
          disabled={busy}
          className="rounded-xl border border-slate-300 px-4 py-2 text-sm font-semibold text-slate-700 transition hover:border-primary hover:text-primary disabled:cursor-not-allowed disabled:opacity-60"
        >
          {cancelLabel}
        </button>
        <button
          type={onSubmit ? 'submit' : 'button'}
          onClick={onSubmit ? undefined : onConfirm}
          disabled={busy || confirmDisabled}
          className={confirmClassName}
        >
          {confirmLabel}
        </button>
      </div>
    </>
  )

  return (
    <div
      className="fixed inset-0 z-[90] flex items-center justify-center bg-slate-900/45 px-4 py-6 backdrop-blur-sm"
      onMouseDown={(event) => {
        if (busy) {
          return
        }
        if (event.target === event.currentTarget) {
          onCancel()
        }
      }}
    >
      {onSubmit ? (
        <form
          onSubmit={onSubmit}
          role="dialog"
          aria-modal="true"
          aria-label={title}
          className="w-full max-w-lg rounded-3xl border border-slate-200 bg-white p-5 shadow-2xl"
        >
          {body}
        </form>
      ) : (
        <div
          role="dialog"
          aria-modal="true"
          aria-label={title}
          className="w-full max-w-lg rounded-3xl border border-slate-200 bg-white p-5 shadow-2xl"
        >
          {body}
        </div>
      )}
    </div>
  )
}

export default ActionDialog
