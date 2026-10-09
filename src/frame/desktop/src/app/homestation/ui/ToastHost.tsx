import { CheckCircle2, Info, TriangleAlert, X } from 'lucide-react'
import { useCallback, useState, type ReactNode } from 'react'
import { useI18n } from '../../../i18n/provider'
import { ToastContext, type ToastSpec } from './toastContext'

interface ActiveToast extends ToastSpec {
  id: number
}

let nextToastId = 1

export function ToastHost({ children }: { children: ReactNode }) {
  const { t } = useI18n()
  const [toasts, setToasts] = useState<ActiveToast[]>([])

  const dismiss = useCallback((id: number) => setToasts(current => current.filter(toast => toast.id !== id)), [])

  const show = useCallback((toast: ToastSpec) => {
    const id = nextToastId++
    setToasts(current => [...current.slice(-2), { ...toast, id }])
    window.setTimeout(() => dismiss(id), toast.action ? 6000 : 4000)
  }, [dismiss])

  return (
    <ToastContext.Provider value={show}>
      {children}
      <div className="pointer-events-none absolute inset-x-0 bottom-4 z-[95] flex flex-col items-center gap-2 px-4" style={{ paddingBottom: 'var(--sab)' }}>
        {toasts.map(toast => (
          <div
            key={toast.id}
            role="status"
            data-testid="hs-toast"
            className="pointer-events-auto flex w-full max-w-md items-start gap-2 rounded-2xl border px-3 py-2.5 text-sm"
            style={{ borderColor: 'var(--cp-border)', background: 'var(--hs-panel-bg)', boxShadow: 'var(--cp-panel-shadow)' }}
          >
            <span className="mt-0.5 flex-shrink-0" style={{ color: toast.tone === 'warning' ? 'var(--cp-warning)' : toast.tone === 'success' ? 'var(--cp-success)' : 'var(--cp-accent)' }}>
              {toast.tone === 'warning' ? <TriangleAlert size={16} /> : toast.tone === 'success' ? <CheckCircle2 size={16} /> : <Info size={16} />}
            </span>
            <span className="min-w-0 flex-1 leading-5">{toast.text}</span>
            {toast.action ? (
              <button
                type="button"
                className="flex-shrink-0 text-sm font-semibold"
                style={{ color: 'var(--cp-accent)' }}
                onClick={() => {
                  toast.action?.onClick()
                  dismiss(toast.id)
                }}
              >
                {toast.action.label}
              </button>
            ) : null}
            <button type="button" aria-label={t('common.close', 'Close')} className="flex-shrink-0" style={{ color: 'var(--cp-muted)' }} onClick={() => dismiss(toast.id)}>
              <X size={14} />
            </button>
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  )
}
