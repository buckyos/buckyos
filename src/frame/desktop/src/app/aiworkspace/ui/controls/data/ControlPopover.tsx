/* eslint-disable react-refresh/only-export-components -- the panel and its menu helper */
/* Pop-overs of a data control (控件机制 TODO G5) and of the table and directory editors: anchored on a projected Block or
 * on the button that opened them — an event's details, a quick new event, a day's "+N more", the repeat settings, a
 * date, an item's menu, a confirmation — drawn in screen space (the canvas zoom does not scale them) and kept inside
 * the window. The content is the host's record form, standard field editors or a menu, not Blocks. Outside presses and
 * Esc close them; while one is open the canvas takes no gestures.
 *
 * `ControlPanel` is the framework's form of them (UI 框架 PRD §6.1, §7.5, §10.1): on a computer a floating panel beside
 * (or below) its anchor with a title and ✕, as the properties panel; on a phone the window's bottom sheet, the one every
 * pop-over of the workspace becomes (`shell/popover.tsx`). */

import { useEffect, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from 'react'
import { createPortal } from 'react-dom'
import { X } from 'lucide-react'
import { useAiwsText } from '../../../useAiwsText'
import { Sheet, useOverlayMounted, usePhoneSheets } from '../../shell/popover'
import './controlPanel.css'

export interface AnchorRect { left: number; top: number; width: number; height: number }

/** The client rectangle of an element, or of a rectangle inside it given in its own coordinates. */
export function anchorOf(el: HTMLElement | null, local?: { x: number; y: number; w: number; h: number }): AnchorRect {
  if (!el) return { left: window.innerWidth / 2, top: window.innerHeight / 3, width: 0, height: 0 }
  const r = el.getBoundingClientRect()
  if (!local) return { left: r.left, top: r.top, width: r.width, height: r.height }
  const sx = el.offsetWidth ? r.width / el.offsetWidth : 1
  const sy = el.offsetHeight ? r.height / el.offsetHeight : 1
  return { left: r.left + (local.x - el.scrollLeft) * sx, top: r.top + (local.y - el.scrollTop) * sy, width: local.w * sx, height: local.h * sy }
}

function portalRoot(): HTMLElement {
  return (document.querySelector('.aiws-root') as HTMLElement | null) ?? document.body
}

export interface ControlPanelProps {
  anchor: AnchorRect
  onClose: () => void
  /** The panel's title (on a phone the sheet's). */
  label: string
  children: ReactNode
  width?: number
  testId?: string
  /** Prefer the side of the anchor (default: right of it on wide screens, below it otherwise). */
  side?: 'right' | 'below'
}

/** Where a panel beside `anchor` goes: right of it (else left) on wide screens, else below (or above), inside the window. */
function useBeside(ref: RefObject<HTMLDivElement | null>, anchor: AnchorRect, side: 'right' | 'below') {
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null)
  useLayoutEffect(() => {
    if (!ref.current) return
    const panel = ref.current.getBoundingClientRect()
    const bounds = portalRoot().getBoundingClientRect()
    const vw = Math.min(window.innerWidth, bounds.right), vh = window.innerHeight
    const margin = 8
    let left: number, top: number
    if (side === 'right' && anchor.left + anchor.width + 8 + panel.width <= vw - margin) {
      left = anchor.left + anchor.width + 8
      top = anchor.top
    } else if (side === 'right' && anchor.left - 8 - panel.width >= margin) {
      left = anchor.left - 8 - panel.width
      top = anchor.top
    } else {
      left = anchor.left
      top = anchor.top + anchor.height + 6
      if (top + panel.height > vh - margin) top = anchor.top - 6 - panel.height
    }
    left = Math.min(Math.max(margin, left), vw - margin - panel.width)
    top = Math.min(Math.max(margin, top), vh - margin - panel.height)
    setPos({ left, top })
  }, [ref, anchor.left, anchor.top, anchor.width, anchor.height, side])
  return pos
}

/** Outside presses and Esc close it (the press that opened it is over before these listen). */
function useDismiss(ref: RefObject<HTMLDivElement | null>, onClose: () => void) {
  useEffect(() => {
    const onDown = (event: PointerEvent) => { if (!ref.current?.contains(event.target as Node)) onClose() }
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') { event.stopPropagation(); onClose() } }
    // the press that opened it is over before these listen
    const timer = window.setTimeout(() => {
      window.addEventListener('pointerdown', onDown, true)
      window.addEventListener('keydown', onKey, true)
    }, 0)
    return () => { window.clearTimeout(timer); window.removeEventListener('pointerdown', onDown, true); window.removeEventListener('keydown', onKey, true) }
  }, [ref, onClose])
}

/** A data control's or an editor's details, forms and menus in the framework's panel: beside the anchor on a computer, the
 * bottom sheet on a phone. */
export function ControlPanel(props: ControlPanelProps) {
  return usePhoneSheets() ? <SheetPanel {...props} /> : <FloatingPanel {...props} />
}

function FloatingPanel({ anchor, onClose, label, children, width = 340, testId, side = 'right' }: ControlPanelProps) {
  const { t } = useAiwsText()
  const ref = useRef<HTMLDivElement>(null)
  useOverlayMounted(true)
  const pos = useBeside(ref, anchor, side)
  useDismiss(ref, onClose)
  // placed before the first paint (a layout effect) and never hidden, so what it opens with can take the focus (autoFocus)
  return createPortal(
    <div ref={ref} className="aiws-panel aiws-ctl-panel" role="dialog" aria-label={label} data-testid={testId}
      style={{ width, left: pos?.left ?? anchor.left, top: pos?.top ?? anchor.top + anchor.height }}
      onPointerDown={(event) => event.stopPropagation()} onKeyDown={(event) => event.stopPropagation()}>
      <div className="aiws-ctl-panel-head">
        <b>{label}</b>
        <button type="button" className="aiws-tool aiws-tool-small" aria-label={t('aiworkspace.common.close')} title={t('aiworkspace.common.close')} data-testid="aiws-ctl-panel-close" onClick={onClose}><X size={16} aria-hidden="true" /></button>
      </div>
      <div className="aiws-ctl-panel-body">{children}</div>
    </div>,
    portalRoot(),
  )
}

function SheetPanel({ onClose, label, children, testId }: ControlPanelProps) {
  const { t } = useAiwsText()
  useOverlayMounted(true)
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') { event.stopPropagation(); onClose() } }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [onClose])
  return (
    <Sheet label={label} testId={testId} className="aiws-sheet-panel aiws-ctl-sheet" onClose={onClose}
      title={(close) => <div className="aiws-sheet-head"><b>{label}</b><span className="aiws-grow" /><button type="button" className="aiws-tool" aria-label={t('aiworkspace.common.close')} title={t('aiworkspace.common.close')} data-testid="aiws-sheet-close" onClick={close}><X size={20} /></button></div>}>
      <div className="aiws-sheet-body" onKeyDown={(event) => event.stopPropagation()}>{children}</div>
    </Sheet>
  )
}

export interface MenuItem { id: string; label: string; icon?: ReactNode; danger?: boolean; disabled?: string | false; checked?: boolean; run: () => void }

/** A small action menu in a pop-over (a card's "Move to…", a day's actions). */
export function ControlMenu({ items, onClose, label }: { items: (MenuItem | 'separator')[]; onClose: () => void; label: string }) {
  const ref = useRef<HTMLDivElement>(null)
  // the first entry takes the keyboard once the panel is in place (a phone's sheet draws its content a render later)
  useEffect(() => {
    const frame = requestAnimationFrame(() => ref.current?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus({ preventScroll: true }))
    return () => cancelAnimationFrame(frame)
  }, [])
  return (
    <div ref={ref} className="aiws-ctl-menu" role="menu" aria-label={label}
      onKeyDown={(event) => {
        if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
        event.preventDefault()
        const buttons = [...(ref.current?.querySelectorAll('button:not(:disabled)') ?? [])] as HTMLButtonElement[]
        const i = buttons.indexOf(document.activeElement as HTMLButtonElement)
        buttons[(i + (event.key === 'ArrowDown' ? 1 : buttons.length - 1)) % buttons.length]?.focus()
      }}>
      {items.map((item, i) => item === 'separator'
        ? <div key={`sep${i}`} className="aiws-ctl-menu-sep" role="separator" />
        : (
          <button key={item.id} type="button" role="menuitemcheckbox" aria-checked={item.checked} className={`aiws-ctl-menu-item${item.danger ? ' is-danger' : ''}`} data-testid={`aiws-ctl-menu-${item.id}`}
            disabled={Boolean(item.disabled)} title={item.disabled || undefined} onClick={() => { onClose(); item.run() }}>
            <span className="aiws-ctl-menu-icon" aria-hidden="true">{item.icon}</span>
            <span className="aiws-ctl-menu-label">{item.label}</span>
          </button>
        ))}
    </div>
  )
}
