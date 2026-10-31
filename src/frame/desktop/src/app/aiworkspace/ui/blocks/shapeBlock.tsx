/* eslint-disable react-refresh/only-export-components -- a Block definition with its picker and art */
/* The `shape` Block (draw.io 导入 TODO §2.1, §2.4): a pure UI Block drawn as an SVG outline — the basic geometries
 * of draw.io (rect … arrows, line, text) or an SVG body of its own — with multi-line text inside it or beside it.
 * Geometry, keys and anchors come from the shared module (`@drawio/shapes`), so what the importer writes draws
 * here unchanged. Colours are `#rrggbb`, `none`, or `auto` (the theme's surface and ink, draw.io's `default`).
 *
 *   look     fill, stroke, stroke_width, dash, opacity, radius, direction, flip_h / flip_v, shape_size
 *   text     text (multi-line; `title` stays the outline's name), font_size, font_color, bold / italic /
 *            underline, align, valign, label_position, wrap, label_fill, line_height (a multiple of the size),
 *            font_family (sans / serif / mono; the theme's when absent — PDF import writes it)
 *   anchors  the outline's compass anchors plus `config.anchors` (draw.io connection points, stencil ports) */

import { useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react'
import { AlignCenter, AlignLeft, AlignRight, AlignVerticalJustifyCenter, AlignVerticalJustifyEnd, AlignVerticalJustifyStart, ALargeSmall, Baseline, Bold, Italic, PenLine, Shapes, Underline } from 'lucide-react'
import { cleanAnchors, type AnchorDef } from '@drawio/anchors'
import { DEFAULT_FILL, DEFAULT_STROKE, FONT_FAMILIES, KIND_GROUPS, SHAPE_KINDS, dashArray, inkOn, labelBox, readLook, shapeAnchors, shapeOutline, shapePaths, type FontFamily, type ShapeKind, type ShapeLook } from '@drawio/shapes'
import { sanitizeSvgBody } from '@drawio/svg'
import { readPdfTextLayout } from '@pdf/text-layout'
import type { CellPayload, Json, Operation } from '../../api/types'
import { aiwsT, type AiwsText } from '../../i18n'
import { useAiwsText } from '../../useAiwsText'
import { useStore } from '../../state/hooks'
import type { WorkspaceStore } from '../../state/store'
import { consumeIntent, useIntent } from './editorToolbar'
import { cellOp, setCellConfig } from './ops'
import type { BlockDefinition, RenderContext, ToolbarItem } from './registry'

type T = AiwsText['t']

/** The names shown for the kinds and the picker's groups (the shared module knows the kinds by id only). */
const KIND_LABEL: Record<ShapeKind, string> = {
  rect: 'aiworkspace.blocks.shape.kind.rect', ellipse: 'aiworkspace.blocks.shape.kind.ellipse', diamond: 'aiworkspace.blocks.shape.kind.diamond',
  triangle: 'aiworkspace.blocks.shape.kind.triangle', hexagon: 'aiworkspace.blocks.shape.kind.hexagon', parallelogram: 'aiworkspace.blocks.shape.kind.parallelogram',
  trapezoid: 'aiworkspace.blocks.shape.kind.trapezoid', cross: 'aiworkspace.blocks.shape.kind.cross', cloud: 'aiworkspace.blocks.shape.kind.cloud',
  cylinder: 'aiworkspace.blocks.shape.kind.cylinder', document: 'aiworkspace.blocks.shape.kind.document', process: 'aiworkspace.blocks.shape.kind.process',
  card: 'aiworkspace.blocks.shape.kind.card', note: 'aiworkspace.blocks.shape.kind.note', callout: 'aiworkspace.blocks.shape.kind.callout',
  step: 'aiworkspace.blocks.shape.kind.step', arrow: 'aiworkspace.blocks.shape.kind.arrow', double_arrow: 'aiworkspace.blocks.shape.kind.doubleArrow',
  line: 'aiworkspace.blocks.shape.kind.line', text: 'aiworkspace.blocks.shape.kind.text', svg: 'aiworkspace.blocks.shape.kind.svg',
}
const FONT_FAMILY_LABEL: Record<FontFamily, string> = {
  sans: 'aiworkspace.blocks.shape.fontFamily.sans', serif: 'aiworkspace.blocks.shape.fontFamily.serif', mono: 'aiworkspace.blocks.shape.fontFamily.mono',
}
const GROUP_LABEL: Record<(typeof SHAPE_KINDS)[number]['group'], string> = {
  basic: 'aiworkspace.blocks.shape.group.basic', flow: 'aiworkspace.blocks.shape.group.flow', arrow: 'aiworkspace.blocks.shape.group.arrow', text: 'aiworkspace.blocks.shape.group.text',
}

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

/** CSS font stacks of the three families (Latin first, then the common CJK faces of each platform). */
const FONT_STACK: Record<FontFamily, string> = {
  sans: 'ui-sans-serif, system-ui, -apple-system, "Segoe UI", Roboto, Arial, "Noto Sans", "PingFang SC", "Microsoft YaHei", "Noto Sans CJK SC", sans-serif',
  serif: '"Times New Roman", Times, "Liberation Serif", "Noto Serif", "Songti SC", SimSun, "Noto Serif CJK SC", serif',
  mono: 'ui-monospace, SFMono-Regular, Menlo, Consolas, "Liberation Mono", "Noto Sans Mono", monospace',
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
    ...(look.lineHeight !== null ? { lineHeight: look.lineHeight } : {}),
    ...(look.fontFamily ? { fontFamily: FONT_STACK[look.fontFamily] } : {}),
  }
}

function ShapeFrame({ context, children }: { context: RenderContext; children?: ReactNode }) {
  const look = readLook(context.payload.config, context.payload.title)
  const { w, h } = context.size
  const pdfLayout = !children && look.kind === 'text' ? readPdfTextLayout(context.payload.config, look) : null
  return (
    <div className={`aiws-shape is-${look.kind}`} style={{ opacity: look.opacity < 1 ? look.opacity : undefined }} data-kind={look.kind} data-testid={`aiws-shape-${context.cell.entity_id}`}>
      <ShapeArt look={look} w={w} h={h} />
      {pdfLayout ? (
        <svg className="aiws-shape-text aiws-pdf-text" viewBox={`0 0 ${pdfLayout.width} ${pdfLayout.height}`} preserveAspectRatio="none" width="100%" height="100%"
          style={{ position: 'absolute', inset: 0, overflow: 'visible', whiteSpace: 'pre', pointerEvents: 'none', fill: inkFor(look), fontFamily: look.fontFamily ? FONT_STACK[look.fontFamily] : undefined, fontWeight: look.bold ? 700 : undefined, fontStyle: look.italic ? 'italic' : undefined, textDecoration: look.underline ? 'underline' : undefined }}>
          {pdfLayout.spans.map((span, i) => (
            <text key={i} x={span.x} y={span.y} fontSize={span.size} textLength={span.width} lengthAdjust="spacingAndGlyphs" xmlSpace="preserve">{span.text}</text>
          ))}
        </svg>
      ) : (
        <div className="aiws-shape-label" style={labelStyle(look, w, h)}>
          {children ?? (look.text && <span className="aiws-shape-text" style={look.labelFill ? { background: paintOf(look.labelFill, 'fill') } : undefined}>{look.text}</span>)}
        </div>
      )}
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
  const source = config.pdf_source
  if (source && typeof source === 'object' && !Array.isArray(source)) {
    const updated = { ...source }
    delete updated.text_layout
    config.pdf_source = updated
  }
  const id = context.cell.entity_id
  const keys: { key: string; value: Json; expect: { rev: number } }[] = [{ key: 'config', value: config, expect: { rev: context.keyRevs.config ?? 0 } }]
  const operations: Operation[] = []
  const title = context.payload.title
  const follows = !title || title === firstLine(before)
  if (follows && firstLine(next) && firstLine(next) !== title) keys.push({ key: 'title', value: firstLine(next), expect: { rev: context.keyRevs.title ?? 0 } })
  operations.push({ op: 'entity.set_keys', entity_id: id, keys })
  if (follows && title && !firstLine(next)) operations.push({ op: 'entity.unset_keys', entity_id: id, keys: [{ key: 'title', expect: { rev: context.keyRevs.title ?? 0 } }] })
  const label = aiwsT('aiworkspace.blocks.edit.change', { label: aiwsT('aiworkspace.blocks.shape.text'), value: firstLine(next) || aiwsT('aiworkspace.blocks.edit.cleared') })
  void store.submit({ editId: `key:${id}:text`, label, operations })
}

function ShapeEditor(context: RenderContext) {
  const { t } = useAiwsText()
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
      <div ref={ref} className="aiws-shape-text aiws-shape-edit" contentEditable="plaintext-only" suppressContentEditableWarning role="textbox" aria-multiline="true" aria-label={t('aiworkspace.blocks.shape.textAria')}
        data-role="editor" data-testid={`aiws-shape-edit-${context.cell.entity_id}`} data-placeholder={t('aiworkspace.blocks.shape.typeText')}
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
  const { t } = useAiwsText()
  return (
    <div className="aiws-shape-picker" data-testid="aiws-shape-picker">
      {KIND_GROUPS.map(({ group }) => (
        <div key={group} className="aiws-shape-picker-group">
          <div className="aiws-shape-picker-title">{t(GROUP_LABEL[group])}</div>
          <div className="aiws-shape-picker-grid">
            {SHAPE_KINDS.filter((k) => k.group === group).map((k) => {
              // parts measured in world units (a note's fold, a cylinder's rim) are scaled down to the icon
              const look = readLook({ shape: k.kind, ...(k.kind === 'text' ? { text: 'Aa', font_size: 13 } : {}), ...(k.size && !k.size.relative ? { shape_size: 6 } : {}), stroke_width: 1.5 })
              return (
                <button key={k.kind} type="button" className="aiws-shape-picker-item" title={t(KIND_LABEL[k.kind])} aria-label={t(KIND_LABEL[k.kind])} aria-pressed={value === k.kind} data-testid={`aiws-shape-kind-${k.kind}`}
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
const widths = (t: T) => [
  { value: '0', label: t('aiworkspace.common.none') }, { value: '1', label: t('aiworkspace.blocks.shape.width.thin') }, { value: '2', label: t('aiworkspace.blocks.shape.width.medium') },
  { value: '4', label: t('aiworkspace.blocks.shape.width.thick') }, { value: '8', label: t('aiworkspace.blocks.shape.width.extraThick') },
]
const dashes = (t: T) => [{ value: 'solid', label: t('aiworkspace.blocks.dash.solid') }, { value: 'dashed', label: t('aiworkspace.blocks.dash.dashed') }, { value: 'dotted', label: t('aiworkspace.blocks.dash.dotted') }]
const OPACITIES = [{ value: '1', label: '100%' }, { value: '0.75', label: '75%' }, { value: '0.5', label: '50%' }, { value: '0.25', label: '25%' }]
const radii = (t: T) => [
  { value: '0', label: t('aiworkspace.blocks.shape.radius.square') }, { value: '4', label: t('aiworkspace.blocks.shape.radius.s') }, { value: '8', label: t('aiworkspace.blocks.shape.radius.m') },
  { value: '16', label: t('aiworkspace.blocks.shape.radius.l') }, { value: '32', label: t('aiworkspace.blocks.shape.radius.xl') },
]
const directions = (t: T) => [
  { value: 'e', label: t('aiworkspace.blocks.shape.dir.e') }, { value: 's', label: t('aiworkspace.blocks.shape.dir.s') },
  { value: 'w', label: t('aiworkspace.blocks.shape.dir.w') }, { value: 'n', label: t('aiworkspace.blocks.shape.dir.n') },
]
const aligns = (t: T) => [
  { value: 'left', label: t('aiworkspace.blocks.shape.align.left'), icon: AlignLeft }, { value: 'center', label: t('aiworkspace.blocks.shape.align.center'), icon: AlignCenter },
  { value: 'right', label: t('aiworkspace.blocks.shape.align.right'), icon: AlignRight },
]
const valigns = (t: T) => [
  { value: 'top', label: t('aiworkspace.blocks.shape.valign.top'), icon: AlignVerticalJustifyStart }, { value: 'middle', label: t('aiworkspace.blocks.shape.valign.middle'), icon: AlignVerticalJustifyCenter },
  { value: 'bottom', label: t('aiworkspace.blocks.shape.valign.bottom'), icon: AlignVerticalJustifyEnd },
]
const labelPositions = (t: T) => [
  { value: 'inside', label: t('aiworkspace.blocks.shape.labelPos.inside') }, { value: 'top', label: t('aiworkspace.blocks.shape.labelPos.top') }, { value: 'bottom', label: t('aiworkspace.blocks.shape.labelPos.bottom') },
  { value: 'left', label: t('aiworkspace.blocks.shape.labelPos.left') }, { value: 'right', label: t('aiworkspace.blocks.shape.labelPos.right') },
]
const fillExtras = (t: T) => [{ value: 'none', label: t('aiworkspace.blocks.shape.noFill') }, { value: 'auto', label: t('aiworkspace.blocks.followTheme') }]
const strokeExtras = (t: T) => [{ value: 'none', label: t('aiworkspace.blocks.shape.noStroke') }, { value: 'auto', label: t('aiworkspace.blocks.followTheme') }]
const inkExtras = (t: T) => [{ value: 'auto', label: t('aiworkspace.blocks.shape.auto') }]
/** "<what> → <value>": the label of a look change in the edit history. */
const changed = (t: T, label: string, value: string) => t('aiworkspace.blocks.edit.change', { label, value })

const canChange = (context: RenderContext): string | false => (context.capabilities.includes('update') && context.mode === 'edit' && context.readOnlyReason === null ? false : aiwsT('aiworkspace.blocks.noPermission'))
const roundCorners = (kind: ShapeKind) => !['ellipse', 'cloud', 'cylinder', 'line', 'svg', 'document'].includes(kind)
const turnable = (kind: ShapeKind) => !['rect', 'ellipse', 'cloud', 'cross', 'text', 'svg'].includes(kind)

function LinePanel({ context, store, look, close }: { context: RenderContext; store: WorkspaceStore; look: ShapeLook; close: () => void }) {
  const { t } = useAiwsText()
  const set = (patch: Record<string, Json | null>, label: string) => setCellConfig(context, store, patch, label)
  const dashList = dashes(t)
  const directionList = directions(t)
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
      {row(t('aiworkspace.blocks.shape.lineWidth'), widths(t), String(look.stroke === 'none' ? 0 : look.strokeWidth), (v) => set(v === '0' ? { stroke: 'none' } : { stroke_width: Number(v), ...(look.stroke === 'none' ? { stroke: null } : {}) }, changed(t, t('aiworkspace.blocks.shape.lineWidth'), v)), 'width')}
      {row(t('aiworkspace.blocks.lineStyle'), dashList, look.dash, (v) => set({ dash: v === 'solid' ? null : v }, changed(t, t('aiworkspace.blocks.lineStyle'), dashList.find((d) => d.value === v)?.label ?? v)), 'dash')}
      {row(t('aiworkspace.blocks.shape.opacity'), OPACITIES, String(look.opacity), (v) => set({ opacity: v === '1' ? null : Number(v) }, changed(t, t('aiworkspace.blocks.shape.opacity'), `${Math.round(Number(v) * 100)}%`)), 'opacity')}
      {roundCorners(look.kind) && row(t('aiworkspace.blocks.shape.cornerRadius'), radii(t), String(look.radius), (v) => set({ radius: v === '0' ? null : Number(v) }, changed(t, t('aiworkspace.blocks.shape.cornerRadius'), v)), 'radius')}
      {turnable(look.kind) && row(t('aiworkspace.blocks.direction'), directionList, look.direction, (v) => set({ direction: v === 'e' ? null : v }, changed(t, t('aiworkspace.blocks.direction'), directionList.find((d) => d.value === v)?.label ?? v)), 'direction')}
      <div className="aiws-shape-panel-row">
        <span>{t('aiworkspace.blocks.shape.flip')}</span>
        <div className="aiws-segmented" role="group" aria-label={t('aiworkspace.blocks.shape.flip')}>
          <button type="button" aria-pressed={look.flipH} data-testid="aiws-shape-flip-h" onMouseDown={(e) => e.preventDefault()} onClick={() => set({ flip_h: look.flipH ? null : true }, t('aiworkspace.blocks.shape.flipH'))}>{t('aiworkspace.blocks.shape.horizontal')}</button>
          <button type="button" aria-pressed={look.flipV} data-testid="aiws-shape-flip-v" onMouseDown={(e) => e.preventDefault()} onClick={() => set({ flip_v: look.flipV ? null : true }, t('aiworkspace.blocks.shape.flipV'))}>{t('aiworkspace.blocks.shape.vertical')}</button>
        </div>
      </div>
      <div className="aiws-dialog-actions"><button type="button" onClick={close}>{t('aiworkspace.blocks.shape.done')}</button></div>
    </div>
  )
}

function shapeToolbar(context: RenderContext, store: WorkspaceStore): ToolbarItem[] {
  const look = readLook(context.payload.config, context.payload.title)
  const reason = canChange(context)
  const set = (patch: Record<string, Json | null>, label: string) => setCellConfig(context, store, patch, label)
  const t = aiwsT
  const alignItems = aligns(t)
  const valignItems = valigns(t)
  return [
    { kind: 'panel', id: 'shape-kind', icon: Shapes, label: t('aiworkspace.blocks.shape.kindLabel', { kind: t(KIND_LABEL[look.kind]) }), disabled: reason,
      render: (close) => <ShapePicker value={look.kind} onPick={(kind) => { close(); if (kind !== look.kind) set({ shape: kind, ...(kind === 'svg' ? {} : { svg: null }) }, changed(t, t('aiworkspace.blocks.shape.shape'), SHAPE_KINDS.some((k) => k.kind === kind) ? t(KIND_LABEL[kind]) : kind)) }} /> },
    ...(look.kind === 'line' ? [] : [{ kind: 'color' as const, id: 'shape-fill', label: t('aiworkspace.blocks.shape.fill'), palette: 'fill' as const, extras: fillExtras(t), value: look.fill, disabled: reason, onPick: (value: string) => set({ fill: value || null }, t('aiworkspace.blocks.shape.fillEdit')) }]),
    { kind: 'color', id: 'shape-stroke', label: t(look.kind === 'line' ? 'aiworkspace.blocks.shape.lineColor' : 'aiworkspace.blocks.shape.stroke'), palette: 'ink', extras: strokeExtras(t), value: look.stroke, disabled: reason, onPick: (value) => set({ stroke: value || null }, t('aiworkspace.blocks.shape.strokeEdit')) },
    { kind: 'panel', id: 'shape-line', icon: PenLine, label: t('aiworkspace.blocks.shape.lineTools'), disabled: reason, render: (close) => <LinePanel context={context} store={store} look={look} close={close} /> },
    { kind: 'separator', id: 'shape-sep-text' },
    { kind: 'menu', id: 'shape-font-size', icon: ALargeSmall, label: t('aiworkspace.blocks.fontSize'), value: String(look.fontSize), disabled: reason, items: FONT_SIZES.map((s) => ({ value: String(s), label: `${s}` })), onPick: (value) => set({ font_size: Number(value) }, changed(t, t('aiworkspace.blocks.fontSize'), value)) },
    { kind: 'color', id: 'shape-font-color', label: t('aiworkspace.blocks.textColor'), palette: 'ink', extras: inkExtras(t), value: look.fontColor || 'auto', disabled: reason, onPick: (value) => set({ font_color: value && value !== 'auto' ? value : null }, t('aiworkspace.blocks.textColor')) },
    { kind: 'button', id: 'shape-bold', icon: Bold, label: t('aiworkspace.blocks.shape.bold'), active: look.bold, disabled: reason, run: () => set({ bold: look.bold ? null : true }, t(look.bold ? 'aiworkspace.blocks.shape.unbold' : 'aiworkspace.blocks.shape.bold')) },
    { kind: 'button', id: 'shape-italic', icon: Italic, label: t('aiworkspace.blocks.shape.italic'), active: look.italic, disabled: reason, run: () => set({ italic: look.italic ? null : true }, t(look.italic ? 'aiworkspace.blocks.shape.unitalic' : 'aiworkspace.blocks.shape.italic')) },
    { kind: 'button', id: 'shape-underline', icon: Underline, label: t('aiworkspace.blocks.shape.underline'), active: look.underline, disabled: reason, run: () => set({ underline: look.underline ? null : true }, t(look.underline ? 'aiworkspace.blocks.shape.ununderline' : 'aiworkspace.blocks.shape.underline')) },
    { kind: 'menu', id: 'shape-align', icon: alignItems.find((a) => a.value === look.align)?.icon ?? AlignCenter, label: t('aiworkspace.blocks.shape.hAlign'), value: look.align, disabled: reason, items: alignItems,
      onPick: (value) => set({ align: value }, changed(t, t('aiworkspace.blocks.shape.alignEdit'), alignItems.find((a) => a.value === value)?.label ?? value)) },
    { kind: 'menu', id: 'shape-valign', icon: valignItems.find((a) => a.value === look.valign)?.icon ?? AlignVerticalJustifyCenter, label: t('aiworkspace.blocks.shape.vAlign'), value: look.valign, disabled: reason, items: valignItems,
      onPick: (value) => set({ valign: value === 'middle' ? null : value }, changed(t, t('aiworkspace.blocks.shape.vAlign'), valignItems.find((a) => a.value === value)?.label ?? value)) },
    ...(context.editorActive ? [] : [{ kind: 'button' as const, id: 'shape-text', icon: Baseline, label: t('aiworkspace.blocks.shape.editText'), key: 'Enter', disabled: reason, run: () => context.activateEditor() }]),
  ]
}

// ---- inspector (属性面板): every key

function ShapeInspector(context: RenderContext) {
  const { t } = useAiwsText()
  const store = useStore()
  const look = readLook(context.payload.config, context.payload.title)
  const config = context.payload.config ?? {}
  const readOnly = canChange(context) !== false
  const set = (patch: Record<string, Json | null>, label: string) => { if (!readOnly) setCellConfig(context, store, patch, label) }
  const select = (key: string, label: string, options: { value: string; label: string }[], value: string, fallback: string) => (
    <label className="aiws-inline-form">{label}
      <select value={value} disabled={readOnly} data-testid={`aiws-shape-field-${key}`} onChange={(e) => set({ [key]: e.target.value === fallback ? null : e.target.value }, changed(t, label, e.target.value))}>
        {options.map((o) => <option key={o.value} value={o.value}>{o.label}</option>)}
      </select>
    </label>
  )
  const number = (key: string, label: string, value: number | null, fallback?: number) => (
    <label className="aiws-inline-form">{label}
      <input type="number" step="any" defaultValue={value ?? ''} key={`${key}:${value}`} disabled={readOnly} data-testid={`aiws-shape-field-${key}`}
        onBlur={(e) => { const v = e.target.value.trim(); const n = Number(v); if (v === '' || n === fallback) set({ [key]: null }, t('aiworkspace.blocks.edit.default', { label })); else if (Number.isFinite(n) && n !== value) set({ [key]: n }, changed(t, label, String(n))) }} />
    </label>
  )
  const color = (key: string, label: string, value: string, extras: string[]) => (
    <label className="aiws-inline-form">{label}
      <span className="aiws-color-field">
        <input type="color" value={value.startsWith('#') ? value : '#ffffff'} disabled={readOnly} data-testid={`aiws-shape-field-${key}`} onChange={(e) => set({ [key]: e.target.value }, label)} />
        {extras.map((x) => (
          <button key={x} type="button" aria-pressed={value === x} disabled={readOnly} onClick={() => set({ [key]: x }, changed(t, label, t(x === 'none' ? 'aiworkspace.common.none' : 'aiworkspace.blocks.followTheme')))}>
            {t(x === 'none' ? 'aiworkspace.common.none' : 'aiworkspace.blocks.shape.theme')}
          </button>
        ))}
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
      {select('shape', t('aiworkspace.blocks.shape.shape'), [...SHAPE_KINDS.map((k) => ({ value: k.kind, label: t(KIND_LABEL[k.kind]) })), ...(look.kind === 'svg' ? [{ value: 'svg', label: t(KIND_LABEL.svg) }] : [])], look.kind, '')}
      {color('fill', t('aiworkspace.blocks.shape.fill'), look.fill, ['none', 'auto'])}
      {color('stroke', t('aiworkspace.blocks.shape.stroke'), look.stroke, ['none', 'auto'])}
      {number('stroke_width', t('aiworkspace.blocks.shape.lineWidth'), look.strokeWidth, 2)}
      {select('dash', t('aiworkspace.blocks.lineStyle'), dashes(t), look.dash, 'solid')}
      {number('opacity', t('aiworkspace.blocks.shape.opacityField'), look.opacity, 1)}
      {roundCorners(look.kind) && number('radius', t('aiworkspace.blocks.shape.cornerRadius'), look.radius, 0)}
      {turnable(look.kind) && select('direction', t('aiworkspace.blocks.direction'), directions(t), look.direction, 'e')}
      {flag('flip_h', t('aiworkspace.blocks.shape.flipH'), look.flipH)}
      {flag('flip_v', t('aiworkspace.blocks.shape.flipV'), look.flipV)}
      {number('shape_size', t('aiworkspace.blocks.shape.sizeParam'), look.size)}
      <label className="aiws-field">{t('aiworkspace.blocks.shape.text')}
        <textarea rows={3} defaultValue={look.text} key={`text:${look.text}`} disabled={readOnly} data-testid="aiws-shape-field-text" onBlur={(e) => { if (!readOnly) saveText(context, store, e.target.value) }} />
      </label>
      {number('font_size', t('aiworkspace.blocks.fontSize'), look.fontSize, 14)}
      {select('font_family', t('aiworkspace.blocks.shape.fontFamily'), [{ value: '', label: t('aiworkspace.blocks.shape.fontFamilyTheme') }, ...FONT_FAMILIES.map((f) => ({ value: f, label: t(FONT_FAMILY_LABEL[f]) }))], look.fontFamily ?? '', '')}
      {number('line_height', t('aiworkspace.blocks.shape.lineHeight'), look.lineHeight)}
      {color('font_color', t('aiworkspace.blocks.textColor'), look.fontColor || 'auto', ['auto'])}
      {flag('bold', t('aiworkspace.blocks.shape.bold'), look.bold)}
      {flag('italic', t('aiworkspace.blocks.shape.italic'), look.italic)}
      {flag('underline', t('aiworkspace.blocks.shape.underline'), look.underline)}
      {select('align', t('aiworkspace.blocks.shape.hAlign'), aligns(t), look.align, '')}
      {select('valign', t('aiworkspace.blocks.shape.vAlign'), valigns(t), look.valign, 'middle')}
      {select('label_position', t('aiworkspace.blocks.shape.labelPosition'), labelPositions(t), look.labelPosition, 'inside')}
      {flag('wrap', t('aiworkspace.blocks.shape.wrap'), look.wrap, true)}
      {color('label_fill', t('aiworkspace.blocks.shape.labelFill'), look.labelFill ?? 'none', ['none', 'auto'])}
      <AnchorsForm anchors={anchors} readOnly={readOnly} onChange={(next) => set({ anchors: next.length ? (next as unknown as Json) : null }, t('aiworkspace.blocks.shape.anchors'))} />
    </div>
  )
}

/** The Block's own connection points (`config.anchors`): id and position in the box (0–1). */
function AnchorsForm({ anchors, readOnly, onChange }: { anchors: AnchorDef[]; readOnly: boolean; onChange: (next: AnchorDef[]) => void }) {
  const { t } = useAiwsText()
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
      <summary>{t('aiworkspace.blocks.shape.anchorsCount', { count: anchors.length })}</summary>
      <div className="aiws-muted">{t('aiworkspace.blocks.shape.anchorsHelp')}</div>
      {anchors.map((a) => (
        <div key={a.id} className="aiws-inline-form"><code>{a.id}</code> ({a.x}, {a.y}){a.dx || a.dy ? ` + (${a.dx ?? 0}, ${a.dy ?? 0})` : ''}
          <button type="button" className="aiws-link" disabled={readOnly} onClick={() => onChange(anchors.filter((b) => b.id !== a.id))}>{t('aiworkspace.common.delete')}</button>
        </div>
      ))}
      {!readOnly && (
        <div className="aiws-inline-form">
          <input aria-label={t('aiworkspace.blocks.shape.anchorId')} placeholder="id" value={draft.id} onChange={(e) => setDraft({ ...draft, id: e.target.value })} data-testid="aiws-anchor-id" />
          <input aria-label="x" type="number" step="0.05" value={draft.x} onChange={(e) => setDraft({ ...draft, x: e.target.value })} />
          <input aria-label="y" type="number" step="0.05" value={draft.y} onChange={(e) => setDraft({ ...draft, y: e.target.value })} />
          <button type="button" onClick={add} data-testid="aiws-anchor-add">{t('aiworkspace.blocks.shape.add')}</button>
        </div>
      )}
    </details>
  )
}

// ---- the definition

const outlineOf = (payload: CellPayload) => shapeOutline(readLook(payload.config))

export const shapeDefinition: BlockDefinition = {
  type: 'shape', version: 1, get title() { return aiwsT('aiworkspace.blocks.shape.shape') }, accepts: [], allowNoSource: true, pureUi: true,
  defaultSize: { w: 160, h: 120 }, cost: { editor: false, html: false },
  Static: ShapeStatic, Editor: ShapeEditor, Inspector: ShapeInspector, chrome: 'none',
  shape: outlineOf,
  anchors: (payload) => shapeAnchors(payload.config, outlineOf(payload)),
  catalog: {
    group: 'layout', get description() { return aiwsT('aiworkspace.blocks.shape.description') }, needs: 'none', standard: true,
    variants: [
      { id: 'text', get title() { return aiwsT('aiworkspace.blocks.shape.kind.text') }, group: 'text', get description() { return aiwsT('aiworkspace.blocks.shape.textDescription') },
        config: { shape: 'text', text: '', font_size: 16 }, size: { w: 240, h: 48 }, editAfterInsert: true, standard: true },
      { id: 'svg', get title() { return aiwsT('aiworkspace.blocks.shape.kind.svg') }, group: 'layout', get description() { return aiwsT('aiworkspace.blocks.shape.svgDescription') }, config: { shape: 'svg' } },
    ],
  },
  actions: [{ id: 'label', get label() { return aiwsT('aiworkspace.blocks.shape.editText') }, modes: ['edit'], needs: ['update'], run: (context) => context.activateEditor(), key: 'Enter' }],
  toolbar: shapeToolbar,
  create: (args) => {
    const preset = args.config ?? {}
    const kind = typeof preset.shape === 'string' ? preset.shape : 'rect'
    const base: Record<string, Json> = kind === 'text' || kind === 'line' ? { shape: kind } : { shape: kind, fill: DEFAULT_FILL, stroke: DEFAULT_STROKE }
    return [cellOp({ ...args, config: undefined }, 'shape', undefined, { config: { ...base, ...preset } })]
  },
}
