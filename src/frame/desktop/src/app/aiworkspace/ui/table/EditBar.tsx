/* The phone grid's edit bar (UI 框架 PRD §7.3): the second tap on a cell edits its value in a bar at the bottom,
 * above the keyboard — the control of the value's type, done, cancel and comment. Leaving it writes the value like the
 * record form does. */

import { useEffect, useState } from 'react'
import { createPortal } from 'react-dom'
import { Check, MessageSquarePlus, X } from 'lucide-react'
import type { CapturedAnchor } from '../../anchors/registry'
import type { FieldDef, QueryRow } from '../../api/types'
import { useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import type { TablePager } from '../tablePager'
import { UNSET, type Input } from '../values'
import { cellAnchor, cellEditId, writeOperation } from './model'
import { EditState, ValueControl, useLiveValue, type FormField } from './RecordForm'

/** How much of the layout viewport the on-screen keyboard covers (the visual viewport is smaller by that much). */
function useKeyboardInset(): number {
  const [inset, setInset] = useState(0)
  useEffect(() => {
    const vv = window.visualViewport
    if (!vv) return
    const update = () => setInset(Math.max(0, Math.round(window.innerHeight - vv.height - vv.offsetTop)))
    vv.addEventListener('resize', update)
    vv.addEventListener('scroll', update)
    return () => { vv.removeEventListener('resize', update); vv.removeEventListener('scroll', update) }
  }, [])
  return inset
}

export function PhoneEditBar({ host, sourceId, field, row, pager, onClose, onAnnotate }: {
  host: HTMLElement | null
  sourceId: string
  field: FieldDef
  row: QueryRow
  pager: TablePager
  onClose: () => void
  onAnnotate?: (anchor: CapturedAnchor) => void
}) {
  const { t } = useAiwsText()
  const store = useStore()
  const inset = useKeyboardInset()
  const form: FormField = {
    id: field.field_id, name: field.name, def: field, value: row.values[field.field_id], rev: row.revs[field.field_id] ?? 0,
    editId: cellEditId(sourceId, row.record_id, field.field_id), locked: null, testId: 'aiws-table-edit-bar-field',
  }
  const write = async (_: FormField, input: Input, baseRev: number) => {
    const outcome = await store.submit({
      editId: form.editId, label: `${field.name} · ${row.record_id}`, mine: input === UNSET ? null : input, hasMine: true,
      operations: [writeOperation(sourceId, field, row.record_id, input, baseRev)],
    })
    if (outcome.status === 'accepted') pager.patchLocal(row.record_id, field.field_id, input === UNSET ? undefined : input, outcome.seq)
  }
  const live = useLiveValue(form, write)
  const done = () => { if (live.commit()) onClose() }
  const target = host ?? (document.querySelector('.aiws-root') as HTMLElement | null) ?? document.body
  return createPortal(
    <div className="aiws-table-edit-bar" role="dialog" aria-label={t('aiworkspace.table.editBar.label', { field: field.name })} data-testid="aiws-table-edit-bar" style={inset ? { bottom: inset } : undefined}
      onPointerDown={(event) => event.stopPropagation()}>
      <div className="aiws-table-edit-bar-head">
        <span className="aiws-table-edit-bar-name">{field.name}</span>
        {live.entry && <EditState field={form} live={live} write={write} />}
        <span className="aiws-grow" />
        {onAnnotate && (
          <button type="button" className="aiws-table-btn" aria-label={t('aiworkspace.table.cell.commentAria', { field: field.name, record: row.record_id })} title={t('aiworkspace.table.cell.comment')} data-testid="aiws-table-edit-bar-comment"
            onPointerDown={(event) => event.preventDefault()} onClick={() => { live.cancel(); onClose(); onAnnotate(cellAnchor(sourceId, field, row)) }}><MessageSquarePlus size={18} aria-hidden="true" /></button>
        )}
        {/* the press keeps the focus in the control, so leaving it does not write what is being cancelled */}
        <button type="button" className="aiws-table-btn" aria-label={t('aiworkspace.common.cancel')} title={t('aiworkspace.common.cancel')} data-testid="aiws-table-edit-bar-cancel"
          onPointerDown={(event) => event.preventDefault()} onMouseDown={(event) => event.preventDefault()} onClick={() => { live.cancel(); onClose() }}><X size={18} aria-hidden="true" /></button>
        <button type="button" className="aiws-table-btn is-primary" aria-label={t('aiworkspace.table.editBar.done')} title={t('aiworkspace.table.editBar.done')} data-testid="aiws-table-edit-bar-done"
          onPointerDown={(event) => event.preventDefault()} onMouseDown={(event) => event.preventDefault()} onClick={done}><Check size={18} aria-hidden="true" /></button>
      </div>
      <div className="aiws-table-edit-bar-control">
        <ValueControl def={field} raw={live.raw} ariaLabel={`${field.name} ${row.record_id}`} autoFocus onFocus={live.begin} onChange={live.change}
          onCommit={(raw) => { if (live.commit(raw) && (field.type === 'select' || field.type === 'boolean')) onClose() }} onCancel={() => { live.cancel(); onClose(); return true }} />
      </div>
      {live.error && <div className="aiws-error" role="alert">{live.error}</div>}
    </div>,
    target,
  )
}
