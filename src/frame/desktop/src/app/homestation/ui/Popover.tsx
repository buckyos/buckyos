import clsx from 'clsx'
import { useEffect, useLayoutEffect, useRef, type ReactNode } from 'react'
import { createPortal } from 'react-dom'

interface PopoverProps {
  anchor: HTMLElement | null
  open: boolean
  onClose: () => void
  children: ReactNode
  align?: 'start' | 'end'
  width?: number
  role?: string
  label?: string
  className?: string
  testId?: string
}

export function Popover({ anchor, open, onClose, children, align = 'end', width, role = 'dialog', label, className, testId }: PopoverProps) {
  const panelRef = useRef<HTMLDivElement>(null)

  useLayoutEffect(() => {
    const panel = panelRef.current
    if (!open || !anchor || !panel) return
    const place = () => {
      const rect = anchor.getBoundingClientRect()
      const panelRect = panel.getBoundingClientRect()
      const margin = 8
      let left = align === 'end' ? rect.right - panelRect.width : rect.left
      left = Math.max(margin, Math.min(left, window.innerWidth - panelRect.width - margin))
      let top = rect.bottom + 6
      if (top + panelRect.height > window.innerHeight - margin) top = Math.max(margin, rect.top - panelRect.height - 6)
      panel.style.left = `${left}px`
      panel.style.top = `${top}px`
      panel.style.visibility = 'visible'
    }
    place()
    window.addEventListener('resize', place)
    return () => window.removeEventListener('resize', place)
  }, [anchor, open, align])

  useEffect(() => {
    if (!open) return
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target as Node
      if (panelRef.current?.contains(target) || anchor?.contains(target)) return
      onClose()
    }
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.stopPropagation()
        onClose()
        anchor?.focus()
      }
    }
    const onScroll = (event: Event) => {
      if (panelRef.current?.contains(event.target as Node)) return
      onClose()
    }
    document.addEventListener('pointerdown', onPointerDown, true)
    document.addEventListener('keydown', onKeyDown, true)
    window.addEventListener('scroll', onScroll, true)
    return () => {
      document.removeEventListener('pointerdown', onPointerDown, true)
      document.removeEventListener('keydown', onKeyDown, true)
      window.removeEventListener('scroll', onScroll, true)
    }
  }, [anchor, open, onClose])

  useEffect(() => {
    if (!open) return
    const first = panelRef.current?.querySelector<HTMLElement>('[role="menuitem"]:not(:disabled), button:not(:disabled), input, textarea')
    first?.focus({ preventScroll: true })
  }, [open])

  if (!open || !anchor) return null
  return createPortal(
    <div className="hs-root">
      <div
        ref={panelRef}
        role={role}
        aria-label={label}
        data-testid={testId}
        className={clsx('hs-popover', className)}
        style={{ visibility: 'hidden', width, top: 0, left: 0 }}
      >
        {children}
      </div>
    </div>,
    document.body,
  )
}

export interface MenuItemSpec {
  id: string
  label: string
  icon?: ReactNode
  hint?: string
  disabled?: boolean
  danger?: boolean
  onSelect: () => void
}

export function Menu({ anchor, open, onClose, items, label, testId }: { anchor: HTMLElement | null; open: boolean; onClose: () => void; items: MenuItemSpec[]; label: string; testId?: string }) {
  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
    const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not(:disabled)')]
    const index = buttons.indexOf(document.activeElement as HTMLButtonElement)
    const next = event.key === 'ArrowDown' ? (index + 1) % buttons.length : (index - 1 + buttons.length) % buttons.length
    buttons[next]?.focus()
    event.preventDefault()
  }
  return (
    <Popover anchor={anchor} open={open} onClose={onClose} role="menu" label={label} width={260} testId={testId}>
      <div className="py-1" onKeyDown={onKeyDown}>
        {items.map(item => (
          <button
            key={item.id}
            type="button"
            role="menuitem"
            disabled={item.disabled}
            className={clsx('hs-menu-item', item.danger && 'is-danger')}
            onClick={() => {
              onClose()
              item.onSelect()
            }}
          >
            <span className="flex h-4 w-4 flex-shrink-0 items-center justify-center" style={{ color: item.danger ? undefined : 'var(--cp-muted)' }}>{item.icon}</span>
            <span className="min-w-0 flex-1">
              <span className="block">{item.label}</span>
              {item.hint ? <span className="block text-[11px] leading-4" style={{ color: 'var(--cp-muted)' }}>{item.hint}</span> : null}
            </span>
          </button>
        ))}
      </div>
    </Popover>
  )
}
