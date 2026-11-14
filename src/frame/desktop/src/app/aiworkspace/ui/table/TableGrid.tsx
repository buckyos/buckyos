/* The grid of the table editor (UI 框架 PRD §7.3): virtualised rows, the head of each row (tick it to select, open
 * the record's form), cells edited in place. On a computer a click edits a cell, arrows and Tab move between cells,
 * column edges drag to resize and headers drag to reorder. On a phone the head and the first column stay put while the
 * rest scrolls sideways; a tap selects a cell and a second tap edits it in the bar at the bottom. */

import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent, type RefObject } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'
import { Maximize2, MessageSquare, Plus, Type } from 'lucide-react'
import type { FieldDef, Json, QueryRow } from '../../api/types'
import type { CapturedAnchor } from '../../anchors/registry'
import { EDIT_STATE_LABEL } from '../../state/edits'
import { useEdit, useStore, type AnnotationMark } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { asFormula, formulaResult } from '../formula'
import type { PagerSnapshot, TablePager } from '../tablePager'
import { ConflictBox } from './ConflictBox'
import { FIELD_ICONS } from './fieldIcons'
import { ValueEditor } from '../ValueEditor'
import { FIELD_TYPE_LABEL, UNSET, formatValue, type Input } from '../values'
import { MAX_WIDTH, MIN_WIDTH, WIDTHS, cellAnchor, cellEditId, writeOperation, type Column } from './model'
import { CellDisplay } from './RecordForm'
import { usePress } from './usePress'

export const ROW_HEIGHT = 34
const PHONE_ROW_HEIGHT = 44
const ADD_COLUMN = 40

export interface CellRef { recordId: string; fieldId: string }

export interface TableGridProps {
  tableId: string
  sourceId: string
  fields: FieldDef[]
  columns: Column[]
  rows: PagerSnapshot
  pager: TablePager
  scrollRef: RefObject<HTMLDivElement | null>
  /** A bounded box (px), or null: fill the editor's height. */
  maxHeight: number | null
  compact: boolean
  phone: boolean
  editable: boolean
  resultTable: boolean
  titleField: FieldDef | undefined
  annotations: AnnotationMark[]
  onAnnotate?: (anchor: CapturedAnchor) => void
  onActivateAnnotation?: (entityId: string | null) => void
  /** The head of each row: tick, open, a comment mark. */
  rowHead: boolean
  selection: ReadonlySet<string> | null
  onToggle: (recordId: string, on: boolean) => void
  onToggleAll: (on: boolean) => void
  onStartSelect: (recordId: string) => void
  onOpen: (recordId: string) => void
  annotatedRecords: ReadonlySet<string>
  phoneCell: CellRef | null
  onPhoneCell: (cell: CellRef | null) => void
  onPhoneEdit: (cell: CellRef) => void
  columnsEditable: boolean
  onResize: (fieldId: string, width: number) => void
  onMove: (fieldId: string, to: number) => void
  /** The header's "+" (a field by name and type), or null. */
  onAddField: ((button: HTMLElement) => void) | null
  addFieldOpen: boolean
}

export function TableGrid(props: TableGridProps) {
  const { t } = useAiwsText()
  const { tableId, columns, rows, pager, scrollRef, phone, compact, rowHead, selection } = props
  const rowHeight = phone && !compact ? PHONE_ROW_HEIGHT : ROW_HEIGHT
  const headWidth = rowHead ? (phone ? 44 : 56) : 0
  const [resizing, setResizing] = useState<{ fieldId: string; width: number } | null>(null)
  const [dragging, setDragging] = useState<{ fieldId: string; to: number; x: number } | null>(null)
  const headRef = useRef<HTMLDivElement>(null)
  // eslint-disable-next-line react-hooks/incompatible-library -- TanStack Virtual returns unstable functions by design; nothing here is memoised on them.
  const virtualizer = useVirtualizer({ count: rows.total, getScrollElement: () => scrollRef.current, estimateSize: () => rowHeight, overscan: 8 })
  const items = virtualizer.getVirtualItems()
  const lastIndex = items.length > 0 ? items[items.length - 1].index : 0
  useEffect(() => { pager.ensure(lastIndex) }, [pager, lastIndex])

  // a phone keeps its fixed first column narrow enough to leave room for the rest
  const widthOf = (column: Column, index: number) => {
    const width = resizing?.fieldId === column.field.field_id ? resizing.width : column.width
    return phone && rowHead && index === 0 ? Math.min(width, WIDTHS.medium) : width
  }
  const totalWidth = headWidth + columns.reduce((sum, column, index) => sum + widthOf(column, index), 0) + (props.onAddField ? ADD_COLUMN : 0)
  const loaded = rows.rows.filter(Boolean)
  const allSelected = Boolean(selection && loaded.length > 0 && loaded.every((row) => selection.has(row.record_id)))

  /** Keyboard move between cells (流式页交互改进 §5.3): rows outside the rendered window are scrolled in first. */
  const navigate = (row: number, col: number) => {
    if (row < 0 || row >= rows.total || col < 0 || col >= columns.length) return
    const focus = () => scrollRef.current?.querySelector<HTMLElement>(`[data-row="${row}"][data-col="${col}"]`)?.focus()
    if (scrollRef.current?.querySelector(`[data-row="${row}"][data-col="${col}"]`)) { focus(); return }
    virtualizer.scrollToIndex(row, { align: 'auto' })
    window.requestAnimationFrame(() => window.requestAnimationFrame(focus))
  }

  const startResize = (event: ReactPointerEvent<HTMLElement>, column: Column) => {
    event.preventDefault()
    event.stopPropagation()
    const handle = event.currentTarget
    handle.setPointerCapture(event.pointerId)
    const x0 = event.clientX
    const w0 = column.width
    let width = w0
    const move = (e: PointerEvent) => { width = Math.round(Math.max(MIN_WIDTH, Math.min(MAX_WIDTH, w0 + e.clientX - x0))); setResizing({ fieldId: column.field.field_id, width }) }
    const up = () => {
      handle.removeEventListener('pointermove', move)
      handle.removeEventListener('pointerup', up)
      handle.removeEventListener('pointercancel', up)
      setResizing(null)
      if (width !== w0) props.onResize(column.field.field_id, width)
    }
    handle.addEventListener('pointermove', move)
    handle.addEventListener('pointerup', up)
    handle.addEventListener('pointercancel', up)
  }
  /** A header dragged sideways moves its column (a click alone does nothing). */
  const startMove = (event: ReactPointerEvent<HTMLElement>, index: number) => {
    if (!props.columnsEditable || phone || event.button !== 0) return
    const el = event.currentTarget
    const x0 = event.clientX
    let started = false
    let to = index
    const targetIndex = (clientX: number) => {
      const left = (headRef.current?.getBoundingClientRect().left ?? 0) + headWidth
      let x = left
      for (let i = 0; i < columns.length; i++) {
        const w = columns[i].width
        if (clientX < x + w / 2) return i
        x += w
      }
      return columns.length
    }
    const move = (e: PointerEvent) => {
      if (!started && Math.abs(e.clientX - x0) < 6) return
      if (!started) { started = true; el.setPointerCapture(e.pointerId) }
      to = targetIndex(e.clientX)
      let x = headWidth
      for (let i = 0; i < to; i++) x += columns[i].width
      setDragging({ fieldId: columns[index].field.field_id, to, x })
    }
    const up = () => {
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', up)
      window.removeEventListener('pointercancel', up)
      setDragging(null)
      if (!started) return
      const target = to > index ? to - 1 : to
      if (target !== index) props.onMove(columns[index].field.field_id, target)
    }
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
    window.addEventListener('pointercancel', up)
  }

  return (
    <div ref={scrollRef} className="aiws-table-scroll" style={props.maxHeight === null ? undefined : { maxHeight: props.maxHeight }} data-testid={`aiws-table-scroll-${tableId}`} role="grid" aria-rowcount={rows.total} data-phone={phone ? 'true' : undefined}>
      <div ref={headRef} className="aiws-table-head" style={{ width: totalWidth }} role="row">
        {rowHead && (
          <div className="aiws-th-rowhead" style={{ width: headWidth }}>
            {(!phone || selection) && (
              <label className="aiws-row-check-hit">
                <input type="checkbox" className="aiws-row-check" aria-label={t('aiworkspace.table.selectAllLoaded')} title={t('aiworkspace.table.selectAllLoaded')} data-testid={`aiws-row-check-all-${tableId}`}
                  checked={allSelected} onChange={(event) => props.onToggleAll(event.target.checked)} />
              </label>
            )}
          </div>
        )}
        {columns.map((column, index) => {
          const { field } = column
          const Icon = FIELD_ICONS[field.type] ?? Type
          return (
            <div key={field.field_id} className={`aiws-th${phone && index === 0 && rowHead ? ' is-sticky' : ''}${dragging?.fieldId === field.field_id ? ' is-dragged' : ''}${props.columnsEditable && !phone ? ' is-movable' : ''}`} role="columnheader" style={{ width: widthOf(column, index), ...(phone && index === 0 && rowHead ? { left: headWidth } : {}) }}
              data-testid={`aiws-th-${field.field_id}`} title={t('aiworkspace.data.table.columnTitle', { name: field.name, type: FIELD_TYPE_LABEL[field.type] })}
              onPointerDown={(event) => startMove(event, index)}>
              <Icon size={13} className="aiws-th-icon" aria-hidden="true" />
              <span className="aiws-th-name">{field.name}</span>
              {field.maintained_by === 'program' && <span className="aiws-chip" title={t('aiworkspace.data.table.programTitle')}>{t('aiworkspace.data.table.program')}</span>}
              {props.columnsEditable && !phone && (
                <span className="aiws-col-resize" role="separator" aria-orientation="vertical" aria-label={t('aiworkspace.table.column.resize', { name: field.name })} data-testid={`aiws-col-resize-${field.field_id}`}
                  onPointerDown={(event) => startResize(event, column)} />
              )}
            </div>
          )
        })}
        {props.onAddField && (
          <div className="aiws-th aiws-th-add" style={{ width: ADD_COLUMN }}>
            <button type="button" className="aiws-table-tool" aria-label={t('aiworkspace.data.field.addColumn')} title={t('aiworkspace.data.field.addColumn')} data-testid={`aiws-add-field-open-${tableId}`} aria-expanded={props.addFieldOpen}
              onClick={(event) => props.onAddField?.(event.currentTarget)}><Plus size={15} aria-hidden="true" /></button>
          </div>
        )}
        {dragging && <span className="aiws-col-drop" style={{ left: dragging.x }} aria-hidden="true" data-testid="aiws-col-drop" />}
      </div>
      <div style={{ height: virtualizer.getTotalSize(), width: totalWidth, position: 'relative' }}>
        {items.map((item) => {
          const row = rows.rows[item.index]
          const selected = Boolean(row && selection?.has(row.record_id))
          return (
            <div key={item.key} className={`aiws-tr${selected ? ' is-selected' : ''}`} role="row" data-testid="aiws-row" data-record-id={row?.record_id} aria-selected={selection ? selected : undefined}
              style={{ transform: `translateY(${item.start}px)`, height: rowHeight }}>
              {row ? (
                <>
                  {rowHead && <RowHead {...props} row={row} width={headWidth} selected={selected} />}
                  {columns.map((column, col) => (
                    <TableCell
                      key={column.field.field_id}
                      {...props}
                      field={column.field}
                      row={row}
                      rowIndex={item.index}
                      colIndex={col}
                      width={widthOf(column, col)}
                      sticky={phone && col === 0 && rowHead ? headWidth : null}
                      onNavigate={navigate}
                      annotation={props.annotations.find((mark) => mark.payload.target?.entity_id === props.sourceId && mark.payload.target.selector?.kind === 'table_cell'
                        && mark.payload.target.selector.record_id === row.record_id && mark.payload.target.selector.field_id === column.field.field_id)}
                    />
                  ))}
                  {props.onAddField && <div className="aiws-td aiws-td-pad" style={{ width: ADD_COLUMN }} />}
                </>
              ) : <div className="aiws-td aiws-muted" style={{ width: totalWidth }}>…</div>}
            </div>
          )
        })}
      </div>
    </div>
  )
}

function RowHead({ row, width, selected, selection, phone, titleField, annotatedRecords, onToggle, onStartSelect, onOpen, tableId }: TableGridProps & { row: QueryRow; width: number; selected: boolean }) {
  const { t } = useAiwsText()
  const name = (titleField && formatValue(titleField, row.values[titleField.field_id])) || row.record_id
  // a phone: a long press on the head starts selecting, a tap opens the record
  const press = usePress({ onTap: () => (selection ? onToggle(row.record_id, !selected) : onOpen(row.record_id)), onLong: () => onStartSelect(row.record_id) })
  return (
    <div className="aiws-td aiws-td-rowhead" style={{ width }} data-testid={`aiws-rowhead-${row.record_id}`}>
      {(!phone || selection) && (
        <label className="aiws-row-check-hit">
          <input type="checkbox" className="aiws-row-check" aria-label={t('aiworkspace.table.selectRecord', { name })} data-testid={`aiws-row-check-${row.record_id}`} checked={selected}
            onChange={(event) => onToggle(row.record_id, event.target.checked)} />
        </label>
      )}
      {(!phone || !selection) && (
        <button type="button" className="aiws-row-open" aria-label={t('aiworkspace.table.openRecord', { name })} title={t('aiworkspace.table.openRecord', { name })} data-testid={`aiws-row-open-${row.record_id}`}
          {...(phone ? press : { onClick: () => onOpen(row.record_id) })} data-table={tableId}>
          {annotatedRecords.has(row.record_id) ? <MessageSquare size={14} aria-hidden="true" /> : <Maximize2 size={13} aria-hidden="true" />}
        </button>
      )}
    </div>
  )
}

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
  annotation?: AnnotationMark
}

function TableCell({ pager, sourceId, field, row, rowIndex, colIndex, onNavigate, width, sticky, editable: canEdit, resultTable, fields, annotation, onAnnotate, onActivateAnnotation, phone, phoneCell, onPhoneCell, onPhoneEdit }: TableCellProps) {
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
  const valueRef = useRef<HTMLButtonElement | HTMLInputElement>(null)
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
  }

  const current = pending ? (entry.mine === null ? undefined : entry.mine as Json) : value
  // a formula (D18) is shown evaluated over this row; the kernel keeps the formula itself, "materialize" is in the record's ⋯
  const formula = !pending ? asFormula(value) : null
  const formulaValue = formula ? formulaResult(formula, row.values, fields) : null
  const editable = canEdit && !formula
  const shown = pending ? (entry.mine === null ? '' : formatValue(field, entry.mine)) : formula ? (formulaValue === null ? `= ${formula.$formula.expr}` : String(formulaValue)) : formatValue(field, value)
  const selectedOnPhone = phone && phoneCell?.recordId === row.record_id && phoneCell.fieldId === field.field_id
  const startEdit = (text?: string) => {
    if (!editable || editing) return
    setEditing({ baseRev: row.revs[field.field_id] ?? 0, ...(text !== undefined ? { text } : {}) })
  }
  /** A phone: the first tap selects the cell, the second opens the edit bar. */
  const tap = () => {
    if (!phone) { startEdit(); return }
    const cell = { recordId: row.record_id, fieldId: field.field_id }
    if (selectedOnPhone && editable) onPhoneEdit(cell)
    else onPhoneCell(cell)
  }
  // arrows move between cells; a typed character starts editing with it (a spreadsheet's habit, 流式页交互改进 §5.3)
  const onCellKey = (event: ReactKeyboardEvent<HTMLElement>) => {
    const arrow = ARROWS[event.key]
    if (arrow && !event.altKey && !event.ctrlKey && !event.metaKey) { event.preventDefault(); onNavigate(rowIndex + arrow[0], colIndex + arrow[1]); return }
    if (event.key === 'Tab' && !event.ctrlKey && !event.metaKey && !event.altKey) {
      const next = colIndex + (event.shiftKey ? -1 : 1)
      if (next < 0 || !event.currentTarget.closest('.aiws-tr')?.querySelector(`[data-col="${next}"]`)) return
      event.preventDefault()
      onNavigate(rowIndex, next)
      return
    }
    if (field.type === 'boolean') return
    if ((event.key === 'Backspace' || event.key === 'Delete') && editable && value !== undefined) { event.preventDefault(); void write(UNSET, row.revs[field.field_id] ?? 0); return }
    if (event.key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey && !event.nativeEvent.isComposing && editable
      && field.type !== 'select' && field.type !== 'multi_select' && field.type !== 'object_ref' && field.type !== 'date' && field.type !== 'datetime') {
      event.preventDefault()
      startEdit(event.key)
    }
  }
  const cellAttrs = { 'data-row': rowIndex, 'data-col': colIndex }

  return (
    <div
      className={`aiws-td${sticky !== null ? ' is-sticky' : ''}${selectedOnPhone ? ' is-picked' : ''}`}
      role="gridcell"
      style={{ width, ...(sticky !== null ? { left: sticky } : {}) }}
      data-testid={`aiws-cell-${row.record_id}-${field.field_id}`}
      data-state={entry?.state ?? 'clean'}
      data-editable={editable ? 'true' : 'false'}
      data-type={field.type}
      aria-selected={phone ? selectedOnPhone : undefined}
      onDoubleClick={() => { if (field.type !== 'boolean' && !phone) startEdit() }}
    >
      {editing ? (
        <ValueEditor
          def={field}
          value={value}
          initialText={editing.text}
          ariaLabel={`${field.name} ${row.record_id}`}
          onCancel={() => setEditing(null)}
          onCommit={(input) => { setEditing(null); void write(input, editing.baseRev) }}
          onTab={(back) => onNavigate(rowIndex, colIndex + (back ? -1 : 1))}
        />
      ) : field.type === 'boolean' && !formula ? (
        <label className="aiws-cell-check">
          <input ref={valueRef as RefObject<HTMLInputElement>} type="checkbox" aria-label={`${field.name} ${row.record_id}`} checked={ticked ?? current === true} disabled={!editable} {...cellAttrs}
            onChange={(event) => { const next = event.target.checked; setTicked(next); void write(next, row.revs[field.field_id] ?? 0).finally(() => setTicked(null)) }} onKeyDown={onCellKey} />
        </label>
      ) : (
        <button ref={valueRef as RefObject<HTMLButtonElement>} type="button" className="aiws-cell-value" disabled={!editable} onClick={tap} onKeyDown={onCellKey} title={formula ? t('aiworkspace.table.cell.formulaTitle', { formula: formula.$formula.expr }) : editable ? shown || undefined : undefined} {...cellAttrs}>
          <CellDisplay def={field} value={formula ? undefined : current} shown={shown} />
        </button>
      )}
      {!editing && formula && <span className="aiws-chip" data-testid="aiws-cell-formula" title={t('aiworkspace.table.cell.formulaTitle', { formula: formula.$formula.expr })}>{t('aiworkspace.table.record.formula')}</span>}
      {!editing && meta?.derived && !resultTable && (
        <span className="aiws-chip aiws-chip-derived" data-testid="aiws-derived" title={meta.derived.program?.startsWith('mock.') ? t('aiworkspace.data.cell.derivedBySimulated', { program: meta.derived.program }) : t('aiworkspace.data.cell.derivedBy', { program: meta.derived.program ?? t('aiworkspace.data.cell.processingProgram') })}>
          {meta.derived.program?.startsWith('mock.') ? t('aiworkspace.data.simulated') : t('aiworkspace.data.cell.derived')}
        </span>
      )}
      {!editing && meta?.manual_override && <span className="aiws-chip aiws-chip-warn" data-testid="aiws-manual-override" title={t('aiworkspace.data.cell.manualOverrideTitle')}>{t('aiworkspace.data.cell.manualOverride')}</span>}
      {!editing && annotation && (
        <button type="button" className="aiws-chip aiws-chip-note" title={annotation.payload.body} data-testid="aiws-cell-annotation"
          {...{ [`data-anno-${annotation.entityId}`]: '' }} onClick={() => onActivateAnnotation?.(annotation.entityId)}>{t('aiworkspace.data.cell.annotation')}</button>
      )}
      {!editing && onAnnotate && !annotation && !phone && (
        <button type="button" className="aiws-cell-annotate" title={t('aiworkspace.table.cell.comment')} aria-label={t('aiworkspace.table.cell.commentAria', { field: field.name, record: row.record_id })}
          onClick={() => onAnnotate(cellAnchor(sourceId, field, row))}>✎</button>
      )}
      {entry && !editing && (
        <span className={`aiws-state aiws-state-${entry.state}`} data-testid="aiws-edit-state" title={entry.detail}>{EDIT_STATE_LABEL[entry.state]}</span>
      )}
      {entry && entry.state === 'needs_attention' && !editing && editable && (
        <ConflictBox
          store={store}
          editId={editId}
          theirs={entry.hasTheirs ? formatValue(field, entry.theirs) || t('aiworkspace.data.value.notSet') : null}
          mine={entry.hasMine ? (entry.mine === null ? t('aiworkspace.data.value.cleared') : formatValue(field, entry.mine ?? undefined)) : null}
          detail={entry.detail ?? ''}
          onKeepMine={entry.hasMine && entry.currentRev !== undefined ? () => { void write(entry.mine === null ? UNSET : (entry.mine as Json), entry.currentRev ?? 0) } : undefined}
          onEditAgain={entry.hasMine ? () => {
            // Start a new edit from my kept input, based on what is current now.
            const mineText = entry.mine === null || entry.mine === undefined ? '' : typeof entry.mine === 'object' ? '' : String(entry.mine)
            setEditing({ baseRev: entry.currentRev ?? row.revs[field.field_id] ?? 0, text: mineText })
          } : undefined}
        />
      )}
      {entry && entry.state === 'unsaved' && entry.unknownKey && !editing && (
        <button type="button" className="aiws-link" onClick={() => {
          const input: Input = entry.mine === null || entry.mine === undefined ? UNSET : entry.mine
          void store.retry({ editId, label: entry.label, mine: entry.mine, hasMine: true, operations: [writeOperation(sourceId, field, row.record_id, input, 0)] }, entry.unknownKey ?? '')
        }}>{t('aiworkspace.common.retry')}</button>
      )}
    </div>
  )
}
