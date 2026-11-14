/* The context window (UI 框架 PRD §6.4): what a tap on a Block of a phone's free canvas opens, in place of the near
 * toolbar. Close to the Block — above it, else below, always on the screen, at the bottom when the Block fills it. The
 * first row is the type's icon and the title; the second the actions: expand, properties, comment, at most two primary
 * actions of the type that apply in view mode (an extension's declared ones too), and ⋯ — the Block's whole menu. All of
 * them are the Block's entries (ui/blockui/entries.ts), the same as its context menu: one name, one reason. A tap on a
 * blank spot closes it, a tap on another Block moves it there (the canvas's selection). */

import { useLayoutEffect, useRef, useState } from 'react'
import { Ellipsis, Sparkles, type LucideIcon } from 'lucide-react'
import { useAiwsText } from '../../useAiwsText'
import type { BlockEntry } from '../blockui/entries'
import type { Rect } from '../canvas/render/camera'
import './phone.css'

/** How many primary actions of the type it shows. */
const PRIMARY = 2
const GAP = 10

export function ContextWindow({ bbox, viewport, insets, icon: Icon, typeTitle, title, entries, onMore }: {
  /** The selection on screen (canvas area coordinates). */
  bbox: Rect
  viewport: { w: number; h: number }
  /** What the title zone and the status bar cover. */
  insets: { top: number; bottom: number }
  icon: LucideIcon
  typeTitle: string
  title: string
  entries: BlockEntry[]
  onMore: (anchor: HTMLElement) => void
}) {
  const { t } = useAiwsText()
  const ref = useRef<HTMLDivElement>(null)
  const [size, setSize] = useState({ w: 0, h: 0 })
  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    const measure = () => setSize((s) => (s.w === el.offsetWidth && s.h === el.offsetHeight ? s : { w: el.offsetWidth, h: el.offsetHeight }))
    measure()
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(measure)
    observer.observe(el)
    return () => observer.disconnect()
  }, [])
  const minY = insets.top + 8
  const maxY = viewport.h - insets.bottom - 8
  const above = bbox.y - GAP - size.h
  const below = bbox.y + bbox.h + GAP
  const at: 'above' | 'below' | 'bottom' = above >= minY ? 'above' : below + size.h <= maxY ? 'below' : 'bottom'
  const top = at === 'above' ? above : at === 'below' ? below : Math.max(minY, maxY - size.h)
  const left = Math.max(8, Math.min(bbox.x + bbox.w / 2 - size.w / 2, viewport.w - 8 - size.w))
  const byId = (id: string) => entries.find((e) => e.id === id)
  const actions = [byId('expand'), byId('inspector'), byId('annotate'), ...entries.filter((e) => e.group === 'type' && e.primary).slice(0, PRIMARY)]
    .filter((e): e is BlockEntry => Boolean(e))
  const name = title || typeTitle
  return (
    <div ref={ref} className="aiws-panel aiws-ctxwin" role="toolbar" aria-label={t('aiworkspace.phone.window', { name })} data-testid="aiws-ctxwin" data-at={at}
      style={{ left: Math.round(left), top: Math.round(top), visibility: size.w === 0 ? 'hidden' : undefined }} onPointerDown={(event) => event.stopPropagation()}>
      <div className="aiws-ctxwin-head">
        <Icon size={16} aria-hidden="true" />
        <b data-testid="aiws-ctxwin-title">{name}</b>
        {title && <span className="aiws-muted">{typeTitle}</span>}
      </div>
      <div className="aiws-ctxwin-actions">
        {actions.map((entry) => {
          const ActionIcon = entry.icon ?? (entry.ai ? Sparkles : null)
          return (
            <button key={entry.id} type="button" className={`aiws-ctxwin-action${entry.ai ? ' is-ai' : ''}`} data-testid={`aiws-ctxwin-${entry.id}`} aria-label={entry.label} aria-pressed={entry.active || undefined}
              title={entry.disabled || entry.label} disabled={Boolean(entry.disabled)} onClick={entry.run}>
              {ActionIcon && <ActionIcon size={20} aria-hidden="true" />}
              <span>{entry.label}</span>
            </button>
          )
        })}
        <button type="button" className="aiws-ctxwin-action" data-testid="aiws-ctxwin-more" aria-label={t('aiworkspace.canvas.near.more')} aria-haspopup="menu" onClick={(event) => onMore(event.currentTarget)}>
          <Ellipsis size={20} aria-hidden="true" />
          <span>{t('aiworkspace.phone.more')}</span>
        </button>
      </div>
    </div>
  )
}
