/* eslint-disable react-refresh/only-export-components -- popover primitives and their hooks */
/* Popovers and menus of the workspace chrome (UI improvement §3.2, §11): screen-space panels next to
 * their trigger; outside pointer and Esc close them and return focus to the trigger; menus support
 * arrow keys, Enter, Esc and nested submenus. While any of them is open the canvas refuses gestures,
 * so the click that closes a menu does not also act on the canvas.
 *
 * On a phone (UI 框架 PRD §6.1) every one of them rises from the bottom instead: a panel is a sheet over the window, a
 * menu is a sheet entered one level at a time with "back" (no hover), and the reason of a disabled entry is printed. */

import { useCallback, useContext, useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { Check, ChevronLeft, ChevronRight, X } from 'lucide-react'
import { useAiwsText } from '../../useAiwsText'
import { ShellContext } from './shellContext'
import '../phone/sheets.css'

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
  // a phone's panel is a sheet away from its anchor: it asks its anchor to close (its backdrop, ✕), so a press in it is no
  // "outside"
  useEffect(() => {
    if (!anchor) return
    const onClose = (event: Event) => { event.stopPropagation(); setOpen(false) }
    anchor.addEventListener(SHEET_CLOSE, onClose)
    return () => anchor.removeEventListener(SHEET_CLOSE, onClose)
  }, [anchor])
  useEffect(() => {
    if (!open) return
    const onDown = (event: PointerEvent) => { if (!anchor?.contains(event.target as Node) && !insideSheet(event.target)) setOpen(false) }
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

/** A panel below its anchor, kept inside the application container horizontally (by a margin, not a transform,
 * so a floating menu inside it is not confined to it). On a phone: a sheet from the bottom. */
export function PopoverPanel(props: { children: ReactNode; align?: 'start' | 'end'; label: string; testId?: string; className?: string }) {
  const phone = usePhoneSheets()
  return phone ? <PanelSheet {...props} /> : <AnchoredPanel {...props} />
}

function AnchoredPanel({ children, align = 'start', label, testId, className = '' }: { children: ReactNode; align?: 'start' | 'end'; label: string; testId?: string; className?: string }) {
  const ref = useRef<HTMLDivElement>(null)
  const [shift, setShift] = useState(0)
  useLayoutEffect(() => { if (ref.current) setShift(containShift(ref.current)) }, [])
  return (
    <div ref={ref} className={`aiws-popover aiws-popover-${align} ${className}`} role="dialog" aria-label={label} data-testid={testId} style={shift ? (align === 'end' ? { marginRight: -shift } : { marginLeft: shift }) : undefined}
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

export interface MenuListProps {
  items: MenuItem[]
  onDone: () => void
  onBack?: () => void
  label: string
  autoFocus?: boolean
  className?: string
  testId?: string
  /** A phone: the menu is a sheet of its own (by default, a pop-over menu is; one drawn inside a sheet already is not). */
  sheet?: boolean
}

export function MenuList(props: MenuListProps) {
  const phone = usePhoneSheets()
  if (!phone) return <AnchoredMenuList {...props} />
  const className = props.className ?? ''
  return <SheetMenuList {...props} sheet={props.sheet ?? (/\baiws-popover\b/.test(className) && !/\bis-floating\b/.test(className))} />
}

function AnchoredMenuList({ items, onDone, onBack, label, autoFocus = true, className = '', testId }: MenuListProps) {
  const ref = useRef<HTMLDivElement>(null)
  const [openSub, setOpenSub] = useState<string | null>(null)
  const [subByKey, setSubByKey] = useState(false)
  const [placement, setPlacement] = useState<{ shift: number; lift: number; flip: boolean }>({ shift: 0, lift: 0, flip: false })
  // a top-level menu moves inside the application; a submenu without room on the right opens to the left
  // and one running past the bottom moves up; a floating menu (fixed, rendered at 0,0 of its containing block)
  // goes under its anchor, or above it when there is no room below
  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    const bounds = (el.closest('.aiws-root') ?? document.documentElement).getBoundingClientRect()
    const rect = el.getBoundingClientRect()
    if (className.includes('is-floating') && el.parentElement) {
      const at = el.parentElement.getBoundingClientRect()
      const left = Math.max(bounds.left + 8, Math.min(className.includes('aiws-popover-end') ? at.right - rect.width : at.left, bounds.right - 8 - rect.width))
      const top = at.bottom + 6 + rect.height > bounds.bottom - 8 ? Math.max(bounds.top + 8, at.top - 6 - rect.height) : at.bottom + 6
      setPlacement({ shift: left - rect.left, lift: top - rect.top, flip: false })
      return
    }
    if (!className.includes('aiws-submenu')) { setPlacement({ shift: containShift(el), lift: 0, flip: false }); return }
    const lift = Math.max(Math.min(0, bounds.bottom - 8 - rect.bottom), bounds.top + 8 - rect.top)
    if (rect.right <= bounds.right - 8) { setPlacement({ shift: 0, lift, flip: false }); return }
    // opened to the left it is kept inside the application; when that would cover most of its own menu (a narrow
    // window, a third level) it stays on the right, moved in over the menus further up
    const parent = el.parentElement?.closest('.aiws-menu')?.getBoundingClientRect()
    const past = parent ? bounds.left + 8 - (parent.left - rect.width + 4) : 0
    if (past <= rect.width / 2) { setPlacement({ shift: Math.max(0, past), lift, flip: true }); return }
    setPlacement({ shift: Math.max(bounds.left + 8 - rect.left, bounds.right - 8 - rect.right), lift, flip: false })
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
    <div ref={ref} className={`aiws-menu ${className}${placement.flip ? ' is-flipped' : ''}${items.some((item) => item.items) ? ' has-submenu' : ''}`} role="menu" aria-label={label} data-testid={testId} onKeyDown={onKey}
      style={placement.shift || placement.lift ? { transform: `translate(${placement.shift}px, ${placement.lift}px)` } : undefined}>
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
            <AnchoredMenuList items={item.items} label={item.label ?? ''} className="aiws-submenu" onDone={onDone} autoFocus={subByKey}
              onBack={() => { setOpenSub(null); ref.current?.querySelector<HTMLButtonElement>(`[data-menu-id="${item.id}"]`)?.focus() }} />
          )}
        </div>
      ))}
    </div>
  )
}

/** A trigger button with a menu (or any panel) under it. */
export function MenuButton({ items, label, title, children, className = 'aiws-tool', testId, menuTestId, align = 'start', disabled, floating = false }: {
  /** A function is evaluated when the menu opens (its items reflect the state at that moment). */
  items: MenuItem[] | (() => MenuItem[]); label: string; title?: string; children: ReactNode; className?: string; testId?: string; menuTestId?: string; align?: 'start' | 'end'; disabled?: boolean
  /** In a scrolling list: the menu is fixed so the list does not clip it, and any scroll closes it. */
  floating?: boolean
}) {
  const { open, setOpen, toggle, close, bindAnchor, bindTrigger } = usePopover()
  const phone = usePhoneSheets()
  useEffect(() => {
    if (!open || !floating || phone) return
    const onScroll = () => setOpen(false)
    window.addEventListener('scroll', onScroll, true)
    return () => window.removeEventListener('scroll', onScroll, true)
  }, [open, floating, setOpen, phone])
  return (
    <span ref={bindAnchor} className="aiws-anchor">
      <button ref={bindTrigger} type="button" className={className} aria-label={label} title={title ?? label} aria-haspopup="menu" aria-expanded={open} data-testid={testId} disabled={disabled} onClick={toggle}>
        {children}
      </button>
      {open && <MenuList items={typeof items === 'function' ? items() : items} label={label} className={`aiws-popover aiws-popover-${align}${floating ? ' is-floating' : ''}`} onDone={close} testId={menuTestId} sheet={phone} />}
    </span>
  )
}

// ---- the phone's sheets (UI 框架 PRD §6.1)

/** Inside a workspace window on a phone: pop-overs rise from the bottom. */
export function usePhoneSheets(): boolean {
  return useContext(ShellContext)?.phone ?? false
}

/** What a sheet without its own close sends from its place under the anchor: the pop-over there closes. */
const SHEET_CLOSE = 'aiws-sheet-close'

/** Is the target inside a sheet (its panel or its backdrop)? A sheet handles its own presses. */
export function insideSheet(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest('.aiws-sheet-layer') !== null
}

/** The sheet's layer over the workspace window: the backdrop (a press on it closes), the panel at the bottom. A touch that
 * opened the sheet (a long press) is followed by its click on whatever lies under the finger: only a click pressed on the
 * sheet counts. */
export function Sheet({ label, testId, className = '', onClose, title, children, role = 'dialog' }: {
  label: string
  testId?: string
  className?: string
  /** Absent: the pop-over whose anchor holds the sheet's place closes. */
  onClose?: () => void
  /** The heading row: what the caller draws (a menu's, with "back"), or given the sheet's close (a panel's ✕). */
  title?: ReactNode | ((close: () => void) => ReactNode)
  children: ReactNode
  role?: 'dialog' | 'none'
}) {
  const [place, setPlace] = useState<HTMLElement | null>(null)
  const pressed = useRef(false)
  // over the workspace window the sheet's place is in (its phone styles apply)
  const host = place ? place.closest<HTMLElement>('.aiws-workspace') ?? place.closest<HTMLElement>('.aiws-root') ?? document.body : null
  const close = () => { if (onClose) onClose(); else place?.dispatchEvent(new Event(SHEET_CLOSE, { bubbles: true })) }
  return (
    <>
      <span ref={setPlace} hidden />
      {host && createPortal(
        <div className="aiws-sheet-layer" onPointerDown={(event) => { event.stopPropagation(); if (event.target === event.currentTarget) close() }}
          onClickCapture={(event) => { if (!pressed.current && event.detail > 0) { event.preventDefault(); event.stopPropagation() } }}>
          <div className={`aiws-sheet ${className}`} role={role === 'none' ? undefined : role} aria-label={role === 'none' ? undefined : label} data-testid={testId}
            onPointerDown={() => { pressed.current = true }}>
            <div className="aiws-sheet-grip" aria-hidden="true" />
            {typeof title === 'function' ? title(close) : title}
            {children}
          </div>
        </div>,
        host,
      )}
    </>
  )
}

/** A panel as a sheet: its title and ✕, then the content, which scrolls. */
function PanelSheet({ children, label, testId, className = '' }: { children: ReactNode; label: string; testId?: string; className?: string }) {
  const { t } = useAiwsText()
  return (
    <Sheet label={label} testId={testId} className={`aiws-sheet-panel ${className}`}
      title={(close) => <div className="aiws-sheet-head"><b>{label}</b><span className="aiws-grow" /><button type="button" className="aiws-tool" aria-label={t('aiworkspace.common.close')} title={t('aiworkspace.common.close')} data-testid="aiws-sheet-close" onClick={close}><X size={20} /></button></div>}>
      <div className="aiws-sheet-body">{children}</div>
    </Sheet>
  )
}

/** A shortcut hint means nothing on a phone (a file type does). */
const KEY_HINT = /\b(Ctrl|Shift|Alt|Delete|Enter|Esc|F\d{1,2})\b|⌘/

/** A menu on a phone: one level at a time, an entry with a submenu goes into it, "back" comes out; a disabled entry prints
 * why. As a sheet of its own, or drawn where it is (inside a sheet already: the context menu's). */
function SheetMenuList({ items, onDone, label, autoFocus = true, className = '', testId, sheet }: MenuListProps) {
  const { t } = useAiwsText()
  const ref = useRef<HTMLDivElement>(null)
  const [path, setPath] = useState<MenuItem[]>([])
  const level = path.at(-1)
  const shown = level?.items ?? items
  useEffect(() => {
    if (!autoFocus) return
    ref.current?.querySelector<HTMLButtonElement>(':scope > .aiws-menu-entry > button[role="menuitem"]:not(:disabled)')?.focus({ preventScroll: true })
  }, [autoFocus, path.length])
  const head = (sheet || level) && (
    <div className="aiws-sheet-head aiws-menu-phone-head">
      {level && <button type="button" className="aiws-tool" aria-label={t('aiworkspace.phone.back')} title={t('aiworkspace.phone.back')} data-testid="aiws-menu-back" onClick={() => setPath(path.slice(0, -1))}><ChevronLeft size={20} /></button>}
      <b>{level?.label ?? label}</b>
      <span className="aiws-grow" />
      {sheet && <button type="button" className="aiws-tool" aria-label={t('aiworkspace.common.close')} title={t('aiworkspace.common.close')} data-testid="aiws-menu-close" onClick={onDone}><X size={20} /></button>}
    </div>
  )
  const list = (
    <div ref={ref} className={`aiws-menu aiws-menu-phone${sheet ? '' : ` ${className}`}`} role="menu" aria-label={level?.label ?? label} data-testid={sheet ? undefined : testId} data-level={path.length}
      onKeyDown={(event) => { if (event.key === 'Escape' && level) { event.preventDefault(); event.stopPropagation(); setPath(path.slice(0, -1)) } }}>
      {!sheet && head}
      {shown.map((item) => item.separator ? <hr key={item.id} /> : (
        <div key={item.id} className="aiws-menu-entry" role="none">
          <button type="button" role="menuitem" data-menu-id={item.id} data-testid={item.testId ?? `aiws-menu-${item.id}`} disabled={item.disabled}
            aria-haspopup={item.items ? 'menu' : undefined} aria-checked={item.checked} className={item.danger ? 'is-danger' : undefined}
            onClick={() => {
              if (item.items) { setPath([...path, item]); return }
              onDone()
              item.run?.()
            }}>
            <span className="aiws-menu-check" aria-hidden="true">{item.checked ? <Check size={16} /> : null}</span>
            <span className="aiws-menu-label">{item.label}{item.disabled && item.reason && <small className="aiws-menu-reason">{item.reason}</small>}</span>
            {item.hint && !KEY_HINT.test(item.hint) && <span className="aiws-menu-hint">{item.hint}</span>}
            {item.items && <ChevronRight size={18} aria-hidden="true" />}
          </button>
        </div>
      ))}
    </div>
  )
  if (!sheet) return list
  return <Sheet label={level?.label ?? label} testId={testId} className={`aiws-sheet-menu ${className.replace(/\baiws-popover(-start|-end)?\b|\bis-floating\b/g, '').trim()}`} onClose={onDone} title={head} role="none">{list}</Sheet>
}
