import { useI18n } from '../../../../i18n/provider'

interface ConfirmDialogProps {
  open: boolean
  title: string
  message: string
  confirmLabel?: string
  onConfirm: () => void
  onCancel: () => void
}

export function ConfirmDialog({
  open,
  title,
  message,
  confirmLabel,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const { t } = useI18n()

  if (!open) return null

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      <div
        className="absolute inset-0"
        style={{ background: 'rgba(0,0,0,0.4)' }}
        onClick={onCancel}
      />
      <div
        className="relative rounded-xl p-6 max-w-sm w-full mx-4 shadow-lg"
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
        <div className="flex justify-end gap-3">
          <button
            type="button"
            onClick={onCancel}
            className="px-4 py-2 rounded-lg text-sm"
            style={{ color: 'var(--cp-muted)' }}
          >
            {t('common.cancel', 'Cancel')}
          </button>
          <button
            type="button"
            onClick={onConfirm}
            className="px-4 py-2 rounded-lg text-sm font-medium"
            style={{ background: 'var(--cp-danger)', color: '#fff' }}
          >
            {confirmLabel ?? t('common.confirm', 'Confirm')}
          </button>
        </div>
      </div>
    </div>
  )
}
