import { TriangleAlert, X } from 'lucide-react'
import { useCallback, useState, type ReactNode } from 'react'
import { useI18n } from '../../../i18n/provider'
import { ToastContext, type ToastSpec } from './toastContext'

interface ActiveToast extends ToastSpec {
  id: number
}

let nextToastId = 1

export function ToastHost({ children, bottom = 16 }: { children: ReactNode; bottom?: number }) {
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
      <div className="pointer-events-none absolute inset-x-0 z-[95] flex flex-col items-center gap-2 px-4" style={{ bottom: `calc(${bottom}px + var(--sab))` }}>
        {toasts.map(toast => (
          <div
            key={toast.id}
            role="status"
            data-testid="hs-toast"
            data-tone={toast.tone ?? 'info'}
            className="pointer-events-auto flex w-full max-w-md items-start gap-3 rounded-xl px-4 py-3 text-[14px]"
            style={{ background: 'var(--cp-text)', color: 'var(--hs-column)', boxShadow: 'var(--cp-panel-shadow)', animation: 'hs-pop-in 150ms var(--hs-ease)' }}
          >
            {toast.tone === 'warning' ? <span className="mt-0.5 flex-shrink-0" style={{ color: 'var(--cp-warning)' }}><TriangleAlert size={16} /></span> : null}
            <span className="min-w-0 flex-1 leading-5">{toast.text}</span>
            {toast.action ? (
              <button
                type="button"
                className="flex-shrink-0 text-[14px] font-semibold leading-5 hover:underline"
                onClick={() => {
                  toast.action?.onClick()
                  dismiss(toast.id)
                }}
              >
                {toast.action.label}
              </button>
            ) : null}
            <button type="button" aria-label={t('common.close', 'Close')} className="flex h-5 flex-shrink-0 items-center opacity-60 hover:opacity-100" onClick={() => dismiss(toast.id)}>
              <X size={16} />
            </button>
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  )
}
