/* A to-do's details (控件机制 TODO G5): the record form in the control's pop-over — title, status, due date, priority,
 * tags and its notes (the record's rich-text body, created the first time it is written). Each field is saved when it
 * is left, as its own undoable Commit guarded by that cell's version. */

import { useState } from 'react'
import { Trash2 } from 'lucide-react'
import { todoUpdateOps, type TodoItem } from 'buckyos'
import type { Json, Operation } from '../../../api/types'
import { randomId } from '../../../api/ids'
import { useEntity, useStore } from '../../../state/hooks'
import { useAiwsText } from '../../../useAiwsText'
import { RichTextCell } from '../../blocks/editors'
import { recordSet } from '../data/records'
import { submitData } from '../data/write'
import type { Priority } from './model'

const PRIORITIES: (Priority | 'none')[] = ['none', 'low', 'normal', 'high', 'urgent']
const PRIORITY_LABEL: Record<Priority | 'none', string> = {
  none: 'aiworkspace.todo.priority.none', low: 'aiworkspace.todo.priority.low', normal: 'aiworkspace.todo.priority.normal',
  high: 'aiworkspace.todo.priority.high', urgent: 'aiworkspace.todo.priority.urgent',
}

export function TodoDetail({ sourceId, item, options, canWrite, noWrite, onDelete }: {
  sourceId: string; item: TodoItem; options: { option_id: string; label: string }[]; canWrite: boolean; noWrite: string; onDelete: () => void
}) {
  const { t } = useAiwsText()
  const store = useStore()
  // what is being typed; the stored value otherwise (it follows changes made elsewhere)
  const [titleDraft, setTitle] = useState<string | null>(null)
  const [tagsDraft, setTags] = useState<string | null>(null)
  const title = titleDraft ?? item.title
  const tags = tagsDraft ?? (Array.isArray(item.ext.tags) ? (item.ext.tags as string[]).join(', ') : '')
  const set = recordSet(store, sourceId)
  const save = (label: string, changes: Parameters<typeof todoUpdateOps>[2], overlay: Record<string, Json | undefined>) => {
    const ops = todoUpdateOps(sourceId, item, changes) as Operation[]
    if (ops.length) void submitData(store, set, { editId: `todo:${sourceId}:${item.record_id}:detail`, label, operations: ops, overlay: { [item.record_id]: overlay } })
  }
  const bodyId = useBodyId(sourceId, item.record_id)
  const body = useEntity(bodyId ?? '')
  const addNotes = async () => {
    const id = randomId('rt')
    await store.submit({ editId: `todo:${sourceId}:${item.record_id}:body`, label: t('aiworkspace.todo.addNotes'), operations: [
      { op: 'entity.create', entity_id: id, type_id: 'buckyos.richtext', parent_id: sourceId, order_key: 'n', payload: { content: { type: 'doc', content: [{ type: 'paragraph', attrs: { block_id: randomId('b') } }] } } },
      { op: 'table.set_body', source_id: sourceId, record_id: item.record_id, body_ref: { entity_id: id }, expect: { rev: item.rev ?? 0 } },
    ] })
  }
  const priority = (typeof item.ext.priority === 'string' ? item.ext.priority : 'none') as Priority | 'none'
  const disabled = !canWrite
  return (
    <div className="aiws-ctl-form" data-testid={`aiws-todo-form-${item.record_id}`}>
      <input className="aiws-ctl-form-title" value={title} disabled={disabled} title={disabled ? noWrite : undefined} aria-label={t('aiworkspace.todo.field.title')}
        onChange={(e) => setTitle(e.target.value)}
        onBlur={() => { if (title.trim() && title !== item.title) save(t('aiworkspace.todo.renameOne'), { title: title.trim() }, { title: title.trim() }); setTitle(null) }}
        onKeyDown={(e) => { if (e.key === 'Enter') (e.target as HTMLInputElement).blur() }} />
      <label className="aiws-ctl-form-row">
        <span>{t('aiworkspace.todo.field.status')}</span>
        <select value={item.status} disabled={disabled} onChange={(e) => save(t('aiworkspace.todo.field.status'), { status: e.target.value }, { status: e.target.value })}>
          {options.map((o) => <option key={o.option_id} value={o.option_id}>{o.label}</option>)}
        </select>
      </label>
      <label className="aiws-ctl-form-row">
        <span>{t('aiworkspace.todo.field.due')}</span>
        <span className="aiws-ctl-form-pair">
          <input type="date" value={item.due ?? ''} disabled={disabled} onChange={(e) => save(t('aiworkspace.todo.field.due'), { due: e.target.value || null }, { due: e.target.value || undefined })} />
          {item.due && !disabled && <button type="button" className="aiws-link" onClick={() => save(t('aiworkspace.todo.field.due'), { due: null }, { due: undefined })}>{t('aiworkspace.todo.clear')}</button>}
        </span>
      </label>
      <div className="aiws-ctl-form-row">
        <span>{t('aiworkspace.todo.field.priority')}</span>
        <span className="aiws-dc-seg is-small" role="radiogroup" aria-label={t('aiworkspace.todo.field.priority')}>
          {PRIORITIES.map((p) => (
            <button key={p} type="button" role="radio" aria-checked={priority === p} className={priority === p ? 'is-on' : undefined} disabled={disabled}
              onClick={() => save(t('aiworkspace.todo.field.priority'), { ext_merge: { priority: p === 'none' ? null : p } }, { ext: { ...item.ext, ...(p === 'none' ? {} : { priority: p }) } })}>
              {t(PRIORITY_LABEL[p])}
            </button>
          ))}
        </span>
      </div>
      <label className="aiws-ctl-form-row">
        <span>{t('aiworkspace.todo.field.tags')}</span>
        <input value={tags} disabled={disabled} placeholder={t('aiworkspace.todo.tagsHint')} onChange={(e) => setTags(e.target.value)}
          onBlur={() => {
            const list = [...new Set(tags.split(/[,，]/).map((x) => x.trim()).filter(Boolean))].slice(0, 32)
            const before = Array.isArray(item.ext.tags) ? (item.ext.tags as string[]) : []
            if (JSON.stringify(list) !== JSON.stringify(before)) save(t('aiworkspace.todo.field.tags'), { ext_merge: { tags: list.length ? list : null } }, {})
            setTags(null)
          }} />
      </label>
      <div className="aiws-ctl-form-notes">
        <span className="aiws-ctl-form-label">{t('aiworkspace.todo.field.notes')}</span>
        {bodyId ? <div className="aiws-ctl-form-body"><RichTextCell entity={body} entityId={bodyId} renderEmbed={() => null} /></div>
          : <button type="button" className="aiws-dc-ghostbtn" disabled={disabled} title={disabled ? noWrite : undefined} onClick={() => void addNotes()}>{t('aiworkspace.todo.addNotes')}</button>}
      </div>
      {item.done_at && <div className="aiws-dc-note">{t('aiworkspace.todo.doneAt', { at: new Date(item.done_at).toLocaleString() })}</div>}
      <div className="aiws-ctl-form-actions">
        <button type="button" className="aiws-danger-link" disabled={disabled} title={disabled ? noWrite : undefined} onClick={onDelete}><Trash2 size={14} aria-hidden="true" /> {t('aiworkspace.common.delete')}</button>
      </div>
    </div>
  )
}

/** The record's body (rich text), read from the records the control already follows. */
function useBodyId(sourceId: string, recordId: string): string | null {
  const store = useStore()
  const set = recordSet(store, sourceId)
  const row = set.snapshot().rows.find((r) => r.record_id === recordId)
  return row?.body_ref?.entity_id ?? null
}
