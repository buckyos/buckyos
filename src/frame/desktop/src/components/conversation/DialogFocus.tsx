import { useEffect, useRef, type ReactNode } from 'react'

export const hubInputClass = 'mt-1 min-h-11 w-full rounded-lg border border-[color:var(--cp-border)] bg-[color:var(--cp-bg)] px-3 py-2 text-sm'
export const hubButtonClass = 'min-h-11 rounded-lg border border-[color:var(--cp-border)] px-3 py-2 text-sm disabled:opacity-40'
export const hubPrimaryButtonClass = 'min-h-11 rounded-lg border border-transparent bg-[color:var(--cp-accent)] px-4 py-2 text-sm font-semibold text-white disabled:opacity-40'
export const hubIconButtonClass = 'flex h-11 w-11 shrink-0 items-center justify-center rounded-xl bg-[color:color-mix(in_srgb,var(--cp-text)_7%,transparent)] text-[color:var(--cp-muted)] transition-colors hover:text-[color:var(--cp-text)] disabled:opacity-40 md:h-9 md:w-9'

export function DialogFocus({ children, onCancel }: { children: ReactNode; onCancel: () => void }) {
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const root = ref.current?.closest('[role="dialog"]')
    if (!(root instanceof HTMLElement)) return
    const buttons = () => [...root.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled)')]
    buttons().find(element => element.hasAttribute('data-autofocus'))?.focus()
    const key = (event: KeyboardEvent) => {
      if (event.key === 'Escape') { event.preventDefault(); onCancel() }
      if (event.key !== 'Tab') return
      const items = buttons(), first = items[0], last = items.at(-1)
      if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus() }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus() }
    }
    root.addEventListener('keydown', key)
    return () => root.removeEventListener('keydown', key)
  }, [onCancel])
  return <div ref={ref}>{children}</div>
}
