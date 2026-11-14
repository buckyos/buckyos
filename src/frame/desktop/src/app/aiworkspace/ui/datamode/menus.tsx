/* eslint-disable react-refresh/only-export-components -- menu primitives and the long-press hook */
/* The data mode's menus (UI 框架 PRD §5.2, §6.1): a context menu where it was opened (a right click, a row's ⋯) — the
 * shared MenuList, with its submenus beside it — and on a phone the same items as a bottom panel entered one level at a
 * time, with "back". Outside pointer and Esc close both. A long press on a touch row opens the menu like a right click. */

import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import { createPortal } from 'react-dom'
import { ChevronLeft, ChevronRight, X } from 'lucide-react'
import { useAiwsText } from '../../useAiwsText'
import { MenuList, useOverlayMounted, type MenuItem } from '../shell/popover'
import './datamode.css'

export interface MenuAt { x: number; y: number }

function useDismiss(ref: React.RefObject<HTMLElement | null>, onClose: () => void) {
  useEffect(() => {
    const onDown = (event: PointerEvent) => { if (!ref.current?.contains(event.target as Node)) onClose() }
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') { event.stopPropagation(); onClose() } }
    window.addEventListener('pointerdown', onDown, true)
    window.addEventListener('keydown', onKey, true)
    return () => { window.removeEventListener('pointerdown', onDown, true); window.removeEventListener('keydown', onKey, true) }
  }, [ref, onClose])
}

/** A menu at a screen point (viewport coordinates), kept inside the application. */
export function FloatingMenu({ at, items, label, testId, onClose }: { at: MenuAt; items: MenuItem[]; label: string; testId?: string; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null)
  useOverlayMounted(true)
  useDismiss(ref, onClose)
  const host = (document.querySelector('.aiws-root') as HTMLElement | null) ?? document.body
  return createPortal(
    <div ref={ref} className="aiws-dm-floating" style={{ left: at.x, top: at.y - 6 }} onPointerDown={(event) => event.stopPropagation()}>
      <MenuList items={items} label={label} className="aiws-popover is-floating aiws-dm-menu" testId={testId} onDone={onClose} />
    </div>,
    host,
  )
}

/** The same items as a bottom panel: one level at a time, a submenu entered with "back" to leave it. */
export function SheetMenu({ items, label, testId, onClose }: { items: MenuItem[]; label: string; testId?: string; onClose: () => void }) {
  const { t } = useAiwsText()
  const ref = useRef<HTMLDivElement>(null)
  const [path, setPath] = useState<MenuItem[]>([])
  useOverlayMounted(true)
  useDismiss(ref, onClose)
  // a touch that opened the sheet (a long press) is followed by its click: only what was pressed on the sheet counts
  const pressed = useRef(false)
  const level = path.at(-1)
  const shown = level?.items ?? items
  const host = (document.querySelector('.aiws-root') as HTMLElement | null) ?? document.body
  return createPortal(
    <div className="aiws-dm-sheet-backdrop">
      <div ref={ref} className="aiws-dm-sheet" role="menu" aria-label={level?.label ?? label} data-testid={testId}
        onPointerDown={() => { pressed.current = true }}
        onClickCapture={(event) => { if (!pressed.current && event.detail > 0) { event.preventDefault(); event.stopPropagation() } }}>
        <div className="aiws-dm-sheet-grip" aria-hidden="true" />
        <div className="aiws-dm-sheet-head">
          {level ? <button type="button" className="aiws-tool" aria-label={t('aiworkspace.datamode.back')} data-testid="aiws-dm-sheet-back" onClick={() => setPath(path.slice(0, -1))}><ChevronLeft size={20} /></button> : null}
          <b>{level?.label ?? label}</b>
          <span className="aiws-grow" />
          <button type="button" className="aiws-tool" aria-label={t('aiworkspace.common.close')} data-testid="aiws-dm-sheet-close" onClick={onClose}><X size={20} /></button>
        </div>
        <div className="aiws-dm-sheet-body">
          {shown.map((item) => item.separator ? <hr key={item.id} /> : (
            <button key={item.id} type="button" role="menuitem" className={`aiws-dm-sheet-item${item.danger ? ' is-danger' : ''}`} data-testid={item.testId ?? `aiws-menu-${item.id}`}
              disabled={item.disabled} aria-checked={item.checked} aria-haspopup={item.items ? 'menu' : undefined} title={item.disabled && item.reason ? item.reason : undefined}
              onClick={() => {
                if (item.items) { setPath([...path, item]); return }
                onClose()
                item.run?.()
              }}>
              <span className="aiws-menu-label">{item.label}{item.disabled && item.reason && <small className="aiws-menu-reason">{item.reason}</small>}</span>
              {item.hint && !item.disabled && <span className="aiws-menu-hint">{item.hint}</span>}
              {item.items && <ChevronRight size={18} aria-hidden="true" />}
            </button>
          ))}
        </div>
      </div>
    </div>,
    host,
  )
}

/** A long press of a finger (500 ms without moving): `onPress` at the touch point; the click that follows is swallowed. */
export function useLongPress(onPress: (at: MenuAt) => void) {
  const timer = useRef<number | null>(null)
  const start = useRef<MenuAt | null>(null)
  const fired = useRef(false)
  const clear = () => { if (timer.current !== null) { window.clearTimeout(timer.current); timer.current = null } }
  useEffect(() => clear, [])
  return {
    onPointerDown: (event: ReactPointerEvent) => {
      fired.current = false
      if (event.pointerType !== 'touch') return
      start.current = { x: event.clientX, y: event.clientY }
      clear()
      timer.current = window.setTimeout(() => { timer.current = null; fired.current = true; if (start.current) onPress(start.current) }, 500)
    },
    onPointerMove: (event: ReactPointerEvent) => {
      if (!start.current || timer.current === null) return
      if (Math.hypot(event.clientX - start.current.x, event.clientY - start.current.y) > 8) clear()
    },
    onPointerUp: clear,
    onPointerCancel: clear,
    /** True once: the click after a long press. */
    consumed: () => { const was = fired.current; fired.current = false; return was },
  }
}
