/* Field and option management of a TableSource (design §3.4.4, §3.4.6), opened from a table view. */

import { useState } from 'react'
import { randomId } from '../api/ids'
import type { ReadOk } from '../api/session'
import type { CellPayload, FieldDef, FieldType, Json, KeyedContent, Operation, PrepareResult, TableSourceContent } from '../api/types'
import { useStore } from '../state/hooks'
import { useAiwsText } from '../useAiwsText'
import { addFieldOps, CREATABLE_FIELD_TYPES } from './fieldOps'
import { COMMON_CURRENCIES, FIELD_TYPE_LABEL, moneyParts } from './values'

/** Conversions the backend implements (anything else answers MIGRATION_UNSUPPORTED). */
const MIGRATIONS: Partial<Record<FieldType, FieldType[]>> = { text: ['date', 'number', 'decimal'], number: ['decimal', 'currency'], decimal: ['decimal', 'currency'], select: ['text'], currency: ['decimal', 'text', 'currency'] }

const CURRENCY_CODE = /^[A-Z]{3}$/
/** A currency code input: the common currencies offered, any ISO 4217 code accepted. */
function CurrencyInput({ value, onChange, label }: { value: string; onChange: (code: string) => void; label: string }) {
  return (
    <>
      <input aria-label={label} title={label} list="aiws-currency-codes" style={{ width: 64 }} maxLength={3} value={value} data-testid="aiws-currency-input" onChange={(event) => onChange(event.target.value.toUpperCase())} />
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

export function FieldManager({ source, cell }: { source: ReadOk<TableSourceContent>; cell: ReadOk<KeyedContent<CellPayload>> }) {
  const { t, locale } = useAiwsText()
  const store = useStore()
  const sourceId = source.entity_id
  const fields = source.content.fields
  const viewFields = cell.content.payload.fields
  const [newName, setNewName] = useState('')
  const [newType, setNewType] = useState<FieldType>('text')
  const [newScale, setNewScale] = useState('2')
  const [newCurrency, setNewCurrency] = useState(() => (locale.startsWith('zh') ? 'CNY' : 'USD'))
  const [currencyEdit, setCurrencyEdit] = useState<{ id: string; code: string } | null>(null)
  const [renaming, setRenaming] = useState<{ id: string; text: string } | null>(null)
  const [optionText, setOptionText] = useState<Record<string, string>>({})
  const [migration, setMigration] = useState<{ fieldId: string; to: FieldType; scale: string; currency: string; onFailure: 'reject' | 'unset'; report: MigrationReport | null; status: string | null } | null>(null)

  const submit = (label: string, operations: Operation[]) => store.submit({ editId: `schema:${sourceId}:${randomId()}`, label, operations })

  const viewFieldsOp = (next: { field_id: string; width?: number }[]): Operation => ({
    op: 'entity.set_keys', entity_id: cell.entity_id,
    keys: [{ key: 'fields', value: next as unknown as Json, expect: { rev: cell.content.key_revs.fields ?? 0 } }],
  })

  const addField = () => {
    const name = newName.trim()
    if (!name) return
    if (newType === 'currency' && !CURRENCY_CODE.test(newCurrency)) { store.notify('info', t('aiworkspace.data.field.currencyInvalid')); return }
    void submit(t('aiworkspace.data.field.addEdit', { name }), addFieldOps(sourceId, cell, name, newType, Number(newScale) || 0, newCurrency)).then((outcome) => { if (outcome.status === 'accepted') setNewName('') })
  }

  const preCheck = async () => {
    if (!migration) return
    const field = fields.find((item) => item.field_id === migration.fieldId)
    if (!field) return
    const result = await store.session.prepare([migrationOp(field, migration)])
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

  const migrationOp = (field: FieldDef, plan: NonNullable<typeof migration>): Operation => ({
    op: 'table.migrate_field', source_id: sourceId, field_id: field.field_id, expect: { rev: field.def_rev },
    to: { type: plan.to, ...(plan.to === 'decimal' || plan.to === 'currency' ? { scale: Number(plan.scale) || 0 } : {}), ...(plan.to === 'currency' ? { currency: plan.currency } : {}) }, on_failure: plan.onFailure,
  })

  return (
    <div className="aiws-fields" data-testid="aiws-field-manager">
      <table>
        <thead><tr><th>{t('aiworkspace.data.field.header')}</th><th>{t('aiworkspace.data.field.typeHeader')}</th><th>{t('aiworkspace.data.field.shownHeader')}</th><th>{t('aiworkspace.data.field.actionsHeader')}</th></tr></thead>
        <tbody>
          {fields.map((field) => {
            const shown = !viewFields || viewFields.length === 0 || viewFields.some((item) => item.field_id === field.field_id)
            return (
              <tr key={field.field_id} data-testid={`aiws-field-row-${field.field_id}`}>
                <td>
                  {renaming?.id === field.field_id ? (
                    <form onSubmit={(event) => {
                      event.preventDefault()
                      const name = renaming.text.trim()
                      if (name && name !== field.name) {
                        void submit(t('aiworkspace.data.field.renameEdit', { from: field.name, to: name }), [{ op: 'table.update_field', source_id: sourceId, field_id: field.field_id, changes: { name }, expect: { rev: field.def_rev } }])
                      }
                      setRenaming(null)
                    }}>
                      <input aria-label={t('aiworkspace.data.field.nameAria', { id: field.field_id })} autoFocus value={renaming.text} onChange={(event) => setRenaming({ id: field.field_id, text: event.target.value })} />
                      <button type="submit">{t('aiworkspace.common.save')}</button>
                    </form>
                  ) : <span title={field.field_id}>{field.name}</span>}
                </td>
                <td>
                  {FIELD_TYPE_LABEL[field.type]}{field.type === 'decimal' ? `(${field.scale ?? 0})` : field.type === 'currency' ? `(${field.currency ?? ''})` : ''}{field.required ? t('aiworkspace.data.field.required') : ''}
                  {field.type === 'currency' && (currencyEdit?.id === field.field_id ? (
                    <form className="aiws-inline-form" onSubmit={(event) => {
                      event.preventDefault()
                      if (!CURRENCY_CODE.test(currencyEdit.code)) { store.notify('info', t('aiworkspace.data.field.currencyInvalid')); return }
                      // only values written later take the new default: stored values carry their own currency
                      if (currencyEdit.code !== field.currency) void submit(t('aiworkspace.data.field.currencyEdit', { name: field.name, currency: currencyEdit.code }), [{ op: 'table.update_field', source_id: sourceId, field_id: field.field_id, changes: { currency: currencyEdit.code }, expect: { rev: field.def_rev } }])
                      setCurrencyEdit(null)
                    }}>
                      <CurrencyInput label={t('aiworkspace.data.field.currency')} value={currencyEdit.code} onChange={(code) => setCurrencyEdit({ id: field.field_id, code })} />
                      <button type="submit">{t('aiworkspace.common.save')}</button>
                    </form>
                  ) : <button type="button" className="aiws-link" title={t('aiworkspace.data.field.currencyHint')} data-testid={`aiws-field-currency-${field.field_id}`} onClick={() => setCurrencyEdit({ id: field.field_id, code: field.currency ?? '' })}>{t('aiworkspace.data.field.changeCurrency')}</button>)}
                  {(field.type === 'select' || field.type === 'multi_select') && (
                    <div className="aiws-options">
                      {(field.options ?? []).map((option) => (
                        <span key={option.option_id} className="aiws-chip" title={option.option_id}>
                          {option.label}
                          <button type="button" className="aiws-link" title={t('aiworkspace.data.option.rename')} onClick={() => {
                            const label = (optionText[`${field.field_id}/${option.option_id}`] ?? '').trim()
                            if (!label) { store.notify('info', t('aiworkspace.data.option.renameHint')); return }
                            void submit(t('aiworkspace.data.option.renameEdit', { from: option.label, to: label }), [{ op: 'table.update_option', source_id: sourceId, field_id: field.field_id, option_id: option.option_id, label, expect: { rev: field.def_rev } }])
                          }}>{t('aiworkspace.data.option.renameShort')}</button>
                          <input aria-label={t('aiworkspace.data.option.newNameAria', { id: option.option_id })} className="aiws-option-input" placeholder={t('aiworkspace.data.option.newNamePlaceholder')} value={optionText[`${field.field_id}/${option.option_id}`] ?? ''}
                            onChange={(event) => setOptionText({ ...optionText, [`${field.field_id}/${option.option_id}`]: event.target.value })} />
                          <button type="button" className="aiws-link" title={t('aiworkspace.data.option.deleteTitle')} onClick={() => {
                            void submit(t('aiworkspace.data.option.deleteEdit', { name: option.label }), [{ op: 'table.delete_option', source_id: sourceId, field_id: field.field_id, option_id: option.option_id, on_values: 'reject_if_used', expect: { rev: field.def_rev } }])
                          }}>{t('aiworkspace.data.option.deleteShort')}</button>
                          <button type="button" className="aiws-link" title={t('aiworkspace.data.option.deleteClearTitle')} onClick={() => {
                            void submit(t('aiworkspace.data.option.deleteClearEdit', { name: option.label }), [{ op: 'table.delete_option', source_id: sourceId, field_id: field.field_id, option_id: option.option_id, on_values: 'unset', expect: { rev: field.def_rev } }])
                          }}>{t('aiworkspace.data.option.deleteClearShort')}</button>
                        </span>
                      ))}
                      <form className="aiws-option-add" onSubmit={(event) => {
                        event.preventDefault()
                        const label = (optionText[field.field_id] ?? '').trim()
                        if (!label) return
                        void submit(t('aiworkspace.data.option.addEdit', { name: label }), [{ op: 'table.add_option', source_id: sourceId, field_id: field.field_id, option: { option_id: randomId('o'), label }, expect: { rev: field.def_rev } }])
                        setOptionText({ ...optionText, [field.field_id]: '' })
                      }}>
                        <input aria-label={t('aiworkspace.data.option.newAria', { id: field.field_id })} placeholder={t('aiworkspace.data.option.newPlaceholder')} value={optionText[field.field_id] ?? ''} onChange={(event) => setOptionText({ ...optionText, [field.field_id]: event.target.value })} />
                        <button type="submit">+</button>
                      </form>
                    </div>
                  )}
                </td>
                <td>
                  <input type="checkbox" aria-label={t('aiworkspace.data.field.showAria', { name: field.name })} checked={shown} onChange={(event) => {
                    const base = viewFields && viewFields.length > 0 ? viewFields : fields.map((item) => ({ field_id: item.field_id }))
                    const next = event.target.checked ? [...base, { field_id: field.field_id }] : base.filter((item) => item.field_id !== field.field_id)
                    void store.submit({ editId: `view:${cell.entity_id}`, label: t('aiworkspace.data.view.columnsEdit'), operations: [viewFieldsOp(next)] })
                  }} />
                </td>
                <td>
                  <button type="button" className="aiws-link" onClick={() => setRenaming({ id: field.field_id, text: field.name })}>{t('aiworkspace.data.rename')}</button>
                  {MIGRATIONS[field.type] && (
                    <button type="button" className="aiws-link" data-testid={`aiws-migrate-${field.field_id}`} onClick={() => setMigration({ fieldId: field.field_id, to: MIGRATIONS[field.type]?.[0] ?? 'text', scale: String(field.scale ?? 2), currency: field.currency ?? newCurrency, onFailure: 'reject', report: null, status: null })}>{t('aiworkspace.data.field.changeType')}</button>
                  )}
                  <button type="button" className="aiws-link" onClick={() => {
                    void submit(t('aiworkspace.data.field.deleteEdit', { name: field.name }), [{ op: 'table.delete_field', source_id: sourceId, field_id: field.field_id, expect: { rev: field.def_rev } }])
                  }}>{t('aiworkspace.common.delete')}</button>
                </td>
              </tr>
            )
          })}
        </tbody>
      </table>
      {migration && (() => {
        const field = fields.find((item) => item.field_id === migration.fieldId)
        if (!field) return null
        return (
          <div className="aiws-inline-form" data-testid="aiws-migration">
            <b>{t('aiworkspace.data.migration.title', { name: field.name, type: FIELD_TYPE_LABEL[field.type] })}</b>
            <select aria-label={t('aiworkspace.data.migration.targetType')} value={migration.to} onChange={(event) => setMigration({ ...migration, to: event.target.value as FieldType, report: null, status: null })}>
              {(MIGRATIONS[field.type] ?? []).map((type) => <option key={type} value={type}>{FIELD_TYPE_LABEL[type]}</option>)}
            </select>
            {(migration.to === 'decimal' || migration.to === 'currency') && <input aria-label={t('aiworkspace.data.field.scale')} style={{ width: 48 }} value={migration.scale} onChange={(event) => setMigration({ ...migration, scale: event.target.value, report: null, status: null })} />}
            {migration.to === 'currency' && <CurrencyInput label={t('aiworkspace.data.field.currency')} value={migration.currency} onChange={(code) => setMigration({ ...migration, currency: code, report: null, status: null })} />}
            <select aria-label={t('aiworkspace.data.migration.onFailure')} value={migration.onFailure} onChange={(event) => setMigration({ ...migration, onFailure: event.target.value === 'unset' ? 'unset' : 'reject', report: null, status: null })}>
              <option value="reject">{t('aiworkspace.data.migration.reject')}</option>
              <option value="unset">{t('aiworkspace.data.migration.unset')}</option>
            </select>
            <button type="button" data-testid="aiws-migration-precheck" onClick={() => { void preCheck() }}>{t('aiworkspace.data.migration.precheck')}</button>
            <button type="button" data-testid="aiws-migration-run" disabled={!migration.report} title={migration.report ? undefined : t('aiworkspace.data.migration.precheckFirst')} onClick={() => {
              void submit(t('aiworkspace.data.migration.edit', { name: field.name, type: FIELD_TYPE_LABEL[migration.to] }), [migrationOp(field, migration)]).then((outcome) => { if (outcome.status === 'accepted') setMigration(null) })
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
        )
      })()}
      <form className="aiws-inline-form" onSubmit={(event) => { event.preventDefault(); addField() }}>
        <input aria-label={t('aiworkspace.data.field.newName')} placeholder={t('aiworkspace.data.field.newName')} value={newName} onChange={(event) => setNewName(event.target.value)} />
        <select aria-label={t('aiworkspace.data.field.newType')} value={newType} onChange={(event) => setNewType(event.target.value as FieldType)}>
          {CREATABLE_FIELD_TYPES.map((type) => <option key={type} value={type}>{FIELD_TYPE_LABEL[type]}</option>)}
        </select>
        {newType === 'decimal' && <input aria-label={t('aiworkspace.data.field.scale')} style={{ width: 48 }} value={newScale} onChange={(event) => setNewScale(event.target.value)} />}
        {newType === 'currency' && <CurrencyInput label={t('aiworkspace.data.field.currency')} value={newCurrency} onChange={setNewCurrency} />}
        <button type="submit" data-testid="aiws-add-field">{t('aiworkspace.data.field.add')}</button>
      </form>
    </div>
  )
}
