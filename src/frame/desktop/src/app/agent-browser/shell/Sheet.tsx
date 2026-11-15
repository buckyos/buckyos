import { X } from 'lucide-react'
import { useCallback, useEffect, useRef, type ReactNode } from 'react'
import { useMobileBackHandler } from '../../../desktop/windows/MobileNavContext'
import { useI18n } from '../../../i18n/provider'

/**
 * A panel over the App: slides up from the bottom on a phone, a centred card on a wide window. Escape, the phone's
 * Back and a press outside close it; `modal` panels (a page's dialog) only close through their own buttons.
 */
export function Sheet({ title, onClose, children, footer, testId, modal = false }: {
  title: string
  onClose: () => void
  children: ReactNode
  footer?: ReactNode
  testId: string
  modal?: boolean
}) {
  const { t } = useI18n()
  const panelRef = useRef<HTMLElement>(null)
  const latest = useRef(onClose)
  useEffect(() => { latest.current = onClose })
  const close = useCallback(() => latest.current(), [])
  useMobileBackHandler(close)
  useEffect(() => {
    panelRef.current?.focus({ preventScroll: true })
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      event.stopPropagation()
      close()
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [close])
  return (
    <div className="ab-sheet-layer" data-testid={`${testId}-layer`} onClick={modal ? undefined : close}>
      <section ref={panelRef} role="dialog" aria-modal="true" aria-label={title} tabIndex={-1} className="ab-sheet" data-testid={testId} onClick={(event) => event.stopPropagation()}>
        <header className="ab-sheet-head">
          <h2>{title}</h2>
          <button type="button" className="ab-icon-button" aria-label={t('common.close')} onClick={close} data-testid={`${testId}-close`}><X size={18} aria-hidden /></button>
        </header>
        <div className="ab-sheet-body">{children}</div>
        {footer ? <footer className="ab-sheet-foot">{footer}</footer> : null}
      </section>
    </div>
  )
}
