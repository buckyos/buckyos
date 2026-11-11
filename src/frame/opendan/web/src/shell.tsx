import type { ReactNode } from 'react'
import { ChevronLeft } from 'lucide-react'
import { t } from './i18n'

/** A detail page in the home page's style: a way back, a title, cards. */
export function HomeShell({ title, extra, children, testId, wide }: { title: ReactNode; extra?: ReactNode; children: ReactNode; testId?: string; wide?: boolean }) {
  return (
    <div className={wide ? 'home mx-auto max-w-5xl' : 'home mx-auto max-w-2xl'} data-testid={testId}>
      <header className="mb-3 flex min-w-0 items-center gap-1">
        <a href="#/" className="tap -ml-2 shrink-0 px-2 text-accent" aria-label={t('backHome')} data-testid="back-home">
          <ChevronLeft size={18} />
          <span className="hidden sm:inline">{t('backHome')}</span>
        </a>
        <h1 className="min-w-0 flex-1 truncate text-lg font-semibold">{title}</h1>
        {extra}
      </header>
      <div className="grid grid-cols-[minmax(0,1fr)] gap-3">{children}</div>
    </div>
  )
}

export function Dialog({ title, onSubmit, children, busy, onClose, label, testId, danger, footer }: {
  title: string
  onSubmit?: () => void
  children: ReactNode
  busy: boolean
  onClose: () => void
  label?: string
  testId?: string
  danger?: boolean
  /** Replaces the cancel / submit pair. */
  footer?: ReactNode
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-end justify-center bg-black/50 p-0 sm:items-center sm:p-4">
      <form role="dialog" aria-modal="true" aria-label={title} data-testid={testId}
        className="home max-h-[92dvh] w-full overflow-y-auto rounded-t-2xl border border-line bg-panel p-5 sm:max-w-md sm:rounded-2xl"
        onSubmit={(event) => { event.preventDefault(); onSubmit?.() }}>
        <h3 className="text-base font-semibold">{title}</h3>
        {children}
        <div className="flex gap-2">
          {footer ?? <>
            <button type="button" className="tap flex-1 border border-line" onClick={onClose} disabled={busy}>{t(onSubmit ? 'cancel' : 'close')}</button>
            {onSubmit && <button type="submit" className={danger ? 'tap flex-1 bg-bad text-panel' : 'tap flex-1 bg-accent text-panel'} disabled={busy}>{label}</button>}
          </>}
        </div>
      </form>
    </div>
  )
}
