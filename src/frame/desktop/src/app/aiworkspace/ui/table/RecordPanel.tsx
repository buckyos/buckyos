/* One record of a table in a panel (UI 框架 PRD §7.3): a side panel on a computer, the whole screen on a phone. The
 * record form, the previous / next record of the view, and the record's ⋯ — comment, copy its link, materialize a
 * formula, delete (asked first). A new record's empty form adds it in one Commit, or adds it and starts the next. */

import { useCallback, useEffect, useState, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { ChevronDown, ChevronLeft, ChevronUp, Link2, MessageSquarePlus, MoreHorizontal, Sigma, Trash2, X } from 'lucide-react'
import { randomId } from '../../api/ids'
import type { ReadOk } from '../../api/session'
import type { FieldDef, Json, QueryRow } from '../../api/types'
import { useLoad, useStore, useVersion, useWorkspaceUi } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { copyRecordLink, recordAnchor } from '../controls/data/focus'
import { asFormula, formulaResult } from '../formula'
import { useOverlayMounted } from '../shell/popover'
import { usePhone } from '../shell/shellContext'
import type { TablePager } from '../tablePager'
import { UNSET, type Input } from '../values'
import { cellEditId, recordTitle, writeOperation } from './model'
import { DraftRecordForm, RecordForm, useDraftRecord, type DraftFieldDef, type FormField } from './RecordForm'
import { ConfirmPop, TableMenu, usePop, type MenuItem } from './TablePop'

export type RecordPanelState = { kind: 'edit'; recordId: string } | { kind: 'new' }

export interface TableRecordPanelProps {
  /** The workspace root the panel is placed in. */
  host: HTMLElement | null
  sourceId: string
  /** The table-view Block, when there is one: a record's link names it. */
  cellId: string | null
  fields: FieldDef[]
  titleField: FieldDef | undefined
  state: RecordPanelState
  rows: readonly QueryRow[]
  pager: TablePager
  canUpdate: boolean
  canAppend: boolean
  canDelete: boolean
  /** Why nothing can be edited here (shown in the panel), or null. */
  readOnlyReason: string | null
  onOpen: (recordId: string) => void
  onClose: () => void
}

export function TableRecordPanel(props: TableRecordPanelProps) {
  const phone = usePhone()
  useOverlayMounted(phone)
  const { onClose } = props
  const host = props.host ?? (document.querySelector('.aiws-root') as HTMLElement | null) ?? document.body
  const body = props.state.kind === 'new' ? <NewRecord {...props} /> : <EditRecord {...props} recordId={props.state.recordId} />
  return createPortal(
    <div className={`aiws-table-record${phone ? ' is-phone' : ''}`} role="dialog" aria-modal={phone ? 'true' : undefined} data-testid="aiws-table-record-panel"
      data-mode={props.state.kind} data-record-id={props.state.kind === 'edit' ? props.state.recordId : undefined}
      onKeyDown={(event) => { if (event.key === 'Escape' && !event.defaultPrevented) { event.stopPropagation(); onClose() } }}
      onPointerDown={(event) => event.stopPropagation()}>
      {body}
    </div>,
    host,
  )
}

function PanelHead({ title, onClose, children }: { title: string; onClose: () => void; children?: ReactNode }) {
  const { t } = useAiwsText()
  const phone = usePhone()
  return (
    <div className="aiws-table-record-head">
      {phone && <button type="button" className="aiws-table-btn" aria-label={t('aiworkspace.table.record.back')} title={t('aiworkspace.table.record.back')} data-testid="aiws-table-record-close" onClick={onClose}><ChevronLeft size={20} aria-hidden="true" /></button>}
      <b className="aiws-table-record-title" data-testid="aiws-table-record-title">{title}</b>
      {children}
      {!phone && <button type="button" className="aiws-table-btn" aria-label={t('aiworkspace.common.close')} title={t('aiworkspace.common.close')} data-testid="aiws-table-record-close" onClick={onClose}><X size={18} aria-hidden="true" /></button>}
    </div>
  )
}

function EditRecord({ recordId, sourceId, cellId, fields, titleField, rows, pager, canUpdate, canDelete, readOnlyReason, onOpen, onClose }: TableRecordPanelProps & { recordId: string }) {
  const { t } = useAiwsText()
  const store = useStore()
  const ui = useWorkspaceUi()
  const index = rows.findIndex((row) => row.record_id === recordId)
  const listed = index >= 0 ? rows[index] : undefined
  // a record outside the loaded rows (a link, a record the filter no longer shows): read on its own
  const version = useVersion(`e:${sourceId}`)
  const load = useCallback(() => (listed ? Promise.resolve(null) : store.session.read<QueryRow>(sourceId, { kind: 'table_record', record_id: recordId })), [listed, store, sourceId, recordId])
  const alone = useLoad<ReadOk<QueryRow> | null>(load, listed ? 0 : version)
  const row: QueryRow | undefined = listed ?? (alone.data ? { ...alone.data.content, record_id: recordId } : undefined)
  useEffect(() => { if (index >= 0) pager.ensure(index + 2) }, [pager, index])
  const pops = usePop<'menu' | 'delete'>()
  const title = recordTitle(titleField, row, t('aiworkspace.table.record.untitled'))

  const write = async (field: FormField, input: Input, baseRev: number) => {
    const def = fields.find((item) => item.field_id === field.id)
    if (!def) return
    const outcome = await store.submit({
      editId: field.editId, label: `${def.name} · ${recordId}`, mine: input === UNSET ? null : input, hasMine: true,
      operations: [writeOperation(sourceId, def, recordId, input, baseRev)],
    })
    if (outcome.status === 'accepted') pager.patchLocal(recordId, def.field_id, input === UNSET ? undefined : input, outcome.seq)
  }
  const formulas = row ? fields.flatMap((field) => { const formula = asFormula(row.values[field.field_id]); return formula ? [{ field, formula, value: formulaResult(formula, row.values, fields) }] : [] }) : []
  const formFields: FormField[] = row ? fields.map((field) => {
    const value = row.values[field.field_id]
    const formula = formulas.find((item) => item.field.field_id === field.field_id)
    return {
      id: field.field_id, name: field.name, def: field, value, rev: row.revs[field.field_id] ?? 0,
      editId: cellEditId(sourceId, recordId, field.field_id), testId: `aiws-record-field-${field.field_id}`,
      locked: !canUpdate ? (readOnlyReason ?? '') : field.type === 'object_ref' ? '' : null,
      ...(formula ? { display: <span title={t('aiworkspace.data.cell.formulaTitle', { formula: formula.formula.$formula.expr })}>{formula.value === null ? `= ${formula.formula.$formula.expr}` : String(formula.value)} <span className="aiws-chip">{t('aiworkspace.table.record.formula')}</span></span> } : {}),
    }
  }) : []

  const materialize = (item: (typeof formulas)[number]) => {
    if (!row || item.value === null) return
    const editId = cellEditId(sourceId, recordId, item.field.field_id)
    void store.submit({
      editId, label: t('aiworkspace.data.cell.materializeLabel', { field: item.field.name, record: recordId }), mine: item.value, hasMine: true,
      operations: [{ op: 'table.set_values', source_id: sourceId, values: [{ record_id: recordId, field_id: item.field.field_id, value: item.value, expect: { rev: row.revs[item.field.field_id] ?? 0 } }], derived: { program: 'formula', formula: item.formula.$formula.expr } }],
    }).then((outcome) => { if (outcome.status === 'accepted') pager.patchLocal(recordId, item.field.field_id, item.value as Json, outcome.seq) })
  }
  const remove = async () => {
    if (!row) return
    const next = rows[index + 1]?.record_id ?? rows[index - 1]?.record_id ?? null
    const outcome = await store.submit({
      editId: `record:${sourceId}:${recordId}`, label: t('aiworkspace.data.record.deleteNamed', { id: title }),
      operations: [{ op: 'table.delete_records', source_id: sourceId, records: [{ record_id: recordId, expect: { rev: row.rev } }] }],
    })
    if (outcome.status === 'accepted' || outcome.status === 'saved_locally') { if (next) onOpen(next); else onClose() }
  }
  const menu: (MenuItem | 'separator')[] = [
    { id: 'record-comment', label: t('aiworkspace.table.record.comment'), icon: <MessageSquarePlus size={14} />, disabled: ui.annotate ? false : t('aiworkspace.table.record.noComment'),
      run: () => ui.annotate?.(recordAnchor(sourceId, recordId, title)) },
    { id: 'record-link', label: t('aiworkspace.table.record.copyLink'), icon: <Link2 size={14} />, disabled: cellId ? false : t('aiworkspace.table.record.noLink'),
      run: () => { if (cellId) copyRecordLink(store, cellId, recordId, null, t('aiworkspace.datactl.linkCopied')) } },
    ...formulas.map((item): MenuItem => ({ id: `record-materialize-${item.field.field_id}`, label: t('aiworkspace.table.record.materialize', { field: item.field.name }), icon: <Sigma size={14} />,
      disabled: !canUpdate ? (readOnlyReason ?? t('aiworkspace.table.readOnly.noUpdate')) : item.value === null ? t('aiworkspace.table.record.formulaInvalid') : false, run: () => materialize(item) })),
    ...(canDelete ? ['separator' as const, { id: 'record-delete', label: t('aiworkspace.data.record.delete'), icon: <Trash2 size={14} />, danger: true, run: () => pops.openAt('delete', pops.pop?.anchor ?? { left: window.innerWidth / 2, top: window.innerHeight / 2, width: 0, height: 0 }) }] : []),
  ]
  const prev = index > 0 ? rows[index - 1] : undefined
  const next = index >= 0 ? rows[index + 1] : undefined
  return (
    <>
      <PanelHead title={title} onClose={onClose}>
        <button type="button" className="aiws-table-btn" aria-label={t('aiworkspace.table.record.prev')} title={t('aiworkspace.table.record.prev')} data-testid="aiws-table-record-prev" disabled={!prev} onClick={() => prev && onOpen(prev.record_id)}><ChevronUp size={18} aria-hidden="true" /></button>
        <button type="button" className="aiws-table-btn" aria-label={t('aiworkspace.table.record.next')} title={t('aiworkspace.table.record.next')} data-testid="aiws-table-record-next" disabled={!next} onClick={() => next && onOpen(next.record_id)}><ChevronDown size={18} aria-hidden="true" /></button>
        <button type="button" className="aiws-table-btn" aria-label={t('aiworkspace.table.record.more')} title={t('aiworkspace.table.record.more')} aria-haspopup="menu" data-testid="aiws-table-record-more" disabled={!row}
          onClick={(event) => pops.open('menu', event.currentTarget)}><MoreHorizontal size={18} aria-hidden="true" /></button>
      </PanelHead>
      <div className="aiws-table-record-body">
        {readOnlyReason && <div className="aiws-muted aiws-table-record-note" data-testid="aiws-table-record-readonly">{readOnlyReason}</div>}
        {row ? <RecordForm key={recordId} fields={formFields} write={write} testId={`aiws-record-form-${recordId}`} />
          : alone.error ? <div className="aiws-error" role="alert">{t('aiworkspace.table.record.readFailed', { error: alone.error })}</div>
            : <div className="aiws-muted">{t('aiworkspace.blocks.loading')}</div>}
      </div>
      {pops.pop?.kind === 'menu' && <TableMenu anchor={pops.pop.anchor} onClose={pops.close} label={t('aiworkspace.table.record.more')} items={menu} testId="aiws-table-record-menu" />}
      {pops.pop?.kind === 'delete' && (
        <ConfirmPop anchor={pops.pop.anchor} title={t('aiworkspace.table.record.deleteAsk', { name: title })} detail={t('aiworkspace.table.undoHint')} confirm={t('aiworkspace.common.delete')}
          onConfirm={() => { void remove() }} onClose={pops.close} />
      )}
    </>
  )
}

function NewRecord({ sourceId, fields, canAppend, readOnlyReason, onClose }: TableRecordPanelProps) {
  const { t } = useAiwsText()
  const store = useStore()
  // the service fills a submitter field; references and JSON objects are not typed into a new record
  const asked: DraftFieldDef[] = fields.filter((field) => !field.submitter && field.type !== 'object_ref')
    .map((field) => ({ id: field.field_id, name: field.name, def: field, testId: `aiws-record-field-${field.field_id}` }))
  const draft = useDraftRecord(asked)
  const [status, setStatus] = useState<{ kind: 'error' | 'info'; text: string } | null>(null)
  const [round, setRound] = useState(0)
  const add = async (keepOpen: boolean) => {
    const values = draft.build()
    if (!values) { setStatus(null); return }
    const typeRevs: Record<string, number> = {}
    for (const id of Object.keys(values)) { const field = fields.find((item) => item.field_id === id); if (field) typeRevs[id] = field.type_rev }
    const recordId = randomId('r')
    const outcome = await store.submit({
      editId: `record:${sourceId}:${recordId}`, label: t('aiworkspace.data.record.add'), mine: values, hasMine: true,
      operations: [{ op: 'table.insert_records', source_id: sourceId, field_type_revs: typeRevs, records: [{ record_id: recordId, values }] }],
    })
    if (outcome.status !== 'accepted' && outcome.status !== 'saved_locally') { setStatus({ kind: 'error', text: t('aiworkspace.data.record.addFailed') }); return }
    if (!keepOpen) { onClose(); return }
    draft.reset()
    setRound((n) => n + 1)
    setStatus({ kind: 'info', text: t('aiworkspace.table.record.added') })
  }
  return (
    <>
      <PanelHead title={t('aiworkspace.table.record.newTitle')} onClose={onClose} />
      <form className="aiws-table-record-body" onSubmit={(event) => { event.preventDefault(); void add(false) }}>
        {!canAppend && <div className="aiws-muted aiws-table-record-note">{readOnlyReason ?? t('aiworkspace.table.readOnly.noAppend')}</div>}
        {asked.length === 0 && <div className="aiws-muted">{t('aiworkspace.data.record.noRequired')}</div>}
        <DraftRecordForm key={round} fields={asked} draft={draft} ariaLabel={(name) => t('aiworkspace.data.record.newAria', { name })} />
        {status && <div className={status.kind === 'error' ? 'aiws-error' : 'aiws-muted'} role={status.kind === 'error' ? 'alert' : 'status'} data-testid="aiws-table-record-status">{status.text}</div>}
        <div className="aiws-table-record-actions">
          <button type="button" onClick={onClose}>{t('aiworkspace.common.cancel')}</button>
          <button type="button" data-testid="aiws-table-record-add-next" disabled={!canAppend} onClick={() => { void add(true) }}>{t('aiworkspace.table.record.addNext')}</button>
          <button type="submit" className="is-primary" data-testid="aiws-table-record-add" disabled={!canAppend}>{t('aiworkspace.table.record.add')}</button>
        </div>
      </form>
    </>
  )
}
