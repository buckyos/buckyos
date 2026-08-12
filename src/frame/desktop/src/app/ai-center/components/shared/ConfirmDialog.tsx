import { useI18n } from '../../../../i18n/provider'

interface ConfirmDialogProps {
  open: boolean
  title: string
  message: string
  confirmLabel?: string
  confirming?: boolean
  error?: string | null
  onConfirm: () => void
  onCancel: () => void
}

export function ConfirmDialog({
  open,
  title,
  message,
  confirmLabel,
  confirming = false,
  error,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const { t } = useI18n()

  if (!open) return null

  return (
    <div className="fixed inset-0 z-50 flex items-end justify-center md:items-center">
      <div
        className="absolute inset-0"
        style={{ background: 'rgba(0,0,0,0.4)' }}
        onClick={onCancel}
      />
      <div
        className="relative max-h-[calc(100dvh-1rem)] w-full overflow-y-auto rounded-t-xl p-6 shadow-lg md:mx-4 md:max-w-sm md:rounded-xl"
        style={{ background: 'var(--cp-surface)' }}
      >
        <h3
          className="text-base font-semibold mb-2"
          style={{ color: 'var(--cp-text)' }}
        >
          {title}
        </h3>
        <p className="text-sm mb-6" style={{ color: 'var(--cp-muted)' }}>
          {message}
        </p>
        <div className="flex flex-col-reverse gap-2 sm:flex-row sm:justify-end sm:gap-3">
          <button
            type="button"
            onClick={onCancel}
            disabled={confirming}
            className="min-h-11 px-4 py-2 rounded-lg text-sm"
            style={{ color: 'var(--cp-muted)' }}
          >
            {t('common.cancel', 'Cancel')}
          </button>
          <button
            type="button"
            onClick={onConfirm}
            disabled={confirming}
            className="min-h-11 px-4 py-2 rounded-lg text-sm font-medium disabled:opacity-60"
            style={{ background: 'var(--cp-danger)', color: '#fff' }}
          >
            {confirming ? t('common.deleting', 'Deleting') : confirmLabel ?? t('common.confirm', 'Confirm')}
          </button>
        </div>
        {error && (
          <div className="mt-3 text-xs" style={{ color: 'var(--cp-danger)' }}>
            {error}
          </div>
        )}
      </div>
    </div>
  )
}
