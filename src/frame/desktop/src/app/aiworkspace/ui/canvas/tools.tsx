/* Canvas tools (phase two §8.1, §8.4; UI improvement §9.2): the near toolbar of the selection (screen
 * coordinates, avoiding the floating toolbars and the window edges, secondary actions under "more"), the
 * context menu of a blank spot or a Block, and the Block inspector. Tools follow the task: nothing sits
 * permanently on every Block. */

import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react'
import { Ellipsis } from 'lucide-react'
import { useWorkspaceUi } from '../../state/hooks'
import { modePolicy, type CanvasMode } from '../blocks/registry'
import { MenuButton } from '../shell/popover'
import type { Rect } from './render/camera'

// ---- near toolbar

export interface NearAction { id: string; label: string; run: () => void; disabled?: boolean; title?: string; key?: string }

export function NearToolbar({ bbox, actions, more = [], viewport, insets = { top: 0, right: 0, bottom: 0, left: 0 } }: {
  bbox: Rect
  actions: NearAction[]
  more?: NearAction[]
  viewport: { w: number; h: number }
  /** Screen margins covered by the floating toolbars. */
  insets?: { top: number; right: number; bottom: number; left: number }
}) {
  const ref = useRef<HTMLDivElement>(null)
  const [size, setSize] = useState({ w: 0, h: 0 })
  useLayoutEffect(() => { const el = ref.current; if (el) setSize({ w: el.offsetWidth, h: el.offsetHeight }) }, [actions.length, more.length])
  // above the selection, inside the unobstructed part of the viewport (edge avoidance, §8.4)
  let top = bbox.y - size.h - 8
  if (top < insets.top + 4) top = Math.min(viewport.h - insets.bottom - size.h - 4, bbox.y + bbox.h + 8)
  top = Math.max(insets.top + 4, top)
  let left = bbox.x + bbox.w / 2 - size.w / 2
  left = Math.max(insets.left + 4, Math.min(left, viewport.w - insets.right - size.w - 4))
  if (actions.length === 0 && more.length === 0) return null
  return (
    <div ref={ref} className="aiws-near" role="toolbar" aria-label="就近工具" data-testid="aiws-near-toolbar" style={{ left, top }} onPointerDown={(event) => event.stopPropagation()}>
      {actions.map((action) => <button key={action.id} type="button" data-testid={`aiws-near-${action.id}`} disabled={action.disabled} title={action.title ?? (action.key ? `${action.label}（${action.key}）` : action.label)} onClick={action.run}>{action.label}</button>)}
      {more.length > 0 && (
        <MenuButton label="更多操作" title="更多操作" className="aiws-near-more" testId="aiws-near-more" align="end"
          items={more.map((action) => ({ id: action.id, testId: `aiws-near-${action.id}`, label: action.label, hint: action.key, disabled: action.disabled, reason: action.title, run: action.run }))}>
          <Ellipsis size={16} />
        </MenuButton>
      )}
    </div>
  )
}

// ---- context menu

export function ContextMenu({ at, items, onClose }: { at: { x: number; y: number }; items: { id: string; label: string; run: () => void; disabled?: boolean; separator?: boolean }[]; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const onDown = (event: PointerEvent) => { if (!ref.current?.contains(event.target as Node)) onClose() }
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') onClose() }
    window.addEventListener('pointerdown', onDown, true)
    window.addEventListener('keydown', onKey)
    return () => { window.removeEventListener('pointerdown', onDown, true); window.removeEventListener('keydown', onKey) }
  }, [onClose])
  return (
    <div ref={ref} className="aiws-menu aiws-context-menu" role="menu" data-testid="aiws-context-menu" style={{ left: at.x, top: at.y }}>
      {items.map((item) => item.separator ? <hr key={item.id} /> : (
        <button key={item.id} type="button" role="menuitem" data-testid={`aiws-menu-${item.id}`} disabled={item.disabled} onClick={() => { onClose(); item.run() }}>{item.label}</button>
      ))}
    </div>
  )
}

// ---- inspector

export function BlockInspector({ cellId, mode, children }: { cellId: string; mode: CanvasMode; children?: ReactNode }) {
  const ui = useWorkspaceUi()
  const entity = ui.byId.get(cellId)
  const policy = modePolicy(mode)
  if (!entity) return null
  return (
    <div className="aiws-inspector" data-testid="aiws-inspector" data-mode={mode}>
      <div className="aiws-panel-title">属性 <span className="aiws-muted">{entity.view_type ?? entity.kind}{entity.view_version ? ` v${entity.view_version}` : ''}</span></div>
      <div className="aiws-muted">{cellId}{entity.source_id ? ` · 数据 ${entity.source_id}` : ' · 纯 UI Block'}</div>
      {!policy.writes && <div className="aiws-muted">{mode === 'view' ? '查看模式：属性只读' : '播放编辑占位：只读'}</div>}
      {children}
    </div>
  )
}
