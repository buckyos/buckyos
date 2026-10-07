/* eslint-disable react-refresh/only-export-components -- popover primitives and their hooks */
/* Popovers and menus of the workspace chrome (UI improvement §3.2, §11): screen-space panels next to
 * their trigger; outside pointer and Esc close them and return focus to the trigger; menus support
 * arrow keys, Enter, Esc and nested submenus. While any of them is open the canvas refuses gestures,
 * so the click that closes a menu does not also act on the canvas. */

import { useCallback, useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from 'react'
import { Check, ChevronRight } from 'lucide-react'

// ---- open overlays (the canvas pauses its gestures while one is open)

let openOverlays = 0
const overlayListeners = new Set<() => void>()
function changeOverlays(delta: number) {
  openOverlays = Math.max(0, openOverlays + delta)
  for (const listener of [...overlayListeners]) listener()
}

export function useOverlayOpen(): boolean {
  return useSyncExternalStore((listener) => { overlayListeners.add(listener); return () => { overlayListeners.delete(listener) } }, () => openOverlays > 0)
}

/** Count an overlay as open while mounted (dialogs). */
export function useOverlayMounted(active = true) {
  useEffect(() => {
    if (!active) return
    changeOverlays(1)
    return () => changeOverlays(-1)
  }, [active])
}

// ---- popover state

export interface PopoverState {
  open: boolean
  setOpen: (open: boolean) => void
  toggle: () => void
  /** Close and give the focus back to the trigger. */
  close: () => void
  /** Callback refs: the element that contains trigger and panel, and the trigger. */
  bindAnchor: (element: HTMLSpanElement | null) => void
  bindTrigger: (element: HTMLButtonElement | null) => void
}

export function usePopover(): PopoverState {
  const [open, setOpen] = useState(false)
  const [anchor, bindAnchor] = useState<HTMLSpanElement | null>(null)
  const [trigger, bindTrigger] = useState<HTMLButtonElement | null>(null)
  useOverlayMounted(open)
  const close = useCallback(() => { setOpen(false); trigger?.focus() }, [trigger])
  useEffect(() => {
    if (!open) return
    const onDown = (event: PointerEvent) => { if (!anchor?.contains(event.target as Node)) setOpen(false) }
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') { event.stopPropagation(); close() } }
    window.addEventListener('pointerdown', onDown, true)
    window.addEventListener('keydown', onKey, true)
    return () => { window.removeEventListener('pointerdown', onDown, true); window.removeEventListener('keydown', onKey, true) }
  }, [open, close, anchor])
  const toggle = useCallback(() => setOpen((value) => !value), [])
  return { open, setOpen, toggle, close, bindAnchor, bindTrigger }
}

/** How far a panel must move horizontally to stay inside the application container (the window, not the browser). */
function containShift(el: HTMLElement): number {
  const bounds = (el.closest('.aiws-root') ?? document.documentElement).getBoundingClientRect()
  const rect = el.getBoundingClientRect()
  const overflowRight = rect.right - (bounds.right - 8)
  const overflowLeft = bounds.left + 8 - rect.left
  return overflowRight > 0 ? -Math.min(overflowRight, Math.max(0, rect.left - bounds.left - 8)) : overflowLeft > 0 ? overflowLeft : 0
}

/** A panel below its anchor, kept inside the application container horizontally. */
export function PopoverPanel({ children, align = 'start', label, testId, className = '' }: { children: ReactNode; align?: 'start' | 'end'; label: string; testId?: string; className?: string }) {
  const ref = useRef<HTMLDivElement>(null)
  const [shift, setShift] = useState(0)
  useLayoutEffect(() => { if (ref.current) setShift(containShift(ref.current)) }, [])
  return (
    <div ref={ref} className={`aiws-popover aiws-popover-${align} ${className}`} role="dialog" aria-label={label} data-testid={testId} style={shift ? { transform: `translateX(${shift}px)` } : undefined}
      onPointerDown={(event) => event.stopPropagation()}>
      {children}
    </div>
  )
}

// ---- menus

export interface MenuItem {
  id: string
  label?: string
  separator?: boolean
  /** Keyboard hint shown on the right. */
  hint?: string
  disabled?: boolean
  /** Why the item is disabled: a tooltip, and shown in the menu when `explain` is set (structural limits, not selection). */
  reason?: string | null
  explain?: boolean
  checked?: boolean
  danger?: boolean
  run?: () => void
  items?: MenuItem[]
  testId?: string
}

export function MenuList({ items, onDone, onBack, label, autoFocus = true, className = '', testId }: { items: MenuItem[]; onDone: () => void; onBack?: () => void; label: string; autoFocus?: boolean; className?: string; testId?: string }) {
  const ref = useRef<HTMLDivElement>(null)
  const [openSub, setOpenSub] = useState<string | null>(null)
  const [subByKey, setSubByKey] = useState(false)
  const [placement, setPlacement] = useState<{ shift: number; flip: boolean }>({ shift: 0, flip: false })
  // a top-level menu moves inside the application; a submenu without room on the right opens to the left
  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    if (!className.includes('aiws-submenu')) { setPlacement({ shift: containShift(el), flip: false }); return }
    const bounds = (el.closest('.aiws-root') ?? document.documentElement).getBoundingClientRect()
    setPlacement({ shift: 0, flip: el.getBoundingClientRect().right > bounds.right - 8 })
  }, [className])
  const enabled = () => [...(ref.current?.querySelectorAll<HTMLButtonElement>(':scope > .aiws-menu-entry > button[role="menuitem"]:not(:disabled)') ?? [])]
  useEffect(() => { if (autoFocus) enabled()[0]?.focus() }, [autoFocus])
  const onKey = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.target instanceof HTMLElement && event.target.closest('[role="menu"]') !== ref.current) return
    const list = enabled()
    const index = list.indexOf(document.activeElement as HTMLButtonElement)
    const move = (to: number) => { event.preventDefault(); list[(to + list.length) % list.length]?.focus() }
    if (event.key === 'ArrowDown') move(index + 1)
    else if (event.key === 'ArrowUp') move(index - 1)
    else if (event.key === 'Home') move(0)
    else if (event.key === 'End') move(list.length - 1)
    else if (event.key === 'ArrowRight') {
      const id = (document.activeElement as HTMLElement | null)?.dataset.menuId
      const item = items.find((entry) => entry.id === id)
      if (item?.items && !item.disabled) { event.preventDefault(); setSubByKey(true); setOpenSub(item.id) }
    } else if (event.key === 'ArrowLeft' && onBack) { event.preventDefault(); onBack() }
  }
  return (
    <div ref={ref} className={`aiws-menu ${className}${placement.flip ? ' is-flipped' : ''}`} role="menu" aria-label={label} data-testid={testId} onKeyDown={onKey}
      style={placement.shift ? { transform: `translateX(${placement.shift}px)` } : undefined}>
      {items.map((item) => item.separator ? <hr key={item.id} /> : (
        <div key={item.id} className="aiws-menu-entry" role="none" onPointerEnter={() => { setSubByKey(false); setOpenSub(item.items && !item.disabled ? item.id : null) }}>
          <button type="button" role="menuitem" data-menu-id={item.id} data-testid={item.testId ?? `aiws-menu-${item.id}`} disabled={item.disabled}
            aria-haspopup={item.items ? 'menu' : undefined} aria-expanded={item.items ? openSub === item.id : undefined} aria-checked={item.checked}
            className={item.danger ? 'is-danger' : undefined} title={item.disabled && item.reason ? item.reason : undefined}
            onClick={() => {
              if (item.items) { setSubByKey(true); setOpenSub(openSub === item.id ? null : item.id); return }
              onDone()
              item.run?.()
            }}>
            <span className="aiws-menu-check" aria-hidden="true">{item.checked ? <Check size={14} /> : null}</span>
            <span className="aiws-menu-label">{item.label}{item.disabled && item.reason && item.explain && <small className="aiws-menu-reason">{item.reason}</small>}</span>
            {item.hint && <span className="aiws-menu-hint">{item.hint}</span>}
            {item.items && <ChevronRight size={14} aria-hidden="true" />}
          </button>
          {item.items && openSub === item.id && (
            <MenuList items={item.items} label={item.label ?? ''} className="aiws-submenu" onDone={onDone} autoFocus={subByKey}
              onBack={() => { setOpenSub(null); ref.current?.querySelector<HTMLButtonElement>(`[data-menu-id="${item.id}"]`)?.focus() }} />
          )}
        </div>
      ))}
    </div>
  )
}

/** A trigger button with a menu (or any panel) under it. */
export function MenuButton({ items, label, title, children, className = 'aiws-tool', testId, menuTestId, align = 'start', disabled }: {
  /** A function is evaluated when the menu opens (its items reflect the state at that moment). */
  items: MenuItem[] | (() => MenuItem[]); label: string; title?: string; children: ReactNode; className?: string; testId?: string; menuTestId?: string; align?: 'start' | 'end'; disabled?: boolean
}) {
  const { open, toggle, close, bindAnchor, bindTrigger } = usePopover()
  return (
    <span ref={bindAnchor} className="aiws-anchor">
      <button ref={bindTrigger} type="button" className={className} aria-label={label} title={title ?? label} aria-haspopup="menu" aria-expanded={open} data-testid={testId} disabled={disabled} onClick={toggle}>
        {children}
      </button>
      {open && <MenuList items={typeof items === 'function' ? items() : items} label={label} className={`aiws-popover aiws-popover-${align}`} onDone={close} testId={menuTestId} />}
    </span>
  )
}
