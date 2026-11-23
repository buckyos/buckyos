import clsx from 'clsx'
import { useEffect, useLayoutEffect, useRef, type ReactNode } from 'react'
import { createPortal } from 'react-dom'

interface PopoverProps {
  anchor: HTMLElement | null
  boundary?: Element | null
  open: boolean
  onClose: () => void
  children: ReactNode
  align?: 'start' | 'end'
  width?: number
  role?: string
  label?: string
  className?: string
  testId?: string
  autoFocus?: boolean
  returnFocusTo?: HTMLElement | null
}

export function Popover({ anchor, boundary, open, onClose, children, align = 'end', width, role = 'dialog', label, className, testId, autoFocus = true, returnFocusTo }: PopoverProps) {
  const panelRef = useRef<HTMLDivElement>(null)

  useLayoutEffect(() => {
    const panel = panelRef.current
    if (!open || !anchor || !panel) return
    const place = () => {
      const rect = anchor.getBoundingClientRect()
      const margin = 8
      const bounds = boundary?.getBoundingClientRect()
      const minLeft = Math.max(0, bounds?.left ?? 0) + margin
      const minTop = Math.max(0, bounds?.top ?? 0) + margin
      const right = Math.min(window.innerWidth, bounds?.right ?? window.innerWidth) - margin
      const bottom = Math.min(window.innerHeight, bounds?.bottom ?? window.innerHeight) - margin
      const gap = 6
      const spaceAbove = rect.top - gap - minTop
      const spaceBelow = bottom - rect.bottom - gap
      if (bounds) {
        panel.style.maxWidth = `${Math.max(0, right - minLeft)}px`
        panel.style.maxHeight = `${Math.max(0, spaceAbove, spaceBelow)}px`
      }
      const panelRect = panel.getBoundingClientRect()
      let left = align === 'end' ? rect.right - panelRect.width : rect.left
      left = Math.max(minLeft, Math.min(left, right - panelRect.width))
      const top = panelRect.height <= spaceBelow ? rect.bottom + gap : Math.max(minTop, rect.top - gap - panelRect.height)
      panel.style.left = `${left}px`
      panel.style.top = `${top}px`
      panel.style.visibility = 'visible'
    }
    place()
    window.addEventListener('resize', place)
    return () => window.removeEventListener('resize', place)
  }, [anchor, boundary, open, align])

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
        const focusTarget = returnFocusTo ?? anchor
        focusTarget?.focus()
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
  }, [anchor, open, onClose, returnFocusTo])

  useEffect(() => {
    if (!open || !autoFocus) return
    const first = panelRef.current?.querySelector<HTMLElement>('[role="menuitem"]:not(:disabled), button:not(:disabled), input, textarea')
    first?.focus({ preventScroll: true })
  }, [open, autoFocus])

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
  current?: boolean
  testId?: string
  onSelect: () => void
}

export function Menu({ anchor, open, onClose, items, label, testId, align }: { anchor: HTMLElement | null; open: boolean; onClose: () => void; items: MenuItemSpec[]; label: string; testId?: string; align?: 'start' | 'end' }) {
  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
    const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not(:disabled)')]
    const index = buttons.indexOf(document.activeElement as HTMLButtonElement)
    const next = event.key === 'ArrowDown' ? (index + 1) % buttons.length : (index - 1 + buttons.length) % buttons.length
    buttons[next]?.focus()
    event.preventDefault()
  }
  return (
    <Popover anchor={anchor} open={open} onClose={onClose} role="menu" label={label} width={280} testId={testId} align={align}>
      <div className="p-1" onKeyDown={onKeyDown}>
        {items.map(item => (
          <button
            key={item.id}
            type="button"
            role="menuitem"
            disabled={item.disabled}
            className={clsx('hs-menu-item', item.danger && 'is-danger')}
            aria-current={item.current ? 'page' : undefined}
            style={item.current ? { fontWeight: 600 } : undefined}
            data-testid={item.testId}
            onClick={() => {
              onClose()
              item.onSelect()
            }}
          >
            {item.icon ? <span className="flex h-5 w-[18px] flex-shrink-0 items-center justify-center self-start" style={{ color: item.danger ? undefined : 'var(--cp-muted)' }}>{item.icon}</span> : null}
            <span className="min-w-0 flex-1">
              <span className="block">{item.label}</span>
              {item.hint ? <span className="block truncate text-[12px] leading-4" style={{ color: 'var(--cp-muted)' }}>{item.hint}</span> : null}
            </span>
          </button>
        ))}
      </div>
    </Popover>
  )
}
