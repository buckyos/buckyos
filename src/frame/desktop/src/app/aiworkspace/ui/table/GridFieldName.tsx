import { useRef, useState } from 'react'
import type { FieldDef } from '../../api/types'
import { useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'

export function GridFieldName({ field, fields, sourceId, editable }: { field: FieldDef; fields: FieldDef[]; sourceId: string; editable: boolean }) {
  const { t } = useAiwsText()
  const store = useStore()
  const [draft, setDraft] = useState<{ text: string; original: string; rev: number } | null>(null)
  const [error, setError] = useState('')
  const input = useRef<HTMLInputElement>(null)
  const settled = useRef(false)
  const edit = () => { settled.current = false; setError(''); setDraft({ text: field.name, original: field.name, rev: field.def_rev }) }
  const cancel = () => { settled.current = true; setDraft(null); setError('') }
  const fail = (message: string) => { setError(message); input.current?.focus({ preventScroll: true }) }
  const commit = async () => {
    if (!draft || settled.current) return
    const name = draft.text.trim()
    if (!name) { fail(t('aiworkspace.table.column.emptyName')); return }
    if (name === draft.original) { cancel(); return }
    if (fields.some((f) => f.field_id !== field.field_id && f.name === name)) { fail(t('aiworkspace.table.column.duplicateName')); return }
    settled.current = true
    const id = `rename:${sourceId}:${field.field_id}`
    const outcome = await store.submit({ editId: id, label: t('aiworkspace.data.field.renameEdit', { from: draft.original, to: name }), operations: [
      { op: 'table.update_field', source_id: sourceId, field_id: field.field_id, changes: { name }, expect: { rev: draft.rev } },
    ] })
    if (outcome.status === 'accepted' || outcome.status === 'saved_locally') { cancel(); return }
    settled.current = false
    fail(store.edits.get(id)?.detail ?? t('aiworkspace.table.column.renameFailed'))
    if (outcome.status === 'conflict') setDraft({ ...draft, rev: outcome.conflicts[0]?.current_rev ?? draft.rev })
  }
  if (!editable) return <span className="aiws-th-name">{field.name}</span>
  if (!draft) return <button type="button" className="aiws-th-name" data-field-name={field.field_id} onClick={edit}>{field.name}</button>
  return (
    <span className="aiws-grid-rename">
      <input ref={input} autoFocus aria-label={t('aiworkspace.data.field.nameAria', { id: field.field_id })} aria-invalid={Boolean(error)}
        value={draft.text} onFocus={(e) => e.currentTarget.select()} onChange={(e) => { setDraft({ ...draft, text: e.target.value }); setError('') }}
        onBlur={() => void commit()} onKeyDown={(e) => {
          if (e.nativeEvent.isComposing) return
          if (e.key === 'Enter' || e.key === 'Escape') {
            e.preventDefault(); e.stopPropagation()
            if (e.key === 'Enter') void commit()
            else cancel()
          }
        }} />
      {error && <span role="alert" className="aiws-error">{error}</span>}
    </span>
  )
}
