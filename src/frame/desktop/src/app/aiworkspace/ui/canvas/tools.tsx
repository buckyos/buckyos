/* Canvas tools (phase two §8.1, §8.4; UI improvement §9.2; 标准对象的交互改进 §5): the near toolbar of the
 * selection (screen coordinates, avoiding the floating toolbars and the window edges), the context menu of a
 * blank spot or a Block, and the Block inspector. Tools follow the task: nothing sits permanently on every
 * Block. The near toolbar is a row of icons — the Editor's format tools while editing, the type's tools, the
 * common ones (annotate, lock, AI) — and "more"; what does not fit moves into "more". */

import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react'
import { ChevronDown, Ellipsis, Palette } from 'lucide-react'
import { useWorkspaceUi } from '../../state/hooks'
import { modePolicy, type CanvasMode, type ToolbarItem } from '../blocks/registry'
import { MenuButton, MenuList, PopoverPanel, usePopover, type MenuItem } from '../shell/popover'
import type { Rect } from './render/camera'

// ---- near toolbar

export interface NearAction { id: string; label: string; run: () => void; disabled?: boolean; title?: string; key?: string }

/** Colours offered by the toolbar's colour control: soft fills (notes, shapes) and inks (text, lines, frames).
 * Fixed values that read on both Desktop themes; "默认" (empty) means the theme's own colour. */
const PALETTES: Record<'fill' | 'ink', { value: string; label: string }[]> = {
  fill: [
    { value: '#fff2cc', label: '黄' }, { value: '#ffe2c6', label: '橙' }, { value: '#ffd9d9', label: '红' }, { value: '#f3dcff', label: '紫' },
    { value: '#dbe8ff', label: '蓝' }, { value: '#d5f2ee', label: '青' }, { value: '#dcf3d6', label: '绿' }, { value: '#ececec', label: '灰' },
  ],
  ink: [
    { value: '', label: '默认' }, { value: '#d1242f', label: '红' }, { value: '#d4600f', label: '橙' }, { value: '#9a6700', label: '黄褐' },
    { value: '#1a7f37', label: '绿' }, { value: '#0969da', label: '蓝' }, { value: '#8250df', label: '紫' }, { value: '#6e7781', label: '灰' },
  ],
}

/** Buttons keep the editor's focus and selection: the press never moves the focus. */
const keepFocus = (event: { preventDefault: () => void }) => event.preventDefault()

function tip(label: string, key?: string, disabled?: string | false) {
  return disabled ? `${label}：${disabled}` : key ? `${label}（${key}）` : label
}

function NearMenu({ item }: { item: Extract<ToolbarItem, { kind: 'menu' }> }) {
  const { open, toggle, close, bindAnchor, bindTrigger } = usePopover()
  const current = item.items.find((option) => option.value === item.value)
  const Icon = current?.icon ?? item.icon
  return (
    <span ref={bindAnchor} className="aiws-anchor">
      <button ref={bindTrigger} type="button" className="aiws-near-btn aiws-near-menu" data-testid={`aiws-near-${item.id}`} aria-label={item.label} title={tip(item.label, undefined, item.disabled)}
        aria-haspopup="menu" aria-expanded={open} disabled={Boolean(item.disabled)} onMouseDown={keepFocus} onClick={toggle}>
        {Icon ? <Icon size={18} aria-hidden="true" /> : <span className="aiws-near-value">{current?.label ?? item.label}</span>}
        <ChevronDown size={12} aria-hidden="true" />
      </button>
      {open && <MenuList label={item.label} className="aiws-popover aiws-popover-start" onDone={close}
        items={item.items.map((option) => ({ id: option.value, testId: `aiws-near-${item.id}-${option.value}`, label: option.label, checked: option.value === item.value, run: () => item.onPick(option.value) }))} />}
    </span>
  )
}

function NearColor({ item }: { item: Extract<ToolbarItem, { kind: 'color' }> }) {
  const { open, toggle, close, bindAnchor, bindTrigger } = usePopover()
  const palette = PALETTES[item.palette ?? 'fill']
  const pick = (value: string) => { close(); item.onPick(value) }
  return (
    <span ref={bindAnchor} className="aiws-anchor">
      <button ref={bindTrigger} type="button" className="aiws-near-btn" data-testid={`aiws-near-${item.id}`} aria-label={item.label} title={tip(item.label, undefined, item.disabled)}
        aria-haspopup="dialog" aria-expanded={open} disabled={Boolean(item.disabled)} onMouseDown={keepFocus} onClick={toggle}>
        {item.value === undefined ? <Palette size={18} aria-hidden="true" /> : <span className="aiws-swatch" style={{ background: item.value || 'var(--cp-text)' }} aria-hidden="true" />}
      </button>
      {open && (
        <PopoverPanel label={item.label} testId={`aiws-palette-${item.id}`} className="aiws-palette">
          <div className="aiws-palette-grid">
            {palette.map((color) => (
              <button key={color.value || 'default'} type="button" className="aiws-palette-swatch" aria-label={color.label} title={color.label} aria-pressed={(item.value ?? '') === color.value}
                data-testid={`aiws-color-${color.value.replace('#', '') || 'default'}`} style={{ background: color.value || 'var(--cp-text)' }} onClick={() => pick(color.value)} />
            ))}
          </div>
          <label className="aiws-palette-custom">自定义 <input type="color" aria-label="自定义颜色" defaultValue={item.value || '#4f8df7'} onChange={(event) => item.onPick(event.target.value)} /></label>
        </PopoverPanel>
      )}
    </span>
  )
}

function NearPanel({ item }: { item: Extract<ToolbarItem, { kind: 'panel' }> }) {
  const { open, toggle, close, bindAnchor, bindTrigger } = usePopover()
  return (
    <span ref={bindAnchor} className="aiws-anchor">
      <button ref={bindTrigger} type="button" className="aiws-near-btn" data-testid={`aiws-near-${item.id}`} aria-label={item.label} title={tip(item.label, undefined, item.disabled)}
        aria-haspopup="dialog" aria-expanded={open} aria-pressed={item.active} disabled={Boolean(item.disabled)} onMouseDown={keepFocus} onClick={toggle}>
        <item.icon size={18} aria-hidden="true" />
      </button>
      {open && <PopoverPanel label={item.label} testId={`aiws-panel-${item.id}`}>{item.render(close)}</PopoverPanel>}
    </span>
  )
}

function NearItem({ item }: { item: ToolbarItem }) {
  switch (item.kind) {
    case 'separator': return <span className="aiws-toolbar-sep" />
    case 'menu': return <NearMenu item={item} />
    case 'color': return <NearColor item={item} />
    case 'panel': return <NearPanel item={item} />
    case 'button': return (
      <button type="button" className={`aiws-near-btn${item.ai ? ' is-ai' : ''}${item.icon ? '' : ' is-text'}`} data-testid={`aiws-near-${item.id}`} aria-label={item.label} title={tip(item.label, item.key, item.disabled)}
        aria-pressed={item.active} disabled={Boolean(item.disabled)} onMouseDown={keepFocus} onClick={item.run}>
        {item.icon ? <item.icon size={18} aria-hidden="true" /> : item.label}
      </button>
    )
  }
}

/** What an item that did not fit becomes in "more". */
function asMenuItem(item: ToolbarItem): MenuItem | null {
  switch (item.kind) {
    case 'separator': return null
    case 'button': return { id: item.id, testId: `aiws-near-${item.id}`, label: item.label, hint: item.key, disabled: Boolean(item.disabled), reason: item.disabled || null, checked: item.active, run: item.run }
    case 'menu': return { id: item.id, testId: `aiws-near-${item.id}`, label: item.label, disabled: Boolean(item.disabled), items: item.items.map((o) => ({ id: o.value, label: o.label, checked: o.value === item.value, run: () => item.onPick(o.value) })) }
    case 'color': return { id: item.id, testId: `aiws-near-${item.id}`, label: item.label, disabled: Boolean(item.disabled), items: PALETTES[item.palette ?? 'fill'].map((c) => ({ id: c.value || 'default', label: c.label, checked: (item.value ?? '') === c.value, run: () => item.onPick(c.value) })) }
    case 'panel': return null
  }
}

export function NearToolbar({ bbox, items, more = [], viewport, insets = { top: 0, right: 0, bottom: 0, left: 0 } }: {
  bbox: Rect
  items: ToolbarItem[]
  more?: NearAction[]
  viewport: { w: number; h: number }
  /** Screen margins covered by the floating toolbars. */
  insets?: { top: number; right: number; bottom: number; left: number }
}) {
  const ref = useRef<HTMLDivElement>(null)
  const [size, setSize] = useState({ w: 0, h: 0 })
  // what fits: all items first, then as many as the clear width allows (the rest go to "more")
  const room = viewport.w - insets.left - insets.right - 8
  const fitKey = `${items.map((item) => item.id).join(',')}|${more.length}|${Math.round(room)}`
  const [fit, setFit] = useState<{ key: string; count: number }>({ key: fitKey, count: items.length })
  const count = fit.key === fitKey ? fit.count : items.length
  if (fit.key !== fitKey) setFit({ key: fitKey, count: items.length })
  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    setSize({ w: el.offsetWidth, h: el.offsetHeight })
    if (el.offsetWidth <= room || count === 0) return
    const children = [...el.querySelectorAll<HTMLElement>(':scope > [data-near-index]')]
    const moreWidth = 36
    let used = 8 + moreWidth
    let n = 0
    for (const child of children) { used += child.offsetWidth + 2; if (used > room) break; n += 1 }
    while (n > 0 && items[n - 1]?.kind === 'separator') n -= 1
    if (n < count) setFit({ key: fitKey, count: n })
  }, [fitKey, count, room, items])
  // above the selection, inside the unobstructed part of the viewport (edge avoidance, §8.4)
  let top = bbox.y - size.h - 8
  if (top < insets.top + 4) top = Math.min(viewport.h - insets.bottom - size.h - 4, bbox.y + bbox.h + 8)
  top = Math.max(insets.top + 4, top)
  let left = bbox.x + bbox.w / 2 - size.w / 2
  left = Math.max(insets.left + 4, Math.min(left, viewport.w - insets.right - size.w - 4))
  const shown = items.slice(0, count)
  while (shown.length > 0 && shown[shown.length - 1].kind === 'separator') shown.pop()
  const overflow = items.slice(count).map(asMenuItem).filter((item): item is MenuItem => item !== null)
  const moreItems: MenuItem[] = [
    ...overflow,
    ...(overflow.length > 0 && more.length > 0 ? [{ id: 'sep-overflow', separator: true }] : []),
    ...more.map((action) => ({ id: action.id, testId: `aiws-near-${action.id}`, label: action.label, hint: action.key, disabled: action.disabled, reason: action.title, run: action.run })),
  ]
  if (shown.length === 0 && moreItems.length === 0) return null
  return (
    <div ref={ref} className="aiws-near" role="toolbar" aria-label="就近工具" data-testid="aiws-near-toolbar" style={{ left, top }} onPointerDown={(event) => event.stopPropagation()}>
      {shown.map((item, i) => <span key={item.id} className="aiws-near-slot" data-near-index={i}><NearItem item={item} /></span>)}
      {moreItems.length > 0 && (
        <MenuButton label="更多操作" title="更多操作" className="aiws-near-btn aiws-near-more" testId="aiws-near-more" align="end" items={moreItems}>
          <Ellipsis size={18} />
        </MenuButton>
      )}
    </div>
  )
}

/** The same items drawn inside an Editor where there is no near toolbar (flow page, data-source view). */
export function InlineTools({ items, label }: { items: ToolbarItem[]; label: string }): ReactNode {
  if (items.length === 0) return null
  return (
    <div className="aiws-inline-tools" role="toolbar" aria-label={label}>
      {items.map((item) => <NearItem key={item.id} item={item} />)}
    </div>
  )
}

// ---- context menu

export function ContextMenu({ at, items, onClose }: { at: { x: number; y: number }; items: { id: string; label: string; run: () => void; disabled?: boolean; separator?: boolean }[]; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null)
  // opened near an edge (a long press on a phone), the menu moves back inside the canvas area
  const [spot, setSpot] = useState(at)
  useLayoutEffect(() => {
    const el = ref.current
    const area = el?.offsetParent
    if (!el || !(area instanceof HTMLElement)) return
    setSpot({ x: Math.max(8, Math.min(at.x, area.clientWidth - el.offsetWidth - 8)), y: Math.max(8, Math.min(at.y, area.clientHeight - el.offsetHeight - 8)) })
  }, [at.x, at.y, items.length])
  useEffect(() => {
    const onDown = (event: PointerEvent) => { if (!ref.current?.contains(event.target as Node)) onClose() }
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') onClose() }
    window.addEventListener('pointerdown', onDown, true)
    window.addEventListener('keydown', onKey)
    return () => { window.removeEventListener('pointerdown', onDown, true); window.removeEventListener('keydown', onKey) }
  }, [onClose])
  return (
    <div ref={ref} className="aiws-menu aiws-context-menu" role="menu" data-testid="aiws-context-menu" style={{ left: spot.x, top: spot.y }}>
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
