/* Field and option management of a TableSource (design §3.4.4, §3.4.6), opened from a table view. */

import { useState } from 'react'
import { randomId } from '../api/ids'
import type { ReadOk } from '../api/session'
import type { CellPayload, FieldDef, FieldType, Json, KeyedContent, Operation, PrepareResult, TableSourceContent } from '../api/types'
import { useStore } from '../state/hooks'
import { FIELD_TYPE_LABEL } from './values'

const CREATABLE: FieldType[] = ['text', 'number', 'decimal', 'date', 'datetime', 'boolean', 'select', 'multi_select']
/** Conversions the backend implements (anything else answers MIGRATION_UNSUPPORTED). */
const MIGRATIONS: Partial<Record<FieldType, FieldType[]>> = { text: ['date', 'number', 'decimal'], number: ['decimal'], select: ['text'] }

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
  const store = useStore()
  const sourceId = source.entity_id
  const fields = source.content.fields
  const viewFields = cell.content.payload.fields
  const [newName, setNewName] = useState('')
  const [newType, setNewType] = useState<FieldType>('text')
  const [newScale, setNewScale] = useState('2')
  const [renaming, setRenaming] = useState<{ id: string; text: string } | null>(null)
  const [optionText, setOptionText] = useState<Record<string, string>>({})
  const [migration, setMigration] = useState<{ fieldId: string; to: FieldType; scale: string; onFailure: 'reject' | 'unset'; report: MigrationReport | null; status: string | null } | null>(null)

  const submit = (label: string, operations: Operation[]) => store.submit({ editId: `schema:${sourceId}:${randomId()}`, label, operations })

  const viewFieldsOp = (next: { field_id: string; width?: number }[]): Operation => ({
    op: 'entity.set_keys', entity_id: cell.entity_id,
    keys: [{ key: 'fields', value: next as unknown as Json, expect: { rev: cell.content.key_revs.fields ?? 0 } }],
  })

  const addField = () => {
    const name = newName.trim()
    if (!name) return
    const fieldId = randomId('f')
    const field: Record<string, Json> = { field_id: fieldId, name, type: newType }
    if (newType === 'decimal') field.scale = Number(newScale) || 0
    if (newType === 'select' || newType === 'multi_select') field.options = []
    const operations: Operation[] = [{ op: 'table.add_field', source_id: sourceId, field }]
    // A view with an explicit column list would not show the new field: add it to this view in the same commit.
    if (viewFields && viewFields.length > 0) operations.push(viewFieldsOp([...viewFields, { field_id: fieldId }]))
    void submit(`新增字段「${name}」`, operations).then((outcome) => { if (outcome.status === 'accepted') setNewName('') })
  }

  const preCheck = async () => {
    if (!migration) return
    const field = fields.find((item) => item.field_id === migration.fieldId)
    if (!field) return
    const result = await store.session.prepare([migrationOp(field, migration)])
    const report = readReport(result)
    const status = result.status === 'ok' ? '预检通过，可以执行。' : `预检未通过：${result.code}${result.status === 'rejected' ? ` — ${result.errors?.[0]?.detail ?? ''}` : ''}`
    setMigration({ ...migration, report, status })
  }

  const migrationOp = (field: FieldDef, plan: NonNullable<typeof migration>): Operation => ({
    op: 'table.migrate_field', source_id: sourceId, field_id: field.field_id, expect: { rev: field.def_rev },
    to: { type: plan.to, ...(plan.to === 'decimal' ? { scale: Number(plan.scale) || 0 } : {}) }, on_failure: plan.onFailure,
  })

  return (
    <div className="aiws-fields" data-testid="aiws-field-manager">
      <table>
        <thead><tr><th>字段</th><th>类型</th><th>在此视图显示</th><th>操作</th></tr></thead>
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
                        void submit(`字段改名「${field.name}」→「${name}」`, [{ op: 'table.update_field', source_id: sourceId, field_id: field.field_id, changes: { name }, expect: { rev: field.def_rev } }])
                      }
                      setRenaming(null)
                    }}>
                      <input aria-label={`字段名 ${field.field_id}`} autoFocus value={renaming.text} onChange={(event) => setRenaming({ id: field.field_id, text: event.target.value })} />
                      <button type="submit">保存</button>
                    </form>
                  ) : <span title={field.field_id}>{field.name}</span>}
                </td>
                <td>
                  {FIELD_TYPE_LABEL[field.type]}{field.type === 'decimal' ? `(${field.scale ?? 0})` : ''}{field.required ? ' · 必填' : ''}
                  {(field.type === 'select' || field.type === 'multi_select') && (
                    <div className="aiws-options">
                      {(field.options ?? []).map((option) => (
                        <span key={option.option_id} className="aiws-chip" title={option.option_id}>
                          {option.label}
                          <button type="button" className="aiws-link" title="改名" onClick={() => {
                            const label = (optionText[`${field.field_id}/${option.option_id}`] ?? '').trim()
                            if (!label) { store.notify('info', '先在右侧输入框里写上新的选项名，再点“改”。'); return }
                            void submit(`选项改名「${option.label}」→「${label}」`, [{ op: 'table.update_option', source_id: sourceId, field_id: field.field_id, option_id: option.option_id, label, expect: { rev: field.def_rev } }])
                          }}>改</button>
                          <input aria-label={`选项新名 ${option.option_id}`} className="aiws-option-input" placeholder="新名" value={optionText[`${field.field_id}/${option.option_id}`] ?? ''}
                            onChange={(event) => setOptionText({ ...optionText, [`${field.field_id}/${option.option_id}`]: event.target.value })} />
                          <button type="button" className="aiws-link" title="删除选项（仍被使用时会被拒绝）" onClick={() => {
                            void submit(`删除选项「${option.label}」`, [{ op: 'table.delete_option', source_id: sourceId, field_id: field.field_id, option_id: option.option_id, on_values: 'reject_if_used', expect: { rev: field.def_rev } }])
                          }}>删</button>
                          <button type="button" className="aiws-link" title="删除选项，并把使用它的单元格清空" onClick={() => {
                            void submit(`删除选项「${option.label}」并清空使用处`, [{ op: 'table.delete_option', source_id: sourceId, field_id: field.field_id, option_id: option.option_id, on_values: 'unset', expect: { rev: field.def_rev } }])
                          }}>删并清空</button>
                        </span>
                      ))}
                      <form className="aiws-option-add" onSubmit={(event) => {
                        event.preventDefault()
                        const label = (optionText[field.field_id] ?? '').trim()
                        if (!label) return
                        void submit(`新增选项「${label}」`, [{ op: 'table.add_option', source_id: sourceId, field_id: field.field_id, option: { option_id: randomId('o'), label }, expect: { rev: field.def_rev } }])
                        setOptionText({ ...optionText, [field.field_id]: '' })
                      }}>
                        <input aria-label={`新选项 ${field.field_id}`} placeholder="新选项" value={optionText[field.field_id] ?? ''} onChange={(event) => setOptionText({ ...optionText, [field.field_id]: event.target.value })} />
                        <button type="submit">+</button>
                      </form>
                    </div>
                  )}
                </td>
                <td>
                  <input type="checkbox" aria-label={`显示 ${field.name}`} checked={shown} onChange={(event) => {
                    const base = viewFields && viewFields.length > 0 ? viewFields : fields.map((item) => ({ field_id: item.field_id }))
                    const next = event.target.checked ? [...base, { field_id: field.field_id }] : base.filter((item) => item.field_id !== field.field_id)
                    void store.submit({ editId: `view:${cell.entity_id}`, label: '调整视图列', operations: [viewFieldsOp(next)] })
                  }} />
                </td>
                <td>
                  <button type="button" className="aiws-link" onClick={() => setRenaming({ id: field.field_id, text: field.name })}>改名</button>
                  {MIGRATIONS[field.type] && (
                    <button type="button" className="aiws-link" data-testid={`aiws-migrate-${field.field_id}`} onClick={() => setMigration({ fieldId: field.field_id, to: MIGRATIONS[field.type]?.[0] ?? 'text', scale: '2', onFailure: 'reject', report: null, status: null })}>改类型…</button>
                  )}
                  <button type="button" className="aiws-link" onClick={() => {
                    void submit(`删除字段「${field.name}」`, [{ op: 'table.delete_field', source_id: sourceId, field_id: field.field_id, expect: { rev: field.def_rev } }])
                  }}>删除</button>
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
            <b>迁移字段「{field.name}」（{FIELD_TYPE_LABEL[field.type]}）到：</b>
            <select aria-label="目标类型" value={migration.to} onChange={(event) => setMigration({ ...migration, to: event.target.value as FieldType, report: null, status: null })}>
              {(MIGRATIONS[field.type] ?? []).map((type) => <option key={type} value={type}>{FIELD_TYPE_LABEL[type]}</option>)}
            </select>
            {migration.to === 'decimal' && <input aria-label="小数位数" style={{ width: 48 }} value={migration.scale} onChange={(event) => setMigration({ ...migration, scale: event.target.value, report: null, status: null })} />}
            <select aria-label="转换失败时" value={migration.onFailure} onChange={(event) => setMigration({ ...migration, onFailure: event.target.value === 'unset' ? 'unset' : 'reject', report: null, status: null })}>
              <option value="reject">有失败则整体拒绝</option>
              <option value="unset">失败的单元格清空</option>
            </select>
            <button type="button" data-testid="aiws-migration-precheck" onClick={() => { void preCheck() }}>预检</button>
            <button type="button" data-testid="aiws-migration-run" disabled={!migration.report} title={migration.report ? undefined : '先预检'} onClick={() => {
              void submit(`迁移字段「${field.name}」为${FIELD_TYPE_LABEL[migration.to]}`, [migrationOp(field, migration)]).then((outcome) => { if (outcome.status === 'accepted') setMigration(null) })
            }}>执行迁移</button>
            <button type="button" onClick={() => setMigration(null)}>关闭</button>
            {migration.status && <div data-testid="aiws-migration-status">{migration.status}</div>}
            {migration.report && (
              <div data-testid="aiws-migration-report">
                共 {migration.report.total} 条：可转换 {migration.report.convertible}，无法转换 {migration.report.failing.count}，未设置 {migration.report.unset}。
                {migration.report.failing.sample.length > 0 && (
                  <ul>{migration.report.failing.sample.slice(0, 10).map((item) => <li key={item.record_id}>{item.record_id}：{JSON.stringify(item.value)}</li>)}</ul>
                )}
              </div>
            )}
          </div>
        )
      })()}
      <form className="aiws-inline-form" onSubmit={(event) => { event.preventDefault(); addField() }}>
        <input aria-label="新字段名" placeholder="新字段名" value={newName} onChange={(event) => setNewName(event.target.value)} />
        <select aria-label="新字段类型" value={newType} onChange={(event) => setNewType(event.target.value as FieldType)}>
          {CREATABLE.map((type) => <option key={type} value={type}>{FIELD_TYPE_LABEL[type]}</option>)}
        </select>
        {newType === 'decimal' && <input aria-label="小数位数" style={{ width: 48 }} value={newScale} onChange={(event) => setNewScale(event.target.value)} />}
        <button type="submit" data-testid="aiws-add-field">新增字段</button>
      </form>
    </div>
  )
}
