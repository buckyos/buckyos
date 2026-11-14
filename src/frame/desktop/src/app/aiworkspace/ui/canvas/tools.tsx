/* Canvas tools (phase two §8.1, §8.4; UI improvement §9.2; 标准对象的交互改进 §5): the near toolbar of the
 * selection (screen coordinates, avoiding the floating toolbars and the window edges), the context menu of a
 * blank spot or a Block, and the Block inspector. Tools follow the task: nothing sits permanently on every
 * Block. The near toolbar is a row of icons — the Editor's format tools while editing, the type's tools, the
 * common ones (annotate, lock, AI) — and "more"; what does not fit moves into "more". */

import { useEffect, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from 'react'
import { ChevronDown, Ellipsis, Palette, PencilRuler, Play } from 'lucide-react'
import type { RunMode } from '../../state/runMode'
import type { TextVars } from '../../i18n'
import { useAiwsText } from '../../useAiwsText'
import type { ToolbarItem } from '../blocks/registry'
import { MenuButton, MenuList, PopoverPanel, useOverlayMounted, usePopover, type MenuItem } from '../shell/popover'
import type { Rect } from './render/camera'
import '../blockui/blockui.css'

// ---- near toolbar

/** Colours offered by the toolbar's colour control: soft fills (notes, shapes) and inks (text, lines, frames).
 * Fixed values that read on both Desktop themes; "默认" (empty) means the theme's own colour. */
const PALETTES: Record<'fill' | 'ink', { value: string; key: string }[]> = {
  fill: [
    { value: '#fff2cc', key: 'aiworkspace.canvas.color.yellow' }, { value: '#ffe2c6', key: 'aiworkspace.canvas.color.orange' }, { value: '#ffd9d9', key: 'aiworkspace.canvas.color.red' }, { value: '#f3dcff', key: 'aiworkspace.canvas.color.purple' },
    { value: '#dbe8ff', key: 'aiworkspace.canvas.color.blue' }, { value: '#d5f2ee', key: 'aiworkspace.canvas.color.teal' }, { value: '#dcf3d6', key: 'aiworkspace.canvas.color.green' }, { value: '#ececec', key: 'aiworkspace.canvas.color.gray' },
  ],
  ink: [
    { value: '', key: 'aiworkspace.canvas.color.default' }, { value: '#d1242f', key: 'aiworkspace.canvas.color.red' }, { value: '#d4600f', key: 'aiworkspace.canvas.color.orange' }, { value: '#9a6700', key: 'aiworkspace.canvas.color.ochre' },
    { value: '#1a7f37', key: 'aiworkspace.canvas.color.green' }, { value: '#0969da', key: 'aiworkspace.canvas.color.blue' }, { value: '#8250df', key: 'aiworkspace.canvas.color.purple' }, { value: '#6e7781', key: 'aiworkspace.canvas.color.gray' },
  ],
}

type Translate = (key: string, vars?: TextVars) => string

/** The colours a colour control offers: its extras, then the palette, named in the current language. */
const paletteOf = (item: Extract<ToolbarItem, { kind: 'color' }>, t: Translate) => [...(item.extras ?? []), ...PALETTES[item.palette ?? 'fill'].map((c) => ({ value: c.value, label: t(c.key) }))]

/** Special colour values: no colour (`none`), the theme's colour (`auto`, or empty for "默认"). */
const swatchClass = (value: string) => (value === 'none' ? ' is-none' : value === 'auto' ? ' is-auto' : '')
const swatchStyle = (value: string) => (value === 'none' || value === 'auto' ? undefined : { background: value || 'var(--cp-text)' })

/** Buttons keep the editor's focus and selection: the press never moves the focus. */
const keepFocus = (event: { preventDefault: () => void }) => event.preventDefault()

function tip(t: Translate, label: string, key?: string, disabled?: string | false) {
  return disabled ? t('aiworkspace.canvas.tip.reason', { label, reason: disabled }) : key ? t('aiworkspace.canvas.tip.key', { label, key }) : label
}

function NearMenu({ item }: { item: Extract<ToolbarItem, { kind: 'menu' }> }) {
  const { t } = useAiwsText()
  const { open, toggle, close, bindAnchor, bindTrigger } = usePopover()
  const current = item.items.find((option) => option.value === item.value)
  const Icon = current?.icon ?? item.icon
  return (
    <span ref={bindAnchor} className="aiws-anchor">
      <button ref={bindTrigger} type="button" className="aiws-near-btn aiws-near-menu" data-testid={`aiws-near-${item.id}`} aria-label={item.label} title={tip(t, item.label, undefined, item.disabled)}
        aria-haspopup="menu" aria-expanded={open} disabled={Boolean(item.disabled)} onMouseDown={keepFocus} onClick={toggle}>
        {Icon ? <Icon size={18} aria-hidden="true" /> : <span className="aiws-near-value">{current?.label ?? item.label}</span>}
        <ChevronDown size={12} aria-hidden="true" />
      </button>
      {open && <MenuList label={item.label} className="aiws-popover aiws-popover-start" onDone={close}
        items={item.items.map((option) => ({ id: option.value, testId: `aiws-near-${item.id}-${option.value}`, label: option.label, checked: option.value === item.value, run: () => item.onPick(option.value) }))} />}
    </span>
  )
}

/** The colours of a colour control as a grid of swatches (the toolbar's palette pop-over, the properties panel). */
export function PaletteGrid({ label, palette = 'fill', extras, value, disabled, testId, onPick }: {
  label: string; palette?: 'fill' | 'ink'; extras?: { value: string; label: string }[]; value: string | undefined; disabled?: boolean; testId?: string; onPick: (hex: string) => void
}) {
  const { t } = useAiwsText()
  const colors = [...(extras ?? []), ...PALETTES[palette].map((c) => ({ value: c.value, label: t(c.key) }))]
  return (
    <div className="aiws-palette-body" role="group" aria-label={label} data-testid={testId}>
      <div className="aiws-palette-grid">
        {colors.map((color) => (
          <button key={color.value || 'default'} type="button" className={`aiws-palette-swatch${swatchClass(color.value)}`} aria-label={color.label} title={color.label} aria-pressed={(value ?? '') === color.value}
            disabled={disabled} data-testid={`aiws-color-${color.value.replace('#', '') || 'default'}`} style={swatchStyle(color.value)} onMouseDown={keepFocus} onClick={() => onPick(color.value)} />
        ))}
      </div>
      <label className="aiws-palette-custom">{t('aiworkspace.canvas.color.custom')} <input type="color" aria-label={t('aiworkspace.canvas.color.customColor')} disabled={disabled} defaultValue={value && value.startsWith('#') ? value : '#4f8df7'} onChange={(event) => onPick(event.target.value)} /></label>
    </div>
  )
}

function NearColor({ item }: { item: Extract<ToolbarItem, { kind: 'color' }> }) {
  const { t } = useAiwsText()
  const { open, toggle, close, bindAnchor, bindTrigger } = usePopover()
  const pick = (value: string) => { close(); item.onPick(value) }
  return (
    <span ref={bindAnchor} className="aiws-anchor">
      <button ref={bindTrigger} type="button" className="aiws-near-btn" data-testid={`aiws-near-${item.id}`} aria-label={item.label} title={tip(t, item.label, undefined, item.disabled)}
        aria-haspopup="dialog" aria-expanded={open} disabled={Boolean(item.disabled)} onMouseDown={keepFocus} onClick={toggle}>
        {item.value === undefined ? <Palette size={18} aria-hidden="true" /> : <span className={`aiws-swatch${swatchClass(item.value)}`} style={swatchStyle(item.value)} aria-hidden="true" />}
      </button>
      {open && (
        <PopoverPanel label={item.label} testId={`aiws-palette-${item.id}`} className="aiws-palette">
          <PaletteGrid label={item.label} palette={item.palette} extras={item.extras} value={item.value} onPick={pick} />
        </PopoverPanel>
      )}
    </span>
  )
}

function NearPanel({ item }: { item: Extract<ToolbarItem, { kind: 'panel' }> }) {
  const { t } = useAiwsText()
  const { open, toggle, close, bindAnchor, bindTrigger } = usePopover()
  return (
    <span ref={bindAnchor} className="aiws-anchor">
      <button ref={bindTrigger} type="button" className="aiws-near-btn" data-testid={`aiws-near-${item.id}`} aria-label={item.label} title={tip(t, item.label, undefined, item.disabled)}
        aria-haspopup="dialog" aria-expanded={open} aria-pressed={item.active} disabled={Boolean(item.disabled)} onMouseDown={keepFocus} onClick={toggle}>
        <item.icon size={18} aria-hidden="true" />
      </button>
      {open && <PopoverPanel label={item.label} testId={`aiws-panel-${item.id}`}>{item.render(close)}</PopoverPanel>}
    </span>
  )
}

/** One tool of a toolbar (the near toolbar, the flow title row, the properties panel's quick tools). */
export function ToolItem({ item }: { item: ToolbarItem }) {
  const { t } = useAiwsText()
  switch (item.kind) {
    case 'separator': return <span className="aiws-toolbar-sep" />
    case 'menu': return <NearMenu item={item} />
    case 'color': return <NearColor item={item} />
    case 'panel': return <NearPanel item={item} />
    case 'button': return (
      <button type="button" className={`aiws-near-btn${item.ai ? ' is-ai' : ''}${item.icon ? '' : ' is-text'}`} data-testid={`aiws-near-${item.id}`} aria-label={item.label} title={tip(t, item.label, item.key, item.disabled)}
        aria-pressed={item.active} disabled={Boolean(item.disabled)} onMouseDown={keepFocus} onClick={item.run}>
        {item.icon ? <item.icon size={18} aria-hidden="true" /> : item.label}
      </button>
    )
  }
}

/** What an item that did not fit becomes in "more"; a panel opens its pop-over from there (`openPanel`). */
function asMenuItem(item: ToolbarItem, t: Translate, openPanel: (item: Extract<ToolbarItem, { kind: 'panel' }>) => void): MenuItem | null {
  switch (item.kind) {
    case 'separator': return null
    case 'button': return { id: item.id, testId: `aiws-near-${item.id}`, label: item.label, hint: item.key, disabled: Boolean(item.disabled), reason: item.disabled || null, checked: item.active, run: item.run }
    case 'menu': return { id: item.id, testId: `aiws-near-${item.id}`, label: item.label, disabled: Boolean(item.disabled), reason: item.disabled || null, items: item.items.map((o) => ({ id: o.value, testId: `aiws-near-${item.id}-${o.value}`, label: o.label, checked: o.value === item.value, run: () => item.onPick(o.value) })) }
    case 'color': return { id: item.id, testId: `aiws-near-${item.id}`, label: item.label, disabled: Boolean(item.disabled), reason: item.disabled || null, items: paletteOf(item, t).map((c) => ({ id: c.value || 'default', label: c.label, checked: (item.value ?? '') === c.value, run: () => item.onPick(c.value) })) }
    case 'panel': return { id: item.id, testId: `aiws-near-${item.id}`, label: item.label, disabled: Boolean(item.disabled), reason: item.disabled || null, checked: item.active, run: () => openPanel(item) }
  }
}

/** How many of `items` fit in `room` (px) after `reserve`: measured from the slots drawn (all of them first). */
function useFit(ref: RefObject<HTMLDivElement | null>, items: ToolbarItem[], room: number, reserve: number, extra: string) {
  const fitKey = `${items.map((item) => item.id).join(',')}|${extra}|${Math.round(room)}`
  const [fit, setFit] = useState<{ key: string; count: number }>({ key: fitKey, count: items.length })
  const count = fit.key === fitKey ? fit.count : items.length
  if (fit.key !== fitKey) setFit({ key: fitKey, count: items.length })
  useLayoutEffect(() => {
    const el = ref.current
    if (!el || el.offsetWidth <= room || count === 0) return
    const children = [...el.querySelectorAll<HTMLElement>(':scope > [data-near-index]')]
    let used = 8 + reserve
    let n = 0
    for (const child of children) { used += child.offsetWidth + 2; if (used > room) break; n += 1 }
    while (n > 0 && items[n - 1]?.kind === 'separator') n -= 1
    if (n < count) setFit({ key: fitKey, count: n })
  }, [fitKey, count, room, items, reserve, ref])
  const shown = items.slice(0, count)
  while (shown.length > 0 && shown[shown.length - 1].kind === 'separator') shown.pop()
  return { shown, rest: items.slice(count) }
}

/** A tool's panel opened from "more": under the "more" button; outside presses and Esc close it. */
function MorePanel({ item, onClose }: { item: Extract<ToolbarItem, { kind: 'panel' }>; onClose: () => void }) {
  const ref = useRef<HTMLSpanElement>(null)
  useOverlayMounted(true)
  useEffect(() => {
    const onDown = (event: PointerEvent) => { if (!ref.current?.contains(event.target as Node)) onClose() }
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') { event.stopPropagation(); onClose() } }
    window.addEventListener('pointerdown', onDown, true)
    window.addEventListener('keydown', onKey, true)
    return () => { window.removeEventListener('pointerdown', onDown, true); window.removeEventListener('keydown', onKey, true) }
  }, [onClose])
  return <span ref={ref}><PopoverPanel label={item.label} testId={`aiws-panel-${item.id}`} align="end">{item.render(onClose)}</PopoverPanel></span>
}

export function NearToolbar({ bbox, items, more = [], viewport, insets = { top: 0, right: 0, bottom: 0, left: 0 } }: {
  bbox: Rect
  items: ToolbarItem[]
  /** The Block's menu (the same list as its context menu, PRD §10.3); tools that did not fit come before it. */
  more?: MenuItem[]
  viewport: { w: number; h: number }
  /** Screen margins covered by the floating toolbars. */
  insets?: { top: number; right: number; bottom: number; left: number }
}) {
  const { t } = useAiwsText()
  const ref = useRef<HTMLDivElement>(null)
  const [size, setSize] = useState({ w: 0, h: 0 })
  const [panel, setPanel] = useState<Extract<ToolbarItem, { kind: 'panel' }> | null>(null)
  // what fits: all items first, then as many as the clear width allows (the rest go to "more")
  const room = viewport.w - insets.left - insets.right - 8
  const { shown, rest } = useFit(ref, items, room, 36, String(more.length))
  // above the selection, inside the unobstructed part of the viewport (edge avoidance, §8.4)
  let top = bbox.y - size.h - 8
  if (top < insets.top + 4) top = Math.min(viewport.h - insets.bottom - size.h - 4, bbox.y + bbox.h + 8)
  top = Math.max(insets.top + 4, top)
  let left = bbox.x + bbox.w / 2 - size.w / 2
  left = Math.max(insets.left + 4, Math.min(left, viewport.w - insets.right - size.w - 4))
  // what did not fit and is not in the menu anyway (an action that is a tool is listed once)
  const listed = new Set(more.flatMap((m) => [m.id, ...(m.items ?? []).map((i) => i.id)]))
  const overflow = rest.filter((item) => !listed.has(item.id)).map((item) => asMenuItem(item, t, setPanel)).filter((item): item is MenuItem => item !== null)
  const moreItems: MenuItem[] = [
    ...overflow,
    ...(overflow.length > 0 && more.length > 0 ? [{ id: 'sep-overflow', separator: true }] : []),
    ...more,
  ]
  const visible = shown.length > 0 || moreItems.length > 0
  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    const measure = () => setSize((s) => (s.w === el.offsetWidth && s.h === el.offsetHeight ? s : { w: el.offsetWidth, h: el.offsetHeight }))
    measure()
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(measure)
    observer.observe(el)
    return () => observer.disconnect()
  }, [visible])
  if (!visible) return null
  return (
    <div ref={ref} className="aiws-near" role="toolbar" aria-label={t('aiworkspace.canvas.near.label')} data-testid="aiws-near-toolbar" style={{ left, top }} onPointerDown={(event) => event.stopPropagation()}>
      {shown.map((item, i) => <span key={item.id} className="aiws-near-slot" data-near-index={i}><ToolItem item={item} /></span>)}
      {moreItems.length > 0 && (
        <span className="aiws-anchor">
          <MenuButton label={t('aiworkspace.canvas.near.more')} title={t('aiworkspace.canvas.near.more')} className="aiws-near-btn aiws-near-more" testId="aiws-near-more" align="end" items={moreItems}>
            <Ellipsis size={18} />
          </MenuButton>
          {panel && <MorePanel item={panel} onClose={() => setPanel(null)} />}
        </span>
      )}
    </div>
  )
}

/** A row of tools in the flow page's title row (§6.2): what does not fit is left out (the Block's menu and its properties
 * panel have all of it); the row ends with ⋯, the Block's whole menu. */
export function ToolRow({ items, label, onMore, moreTestId, className = '' }: { items: ToolbarItem[]; label: string; onMore: (anchor: HTMLElement) => void; moreTestId: string; className?: string }) {
  const { t } = useAiwsText()
  const ref = useRef<HTMLDivElement>(null)
  const [room, setRoom] = useState(10_000)
  useLayoutEffect(() => {
    const el = ref.current?.parentElement
    if (!el || typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(() => setRoom(el.clientWidth))
    observer.observe(el)
    return () => observer.disconnect()
  }, [])
  const { shown } = useFit(ref, items, room, 36, '')
  return (
    <div ref={ref} className={`aiws-near aiws-tool-row ${className}`} role="toolbar" aria-label={label} onPointerDown={(event) => event.stopPropagation()}>
      {shown.map((item, i) => <span key={item.id} className="aiws-near-slot" data-near-index={i}><ToolItem item={item} /></span>)}
      <button type="button" className="aiws-near-btn aiws-near-more" aria-label={t('aiworkspace.canvas.near.more')} title={t('aiworkspace.canvas.near.more')} aria-haspopup="menu" data-testid={moreTestId}
        onMouseDown={keepFocus} onClick={(event) => onMore(event.currentTarget)}><Ellipsis size={18} aria-hidden="true" /></button>
    </div>
  )
}

/** The same items drawn inside an Editor where there is no near toolbar (flow page, data-source view). */
export function InlineTools({ items, label }: { items: ToolbarItem[]; label: string }): ReactNode {
  if (items.length === 0) return null
  return (
    <div className="aiws-inline-tools" role="toolbar" aria-label={label}>
      {items.map((item) => <ToolItem key={item.id} item={item} />)}
    </div>
  )
}

// ---- design / run mode (扩展控件 P2, Q27)

/** ▶ runs the canvas's extensions (buttons, HTML Blocks on a click, event handlers); ✎ designs it (clicks select). */
export function RunModeToggle({ runMode }: { runMode: RunMode }) {
  const { t } = useAiwsText()
  if (runMode.locked) return null
  const label = t(runMode.interactive ? 'aiworkspace.canvas.runMode.design' : 'aiworkspace.canvas.runMode.run')
  return (
    <button type="button" className="aiws-tool" data-testid="aiws-run-toggle" data-interactive={runMode.interactive ? 'true' : 'false'}
      aria-pressed={runMode.interactive} aria-label={label} title={label} onClick={() => runMode.set(!runMode.interactive)}>
      {runMode.interactive ? <PencilRuler size={16} aria-hidden="true" /> : <Play size={16} aria-hidden="true" />}
    </button>
  )
}

// ---- context menu

/** The context menu (§10.3): of a Block (its whole menu, folded groups as submenus) or of a blank spot. Opened near an
 * edge it moves back inside the canvas area; on a phone it is a sheet from the bottom that scrolls. */
export function ContextMenu({ at, items, onClose, label, testId = 'aiws-context-menu', sheet = false }: {
  at: { x: number; y: number }
  items: MenuItem[]
  onClose: () => void
  label?: string
  testId?: string
  sheet?: boolean
}) {
  const { t } = useAiwsText()
  const ref = useRef<HTMLDivElement>(null)
  const [spot, setSpot] = useState(at)
  // a touch that opened the menu (a tap's pointerup, a long press) is followed by its click: on a phone the sheet may lie
  // under the finger. A click is a choice only when it was pressed on the menu
  const pressed = useRef(false)
  useOverlayMounted(true)
  useLayoutEffect(() => {
    const el = ref.current
    const menu = el?.firstElementChild
    const area = el?.offsetParent
    if (!el || sheet || !(menu instanceof HTMLElement) || !(area instanceof HTMLElement)) return
    setSpot({ x: Math.max(8, Math.min(at.x, area.clientWidth - menu.offsetWidth - 8)), y: Math.max(8, Math.min(at.y, area.clientHeight - menu.offsetHeight - 8)) })
  }, [at.x, at.y, items.length, sheet])
  useEffect(() => {
    const onDown = (event: PointerEvent) => { if (!ref.current?.contains(event.target as Node)) onClose() }
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') onClose() }
    window.addEventListener('pointerdown', onDown, true)
    window.addEventListener('keydown', onKey)
    return () => { window.removeEventListener('pointerdown', onDown, true); window.removeEventListener('keydown', onKey) }
  }, [onClose])
  return (
    <div ref={ref} className={`aiws-context-menu${sheet ? ' is-sheet' : ''}`} data-testid={testId} style={sheet ? undefined : { left: spot.x, top: spot.y }}
      onPointerDown={(event) => { pressed.current = true; event.stopPropagation() }}
      onClickCapture={(event) => { if (!pressed.current && event.detail > 0) { event.preventDefault(); event.stopPropagation() } }}>
      <MenuList items={items} label={label ?? t('aiworkspace.blockui.menu')} className="aiws-context-menu-list" onDone={onClose} />
    </div>
  )
}
