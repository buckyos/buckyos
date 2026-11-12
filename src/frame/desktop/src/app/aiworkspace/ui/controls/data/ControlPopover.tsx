/* eslint-disable react-refresh/only-export-components -- the pop-over and its menu helper */
/* Pop-overs of a data control (控件机制 TODO G5): anchored on a projected Block — an event's details, a quick new
 * event, a day's "+N more", the repeat settings, a date — drawn in screen space (the canvas zoom does not scale them),
 * kept inside the window; on a phone a sheet from the bottom. The content is the host's record form and standard field
 * editors, not Blocks. Outside presses and Esc close it; while it is open the canvas takes no gestures. */

import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { X } from 'lucide-react'
import { useAiwsText } from '../../../useAiwsText'
import { useOverlayMounted } from '../../shell/popover'

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

export interface ControlPopoverProps {
  anchor: AnchorRect
  onClose: () => void
  label: string
  children: ReactNode
  /** A sheet from the bottom instead of a panel beside the anchor. */
  sheet?: boolean
  width?: number
  testId?: string
  /** Prefer the side of the anchor (default: right of it on wide screens, below it otherwise). */
  side?: 'right' | 'below'
}

export function ControlPopover({ anchor, onClose, label, children, sheet, width = 340, testId, side = 'right' }: ControlPopoverProps) {
  const { t } = useAiwsText()
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null)
  useOverlayMounted(true)
  useLayoutEffect(() => {
    if (sheet || !ref.current) return
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
  }, [anchor.left, anchor.top, anchor.width, anchor.height, sheet, side])
  useEffect(() => {
    const onDown = (event: PointerEvent) => { if (!ref.current?.contains(event.target as Node)) onClose() }
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') { event.stopPropagation(); onClose() } }
    // the press that opened it is over before these listen
    const timer = window.setTimeout(() => {
      window.addEventListener('pointerdown', onDown, true)
      window.addEventListener('keydown', onKey, true)
    }, 0)
    return () => { window.clearTimeout(timer); window.removeEventListener('pointerdown', onDown, true); window.removeEventListener('keydown', onKey, true) }
  }, [onClose])
  const body = (
    <div ref={ref} className={`aiws-ctl-popover${sheet ? ' is-sheet' : ''}`} role="dialog" aria-label={label} data-testid={testId}
      style={sheet ? undefined : { width, left: pos?.left ?? anchor.left, top: pos?.top ?? anchor.top + anchor.height, visibility: pos ? 'visible' : 'hidden' }}
      onPointerDown={(event) => event.stopPropagation()} onKeyDown={(event) => event.stopPropagation()}>
      {sheet && <div className="aiws-ctl-sheet-grip" aria-hidden="true" />}
      <button type="button" className="aiws-ctl-popover-close" aria-label={t('aiworkspace.common.close')} title={t('aiworkspace.common.close')} onClick={onClose}><X size={16} aria-hidden="true" /></button>
      {children}
    </div>
  )
  return createPortal(sheet ? <div className="aiws-ctl-sheet-backdrop" onPointerDown={(e) => { if (e.target === e.currentTarget) onClose() }}>{body}</div> : body, portalRoot())
}

export interface MenuItem { id: string; label: string; icon?: ReactNode; danger?: boolean; disabled?: string | false; checked?: boolean; run: () => void }

/** A small action menu in a pop-over (a card's "Move to…", a day's actions). */
export function ControlMenu({ items, onClose, label }: { items: (MenuItem | 'separator')[]; onClose: () => void; label: string }) {
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => { (ref.current?.querySelector('button:not(:disabled)') as HTMLButtonElement | null)?.focus() }, [])
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
