/* eslint-disable react-refresh/only-export-components -- a Block definition with its renderers and formatter */
/* The `label` Block (05 标准库 “基础呈现”, 控件机制 TODO G1): one line (or a few) of text showing a constant — its title —
 * or a bound value, formatted for the reader (date, time, relative time, number …). It owns no second copy of the
 * value: bound to a Record (`config.prop`) or to a table cell (`config.record_id` + `config.field_id`), it reads the
 * data and shows what is there now. Its size is its text's within a width (a text view: `"auto"` inside a control).
 *
 * `LabelArt` is the same drawing without a Block around it: what data controls use for the labels they project from
 * records (G2, G7), so hundreds of them cost a span each. */

import { useCallback, useLayoutEffect, useRef, useState, type CSSProperties } from 'react'
import { AlignCenter, AlignLeft, AlignRight, Baseline, Bold, Tag, Type, WrapText } from 'lucide-react'
import type { Json, RecordContent } from '../../api/types'
import { aiwsT, aiwsText } from '../../i18n'
import { useLoad, useStore, useVersion } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { consumeIntent, useIntent } from './editorToolbar'
import { setCellConfig } from './ops'
import type { BlockDefinition, ConfigField, RenderContext } from './registry'

export const LABEL_VIEW = 'label'

export type LabelFormat = 'text' | 'date' | 'date_short' | 'weekday' | 'weekday_short' | 'day' | 'month' | 'time' | 'datetime' | 'relative' | 'number' | 'percent' | 'duration' | 'countdown'

/** How a label looks: the keys of its `config` (and of a projected label's template config). */
export interface LabelLook {
  font_size?: number
  font_color?: string
  weight?: 'normal' | 'medium' | 'bold'
  italic?: boolean
  strike?: boolean
  align?: 'left' | 'center' | 'right'
  valign?: 'top' | 'middle' | 'bottom'
  /** Several lines (default: one line, cut with an ellipsis). */
  wrap?: boolean
  max_lines?: number
  mono?: boolean
  muted?: boolean
}

const FORMAT_LABEL: Partial<Record<LabelFormat, string>> = {
  text: 'aiworkspace.label.fmt.text', date: 'aiworkspace.label.fmt.date', date_short: 'aiworkspace.label.fmt.date_short', weekday: 'aiworkspace.label.fmt.weekday',
  day: 'aiworkspace.label.fmt.day', month: 'aiworkspace.label.fmt.month', time: 'aiworkspace.label.fmt.time', datetime: 'aiworkspace.label.fmt.datetime',
  relative: 'aiworkspace.label.fmt.relative', number: 'aiworkspace.label.fmt.number', percent: 'aiworkspace.label.fmt.percent', duration: 'aiworkspace.label.fmt.duration',
}
const FORMATS = Object.keys(FORMAT_LABEL) as LabelFormat[]
const SIZES = [11, 12, 13, 14, 16, 18, 20, 24, 28, 32, 40, 48]

const dateOnly = (s: string) => /^\d{4}-\d{2}-\d{2}$/.test(s)
/** A date (`YYYY-MM-DD`) is a day on the reader's calendar, not an instant: shown without shifting it. */
function toDate(value: Json | undefined): { date: Date; dayOnly: boolean } | null {
  if (typeof value === 'number') return { date: new Date(value), dayOnly: false }
  if (typeof value !== 'string' || !value) return null
  if (dateOnly(value)) {
    const [y, m, d] = value.split('-').map(Number)
    return { date: new Date(y, m - 1, d), dayOnly: true }
  }
  const t = Date.parse(value)
  return Number.isFinite(t) ? { date: new Date(t), dayOnly: false } : null
}

const startOfDay = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime()

/** A value as the reader reads it, in the app's language and the device's time zone. */
export function formatLabelValue(value: Json | undefined, format: LabelFormat | string | undefined, now: number = Date.now()): string {
  if (value === undefined || value === null) return ''
  const text = aiwsText()
  const locale = text.locale === 'zh-CN' ? 'zh-CN' : 'en'
  const f = (format ?? 'text') as LabelFormat
  if (f === 'number' || f === 'percent') {
    const n = typeof value === 'number' ? value : Number(value)
    if (!Number.isFinite(n)) return String(value)
    return f === 'percent' ? new Intl.NumberFormat(locale, { style: 'percent', maximumFractionDigits: 1 }).format(n) : text.num(n)
  }
  if (f === 'duration') {
    const ms = typeof value === 'number' ? value : Number(value)
    if (!Number.isFinite(ms)) return String(value)
    const m = Math.round(Math.abs(ms) / 60_000)
    const h = Math.floor(m / 60)
    return h ? aiwsT('aiworkspace.label.durationHM', { h, m: m % 60 }) : aiwsT('aiworkspace.label.durationM', { m })
  }
  if (f === 'countdown') {
    // a remaining time to the second: m:ss, h:mm:ss
    const ms = typeof value === 'number' ? value : Number(value)
    if (!Number.isFinite(ms)) return String(value)
    const total = Math.max(0, Math.ceil(ms / 1000))
    const two = (n: number) => String(n).padStart(2, '0')
    const h = Math.floor(total / 3600)
    return h ? `${h}:${two(Math.floor(total / 60) % 60)}:${two(total % 60)}` : `${Math.floor(total / 60)}:${two(total % 60)}`
  }
  if (f === 'text') return typeof value === 'object' ? JSON.stringify(value) : String(value)
  const at = toDate(value)
  if (!at) return String(value)
  const d = at.date
  switch (f) {
    case 'date': return new Intl.DateTimeFormat(locale, { year: 'numeric', month: 'short', day: 'numeric' }).format(d)
    case 'date_short': return new Intl.DateTimeFormat(locale, { month: 'numeric', day: 'numeric' }).format(d)
    case 'weekday': return new Intl.DateTimeFormat(locale, { weekday: 'long' }).format(d)
    case 'weekday_short': return new Intl.DateTimeFormat(locale, { weekday: 'short' }).format(d)
    case 'day': return String(d.getDate())
    case 'month': return new Intl.DateTimeFormat(locale, { year: 'numeric', month: 'long' }).format(d)
    case 'time': return at.dayOnly ? '' : new Intl.DateTimeFormat(locale, { hour: '2-digit', minute: '2-digit' }).format(d)
    case 'datetime': return at.dayOnly ? new Intl.DateTimeFormat(locale, { year: 'numeric', month: 'short', day: 'numeric' }).format(d) : new Intl.DateTimeFormat(locale, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }).format(d)
    case 'relative': {
      const days = Math.round((startOfDay(d) - startOfDay(new Date(now))) / 86_400_000)
      const rtf = new Intl.RelativeTimeFormat(locale, { numeric: 'auto' })
      if (at.dayOnly || Math.abs(d.getTime() - now) >= 86_400_000) {
        if (Math.abs(days) < 7) return rtf.format(days, 'day')
        if (Math.abs(days) < 31) return rtf.format(Math.round(days / 7), 'week')
        return new Intl.DateTimeFormat(locale, { month: 'short', day: 'numeric' }).format(d)
      }
      const minutes = Math.round((d.getTime() - now) / 60_000)
      return Math.abs(minutes) < 60 ? rtf.format(minutes, 'minute') : rtf.format(Math.round(minutes / 60), 'hour')
    }
    default: return String(value)
  }
}

/** The CSS of a label's look. */
export function labelStyle(look: LabelLook): CSSProperties {
  const lines = look.wrap ? look.max_lines : 1
  return {
    fontSize: look.font_size ?? 14,
    color: look.font_color && look.font_color !== 'auto' ? look.font_color : look.muted ? 'var(--cp-muted)' : undefined,
    fontWeight: look.weight === 'bold' ? 700 : look.weight === 'medium' ? 500 : undefined,
    fontStyle: look.italic ? 'italic' : undefined,
    textDecoration: look.strike ? 'line-through' : undefined,
    textAlign: look.align ?? 'left',
    justifyContent: look.valign === 'top' ? 'flex-start' : look.valign === 'bottom' ? 'flex-end' : 'center',
    fontFamily: look.mono ? 'var(--aiws-mono, ui-monospace, monospace)' : undefined,
    whiteSpace: look.wrap ? 'pre-wrap' : 'pre',
    ...(lines ? { WebkitLineClamp: lines } : {}),
  }
}

/** A label without a Block around it: one span. */
export function LabelArt({ text, look, className, testId, title }: { text: string; look: LabelLook; className?: string; testId?: string; title?: string }) {
  return (
    <span className={`aiws-label${look.wrap ? ' is-wrap' : ''}${className ? ` ${className}` : ''}`} style={labelStyle(look)} data-testid={testId} title={title}>
      <span className="aiws-label-text">{text}</span>
    </span>
  )
}

/** The value a bound label shows: a Record property, or a cell of a table record. */
function useBoundValue(context: RenderContext): { value: Json | undefined; bound: boolean } {
  const store = useStore()
  const source = context.source
  const config = context.payload.config ?? {}
  const version = useVersion(source ? `e:${source.entity_id}` : '')
  const tableCell = source?.type_id === 'buckyos.table-source' && typeof config.record_id === 'string' && typeof config.field_id === 'string'
  const load = useCallback(async (): Promise<Json | undefined> => {
    if (!source) return undefined
    if (source.type_id === 'buckyos.record' && typeof config.prop === 'string') {
      const read = await store.session.read<RecordContent>(source.entity_id)
      return read.content.props[config.prop as string]
    }
    if (tableCell) {
      const read = await store.session.read<{ values?: Record<string, Json> }>(source.entity_id, { kind: 'table_record', record_id: config.record_id as string })
      return read.content.values?.[config.field_id as string]
    }
    return undefined
  }, [store, source, config.prop, config.record_id, config.field_id, tableCell])
  const loaded = useLoad(load, `${version}:${source?.content_rev ?? 0}`)
  return { value: loaded.data, bound: Boolean(source) }
}

function lookOf(config: Record<string, Json> | undefined): LabelLook {
  return (config ?? {}) as LabelLook
}

function LabelStatic(context: RenderContext) {
  const { t } = useAiwsText()
  const config = context.payload.config ?? {}
  const { value, bound } = useBoundValue(context)
  const text = bound ? formatLabelValue(value, config.format as string | undefined) : context.payload.title ?? ''
  const empty = !text && context.mode === 'edit'
  return (
    <div className="aiws-label-block" data-testid={`aiws-label-${context.cell.entity_id}`}>
      <LabelArt text={empty ? t('aiworkspace.label.empty') : text} look={lookOf(config)} className={empty ? 'is-empty' : undefined} />
    </div>
  )
}

function saveTitle(context: RenderContext, store: ReturnType<typeof useStore>, next: string) {
  const before = context.payload.title ?? ''
  if (next === before) return
  const id = context.cell.entity_id
  const expect = { rev: context.keyRevs.title ?? 0 }
  const label = aiwsT('aiworkspace.blocks.edit.change', { label: aiwsT('aiworkspace.label.title'), value: next || aiwsT('aiworkspace.blocks.edit.cleared') })
  void store.submit({ editId: `key:${id}:title`, label, operations: [next
    ? { op: 'entity.set_keys', entity_id: id, keys: [{ key: 'title', value: [...next].slice(0, 256).join(''), expect }] }
    : { op: 'entity.unset_keys', entity_id: id, keys: [{ key: 'title', expect }] }] })
}

/** Typing replaces the constant text (a bound label shows its data: edit the data instead). */
function LabelEditor(context: RenderContext) {
  const { t } = useAiwsText()
  const store = useStore()
  const ref = useRef<HTMLDivElement>(null)
  const closed = useRef(false)
  const intent = useIntent(context.cell.entity_id)
  const [started] = useState(() => (intent?.value.startsWith('type:') ? intent.value.slice(5) : null))
  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    el.textContent = started ?? context.payload.title ?? ''
    el.focus()
    const range = document.createRange()
    range.selectNodeContents(el)
    range.collapse(false)
    window.getSelection()?.removeAllRanges()
    window.getSelection()?.addRange(range)
    if (intent) consumeIntent(context.cell.entity_id, intent)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])
  if (context.source) return <LabelStatic {...context} />
  const done = (save: boolean) => {
    if (closed.current) return
    closed.current = true
    if (save && ref.current) saveTitle(context, store, (ref.current.innerText ?? '').replace(/\n$/, ''))
    context.deactivateEditor()
  }
  const look = lookOf(context.payload.config)
  return (
    <div className="aiws-label-block">
      <div ref={ref} className="aiws-label is-editing" style={{ ...labelStyle(look), whiteSpace: 'pre-wrap' }} contentEditable="plaintext-only" suppressContentEditableWarning role="textbox"
        aria-label={t('aiworkspace.label.title')} data-role="editor" data-testid={`aiws-label-edit-${context.cell.entity_id}`} data-placeholder={t('aiworkspace.label.empty')}
        onBlur={() => done(true)}
        onKeyDown={(event) => {
          if (event.nativeEvent.isComposing) return
          if (event.key === 'Enter' && !event.shiftKey) { event.preventDefault(); done(true) }
          else if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); done(false) }
        }} />
    </div>
  )
}

/** The label's look (its Cell configuration): the properties panel shows them all, the Block toolbar every one of them. */
const labelFields = (): ConfigField[] => [
  { key: 'font_size', id: 'label-size', label: aiwsT('aiworkspace.label.fontSize'), kind: 'number', quick: true, icon: Type, default: 14, presets: SIZES },
  { key: 'font_color', id: 'label-color', label: aiwsT('aiworkspace.label.color'), kind: 'color', quick: true, palette: 'ink', default: 'auto', extras: [{ value: 'auto', label: aiwsT('aiworkspace.label.colorAuto') }] },
  { key: 'weight', id: 'label-bold', label: aiwsT('aiworkspace.label.bold'), kind: 'boolean', quick: true, icon: Bold,
    get: (context) => lookOf(context.payload.config).weight === 'bold', set: (context, store, value) => setCellConfig(context, store, { weight: value === true ? 'bold' : null }, aiwsT('aiworkspace.label.bold')) },
  { key: 'align', id: 'label-align', label: aiwsT('aiworkspace.label.align'), kind: 'select', quick: true, default: 'left',
    options: [{ value: 'left', label: aiwsT('aiworkspace.label.alignLeft'), icon: AlignLeft }, { value: 'center', label: aiwsT('aiworkspace.label.alignCenter'), icon: AlignCenter }, { value: 'right', label: aiwsT('aiworkspace.label.alignRight'), icon: AlignRight }] },
  { key: 'wrap', id: 'label-wrap', label: aiwsT('aiworkspace.label.wrap'), kind: 'boolean', quick: true, icon: WrapText },
  { key: 'format', id: 'label-format', label: aiwsT('aiworkspace.label.format'), kind: 'select', quick: true, icon: Baseline, default: 'text', when: (context) => Boolean(context.source),
    options: FORMATS.map((f) => ({ value: f, label: aiwsT(FORMAT_LABEL[f]!) })) },
]

export const labelDefinition: BlockDefinition = {
  type: LABEL_VIEW, version: 1, get title() { return aiwsT('aiworkspace.label.blockTitle') }, accepts: ['buckyos.record', 'buckyos.table-source'], allowNoSource: true, pureUi: true,
  defaultSize: { w: 160, h: 32 }, cost: { editor: false, html: false }, chrome: 'none', icon: Tag,
  Static: LabelStatic, Editor: LabelEditor,
  shape: 'rect',
  catalog: { group: 'text', get description() { return aiwsT('aiworkspace.label.description') }, needs: 'none', editAfterInsert: true },
  hover: () => [],
  get configFields() { return labelFields() },
  create: (args) => [{
    op: 'entity.create', entity_id: args.cellId, type_id: 'buckyos.cell', parent_id: args.parentId, order_key: args.orderKey, placement: args.placement,
    payload: { view: { type: LABEL_VIEW }, ...(args.title ? { title: args.title } : {}), ...(args.config ? { config: args.config } : {}) },
  }],
}
