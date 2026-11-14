/* Filter and sort of the table editor (UI 框架 PRD §7.3): any number of conditions, all of which must hold, and a
 * sort, for this window only until "save view" writes them into the Block's view. The conditions in force show as
 * chips under the title row, each one removable on its own. */

import { useState } from 'react'
import { Plus, X } from 'lucide-react'
import type { FieldDef, FilterNode, Json } from '../../api/types'
import { useAiwsText } from '../../useAiwsText'
import { UNSET, editText, operatorsFor, parseInput, SORTABLE } from '../values'
import { conditionsOf, type Condition, type SessionView } from './model'

interface Row { key: number; fieldId: string; operator: string; text: string }

function rowsOf(fields: readonly FieldDef[], filter: FilterNode | null): Row[] {
  const conditions = conditionsOf(filter).filter((c): c is Condition => c !== null)
  if (conditions.length === 0) return [{ key: 0, fieldId: fields[0]?.field_id ?? '', operator: fields[0] ? operatorsFor(fields[0].type)[0].id : 'eq', text: '' }]
  return conditions.map((c, key) => {
    const field = fields.find((item) => item.field_id === c.field_id)
    return { key, fieldId: c.field_id, operator: c.operator, text: c.value === undefined ? '' : field ? (field.type === 'multi_select' && Array.isArray(c.value) ? c.value.join(', ') : editText(field, c.value)) : String(c.value) }
  })
}

function conditionOf(fields: readonly FieldDef[], row: Row): Condition | null {
  const field = fields.find((item) => item.field_id === row.fieldId)
  if (!field) return null
  const operator = operatorsFor(field.type).find((item) => item.id === row.operator) ?? operatorsFor(field.type)[0]
  if (operator.needsValue && row.text === '') return null
  let value: Json | undefined
  if (operator.needsValue) {
    const text = row.text
    if (field.type === 'number') value = Number(text)
    else if (field.type === 'boolean') value = text === 'true'
    else if (field.type === 'multi_select') value = text.split(',').map((item) => item.trim()).filter(Boolean)
    // `100` (the field's currency) or `CNY 100`, `¥100`
    else if (field.type === 'currency') { const parsed = parseInput(field, text); value = parsed.ok && parsed.value !== UNSET ? parsed.value : text }
    else value = text
  }
  return { op: 'cmp', field_id: field.field_id, operator: operator.id, ...(value !== undefined ? { value } : {}) }
}

export function FilterSortPanel({ fields, session, hasSavedFilter, hasSavedSorts, canSave, onApply, onSave, onClearSaved }: {
  fields: FieldDef[]
  session: SessionView
  hasSavedFilter: boolean
  hasSavedSorts: boolean
  canSave: boolean
  onApply: (view: SessionView) => void
  onSave: () => void
  onClearSaved: (key: 'filter' | 'sorts') => void
}) {
  const { t } = useAiwsText()
  const [rows, setRows] = useState<Row[]>(() => rowsOf(fields, session.filter))
  const [nextKey, setNextKey] = useState(() => rows.length)
  const sortable = fields.filter((item) => SORTABLE.has(item.type))
  const [sortField, setSortField] = useState(session.sorts?.[0]?.field_id ?? '')
  const [direction, setDirection] = useState<'asc' | 'desc'>(session.sorts?.[0]?.direction ?? 'asc')
  const update = (key: number, change: Partial<Row>) => setRows((list) => list.map((row) => (row.key === key ? { ...row, ...change } : row)))
  const apply = () => {
    const conditions = rows.map((row) => conditionOf(fields, row)).filter((c): c is Condition => c !== null)
    const filter: FilterNode | null = conditions.length === 0 ? null : conditions.length === 1 ? conditions[0] : { op: 'and', args: conditions }
    onApply({ filter, sorts: sortField ? [{ field_id: sortField, direction }] : null })
  }
  const clear = () => {
    onApply({ filter: null, sorts: null })
    setRows(rowsOf(fields, null))
    setSortField('')
  }
  return (
    <div className="aiws-inline-form aiws-filter-panel" data-testid="aiws-filter-panel">
      <div className="aiws-filter-title">{t('aiworkspace.data.filter.temporaryFilter')}</div>
      {rows.map((row, index) => {
        const field = fields.find((item) => item.field_id === row.fieldId)
        const operators = field ? operatorsFor(field.type) : []
        const operator = operators.find((item) => item.id === row.operator) ?? operators[0]
        return (
          <div key={row.key} className="aiws-filter-row" data-testid="aiws-filter-row">
            {index > 0 && <span className="aiws-muted aiws-filter-and">{t('aiworkspace.table.filter.and')}</span>}
            <select aria-label={t('aiworkspace.data.filter.field')} value={row.fieldId}
              onChange={(event) => { const next = fields.find((item) => item.field_id === event.target.value); update(row.key, { fieldId: event.target.value, operator: next ? operatorsFor(next.type)[0].id : 'eq', text: '' }) }}>
              {fields.map((item) => <option key={item.field_id} value={item.field_id}>{item.name}</option>)}
            </select>
            <select aria-label={t('aiworkspace.data.filter.operator')} value={operator?.id ?? ''} onChange={(event) => update(row.key, { operator: event.target.value })}>
              {operators.map((item) => <option key={item.id} value={item.id}>{item.label}</option>)}
            </select>
            {operator?.needsValue && (field?.type === 'select' || field?.type === 'boolean' ? (
              <select aria-label={t('aiworkspace.data.filter.value')} value={row.text} onChange={(event) => update(row.key, { text: event.target.value })}>
                <option value="">{t('aiworkspace.data.filter.choose')}</option>
                {field.type === 'boolean'
                  ? [<option key="t" value="true">{t('aiworkspace.common.yes')}</option>, <option key="f" value="false">{t('aiworkspace.common.no')}</option>]
                  : (field.options ?? []).map((option) => <option key={option.option_id} value={option.option_id}>{option.label}</option>)}
              </select>
            ) : (
              <input aria-label={t('aiworkspace.data.filter.value')} type={field?.type === 'date' ? 'date' : 'text'} inputMode={field?.type === 'number' || field?.type === 'decimal' ? 'decimal' : undefined}
                value={row.text} onChange={(event) => update(row.key, { text: event.target.value })}
                onKeyDown={(event) => { if (event.key === 'Enter') { event.preventDefault(); apply() } }}
                placeholder={field?.type === 'multi_select' ? t('aiworkspace.data.filter.optionIds') : t('aiworkspace.data.filter.valuePlaceholder')} />
            ))}
            {rows.length > 1 && (
              <button type="button" className="aiws-table-btn" aria-label={t('aiworkspace.table.filter.removeRow')} title={t('aiworkspace.table.filter.removeRow')} data-testid="aiws-filter-row-remove"
                onClick={() => setRows((list) => list.filter((item) => item.key !== row.key))}><X size={16} aria-hidden="true" /></button>
            )}
          </div>
        )
      })}
      <div>
        <button type="button" className="aiws-link" data-testid="aiws-filter-add-condition" disabled={fields.length === 0}
          onClick={() => { setRows((list) => [...list, { key: nextKey, fieldId: fields[0]?.field_id ?? '', operator: fields[0] ? operatorsFor(fields[0].type)[0].id : 'eq', text: '' }]); setNextKey((n) => n + 1) }}>
          <Plus size={14} aria-hidden="true" />{t('aiworkspace.table.filter.addCondition')}
        </button>
      </div>
      <div className="aiws-filter-title">{t('aiworkspace.data.filter.temporarySort')}</div>
      <div className="aiws-filter-row">
        <select aria-label={t('aiworkspace.data.filter.sortField')} value={sortField} onChange={(event) => setSortField(event.target.value)}>
          <option value="">{t('aiworkspace.data.filter.keepViewSort')}</option>
          {sortable.map((item) => <option key={item.field_id} value={item.field_id}>{item.name}</option>)}
        </select>
        <select aria-label={t('aiworkspace.data.filter.sortDirection')} value={direction} onChange={(event) => setDirection(event.target.value === 'desc' ? 'desc' : 'asc')}>
          <option value="asc">{t('aiworkspace.data.filter.ascending')}</option><option value="desc">{t('aiworkspace.data.filter.descending')}</option>
        </select>
      </div>
      <div className="aiws-filter-actions">
        <button type="button" className="is-primary" data-testid="aiws-filter-apply" onClick={apply}>{t('aiworkspace.data.filter.apply')}</button>
        <button type="button" data-testid="aiws-filter-clear" onClick={clear}>{t('aiworkspace.data.filter.clear')}</button>
        {canSave && <button type="button" data-testid="aiws-view-save" disabled={!session.filter && !session.sorts} onClick={onSave} title={t('aiworkspace.data.filter.saveTitle')}>{t('aiworkspace.data.filter.save')}</button>}
        {canSave && hasSavedFilter && <button type="button" onClick={() => onClearSaved('filter')}>{t('aiworkspace.data.filter.removeSavedFilter')}</button>}
        {canSave && hasSavedSorts && <button type="button" onClick={() => onClearSaved('sorts')}>{t('aiworkspace.data.filter.removeSavedSort')}</button>}
      </div>
      <div className="aiws-muted">{t('aiworkspace.data.filter.hint')}</div>
    </div>
  )
}
