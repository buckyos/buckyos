/* eslint-disable react-refresh/only-export-components -- a Block definition with its picker and art */
/* The `shape` Block (draw.io 导入 TODO §2.1, §2.4): a pure UI Block drawn as an SVG outline — the basic geometries
 * of draw.io (rect … arrows, line, text) or an SVG body of its own — with multi-line text inside it or beside it.
 * Geometry, keys and anchors come from the shared module (`@drawio/shapes`), so what the importer writes draws
 * here unchanged. Colours are `#rrggbb`, `none`, or `auto` (the theme's surface and ink, draw.io's `default`).
 *
 *   look     fill, stroke, stroke_width, dash, opacity, radius, direction, flip_h / flip_v, shape_size
 *   text     text (multi-line; `title` stays the outline's name), font_size, font_color, bold / italic /
 *            underline, align, valign, label_position, wrap, label_fill
 *   anchors  the outline's compass anchors plus `config.anchors` (draw.io connection points, stencil ports) */

import { useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react'
import { AlignCenter, AlignLeft, AlignRight, AlignVerticalJustifyCenter, AlignVerticalJustifyEnd, AlignVerticalJustifyStart, ALargeSmall, Baseline, Bold, Italic, PenLine, Shapes, Underline } from 'lucide-react'
import { cleanAnchors, type AnchorDef } from '@drawio/anchors'
import { DEFAULT_FILL, DEFAULT_STROKE, KIND_GROUPS, SHAPE_KINDS, dashArray, inkOn, labelBox, readLook, shapeAnchors, shapeOutline, shapePaths, type ShapeKind, type ShapeLook } from '@drawio/shapes'
import { sanitizeSvgBody } from '@drawio/svg'
import type { CellPayload, Json, Operation } from '../../api/types'
import { useStore } from '../../state/hooks'
import type { WorkspaceStore } from '../../state/store'
import { consumeIntent, useIntent } from './editorToolbar'
import { cellOp, setCellConfig } from './ops'
import type { BlockDefinition, RenderContext, ToolbarItem } from './registry'

// ---- drawing

const paintOf = (value: string, kind: 'fill' | 'stroke') => (value === 'auto' ? `var(--aiws-shape-auto-${kind})` : value)

/** The ink of the text: its own colour, else readable on the fill (dark on light), else the theme's. */
function inkFor(look: ShapeLook): string {
  if (look.fontColor && look.fontColor !== 'auto') return look.fontColor
  const onFill = look.labelPosition === 'inside' && look.fill.startsWith('#') ? inkOn(look.fill) : null
  return onFill ?? 'var(--cp-text)'
}

/** The outline (or SVG body) of a shape in a `w × h` box. */
export function ShapeArt({ look, w, h }: { look: ShapeLook; w: number; h: number }) {
  const fill = paintOf(look.fill, 'fill')
  const stroke = paintOf(look.stroke, 'stroke')
  if (look.kind === 'svg') {
    if (!look.svg) return <svg className="aiws-shape-art" viewBox="0 0 10 10" preserveAspectRatio="none"><rect width="10" height="10" style={{ fill, stroke }} /></svg>
    const vars = { '--aiws-shape-fill': fill, '--aiws-shape-stroke': stroke, '--aiws-shape-stroke-width': look.strokeWidth } as CSSProperties
    return <svg className="aiws-shape-art" viewBox={look.svg.viewBox} preserveAspectRatio={look.svg.aspect === 'fixed' ? 'xMidYMid meet' : 'none'} style={vars} dangerouslySetInnerHTML={{ __html: sanitizeSvgBody(look.svg.body) }} />
  }
  // plain text draws a box only when it has a background or a border
  if (look.kind === 'text' && look.fill === 'none' && look.stroke === 'none') return null
  const paths = shapePaths(look.kind === 'text' ? { ...look, kind: 'rect' } : look, Math.max(w, 1), Math.max(h, 1))
  const width = look.stroke === 'none' ? 0 : look.strokeWidth
  const style: CSSProperties = { fill: paths.open ? 'none' : fill, stroke, strokeWidth: width, strokeLinejoin: 'round', strokeDasharray: dashArray(look.dash, width) ?? undefined }
  return (
    <svg className="aiws-shape-art" viewBox={`0 0 ${Math.max(w, 1)} ${Math.max(h, 1)}`} preserveAspectRatio="none">
      <path d={paths.body} style={style} />
      {paths.details.map((d, i) => <path key={i} d={d} style={{ fill: 'none', stroke, strokeWidth: width }} />)}
    </svg>
  )
}

/** Where the text sits (inside, or one box away on a side) and how it is set. */
function labelStyle(look: ShapeLook, w: number, h: number): CSSProperties {
  const box = labelBox(look, Math.max(w, 1), Math.max(h, 1))
  return {
    left: `${(box.x / Math.max(w, 1)) * 100}%`, top: `${(box.y / Math.max(h, 1)) * 100}%`,
    justifyContent: look.valign === 'top' ? 'flex-start' : look.valign === 'bottom' ? 'flex-end' : 'center',
    textAlign: look.align, fontSize: look.fontSize, color: inkFor(look),
    fontWeight: look.bold ? 700 : undefined, fontStyle: look.italic ? 'italic' : undefined, textDecoration: look.underline ? 'underline' : undefined,
    whiteSpace: look.wrap ? 'pre-wrap' : 'pre',
  }
}

function ShapeFrame({ context, children }: { context: RenderContext; children?: ReactNode }) {
  const look = readLook(context.payload.config, context.payload.title)
  const { w, h } = context.size
  return (
    <div className={`aiws-shape is-${look.kind}`} style={{ opacity: look.opacity < 1 ? look.opacity : undefined }} data-kind={look.kind} data-testid={`aiws-shape-${context.cell.entity_id}`}>
      <ShapeArt look={look} w={w} h={h} />
      <div className="aiws-shape-label" style={labelStyle(look, w, h)}>
        {children ?? (look.text && <span className="aiws-shape-text" style={look.labelFill ? { background: paintOf(look.labelFill, 'fill') } : undefined}>{look.text}</span>)}
      </div>
    </div>
  )
}

function ShapeStatic(context: RenderContext) {
  return <ShapeFrame context={context} />
}

// ---- text editing: typed where it is shown; Enter (without Shift) or leaving saves, Esc keeps the old text

const firstLine = (text: string) => text.split('\n').find((line) => line.trim())?.trim().slice(0, 256) ?? ''

/** Write the text; the outline name (`title`) follows its first line while it was that line (or absent). */
function saveText(context: RenderContext, store: WorkspaceStore, next: string) {
  const config: Record<string, Json> = { ...(context.payload.config ?? {}) }
  const before = typeof config.text === 'string' ? config.text : context.payload.title ?? ''
  if (next === before) return
  if (next) config.text = next
  else delete config.text
  const id = context.cell.entity_id
  const keys: { key: string; value: Json; expect: { rev: number } }[] = [{ key: 'config', value: config, expect: { rev: context.keyRevs.config ?? 0 } }]
  const operations: Operation[] = []
  const title = context.payload.title
  const follows = !title || title === firstLine(before)
  if (follows && firstLine(next) && firstLine(next) !== title) keys.push({ key: 'title', value: firstLine(next), expect: { rev: context.keyRevs.title ?? 0 } })
  operations.push({ op: 'entity.set_keys', entity_id: id, keys })
  if (follows && title && !firstLine(next)) operations.push({ op: 'entity.unset_keys', entity_id: id, keys: [{ key: 'title', expect: { rev: context.keyRevs.title ?? 0 } }] })
  void store.submit({ editId: `key:${id}:text`, label: `文字 → ${firstLine(next) || '（清除）'}`, operations })
}

function ShapeEditor(context: RenderContext) {
  const store = useStore()
  const look = readLook(context.payload.config, context.payload.title)
  const ref = useRef<HTMLDivElement>(null)
  const closed = useRef(false)
  const intent = useIntent(context.cell.entity_id)
  const [started] = useState(() => intent?.value.startsWith('type:') ? intent.value.slice(5) : null)
  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    // typing on a selected shape replaces its text (as in draw.io); "edit" keeps it with the caret at the end
    el.textContent = started ?? look.text
    el.focus()
    const range = document.createRange()
    range.selectNodeContents(el)
    range.collapse(false)
    const selection = window.getSelection()
    selection?.removeAllRanges()
    selection?.addRange(range)
    if (intent) consumeIntent(context.cell.entity_id, intent)
    // the initial text is set once: later renders (a format change) must not touch what is being typed
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])
  const done = (save: boolean) => {
    if (closed.current) return
    closed.current = true
    if (save && ref.current) saveText(context, store, (ref.current.innerText ?? '').replace(/\n$/, ''))
    context.deactivateEditor()
  }
  return (
    <ShapeFrame context={context}>
      <div ref={ref} className="aiws-shape-text aiws-shape-edit" contentEditable="plaintext-only" suppressContentEditableWarning role="textbox" aria-multiline="true" aria-label="形状文字"
        data-role="editor" data-testid={`aiws-shape-edit-${context.cell.entity_id}`} data-placeholder="输入文字"
        onBlur={() => done(true)}
        onKeyDown={(event) => {
          if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); done(true) }
          else if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); done(false) }
        }} />
    </ShapeFrame>
  )
}

// ---- picking a kind: the grouped picker of the object toolbar, the insert catalog and the near toolbar

export function ShapePicker({ value, onPick, extra }: { value?: ShapeKind; onPick: (kind: ShapeKind) => void; extra?: ReactNode }) {
  return (
    <div className="aiws-shape-picker" data-testid="aiws-shape-picker">
      {KIND_GROUPS.map(({ group, label }) => (
        <div key={group} className="aiws-shape-picker-group">
          <div className="aiws-shape-picker-title">{label}</div>
          <div className="aiws-shape-picker-grid">
            {SHAPE_KINDS.filter((k) => k.group === group).map((k) => {
              // parts measured in world units (a note's fold, a cylinder's rim) are scaled down to the icon
              const look = readLook({ shape: k.kind, ...(k.kind === 'text' ? { text: 'Aa', font_size: 13 } : {}), ...(k.size && !k.size.relative ? { shape_size: 6 } : {}), stroke_width: 1.5 })
              return (
                <button key={k.kind} type="button" className="aiws-shape-picker-item" title={k.label} aria-label={k.label} aria-pressed={value === k.kind} data-testid={`aiws-shape-kind-${k.kind}`}
                  onMouseDown={(event) => event.preventDefault()} onClick={() => onPick(k.kind)}>
                  <span className="aiws-shape-picker-art">
                    <ShapeArt look={look} w={34} h={24} />
                    {k.kind === 'text' && <span className="aiws-shape-picker-text">Aa</span>}
                  </span>
                </button>
              )
            })}
          </div>
        </div>
      ))}
      {extra}
    </div>
  )
}

// ---- tools (标准对象的交互改进 §5.2): kind, fill, stroke, line, text

const FONT_SIZES = [10, 12, 14, 16, 20, 24, 32, 48]
const WIDTHS = [{ value: '0', label: '无' }, { value: '1', label: '细（1）' }, { value: '2', label: '中（2）' }, { value: '4', label: '粗（4）' }, { value: '8', label: '特粗（8）' }]
const DASHES = [{ value: 'solid', label: '实线' }, { value: 'dashed', label: '虚线' }, { value: 'dotted', label: '点线' }]
const OPACITIES = [{ value: '1', label: '100%' }, { value: '0.75', label: '75%' }, { value: '0.5', label: '50%' }, { value: '0.25', label: '25%' }]
const RADII = [{ value: '0', label: '直角' }, { value: '4', label: '小（4）' }, { value: '8', label: '中（8）' }, { value: '16', label: '大（16）' }, { value: '32', label: '特大（32）' }]
const DIRECTIONS = [{ value: 'e', label: '向右' }, { value: 's', label: '向下' }, { value: 'w', label: '向左' }, { value: 'n', label: '向上' }]
const ALIGNS = [{ value: 'left', label: '左对齐', icon: AlignLeft }, { value: 'center', label: '居中', icon: AlignCenter }, { value: 'right', label: '右对齐', icon: AlignRight }]
const VALIGNS = [{ value: 'top', label: '顶端', icon: AlignVerticalJustifyStart }, { value: 'middle', label: '垂直居中', icon: AlignVerticalJustifyCenter }, { value: 'bottom', label: '底端', icon: AlignVerticalJustifyEnd }]
const LABEL_POSITIONS = [{ value: 'inside', label: '形状内' }, { value: 'top', label: '上方' }, { value: 'bottom', label: '下方' }, { value: 'left', label: '左侧' }, { value: 'right', label: '右侧' }]
const FILL_EXTRAS = [{ value: 'none', label: '无填充' }, { value: 'auto', label: '跟随主题' }]
const STROKE_EXTRAS = [{ value: 'none', label: '无描边' }, { value: 'auto', label: '跟随主题' }]
const INK_EXTRAS = [{ value: 'auto', label: '自动' }]

const canChange = (context: RenderContext): string | false => (context.capabilities.includes('update') && context.mode === 'edit' && context.readOnlyReason === null ? false : '没有修改此 Block 的权限')
const roundCorners = (kind: ShapeKind) => !['ellipse', 'cloud', 'cylinder', 'line', 'svg', 'document'].includes(kind)
const turnable = (kind: ShapeKind) => !['rect', 'ellipse', 'cloud', 'cross', 'text', 'svg'].includes(kind)

function LinePanel({ context, store, look, close }: { context: RenderContext; store: WorkspaceStore; look: ShapeLook; close: () => void }) {
  const set = (patch: Record<string, Json | null>, label: string) => setCellConfig(context, store, patch, label)
  const row = (label: string, options: { value: string; label: string }[], value: string, pick: (v: string) => void, id: string) => (
    <div className="aiws-shape-panel-row">
      <span>{label}</span>
      <div className="aiws-segmented" role="group" aria-label={label}>
        {options.map((o) => <button key={o.value} type="button" aria-pressed={o.value === value} data-testid={`aiws-shape-${id}-${o.value}`} onMouseDown={(e) => e.preventDefault()} onClick={() => pick(o.value)}>{o.label}</button>)}
      </div>
    </div>
  )
  return (
    <div className="aiws-shape-panel" data-testid="aiws-shape-line-panel">
      {row('线宽', WIDTHS, String(look.stroke === 'none' ? 0 : look.strokeWidth), (v) => set(v === '0' ? { stroke: 'none' } : { stroke_width: Number(v), ...(look.stroke === 'none' ? { stroke: null } : {}) }, `线宽 → ${v}`), 'width')}
      {row('线型', DASHES, look.dash, (v) => set({ dash: v === 'solid' ? null : v }, `线型 → ${DASHES.find((d) => d.value === v)?.label}`), 'dash')}
      {row('透明度', OPACITIES, String(look.opacity), (v) => set({ opacity: v === '1' ? null : Number(v) }, `透明度 → ${Math.round(Number(v) * 100)}%`), 'opacity')}
      {roundCorners(look.kind) && row('圆角', RADII, String(look.radius), (v) => set({ radius: v === '0' ? null : Number(v) }, `圆角 → ${v}`), 'radius')}
      {turnable(look.kind) && row('方向', DIRECTIONS, look.direction, (v) => set({ direction: v === 'e' ? null : v }, `方向 → ${DIRECTIONS.find((d) => d.value === v)?.label}`), 'direction')}
      <div className="aiws-shape-panel-row">
        <span>翻转</span>
        <div className="aiws-segmented" role="group" aria-label="翻转">
          <button type="button" aria-pressed={look.flipH} data-testid="aiws-shape-flip-h" onMouseDown={(e) => e.preventDefault()} onClick={() => set({ flip_h: look.flipH ? null : true }, '水平翻转')}>水平</button>
          <button type="button" aria-pressed={look.flipV} data-testid="aiws-shape-flip-v" onMouseDown={(e) => e.preventDefault()} onClick={() => set({ flip_v: look.flipV ? null : true }, '垂直翻转')}>垂直</button>
        </div>
      </div>
      <div className="aiws-dialog-actions"><button type="button" onClick={close}>完成</button></div>
    </div>
  )
}

function shapeToolbar(context: RenderContext, store: WorkspaceStore): ToolbarItem[] {
  const look = readLook(context.payload.config, context.payload.title)
  const reason = canChange(context)
  const set = (patch: Record<string, Json | null>, label: string) => setCellConfig(context, store, patch, label)
  const kindLabel = SHAPE_KINDS.find((k) => k.kind === look.kind)?.label ?? 'SVG 形状'
  return [
    { kind: 'panel', id: 'shape-kind', icon: Shapes, label: `形状：${kindLabel}`, disabled: reason,
      render: (close) => <ShapePicker value={look.kind} onPick={(kind) => { close(); if (kind !== look.kind) set({ shape: kind, ...(kind === 'svg' ? {} : { svg: null }) }, `形状 → ${SHAPE_KINDS.find((k) => k.kind === kind)?.label ?? kind}`) }} /> },
    ...(look.kind === 'line' ? [] : [{ kind: 'color' as const, id: 'shape-fill', label: '填充', palette: 'fill' as const, extras: FILL_EXTRAS, value: look.fill, disabled: reason, onPick: (value: string) => set({ fill: value || null }, '形状填充') }]),
    { kind: 'color', id: 'shape-stroke', label: look.kind === 'line' ? '线条颜色' : '描边', palette: 'ink', extras: STROKE_EXTRAS, value: look.stroke, disabled: reason, onPick: (value) => set({ stroke: value || null }, '形状描边') },
    { kind: 'panel', id: 'shape-line', icon: PenLine, label: '线宽、线型、透明度、圆角、方向', disabled: reason, render: (close) => <LinePanel context={context} store={store} look={look} close={close} /> },
    { kind: 'separator', id: 'shape-sep-text' },
    { kind: 'menu', id: 'shape-font-size', icon: ALargeSmall, label: '字号', value: String(look.fontSize), disabled: reason, items: FONT_SIZES.map((s) => ({ value: String(s), label: `${s}` })), onPick: (value) => set({ font_size: Number(value) }, `字号 → ${value}`) },
    { kind: 'color', id: 'shape-font-color', label: '文字颜色', palette: 'ink', extras: INK_EXTRAS, value: look.fontColor || 'auto', disabled: reason, onPick: (value) => set({ font_color: value && value !== 'auto' ? value : null }, '文字颜色') },
    { kind: 'button', id: 'shape-bold', icon: Bold, label: '粗体', active: look.bold, disabled: reason, run: () => set({ bold: look.bold ? null : true }, look.bold ? '取消粗体' : '粗体') },
    { kind: 'button', id: 'shape-italic', icon: Italic, label: '斜体', active: look.italic, disabled: reason, run: () => set({ italic: look.italic ? null : true }, look.italic ? '取消斜体' : '斜体') },
    { kind: 'button', id: 'shape-underline', icon: Underline, label: '下划线', active: look.underline, disabled: reason, run: () => set({ underline: look.underline ? null : true }, look.underline ? '取消下划线' : '下划线') },
    { kind: 'menu', id: 'shape-align', icon: ALIGNS.find((a) => a.value === look.align)?.icon ?? AlignCenter, label: '水平对齐', value: look.align, disabled: reason, items: ALIGNS, onPick: (value) => set({ align: value }, `对齐 → ${ALIGNS.find((a) => a.value === value)?.label}`) },
    { kind: 'menu', id: 'shape-valign', icon: VALIGNS.find((a) => a.value === look.valign)?.icon ?? AlignVerticalJustifyCenter, label: '垂直对齐', value: look.valign, disabled: reason, items: VALIGNS, onPick: (value) => set({ valign: value === 'middle' ? null : value }, `垂直对齐 → ${VALIGNS.find((a) => a.value === value)?.label}`) },
    ...(context.editorActive ? [] : [{ kind: 'button' as const, id: 'shape-text', icon: Baseline, label: '编辑文字', key: 'Enter', disabled: reason, run: () => context.activateEditor() }]),
  ]
}

// ---- inspector (属性面板): every key

function ShapeInspector(context: RenderContext) {
  const store = useStore()
  const look = readLook(context.payload.config, context.payload.title)
  const config = context.payload.config ?? {}
  const readOnly = canChange(context) !== false
  const set = (patch: Record<string, Json | null>, label: string) => { if (!readOnly) setCellConfig(context, store, patch, label) }
  const select = (key: string, label: string, options: { value: string; label: string }[], value: string, fallback: string) => (
    <label className="aiws-inline-form">{label}
      <select value={value} disabled={readOnly} data-testid={`aiws-shape-field-${key}`} onChange={(e) => set({ [key]: e.target.value === fallback ? null : e.target.value }, `${label} → ${e.target.value}`)}>
        {options.map((o) => <option key={o.value} value={o.value}>{o.label}</option>)}
      </select>
    </label>
  )
  const number = (key: string, label: string, value: number | null, fallback?: number) => (
    <label className="aiws-inline-form">{label}
      <input type="number" step="any" defaultValue={value ?? ''} key={`${key}:${value}`} disabled={readOnly} data-testid={`aiws-shape-field-${key}`}
        onBlur={(e) => { const v = e.target.value.trim(); const n = Number(v); if (v === '' || n === fallback) set({ [key]: null }, `${label}（默认）`); else if (Number.isFinite(n) && n !== value) set({ [key]: n }, `${label} → ${n}`) }} />
    </label>
  )
  const color = (key: string, label: string, value: string, extras: string[]) => (
    <label className="aiws-inline-form">{label}
      <span className="aiws-color-field">
        <input type="color" value={value.startsWith('#') ? value : '#ffffff'} disabled={readOnly} data-testid={`aiws-shape-field-${key}`} onChange={(e) => set({ [key]: e.target.value }, label)} />
        {extras.map((x) => <button key={x} type="button" aria-pressed={value === x} disabled={readOnly} onClick={() => set({ [key]: x }, `${label} → ${x === 'none' ? '无' : '跟随主题'}`)}>{x === 'none' ? '无' : '主题'}</button>)}
      </span>
    </label>
  )
  /** A flag whose default is `on` (stored only when it differs). */
  const flag = (key: string, label: string, value: boolean, on = false) => (
    <label className="aiws-inline-form"><input type="checkbox" checked={value} disabled={readOnly} data-testid={`aiws-shape-field-${key}`} onChange={(e) => set({ [key]: e.target.checked === on ? null : e.target.checked }, label)} />{label}</label>
  )
  const anchors = cleanAnchors(config.anchors) ?? []
  return (
    <div className="aiws-shape-inspector" data-testid="aiws-shape-inspector">
      {select('shape', '形状', [...SHAPE_KINDS.map((k) => ({ value: k.kind, label: k.label })), ...(look.kind === 'svg' ? [{ value: 'svg', label: 'SVG 形状' }] : [])], look.kind, '')}
      {color('fill', '填充', look.fill, ['none', 'auto'])}
      {color('stroke', '描边', look.stroke, ['none', 'auto'])}
      {number('stroke_width', '线宽', look.strokeWidth, 2)}
      {select('dash', '线型', DASHES, look.dash, 'solid')}
      {number('opacity', '透明度（0–1）', look.opacity, 1)}
      {roundCorners(look.kind) && number('radius', '圆角', look.radius, 0)}
      {turnable(look.kind) && select('direction', '方向', DIRECTIONS, look.direction, 'e')}
      {flag('flip_h', '水平翻转', look.flipH)}
      {flag('flip_v', '垂直翻转', look.flipV)}
      {number('shape_size', '形状参数（draw.io size）', look.size)}
      <label className="aiws-field">文字
        <textarea rows={3} defaultValue={look.text} key={`text:${look.text}`} disabled={readOnly} data-testid="aiws-shape-field-text" onBlur={(e) => { if (!readOnly) saveText(context, store, e.target.value) }} />
      </label>
      {number('font_size', '字号', look.fontSize, 14)}
      {color('font_color', '文字颜色', look.fontColor || 'auto', ['auto'])}
      {flag('bold', '粗体', look.bold)}
      {flag('italic', '斜体', look.italic)}
      {flag('underline', '下划线', look.underline)}
      {select('align', '水平对齐', ALIGNS, look.align, '')}
      {select('valign', '垂直对齐', VALIGNS, look.valign, 'middle')}
      {select('label_position', '文字位置', LABEL_POSITIONS, look.labelPosition, 'inside')}
      {flag('wrap', '自动换行', look.wrap, true)}
      {color('label_fill', '文字底色', look.labelFill ?? 'none', ['none', 'auto'])}
      <AnchorsForm anchors={anchors} readOnly={readOnly} onChange={(next) => set({ anchors: next.length ? (next as unknown as Json) : null }, '连接点')} />
    </div>
  )
}

/** The Block's own connection points (`config.anchors`): id and position in the box (0–1). */
function AnchorsForm({ anchors, readOnly, onChange }: { anchors: AnchorDef[]; readOnly: boolean; onChange: (next: AnchorDef[]) => void }) {
  const [draft, setDraft] = useState({ id: '', x: '0.5', y: '0' })
  const add = () => {
    const x = Number(draft.x), y = Number(draft.y)
    const id = draft.id.trim() || `p${anchors.length + 1}`
    if (!/^[A-Za-z0-9_.:-]{1,64}$/.test(id) || !Number.isFinite(x) || !Number.isFinite(y) || anchors.some((a) => a.id === id)) return
    onChange([...anchors, { id, x: Math.min(1, Math.max(0, x)), y: Math.min(1, Math.max(0, y)) }])
    setDraft({ id: '', x: '0.5', y: '0' })
  }
  return (
    <details className="aiws-details" data-testid="aiws-shape-anchors">
      <summary>连接点（{anchors.length}）</summary>
      <div className="aiws-muted">形状自带 16 个方位锚点；这里加的连接点以 0–1 表示在形状框内的位置，连接线可以连到它们上。</div>
      {anchors.map((a) => (
        <div key={a.id} className="aiws-inline-form"><code>{a.id}</code> ({a.x}, {a.y}){a.dx || a.dy ? ` + (${a.dx ?? 0}, ${a.dy ?? 0})` : ''}
          <button type="button" className="aiws-link" disabled={readOnly} onClick={() => onChange(anchors.filter((b) => b.id !== a.id))}>删除</button>
        </div>
      ))}
      {!readOnly && (
        <div className="aiws-inline-form">
          <input aria-label="连接点 id" placeholder="id" value={draft.id} onChange={(e) => setDraft({ ...draft, id: e.target.value })} data-testid="aiws-anchor-id" />
          <input aria-label="x" type="number" step="0.05" value={draft.x} onChange={(e) => setDraft({ ...draft, x: e.target.value })} />
          <input aria-label="y" type="number" step="0.05" value={draft.y} onChange={(e) => setDraft({ ...draft, y: e.target.value })} />
          <button type="button" onClick={add} data-testid="aiws-anchor-add">添加</button>
        </div>
      )}
    </details>
  )
}

// ---- the definition

const outlineOf = (payload: CellPayload) => shapeOutline(readLook(payload.config))

export const shapeDefinition: BlockDefinition = {
  type: 'shape', version: 1, title: '形状', accepts: [], allowNoSource: true, pureUi: true,
  defaultSize: { w: 160, h: 120 }, cost: { editor: false, html: false },
  Static: ShapeStatic, Editor: ShapeEditor, Inspector: ShapeInspector, chrome: 'none',
  shape: outlineOf,
  anchors: (payload) => shapeAnchors(payload.config, outlineOf(payload)),
  catalog: {
    group: 'layout', description: '矩形、椭圆、菱形、流程图符号、箭头等基本图形，可改填充、描边、线型和文字；也可以是导入或粘贴的 SVG 形状。', needs: 'none', standard: true,
    variants: [
      { id: 'text', title: '文字', group: 'text', description: '画布上的一段文字（无边框的形状），插入后直接输入。', config: { shape: 'text', text: '', font_size: 16 }, size: { w: 240, h: 48 }, editAfterInsert: true, standard: true },
      { id: 'svg', title: 'SVG 形状', group: 'layout', description: '粘贴 SVG 代码或选择 .svg 文件，变成可缩放、可改填充和描边颜色、可连线的形状。也可以直接把 SVG 粘贴或拖到画布上。', config: { shape: 'svg' } },
    ],
  },
  actions: [{ id: 'label', label: '编辑文字', modes: ['edit'], needs: ['update'], run: (context) => context.activateEditor(), key: 'Enter' }],
  toolbar: shapeToolbar,
  create: (args) => {
    const preset = args.config ?? {}
    const kind = typeof preset.shape === 'string' ? preset.shape : 'rect'
    const base: Record<string, Json> = kind === 'text' || kind === 'line' ? { shape: kind } : { shape: kind, fill: DEFAULT_FILL, stroke: DEFAULT_STROKE }
    return [cellOp({ ...args, config: undefined }, 'shape', undefined, { config: { ...base, ...preset } })]
  },
}
