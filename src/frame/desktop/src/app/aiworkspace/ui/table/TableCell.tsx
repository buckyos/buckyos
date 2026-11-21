import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent, type RefObject } from 'react'
import { ExternalLink, MessageSquare, MessageSquarePlus } from 'lucide-react'
import type { TableGridProps } from './TableGrid'
import { ReferenceDisplay } from './TableReference'
import type { FieldDef, QueryRow } from '../../api/types'
import { contactRecordHref } from '../../links'
import { EDIT_STATE_LABEL } from '../../state/edits'
import { useEdit, useStore, type AnnotationMark } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { asFormula, formulaResult } from '../formula'
import { ConflictBox } from './ConflictBox'
import { ValueEditor } from '../ValueEditor'
import { UNSET, emptyInput, formatValue, type Input } from '../values'
import { cellAnchor, cellEditId, writeOperation } from './model'
import { CellDisplay } from './RecordForm'

const ARROWS: Record<string, [number, number]> = { ArrowUp: [-1, 0], ArrowDown: [1, 0], ArrowLeft: [0, -1], ArrowRight: [0, 1] }

interface TableCellProps extends TableGridProps {
  field: FieldDef
  row: QueryRow
  rowIndex: number
  colIndex: number
  width: number
  /** Kept in place while the rest scrolls sideways (a phone's first column): its left offset. */
  sticky: number | null
  onNavigate: (row: number, col: number) => void
  onTab: (row: number, col: number, back: boolean) => void
  onExit: () => void
  activeColumn: boolean
  resizeColumn?: (e: PointerEvent<HTMLElement>) => void
  annotation?: AnnotationMark
}

export function TableCell({ showHandles, annotatedRecords, pager, sourceId, field, row, rowIndex, colIndex, onNavigate, onTab, onExit, activeColumn, resizeColumn, compact, activateCells, width, sticky, editable: canEdit, resultTable, fields, annotation, onAnnotate, onActivateAnnotation, phone, phoneCell, onPhoneCell, onPhoneEdit }: TableCellProps) {
  const { t } = useAiwsText()
  const store = useStore()
  const editId = cellEditId(sourceId, row.record_id, field.field_id)
  const entry = useEdit(editId)
  // The rev the user saw when the edit began: a change that arrives while typing must conflict, not be overwritten.
  const [editing, setEditing] = useState<{ baseRev: number; text?: string } | null>(null)
  /** A tick box shows the click at once; the write follows (and the stored value takes over when it lands). */
  const [ticked, setTicked] = useState<boolean | null>(null)
  const value = row.values[field.field_id]
  const meta = row.meta?.[field.field_id]
  const pending = entry && entry.state !== 'committed' && entry.hasMine
  // When the inline editor closes by Enter/Escape focus would fall to <body>: give it back to the cell.
  const valueRef = useRef<HTMLButtonElement | HTMLInputElement | HTMLDivElement>(null)
  const wasEditing = useRef(false)
  useEffect(() => {
    if (editing) { wasEditing.current = true; return }
    if (!wasEditing.current) return
    wasEditing.current = false
    if (document.activeElement === document.body || document.activeElement === null) valueRef.current?.focus()
  }, [editing])

  const write = async (input: Input, baseRev: number) => {
    const label = `${field.name} · ${row.record_id}`
    const outcome = await store.submit({
      editId, label, mine: input === UNSET ? null : input, hasMine: true,
      operations: [writeOperation(sourceId, field, row.record_id, input, baseRev)],
    })
    if (outcome.status === 'accepted') pager.patchLocal(row.record_id, field.field_id, input === UNSET ? undefined : input, outcome.seq)
    return outcome.status === 'accepted' || outcome.status === 'saved_locally'
  }

  const mine = entry?.mine === null ? emptyInput(field) : entry?.mine
  const current = pending ? (mine === UNSET ? undefined : mine) : value
  // a formula (D18) is shown evaluated over this row; the kernel keeps the formula itself, "materialize" is in the record's ⋯
  const formula = !pending ? asFormula(value) : null
  const formulaValue = formula ? formulaResult(formula, row.values, fields) : null
  const allowsInput = !formula && !field.submitter && (field.type !== 'object_ref' || Boolean(field.ref_purpose))
  const editable = canEdit && allowsInput
  const disabled = !(editable || (activateCells && allowsInput))
  const shown = pending ? formatValue(field, current) : formula ? (formulaValue === null ? `= ${formula.$formula.expr}` : String(formulaValue)) : formatValue(field, value)
  const selectedOnPhone = phone && phoneCell?.recordId === row.record_id && phoneCell.fieldId === field.field_id
  const startEdit = (text?: string) => {
    if (!editable || editing) return
    setEditing({ baseRev: row.revs[field.field_id] ?? 0, ...(text !== undefined ? { text } : {}) })
  }
  const tap = () => {
    if (!editable) return
    if (!phone) { startEdit(); return }
    const cell = { recordId: row.record_id, fieldId: field.field_id }
    onPhoneCell(cell)
    onPhoneEdit(cell)
  }
  // arrows move between cells; a typed character starts editing with it (a spreadsheet's habit, 流式页交互改进 §5.3)
  const onCellKey = (event: ReactKeyboardEvent<HTMLElement>) => {
    const arrow = ARROWS[event.key]
    if (arrow && !event.altKey && !event.ctrlKey && !event.metaKey) { event.preventDefault(); onNavigate(rowIndex + arrow[0], colIndex + arrow[1]); return }
    if (event.key === 'Tab' && !event.ctrlKey && !event.metaKey && !event.altKey) {
      event.preventDefault(); event.stopPropagation()
      if (!event.repeat) onTab(rowIndex, colIndex, event.shiftKey)
      return
    }
    if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); onExit(); return }
    if ((event.key === 'Backspace' || event.key === 'Delete') && editable && value !== undefined) {
      event.preventDefault()
      if (field.required && !field.nullable) store.notify('error', t('aiworkspace.data.record.required', { name: field.name }))
      else void write(emptyInput(field), row.revs[field.field_id] ?? 0)
      return
    }
    if (event.key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey && !event.nativeEvent.isComposing && editable
      && field.type !== 'boolean' && field.type !== 'select' && field.type !== 'multi_select' && field.type !== 'object_ref' && field.type !== 'date' && field.type !== 'datetime') {
      event.preventDefault()
      startEdit(event.key)
    }
  }
  const href = contactRecordHref(field.field_id, current)
  const cellAttrs = { 'data-row': rowIndex, 'data-col': colIndex, tabIndex: compact ? -1 : 0, 'aria-label': `${field.name} ${row.record_id}` }

  return (
    <div
      className={`aiws-td${sticky !== null ? ' is-sticky' : ''}${selectedOnPhone ? ' is-picked' : ''}${activeColumn ? ' is-selected-column' : ''}`}
      role="gridcell"
      data-record-id={row.record_id} data-field-id={field.field_id} aria-colindex={colIndex + 1} aria-label={field.name}
      style={{ width, ...(sticky !== null ? { left: sticky } : {}) }}
      data-testid={`aiws-cell-${row.record_id}-${field.field_id}`}
      data-state={entry?.state ?? 'clean'}
      data-editable={editable ? 'true' : 'false'}
      data-type={field.type}
      aria-selected={phone ? selectedOnPhone : undefined}
      onClick={(event) => { if (event.target === event.currentTarget && editable) valueRef.current?.click() }}
    >
      {editing ? (
        <ValueEditor
          def={field}
          value={value}
          initialText={editing.text}
          ariaLabel={`${field.name} ${row.record_id}`}
          onCancel={() => setEditing(null)}
          onCommit={async (input) => { const ok = await write(input, editing.baseRev); setEditing(null); return ok }}
          onTab={(back) => onTab(rowIndex, colIndex, back)}
        />
      ) : field.type === 'boolean' && !formula ? (
        <label className="aiws-cell-check">
          <input ref={valueRef as RefObject<HTMLInputElement>} type="checkbox" checked={ticked ?? current === true} aria-disabled={disabled} {...cellAttrs}
            onChange={(event) => { if (!editable) return; const next = event.target.checked; setTicked(next); void write(next, row.revs[field.field_id] ?? 0).finally(() => setTicked(null)) }} onKeyDown={onCellKey} />
        </label>
      ) : field.ref_purpose && !formula ? (
        <div ref={valueRef as RefObject<HTMLDivElement>} role="button" aria-disabled={disabled} className="aiws-cell-value aiws-ref-value" onClick={tap}
          onKeyDown={(event) => { if (event.target !== event.currentTarget) return; if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); tap() } else onCellKey(event) }} {...cellAttrs}><ReferenceDisplay field={field} value={current} /></div>
      ) : href && !editable ? (
        <a href={href} target="_blank" rel="noreferrer" className="aiws-cell-value" onKeyDown={onCellKey} {...cellAttrs}>{shown}</a>
      ) : (
        <button ref={valueRef as RefObject<HTMLButtonElement>} type="button" className="aiws-cell-value" aria-disabled={disabled} onClick={tap} onKeyDown={onCellKey} title={formula ? t('aiworkspace.table.cell.formulaTitle', { formula: formula.$formula.expr }) : editable ? shown || undefined : undefined} {...cellAttrs}>
          <CellDisplay def={field} value={formula ? undefined : current} shown={shown} />
        </button>
      )}
      {resizeColumn && !editing && <span className="aiws-grid-resize is-column" aria-hidden="true" onPointerDown={resizeColumn} />}
      {!editing && href && editable && <a href={href} target="_blank" rel="noreferrer" aria-label={t('aiworkspace.blockui.openLink')}><ExternalLink size={14} /></a>}
      {!editing && formula && <span className="aiws-chip" data-testid="aiws-cell-formula" title={t('aiworkspace.table.cell.formulaTitle', { formula: formula.$formula.expr })}>{t('aiworkspace.table.record.formula')}</span>}
      {!editing && meta?.derived && !resultTable && (
        <span className="aiws-chip aiws-chip-derived" data-testid="aiws-derived" title={t('aiworkspace.data.cell.derivedBy', { program: meta.derived.program ?? t('aiworkspace.data.cell.processingProgram') })}>
          {t('aiworkspace.data.cell.derived')}
        </span>
      )}
      {!editing && meta?.manual_override && <span className="aiws-chip aiws-chip-warn" data-testid="aiws-manual-override" title={t('aiworkspace.data.cell.manualOverrideTitle')}>{t('aiworkspace.data.cell.manualOverride')}</span>}
      {!editing && annotation && (
        <button type="button" className="aiws-chip aiws-chip-note" title={annotation.payload.body} data-testid="aiws-cell-annotation"
          {...{ [`data-anno-${annotation.entityId}`]: '' }} onClick={() => onActivateAnnotation?.(annotation.entityId)}>{t('aiworkspace.data.cell.annotation')}</button>
      )}
      {!editing && onAnnotate && !annotation && !phone && (
        <button type="button" className="aiws-cell-annotate" title={t('aiworkspace.table.cell.comment')} aria-label={t('aiworkspace.table.cell.commentAria', { field: field.name, record: row.record_id })}
          onClick={() => onAnnotate(cellAnchor(sourceId, field, row))}><MessageSquarePlus size={14} aria-hidden="true" /></button>
      )}
      {entry && !editing && (
        <span className={`aiws-state aiws-state-${entry.state}`} data-testid="aiws-edit-state" title={entry.detail}>{EDIT_STATE_LABEL[entry.state]}</span>
      )}
      {entry && entry.state === 'needs_attention' && !editing && editable && (
        <ConflictBox
          store={store}
          editId={editId}
          theirs={entry.hasTheirs ? formatValue(field, entry.theirs) || t('aiworkspace.data.value.notSet') : null}
          mine={entry.hasMine ? (mine === UNSET ? t('aiworkspace.data.value.cleared') : formatValue(field, mine)) : null}
          detail={entry.detail ?? ''}
          onKeepMine={entry.hasMine && entry.currentRev !== undefined ? () => { void write(mine === UNSET || mine === undefined ? UNSET : mine, entry.currentRev ?? 0) } : undefined}
          onEditAgain={entry.hasMine ? () => {
            // Start a new edit from my kept input, based on what is current now.
            const mineText = entry.mine === null || entry.mine === undefined ? '' : typeof entry.mine === 'object' ? '' : String(entry.mine)
            setEditing({ baseRev: entry.currentRev ?? row.revs[field.field_id] ?? 0, text: mineText })
          } : undefined}
        />
      )}
      {entry && entry.state === 'unsaved' && entry.unknownKey && !editing && (
        <button type="button" className="aiws-link" onClick={() => {
          const input: Input = mine === undefined ? UNSET : mine
          void store.retry({ editId, label: entry.label, mine: entry.mine, hasMine: true, operations: [writeOperation(sourceId, field, row.record_id, input, 0)] }, entry.unknownKey ?? '')
        }}>{t('aiworkspace.common.retry')}</button>
      )}
      {!editing && showHandles && colIndex === 0 && annotatedRecords.has(row.record_id) && (
        <MessageSquare size={13} className="aiws-table-record-note-icon" aria-label={t('aiworkspace.table.record.hasComments')} />
      )}
    </div>
  )
}
