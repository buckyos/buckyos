/* Field management of a table (UI 框架 PRD §7.3; design §3.4.4, §3.4.6): a list of the fields — name, type, shown in
 * this view, a handle to drag the order — and a field's details when its row is tapped: name, type (changed through the
 * pre-checked migration), options, required, default currency, the column's width and place in the view, delete. What
 * the view shows (columns, order, width) belongs to the Block's view, or to this person when there is no Block; the
 * rest is the table's schema. */

import { useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import { ChevronLeft, ChevronRight, GripVertical, Plus, Trash2, Type } from 'lucide-react'
import { randomId } from '../api/ids'
import type { ReadOk } from '../api/session'
import type { CellPayload, FieldDef, FieldType, Json, KeyedContent, Operation, PrepareResult, TableSourceContent } from '../api/types'
import { useStore } from '../state/hooks'
import { useAiwsText } from '../useAiwsText'
import { addFieldOps, CREATABLE_FIELD_TYPES } from './fieldOps'
import { DEFAULT_WIDTH, WIDTHS, moveEntry, widthPreset, type ColumnEntry, type WidthPreset } from './table/model'
import { FIELD_ICONS } from './table/fieldIcons'
import { COMMON_CURRENCIES, FIELD_TYPE_LABEL, moneyParts } from './values'

/** Conversions the backend implements (anything else answers MIGRATION_UNSUPPORTED). */
const MIGRATIONS: Partial<Record<FieldType, FieldType[]>> = { text: ['date', 'number', 'decimal'], number: ['decimal', 'currency'], decimal: ['decimal', 'currency'], select: ['text'], currency: ['decimal', 'text', 'currency'] }

const WIDTH_LABEL: Record<WidthPreset, string> = { narrow: 'aiworkspace.table.fields.width.narrow', medium: 'aiworkspace.table.fields.width.medium', wide: 'aiworkspace.table.fields.width.wide' }

const CURRENCY_CODE = /^[A-Z]{3}$/
/** A currency code input: the common currencies offered, any ISO 4217 code accepted. */
function CurrencyInput({ value, onChange, label }: { value: string; onChange: (code: string) => void; label: string }) {
  return (
    <>
      <input aria-label={label} title={label} list="aiws-currency-codes" style={{ width: 72 }} maxLength={3} value={value} data-testid="aiws-currency-input" onChange={(event) => onChange(event.target.value.toUpperCase())} />
      <datalist id="aiws-currency-codes">{COMMON_CURRENCIES.map((code) => <option key={code} value={code} />)}</datalist>
    </>
  )
}

interface MigrationReport { total: number; convertible: number; unset: number; failing: { count: number; sample: { record_id: string; value: Json }[] } }

function readReport(result: PrepareResult): MigrationReport | null {
  if (result.status === 'ok') {
    const reports = (result.reports ?? []) as { kind?: string; report?: MigrationReport }[]
    return reports.find((item) => item.kind === 'migration')?.report ?? null
  }
  if (result.status === 'rejected') {
    for (const error of result.errors ?? []) {
      const report = error.data?.report as MigrationReport | undefined
      if (report) return report
    }
  }
  return null
}

export interface FieldManagerProps {
  source: ReadOk<TableSourceContent>
  /** The view (a Block, or the table's synthetic view): a new field joins its explicit column list. */
  cell: ReadOk<KeyedContent<CellPayload>>
  /** Every field in the view's order, shown or not. */
  entries: ColumnEntry[]
  /** Why the view's columns cannot be changed here, or null. */
  columnsReason: string | null
  onColumns: (next: ColumnEntry[]) => void
  /** Why the table's fields cannot be changed here, or null. */
  structureReason: string | null
}

export function FieldManager(props: FieldManagerProps) {
  const [detail, setDetail] = useState<string | null>(null)
  const entry = detail ? props.entries.find((item) => item.field.field_id === detail) : undefined
  return (
    <div className="aiws-fields" data-testid="aiws-field-manager">
      {entry ? <FieldDetail {...props} entry={entry} onBack={() => setDetail(null)} /> : <FieldList {...props} onOpen={setDetail} />}
    </div>
  )
}

function typeText(field: FieldDef): string {
  return `${FIELD_TYPE_LABEL[field.type]}${field.type === 'decimal' ? `(${field.scale ?? 0})` : field.type === 'currency' ? `(${field.currency ?? ''})` : ''}`
}

function FieldList({ source, cell, entries, columnsReason, onColumns, structureReason, onOpen }: FieldManagerProps & { onOpen: (fieldId: string) => void }) {
  const { t, locale } = useAiwsText()
  const store = useStore()
  const sourceId = source.entity_id
  const [newName, setNewName] = useState('')
  const [newType, setNewType] = useState<FieldType>('text')
  const [newScale, setNewScale] = useState('2')
  const [newCurrency, setNewCurrency] = useState(() => (locale.startsWith('zh') ? 'CNY' : 'USD'))
  const [drag, setDrag] = useState<{ id: string; to: number } | null>(null)
  const listRef = useRef<HTMLUListElement>(null)

  const addField = () => {
    const name = newName.trim()
    if (!name || structureReason) return
    if (newType === 'currency' && !CURRENCY_CODE.test(newCurrency)) { store.notify('info', t('aiworkspace.data.field.currencyInvalid')); return }
    void store.submit({ editId: `schema:${sourceId}:${randomId()}`, label: t('aiworkspace.data.field.addEdit', { name }), operations: addFieldOps(sourceId, cell, name, newType, Number(newScale) || 0, newCurrency) })
      .then((outcome) => { if (outcome.status === 'accepted') setNewName('') })
  }
  /** The handle drags a row up or down (touch or mouse); the order is written when it is let go. */
  const startDrag = (event: ReactPointerEvent<HTMLElement>, id: string) => {
    if (columnsReason) return
    event.preventDefault()
    const handle = event.currentTarget
    handle.setPointerCapture(event.pointerId)
    const from = entries.findIndex((entry) => entry.field.field_id === id)
    let to = from
    const target = (clientY: number) => {
      const rows = [...(listRef.current?.querySelectorAll<HTMLElement>('[data-field-row]') ?? [])]
      for (let i = 0; i < rows.length; i++) { const r = rows[i].getBoundingClientRect(); if (clientY < r.top + r.height / 2) return i }
      return rows.length
    }
    const move = (e: PointerEvent) => { to = target(e.clientY); setDrag({ id, to }) }
    const up = () => {
      handle.removeEventListener('pointermove', move)
      handle.removeEventListener('pointerup', up)
      handle.removeEventListener('pointercancel', up)
      setDrag(null)
      const index = to > from ? to - 1 : to
      if (index !== from) onColumns(moveEntry(entries, from, index))
    }
    handle.addEventListener('pointermove', move)
    handle.addEventListener('pointerup', up)
    handle.addEventListener('pointercancel', up)
    setDrag({ id, to })
  }
  return (
    <>
      <div className="aiws-fields-head"><b>{t('aiworkspace.data.table.fields')}</b><span className="aiws-muted">{t('aiworkspace.table.fields.hint')}</span></div>
      {columnsReason && <div className="aiws-muted">{columnsReason}</div>}
      <ul ref={listRef} className="aiws-field-list" aria-label={t('aiworkspace.data.table.fields')}>
        {entries.map((entry, index) => {
          const { field } = entry
          const Icon = FIELD_ICONS[field.type] ?? Type
          return (
            <li key={field.field_id} className={`aiws-field-item${drag?.id === field.field_id ? ' is-dragged' : ''}${drag && drag.to === index && drag.id !== field.field_id ? ' is-drop-before' : ''}`} data-field-row="" data-testid={`aiws-field-row-${field.field_id}`}>
              <span className={`aiws-field-grip${columnsReason ? ' is-disabled' : ''}`} role="button" tabIndex={-1} aria-label={t('aiworkspace.table.fields.drag', { name: field.name })} title={columnsReason ?? t('aiworkspace.table.fields.drag', { name: field.name })}
                data-testid={`aiws-field-drag-${field.field_id}`} onPointerDown={(event) => startDrag(event, field.field_id)}><GripVertical size={16} aria-hidden="true" /></span>
              <button type="button" className="aiws-field-open" data-testid={`aiws-field-open-${field.field_id}`} title={field.field_id} onClick={() => onOpen(field.field_id)}>
                <Icon size={15} className="aiws-muted" aria-hidden="true" />
                <span className="aiws-field-name">{field.name}</span>
                <span className="aiws-muted aiws-field-type">{typeText(field)}{field.required ? t('aiworkspace.data.field.required') : ''}</span>
                <ChevronRight size={16} className="aiws-muted" aria-hidden="true" />
              </button>
              <label className="aiws-field-show" title={columnsReason ?? undefined}>
                <input type="checkbox" aria-label={t('aiworkspace.data.field.showAria', { name: field.name })} data-testid={`aiws-field-show-${field.field_id}`} checked={entry.shown} disabled={Boolean(columnsReason)}
                  onChange={(event) => onColumns(entries.map((item) => (item.field.field_id === field.field_id ? { ...item, shown: event.target.checked } : item)))} />
              </label>
            </li>
          )
        })}
        {drag && drag.to === entries.length && <li className="aiws-field-drop-end" aria-hidden="true" />}
      </ul>
      {structureReason ? <div className="aiws-muted">{structureReason}</div> : (
        <form className="aiws-inline-form aiws-field-add" onSubmit={(event) => { event.preventDefault(); addField() }}>
          <input aria-label={t('aiworkspace.data.field.newName')} placeholder={t('aiworkspace.data.field.newName')} value={newName} onChange={(event) => setNewName(event.target.value)} />
          <select aria-label={t('aiworkspace.data.field.newType')} value={newType} onChange={(event) => setNewType(event.target.value as FieldType)}>
            {CREATABLE_FIELD_TYPES.map((type) => <option key={type} value={type}>{FIELD_TYPE_LABEL[type]}</option>)}
          </select>
          {newType === 'decimal' && <input aria-label={t('aiworkspace.data.field.scale')} inputMode="numeric" style={{ width: 56 }} value={newScale} onChange={(event) => setNewScale(event.target.value)} />}
          {newType === 'currency' && <CurrencyInput label={t('aiworkspace.data.field.currency')} value={newCurrency} onChange={setNewCurrency} />}
          <button type="submit" data-testid="aiws-add-field" disabled={!newName.trim()}><Plus size={14} aria-hidden="true" />{t('aiworkspace.data.field.add')}</button>
        </form>
      )}
    </>
  )
}

function FieldDetail({ source, entries, entry, columnsReason, onColumns, structureReason, onBack }: FieldManagerProps & { entry: ColumnEntry; onBack: () => void }) {
  const { t } = useAiwsText()
  const store = useStore()
  const sourceId = source.entity_id
  const { field } = entry
  const fields = source.content.fields
  const [name, setName] = useState<string | null>(null)
  const [currency, setCurrency] = useState<string | null>(null)
  const [optionText, setOptionText] = useState<Record<string, string>>({})
  const [migration, setMigration] = useState<{ to: FieldType; scale: string; currency: string; onFailure: 'reject' | 'unset'; report: MigrationReport | null; status: string | null } | null>(null)
  const [asking, setAsking] = useState(false)
  const index = entries.findIndex((item) => item.field.field_id === field.field_id)
  const locked = structureReason !== null

  const submit = (label: string, operations: Operation[]) => store.submit({ editId: `schema:${sourceId}:${randomId()}`, label, operations })
  const rename = () => {
    const next = (name ?? field.name).trim()
    setName(null)
    if (next && next !== field.name) void submit(t('aiworkspace.data.field.renameEdit', { from: field.name, to: next }), [{ op: 'table.update_field', source_id: sourceId, field_id: field.field_id, changes: { name: next }, expect: { rev: field.def_rev } }])
  }
  const change = (label: string, changes: Record<string, Json>) => submit(label, [{ op: 'table.update_field', source_id: sourceId, field_id: field.field_id, changes, expect: { rev: field.def_rev } }])
  const migrationOp = (plan: NonNullable<typeof migration>): Operation => ({
    op: 'table.migrate_field', source_id: sourceId, field_id: field.field_id, expect: { rev: field.def_rev },
    to: { type: plan.to, ...(plan.to === 'decimal' || plan.to === 'currency' ? { scale: Number(plan.scale) || 0 } : {}), ...(plan.to === 'currency' ? { currency: plan.currency } : {}) }, on_failure: plan.onFailure,
  })
  const preCheck = async () => {
    if (!migration) return
    const result = await store.session.prepare([migrationOp(migration)])
    const report = readReport(result)
    let status = result.status === 'ok' ? t('aiworkspace.data.migration.precheckOk')
      : result.status === 'rejected' ? t('aiworkspace.data.migration.precheckFailedDetail', { code: result.code, detail: result.errors?.[0]?.detail ?? '' })
        : t('aiworkspace.data.migration.precheckFailed', { code: result.code })
    // currency → decimal drops each value's currency: say how many are not in the field's default (§6.1)
    if (field.type === 'currency' && migration.to === 'decimal' && field.currency) {
      let other = 0, cursor: string | undefined
      for (let pages = 0; pages < 20; pages++) {
        const page = await store.session.query({ source_id: sourceId, fields: [field.field_id], limit: 1000, consistency: 'best_effort', ...(cursor ? { cursor } : {}) })
        for (const row of page.rows) { const money = moneyParts(row.values[field.field_id]); if (money && money.currency !== field.currency) other += 1 }
        if (!page.next_cursor || page.rows.length === 0) break
        cursor = page.next_cursor
      }
      if (other > 0) status = `${status} ${t('aiworkspace.data.migration.otherCurrencies', { count: other, currency: field.currency })}`
    }
    setMigration({ ...migration, report, status })
  }
  const setWidth = (preset: WidthPreset) => onColumns(entries.map((item) => (item.field.field_id === field.field_id ? { ...item, width: WIDTHS[preset] } : item)))
  const moveBy = (delta: number) => onColumns(moveEntry(entries, index, index + delta))
  const preset = widthPreset(entry.width)

  return (
    <div className="aiws-field-detail" data-testid="aiws-field-detail" data-field-id={field.field_id}>
      <div className="aiws-fields-head">
        <button type="button" className="aiws-table-btn" aria-label={t('aiworkspace.table.fields.back')} title={t('aiworkspace.table.fields.back')} data-testid="aiws-field-back" onClick={onBack}><ChevronLeft size={18} aria-hidden="true" /></button>
        <b>{field.name}</b>
        <span className="aiws-muted">{typeText(field)}</span>
      </div>
      {structureReason && <div className="aiws-muted">{structureReason}</div>}

      <label className="aiws-field-section">
        <span>{t('aiworkspace.data.field.name')}</span>
        <input aria-label={t('aiworkspace.data.field.nameAria', { id: field.field_id })} value={name ?? field.name} disabled={locked} title={field.field_id}
          onChange={(event) => setName(event.target.value)} onBlur={rename}
          onKeyDown={(event) => { if (event.key === 'Enter') { event.preventDefault(); rename() } else if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); setName(null) } }} />
      </label>

      <div className="aiws-field-section">
        <span>{t('aiworkspace.data.field.type')}</span>
        <div className="aiws-field-row">
          <span>{typeText(field)}</span>
          {MIGRATIONS[field.type] && !locked && (
            <button type="button" data-testid={`aiws-migrate-${field.field_id}`} aria-expanded={migration !== null}
              onClick={() => setMigration(migration ? null : { to: MIGRATIONS[field.type]?.[0] ?? 'text', scale: String(field.scale ?? 2), currency: field.currency ?? 'USD', onFailure: 'reject', report: null, status: null })}>{t('aiworkspace.data.field.changeType')}</button>
          )}
        </div>
        {migration && (
          <div className="aiws-inline-form aiws-field-migration" data-testid="aiws-migration">
            <b>{t('aiworkspace.data.migration.title', { name: field.name, type: FIELD_TYPE_LABEL[field.type] })}</b>
            <select aria-label={t('aiworkspace.data.migration.targetType')} value={migration.to} onChange={(event) => setMigration({ ...migration, to: event.target.value as FieldType, report: null, status: null })}>
              {(MIGRATIONS[field.type] ?? []).map((type) => <option key={type} value={type}>{FIELD_TYPE_LABEL[type]}</option>)}
            </select>
            {(migration.to === 'decimal' || migration.to === 'currency') && <input aria-label={t('aiworkspace.data.field.scale')} inputMode="numeric" style={{ width: 56 }} value={migration.scale} onChange={(event) => setMigration({ ...migration, scale: event.target.value, report: null, status: null })} />}
            {migration.to === 'currency' && <CurrencyInput label={t('aiworkspace.data.field.currency')} value={migration.currency} onChange={(code) => setMigration({ ...migration, currency: code, report: null, status: null })} />}
            <select aria-label={t('aiworkspace.data.migration.onFailure')} value={migration.onFailure} onChange={(event) => setMigration({ ...migration, onFailure: event.target.value === 'unset' ? 'unset' : 'reject', report: null, status: null })}>
              <option value="reject">{t('aiworkspace.data.migration.reject')}</option>
              <option value="unset">{t('aiworkspace.data.migration.unset')}</option>
            </select>
            <button type="button" data-testid="aiws-migration-precheck" onClick={() => { void preCheck() }}>{t('aiworkspace.data.migration.precheck')}</button>
            <button type="button" data-testid="aiws-migration-run" disabled={!migration.report} title={migration.report ? undefined : t('aiworkspace.data.migration.precheckFirst')} onClick={() => {
              void submit(t('aiworkspace.data.migration.edit', { name: field.name, type: FIELD_TYPE_LABEL[migration.to] }), [migrationOp(migration)]).then((outcome) => { if (outcome.status === 'accepted') setMigration(null) })
            }}>{t('aiworkspace.data.migration.run')}</button>
            <button type="button" onClick={() => setMigration(null)}>{t('aiworkspace.common.close')}</button>
            {migration.status && <div data-testid="aiws-migration-status">{migration.status}</div>}
            {migration.report && (
              <div data-testid="aiws-migration-report">
                {t('aiworkspace.data.migration.report', { total: migration.report.total, convertible: migration.report.convertible, failing: migration.report.failing.count, unset: migration.report.unset })}
                {migration.report.failing.sample.length > 0 && (
                  <ul>{migration.report.failing.sample.slice(0, 10).map((item) => <li key={item.record_id}>{t('aiworkspace.data.migration.sample', { record: item.record_id, value: JSON.stringify(item.value) })}</li>)}</ul>
                )}
              </div>
            )}
          </div>
        )}
      </div>

      {field.type === 'currency' && (
        <form className="aiws-field-section" onSubmit={(event) => {
          event.preventDefault()
          const code = currency ?? field.currency ?? ''
          if (!CURRENCY_CODE.test(code)) { store.notify('info', t('aiworkspace.data.field.currencyInvalid')); return }
          // only values written later take the new default: stored values carry their own currency
          if (code !== field.currency) void change(t('aiworkspace.data.field.currencyEdit', { name: field.name, currency: code }), { currency: code })
          setCurrency(null)
        }}>
          <span title={t('aiworkspace.data.field.currencyHint')}>{t('aiworkspace.data.field.currency')}</span>
          <div className="aiws-field-row">
            <CurrencyInput label={t('aiworkspace.data.field.currency')} value={currency ?? field.currency ?? ''} onChange={setCurrency} />
            <button type="submit" data-testid={`aiws-field-currency-${field.field_id}`} disabled={locked}>{t('aiworkspace.common.save')}</button>
          </div>
        </form>
      )}

      {(field.type === 'select' || field.type === 'multi_select') && (
        <div className="aiws-field-section">
          <span>{t('aiworkspace.table.fields.options')}</span>
          <ul className="aiws-option-list">
            {(field.options ?? []).map((option) => {
              const key = `${field.field_id}/${option.option_id}`
              const text = optionText[key]
              const commit = () => {
                const label = (text ?? option.label).trim()
                setOptionText((all) => { const next = { ...all }; delete next[key]; return next })
                if (label && label !== option.label) void submit(t('aiworkspace.data.option.renameEdit', { from: option.label, to: label }), [{ op: 'table.update_option', source_id: sourceId, field_id: field.field_id, option_id: option.option_id, label, expect: { rev: field.def_rev } }])
              }
              return (
                <li key={option.option_id} className="aiws-option-item" data-testid={`aiws-option-${option.option_id}`}>
                  <input aria-label={t('aiworkspace.data.option.newNameAria', { id: option.option_id })} title={option.option_id} value={text ?? option.label} disabled={locked}
                    onChange={(event) => setOptionText({ ...optionText, [key]: event.target.value })} onBlur={commit}
                    onKeyDown={(event) => { if (event.key === 'Enter') { event.preventDefault(); commit() } }} />
                  <button type="button" disabled={locked} title={t('aiworkspace.data.option.deleteTitle')} data-testid={`aiws-option-delete-${option.option_id}`} onClick={() => {
                    void submit(t('aiworkspace.data.option.deleteEdit', { name: option.label }), [{ op: 'table.delete_option', source_id: sourceId, field_id: field.field_id, option_id: option.option_id, on_values: 'reject_if_used', expect: { rev: field.def_rev } }])
                  }}>{t('aiworkspace.common.delete')}</button>
                  <button type="button" disabled={locked} title={t('aiworkspace.data.option.deleteClearTitle')} data-testid={`aiws-option-delete-clear-${option.option_id}`} onClick={() => {
                    void submit(t('aiworkspace.data.option.deleteClearEdit', { name: option.label }), [{ op: 'table.delete_option', source_id: sourceId, field_id: field.field_id, option_id: option.option_id, on_values: 'unset', expect: { rev: field.def_rev } }])
                  }}>{t('aiworkspace.data.option.deleteClearShort')}</button>
                </li>
              )
            })}
          </ul>
          {!locked && (
            <form className="aiws-option-add" onSubmit={(event) => {
              event.preventDefault()
              const label = (optionText[field.field_id] ?? '').trim()
              if (!label) return
              void submit(t('aiworkspace.data.option.addEdit', { name: label }), [{ op: 'table.add_option', source_id: sourceId, field_id: field.field_id, option: { option_id: randomId('o'), label }, expect: { rev: field.def_rev } }])
              setOptionText({ ...optionText, [field.field_id]: '' })
            }}>
              <input aria-label={t('aiworkspace.data.option.newAria', { id: field.field_id })} placeholder={t('aiworkspace.data.option.newPlaceholder')} value={optionText[field.field_id] ?? ''} onChange={(event) => setOptionText({ ...optionText, [field.field_id]: event.target.value })} />
              <button type="submit" aria-label={t('aiworkspace.table.fields.addOption')} title={t('aiworkspace.table.fields.addOption')}><Plus size={14} aria-hidden="true" /></button>
            </form>
          )}
        </div>
      )}

      <label className="aiws-field-section aiws-field-toggle">
        <input type="checkbox" data-testid="aiws-field-required" checked={Boolean(field.required)} disabled={locked}
          onChange={(event) => { void change(t(event.target.checked ? 'aiworkspace.table.fields.requiredOnEdit' : 'aiworkspace.table.fields.requiredOffEdit', { name: field.name }), { required: event.target.checked }) }} />
        <span>{t('aiworkspace.table.fields.required')}</span>
      </label>

      <div className="aiws-field-section" title={columnsReason ?? undefined}>
        <span>{t('aiworkspace.table.fields.inView')}</span>
        <label className="aiws-field-toggle">
          <input type="checkbox" aria-label={t('aiworkspace.data.field.showAria', { name: field.name })} checked={entry.shown} disabled={Boolean(columnsReason)}
            onChange={(event) => onColumns(entries.map((item) => (item.field.field_id === field.field_id ? { ...item, shown: event.target.checked } : item)))} />
          <span>{t('aiworkspace.table.fields.shown')}</span>
        </label>
        <div className="aiws-field-row" role="radiogroup" aria-label={t('aiworkspace.table.fields.width')}>
          <span className="aiws-muted">{t('aiworkspace.table.fields.width')}</span>
          {(Object.keys(WIDTHS) as WidthPreset[]).map((key) => (
            <button key={key} type="button" role="radio" aria-checked={preset === key || (preset === null && key === 'medium' && entry.width === DEFAULT_WIDTH)} disabled={Boolean(columnsReason)}
              data-testid={`aiws-field-width-${key}`} onClick={() => setWidth(key)}>{t(WIDTH_LABEL[key])}</button>
          ))}
        </div>
        <div className="aiws-field-row">
          <span className="aiws-muted">{t('aiworkspace.table.fields.order')}</span>
          <button type="button" data-testid="aiws-field-up" disabled={Boolean(columnsReason) || index <= 0} onClick={() => moveBy(-1)}>{t('aiworkspace.table.fields.up')}</button>
          <button type="button" data-testid="aiws-field-down" disabled={Boolean(columnsReason) || index < 0 || index >= entries.length - 1} onClick={() => moveBy(1)}>{t('aiworkspace.table.fields.down')}</button>
        </div>
      </div>

      {!locked && (
        <div className="aiws-field-section">
          {/* asked in place: the field list may itself be in a pop-over (a phone's sheet) */}
          {asking ? (
            <div className="aiws-field-confirm" role="alertdialog" aria-label={t('aiworkspace.table.fields.deleteAsk', { name: field.name })} data-testid="aiws-table-confirm">
              <b>{t('aiworkspace.table.fields.deleteAsk', { name: field.name })}</b>
              <span className="aiws-muted">{t('aiworkspace.table.undoHint')}</span>
              <div className="aiws-table-pop-actions">
                <button type="button" onClick={() => setAsking(false)}>{t('aiworkspace.common.cancel')}</button>
                <button type="button" className="is-danger" data-testid="aiws-table-confirm-ok" autoFocus onClick={() => {
                  setAsking(false)
                  void submit(t('aiworkspace.data.field.deleteEdit', { name: field.name }), [{ op: 'table.delete_field', source_id: sourceId, field_id: field.field_id, expect: { rev: field.def_rev } }]).then((outcome) => { if (outcome.status === 'accepted') onBack() })
                }}>{t('aiworkspace.common.delete')}</button>
              </div>
            </div>
          ) : (
            <button type="button" className="is-danger" data-testid="aiws-field-delete" disabled={fields.length <= 1} title={fields.length <= 1 ? t('aiworkspace.table.fields.lastField') : undefined}
              onClick={() => setAsking(true)}><Trash2 size={14} aria-hidden="true" />{t('aiworkspace.table.fields.delete')}</button>
          )}
        </div>
      )}
    </div>
  )
}
