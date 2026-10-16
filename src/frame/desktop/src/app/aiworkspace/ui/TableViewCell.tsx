/* TableView cell (design §3.5): a virtualised, paged view of a TableSource with inline editors.
 * Every write carries the rev the user was looking at when the edit began. */

import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore, type KeyboardEvent as ReactKeyboardEvent, type RefObject } from 'react'
import { createPortal } from 'react-dom'
import { useVirtualizer } from '@tanstack/react-virtual'
import { Calendar, CalendarClock, CircleChevronDown, Columns3, Hash, Link2, ListFilter, Plus, SquareCheck, Tags, Trash2, Type } from 'lucide-react'
import { randomId } from '../api/ids'
import type { ReadOk } from '../api/session'
import type {
  CellPayload, FieldDef, FilterNode, Json, KeyedContent, Operation, QueryRow, SortSpec, TableSourceContent,
} from '../api/types'
import { EDIT_STATE_LABEL } from '../state/edits'
import type { CapturedAnchor } from '../anchors/registry'
import { useEdit, useLoad, useStore, useUserState, useVersion, type AnnotationMark } from '../state/hooks'
import type { WorkspaceStore } from '../state/store'
import { useAiwsText } from '../useAiwsText'
import { consumeIntent, useEditorToolbar, useIntent } from './blocks/editorToolbar'
import type { ToolbarItem } from './blocks/registry'
import { FieldManager } from './FieldManager'
import { addFieldOps, CREATABLE_FIELD_TYPES } from './fieldOps'
import { useOverlayMounted } from './shell/popover'
import { TablePager, type PagerQuery } from './tablePager'
import { ValueEditor } from './ValueEditor'
import { FIELD_TYPE_LABEL, UNSET, formatValue, operatorsFor, SORTABLE, type Input } from './values'

const ROW_HEIGHT = 34
const DEFAULT_WIDTH = 150
/** The header's "+" column and the row actions column. */
const ADD_COLUMN = 40
const ROW_ACTIONS = 36

interface Props {
  /** A table-view Cell; or, without one, the table itself (data-source view, phase two §6.1). */
  cellId?: string
  sourceId?: string
  /** No editing at all (embedded rendering, missing capability, write lock not held). */
  readOnly: boolean
  compact?: boolean
  annotations: AnnotationMark[]
  onAnnotate?: (anchor: CapturedAnchor) => void
  onActivateAnnotation?: (entityId: string | null) => void
}

type CellRead = ReadOk<KeyedContent<CellPayload>>
type SourceRead = ReadOk<TableSourceContent>

interface SessionView { filter: FilterNode | null; sorts: SortSpec[] | null }

export function TableViewCell({ cellId, sourceId, readOnly, compact, annotations, onAnnotate, onActivateAnnotation }: Props) {
  const { t } = useAiwsText()
  const store = useStore()
  const id = cellId ?? ''
  const cellVersion = useVersion(`e:${id}`)
  const loadCell = useCallback(() => (cellId ? store.session.read<KeyedContent<CellPayload>>(cellId) : Promise.resolve(null)), [store, cellId])
  const cell = useLoad<CellRead | null>(loadCell, cellVersion)
  if (!cellId && sourceId) {
    // no Block: the table itself, with the sort / filter kept in the user work state (§4.4)
    const synthetic: CellRead = {
      entity_id: `source:${sourceId}`, type_id: 'buckyos.cell', schema_version: 1, name: null, scope: 'shared', deleted: false, content_rev: 0, meta_rev: 0, life_rev: 0,
      write_policy: 'open', capabilities: [], head_seq: 0,
      content: { payload: { source_ref: { entity_id: sourceId }, view: { type: 'table' } }, key_revs: {} },
    }
    return <TableViewBody key={sourceId} cell={synthetic} sourceMode readOnly={readOnly} compact={compact} annotations={annotations} onAnnotate={onAnnotate} onActivateAnnotation={onActivateAnnotation} />
  }
  if (cell.error && !cell.data) return <div className="aiws-error" role="alert">{t('aiworkspace.data.table.viewReadError', { error: cell.error })}</div>
  if (!cell.data) return <div className="aiws-muted">{t('aiworkspace.data.table.viewLoading')}</div>
  const sourceRef = cell.data.content.payload.source_ref
  if (!sourceRef) return <div className="aiws-warning">{t('aiworkspace.data.table.unbound')}</div>
  return <TableViewBody key={sourceRef.entity_id} cell={cell.data} readOnly={readOnly} compact={compact} annotations={annotations} onAnnotate={onAnnotate} onActivateAnnotation={onActivateAnnotation} />
}

function TableViewBody({ cell, sourceMode, readOnly, compact, annotations, onAnnotate, onActivateAnnotation }: Omit<Props, 'cellId' | 'sourceId'> & { cell: CellRead; sourceMode?: boolean }) {
  const { t } = useAiwsText()
  const store = useStore()
  const cellId = cell.entity_id
  const payload = cell.content.payload
  const sourceId = payload.source_ref?.entity_id ?? ''
  const sourceVersion = useVersion(`e:${sourceId}`)
  const loadSource = useCallback(() => store.session.read<TableSourceContent>(sourceId), [store, sourceId])
  const source = useLoad<SourceRead>(loadSource, sourceVersion)
  // Session-level filter and sort (design §3.5.2): not a commit until "save view". Without a Block they
  // live in the user work state instead (phase two §6.1).
  const stateKey = `tableview:${sourceId}`
  const remembered = useUserState<Json>(stateKey) as SessionView | undefined
  const [localView, setLocalView] = useState<SessionView>({ filter: null, sorts: null })
  const sessionView = sourceMode ? (remembered ?? localView) : localView
  const setSessionView = useCallback((view: SessionView) => {
    setLocalView(view)
    if (sourceMode) store.userState.set(stateKey, (view.filter || view.sorts) ? (view as unknown as Json) : null)
  }, [sourceMode, store, stateKey])
  // a toolbar button pressed while the Block was only selected names the panel to open (标准对象的交互改进 §5.2)
  type Panel = 'none' | 'filter' | 'fields' | 'add'
  const asPanel = (intent: string | undefined): Panel => (intent === 'filter' || intent === 'fields' || intent === 'add' ? intent : 'none')
  // only the editor takes "open this panel" (the static table of the same Block must not use it up)
  const intentKey = sourceMode || compact ? '' : cellId
  const intent = useIntent(intentKey)
  const [panel, setPanel] = useState<Panel>('none')
  const [intentSeen, setIntentSeen] = useState(0)
  if (intent && intent.seq !== intentSeen) { setIntentSeen(intent.seq); setPanel(asPanel(intent.value)) }
  useEffect(() => { if (intent) consumeIntent(intentKey, intent) }, [intent, intentKey])

  const savedKey = JSON.stringify([payload.filter ?? null, payload.sorts ?? null, payload.fields ?? null])
  const sessionKey = JSON.stringify(sessionView)
  const pager = useMemo(() => {
    const saved = JSON.parse(savedKey) as [FilterNode | null, SortSpec[] | null, unknown]
    const session = JSON.parse(sessionKey) as SessionView
    const query: PagerQuery = {
      viewId: sourceMode ? undefined : cellId, sourceId, filter: session.filter, sorts: session.sorts,
      orderDependsOnData: Boolean(saved[0] || (saved[1] && saved[1].length > 0) || session.filter || (session.sorts && session.sorts.length > 0)),
    }
    return new TablePager(store.session, query)
  }, [store, cellId, sourceId, savedKey, sessionKey, sourceMode])
  const rows = useSyncExternalStore(pager.subscribe, pager.snapshot)
  // A schema change (field added, renamed, deleted, migrated) changes what a row means: read again.
  const schemaKey = (source.data?.content.fields ?? []).map((field) => `${field.field_id}:${field.def_rev}`).join(',')
  const lastSchema = useRef<string | null>(null)
  useEffect(() => {
    if (lastSchema.current !== null && lastSchema.current !== schemaKey) pager.refresh()
    lastSchema.current = schemaKey
  }, [schemaKey, pager])

  const fields = source.data?.content.fields
  const columns = useMemo(() => {
    if (!fields) return []
    const byId = new Map(fields.map((field) => [field.field_id, field]))
    const listed = payload.fields
    if (listed && listed.length > 0) {
      return listed.flatMap((item) => { const field = byId.get(item.field_id); return field ? [{ field, width: item.width ?? DEFAULT_WIDTH }] : [] })
    }
    return fields.map((field) => ({ field, width: DEFAULT_WIDTH }))
  }, [fields, payload.fields])

  const canUpdate = !readOnly && (source.data?.capabilities.includes('update') ?? false)
  const canAppend = !readOnly && Boolean(source.data?.capabilities.some((capability) => capability === 'append' || capability === 'update'))
  const canDelete = !readOnly && (source.data?.capabilities.includes('delete') ?? false)
  const canStructure = !readOnly && (source.data?.capabilities.includes('structure') ?? false)
  const canEditView = !readOnly && !sourceMode && cell.capabilities.includes('update')

  // on a canvas the bar's controls are tools of the near toolbar; elsewhere the bar stays above the table
  const toggle = (next: Panel) => setPanel(panel === next ? 'none' : next)
  const tools: ToolbarItem[] | null = compact || !source.data ? null : [
    { kind: 'button', id: 'table-filter', icon: ListFilter, label: sessionView.filter || sessionView.sorts ? t('aiworkspace.data.table.filterSortSession') : payload.filter || payload.sorts?.length ? t('aiworkspace.data.table.filterSortSaved') : t('aiworkspace.data.table.filterSort'), active: panel === 'filter' || Boolean(sessionView.filter || sessionView.sorts), run: () => toggle('filter') },
    ...(canStructure ? [{ kind: 'button' as const, id: 'table-fields', icon: Columns3, label: t('aiworkspace.data.table.fields'), active: panel === 'fields', run: () => toggle('fields') }] : []),
    ...(canAppend ? [{ kind: 'button' as const, id: 'table-add', icon: Plus, label: t('aiworkspace.data.record.add'), active: panel === 'add', run: () => toggle('add') }] : []),
  ]
  const inNearToolbar = useEditorToolbar('table', tools)

  const scrollRef = useRef<HTMLDivElement>(null)
  const [root, bindRoot] = useState<HTMLDivElement | null>(null)
  /** The header's "+" panel, placed under the button inside the table (the grid itself scrolls and would clip it). */
  const [addingField, setAddingField] = useState<{ top: number; left: number } | null>(null)
  // eslint-disable-next-line react-hooks/incompatible-library -- TanStack Virtual returns unstable functions by design; nothing here is memoised on them.
  const virtualizer = useVirtualizer({ count: rows.total, getScrollElement: () => scrollRef.current, estimateSize: () => ROW_HEIGHT, overscan: 8 })
  const items = virtualizer.getVirtualItems()
  const lastIndex = items.length > 0 ? items[items.length - 1].index : 0
  useEffect(() => { pager.ensure(lastIndex) }, [pager, lastIndex])

  if (source.error && !source.data) return <div className="aiws-error" role="alert">{t('aiworkspace.data.table.sourceReadError', { error: source.error })}</div>
  if (!source.data || !fields) return <div className="aiws-muted">{t('aiworkspace.data.table.sourceLoading')}</div>

  const totalWidth = columns.reduce((sum, column) => sum + column.width, 0) + (canStructure && !compact ? ADD_COLUMN : 0) + (canDelete ? ROW_ACTIONS : 0)
  const brokenFilter = rows.error?.subCode === 'VIEW_BROKEN'
  const diagnostics = [...(cell.content.diagnostics ?? [])]
  for (const item of rows.diagnostics) if (!diagnostics.some((known) => known.key === item.key && known.field_id === item.field_id && known.code === item.code)) diagnostics.push(item)

  const saveView = async () => {
    const keys: { key: string; value: Json; expect: { rev: number } }[] = []
    if (sessionView.filter) {
      const merged: FilterNode = payload.filter ? { op: 'and', args: [payload.filter, sessionView.filter] } : sessionView.filter
      keys.push({ key: 'filter', value: merged as unknown as Json, expect: { rev: cell.content.key_revs.filter ?? 0 } })
    }
    if (sessionView.sorts) keys.push({ key: 'sorts', value: sessionView.sorts as unknown as Json, expect: { rev: cell.content.key_revs.sorts ?? 0 } })
    if (keys.length === 0) return
    const outcome = await store.submit({ editId: `view:${cellId}`, label: t('aiworkspace.data.view.saveEdit', { name: payload.title ?? cellId }), operations: [{ op: 'entity.set_keys', entity_id: cellId, keys }] })
    if (outcome.status === 'accepted') setSessionView({ filter: null, sorts: null })
  }
  const clearSaved = (key: 'filter' | 'sorts') => store.submit({
    editId: `view:${cellId}`, label: key === 'filter' ? t('aiworkspace.data.view.removeFilterEdit') : t('aiworkspace.data.view.removeSortEdit'),
    operations: [{ op: 'entity.unset_keys', entity_id: cellId, keys: [{ key, expect: { rev: cell.content.key_revs[key] ?? 0 } }] }],
  })
  /** Keyboard move between cells (流式页交互改进 §5.3): rows outside the rendered window are scrolled in first. */
  const navigate = (row: number, col: number) => {
    if (row < 0 || row >= rows.total || col < 0 || col >= columns.length) return
    const focus = () => scrollRef.current?.querySelector<HTMLElement>(`[data-row="${row}"][data-col="${col}"]`)?.focus()
    if (scrollRef.current?.querySelector(`[data-row="${row}"][data-col="${col}"]`)) { focus(); return }
    virtualizer.scrollToIndex(row, { align: 'auto' })
    window.requestAnimationFrame(() => window.requestAnimationFrame(focus))
  }
  const filtered = Boolean(sessionView.filter || sessionView.sorts)

  return (
    <div ref={bindRoot} className="aiws-table" data-testid={`aiws-table-${cellId}`} data-total={rows.total} data-compact={compact ? 'true' : 'false'}>
      {!compact && !inNearToolbar && (
        <div className="aiws-table-bar">
          {payload.filter && <span className="aiws-chip">{t('aiworkspace.data.table.savedFilter')}</span>}
          {payload.sorts && payload.sorts.length > 0 && <span className="aiws-chip">{t('aiworkspace.data.table.savedSort')}</span>}
          {filtered && <span className="aiws-chip aiws-chip-warn">{t('aiworkspace.data.table.sessionConditions')}</span>}
          <span className="aiws-grow" />
          <button type="button" className="aiws-table-tool" aria-label={t('aiworkspace.data.table.filterSort')} title={t('aiworkspace.data.table.filterSort')} aria-pressed={panel === 'filter' || filtered} data-testid={`aiws-filter-open-${cellId}`}
            onClick={() => setPanel(panel === 'filter' ? 'none' : 'filter')}><ListFilter size={15} aria-hidden="true" /></button>
          {canStructure && <button type="button" className="aiws-table-tool" aria-label={t('aiworkspace.data.table.fields')} title={t('aiworkspace.data.table.fieldsTitle')} aria-pressed={panel === 'fields'} data-testid={`aiws-fields-${cellId}`}
            onClick={() => setPanel(panel === 'fields' ? 'none' : 'fields')}><Columns3 size={15} aria-hidden="true" /></button>}
        </div>
      )}
      {diagnostics.length > 0 && (
        <div className="aiws-warning" role="status" data-testid={`aiws-diagnostics-${cellId}`}>
          {diagnostics.map((item, index) => (
            <div key={index}>{(() => {
              const part = item.key === 'filter' ? t('aiworkspace.data.table.diagnosticFilter') : item.key === 'sorts' ? t('aiworkspace.data.table.diagnosticSorts')
                : item.key === 'group' ? t('aiworkspace.data.table.diagnosticGroup') : t('aiworkspace.data.table.diagnosticColumns')
              const what = item.code === 'FIELD_DELETED' ? t('aiworkspace.data.table.diagnosticFieldDeleted') : item.code === 'OPTION_DELETED' ? t('aiworkspace.data.table.diagnosticOptionDeleted') : item.code
              return item.field_id ? t('aiworkspace.data.table.diagnosticField', { part, what, field: item.field_id }) : t('aiworkspace.data.table.diagnostic', { part, what })
            })()}</div>
          ))}
        </div>
      )}
      {panel === 'filter' && (
        <FilterSortPanel
          fields={fields}
          session={sessionView}
          hasSavedFilter={Boolean(payload.filter)}
          hasSavedSorts={Boolean(payload.sorts && payload.sorts.length > 0)}
          canSave={canEditView}
          onApply={setSessionView}
          onSave={() => { void saveView() }}
          onClearSaved={(key) => { void clearSaved(key) }}
        />
      )}
      {panel === 'fields' && <FieldManager source={source.data} cell={cell} />}
      {panel === 'add' && <AddRecordForm store={store} source={source.data} onDone={() => setPanel('none')} />}
      {rows.error ? (
        <div className="aiws-error" role="alert" data-testid={`aiws-table-error-${cellId}`}>
          {brokenFilter ? t('aiworkspace.data.table.viewBroken') : t('aiworkspace.data.table.queryFailed', { error: rows.error.text })}
          {brokenFilter && canEditView && <button type="button" onClick={() => { void clearSaved('filter') }}>{t('aiworkspace.data.table.removeBrokenFilter')}</button>}
          {!brokenFilter && <button type="button" onClick={() => pager.refresh()}>{t('aiworkspace.common.retry')}</button>}
        </div>
      ) : (
        <div ref={scrollRef} className="aiws-table-scroll" style={{ maxHeight: compact ? 220 : 420 }} data-testid={`aiws-table-scroll-${cellId}`} role="grid" aria-rowcount={rows.total}>
          <div className="aiws-table-head" style={{ width: totalWidth }} role="row">
            {columns.map(({ field, width }) => {
              const Icon = FIELD_ICONS[field.type] ?? Type
              return (
                <div key={field.field_id} className="aiws-th" role="columnheader" style={{ width }} title={t('aiworkspace.data.table.columnTitle', { name: field.name, type: FIELD_TYPE_LABEL[field.type] })}>
                  <Icon size={13} className="aiws-th-icon" aria-hidden="true" />
                  <span className="aiws-th-name">{field.name}</span>
                  {field.maintained_by === 'program' && <span className="aiws-chip" title={t('aiworkspace.data.table.programTitle')}>{t('aiworkspace.data.table.program')}</span>}
                </div>
              )
            })}
            {canStructure && !compact && (
              <div className="aiws-th aiws-th-add" style={{ width: ADD_COLUMN }}>
                <button type="button" className="aiws-table-tool" aria-label={t('aiworkspace.data.field.addColumn')} title={t('aiworkspace.data.field.addColumn')} data-testid={`aiws-add-field-open-${cellId}`} aria-expanded={addingField !== null}
                  onClick={(event) => {
                    if (addingField || !root) { setAddingField(null); return }
                    const box = root.getBoundingClientRect()
                    const r = event.currentTarget.getBoundingClientRect()
                    setAddingField({ top: r.bottom - box.top + 4, left: Math.max(0, Math.min(r.right - box.left - 260, box.width - 260)) })
                  }}><Plus size={15} aria-hidden="true" /></button>
              </div>
            )}
            {canDelete && <div className="aiws-th aiws-th-actions" style={{ width: ROW_ACTIONS }} />}
          </div>
          <div style={{ height: virtualizer.getTotalSize(), width: totalWidth, position: 'relative' }}>
            {items.map((item) => {
              const row = rows.rows[item.index]
              return (
                <div key={item.key} className="aiws-tr" role="row" data-testid="aiws-row" data-record-id={row?.record_id} style={{ transform: `translateY(${item.start}px)`, height: ROW_HEIGHT }}>
                  {row ? (
                    <>
                      {columns.map(({ field, width }, col) => (
                        <TableCell
                          key={field.field_id}
                          pager={pager}
                          sourceId={sourceId}
                          field={field}
                          row={row}
                          rowIndex={item.index}
                          colIndex={col}
                          onNavigate={navigate}
                          width={width}
                          editable={canUpdate}
                          resultTable={Boolean(source.data?.derived)}
                          annotation={annotations.find((mark) => mark.payload.target?.entity_id === sourceId && mark.payload.target.selector?.kind === 'table_cell'
                            && mark.payload.target.selector.record_id === row.record_id && mark.payload.target.selector.field_id === field.field_id)}
                          onAnnotate={onAnnotate}
                          onActivateAnnotation={onActivateAnnotation}
                        />
                      ))}
                      {canStructure && !compact && <div className="aiws-td aiws-td-pad" style={{ width: ADD_COLUMN }} />}
                      {canDelete && <div className="aiws-td aiws-td-actions" style={{ width: ROW_ACTIONS }}><DeleteRecordButton sourceId={sourceId} row={row} /></div>}
                    </>
                  ) : <div className="aiws-td aiws-muted" style={{ width: totalWidth }}>…</div>}
                </div>
              )
            })}
          </div>
        </div>
      )}
      {addingField && root && createPortal(
        <QuickAddField source={source.data} cell={cell} at={addingField} onClose={() => setAddingField(null)} />,
        root,
      )}
      {!compact && !inNearToolbar && (
        <div className="aiws-table-foot">
          {canAppend && !rows.error && <NewRecordRow store={store} source={source.data} cellId={cellId} onNeedForm={() => setPanel('add')} />}
          <span className="aiws-grow" />
          <span className="aiws-table-count" data-testid={`aiws-table-count-${cellId}`}>{rows.loading ? t('aiworkspace.data.table.countLoading', { count: rows.total }) : t('aiworkspace.data.table.count', { count: rows.total })}</span>
        </div>
      )}
    </div>
  )
}

/** Field type → header icon. */
const FIELD_ICONS: Partial<Record<FieldDef['type'], typeof Type>> = {
  text: Type, number: Hash, decimal: Hash, date: Calendar, datetime: CalendarClock, boolean: SquareCheck, select: CircleChevronDown, multi_select: Tags, object_ref: Link2,
}

/** "+" of the header: a field by name and type, nothing else asked (the field manager has the rest). */
function QuickAddField({ source, cell, at, onClose }: { source: SourceRead; cell: CellRead; at: { top: number; left: number }; onClose: () => void }) {
  const { t } = useAiwsText()
  const store = useStore()
  const [name, setName] = useState('')
  const [type, setType] = useState<FieldDef['type']>('text')
  const ref = useRef<HTMLFormElement>(null)
  useOverlayMounted()
  useEffect(() => {
    const onDown = (event: PointerEvent) => {
      const target = event.target as HTMLElement
      if (!ref.current?.contains(target) && !target.closest('[data-testid^="aiws-add-field-open-"]')) onClose()
    }
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') { event.stopPropagation(); onClose() } }
    window.addEventListener('pointerdown', onDown, true)
    window.addEventListener('keydown', onKey, true)
    return () => { window.removeEventListener('pointerdown', onDown, true); window.removeEventListener('keydown', onKey, true) }
  }, [onClose])
  const submit = () => {
    const text = name.trim()
    if (!text) return
    void store.submit({ editId: `schema:${source.entity_id}:${randomId()}`, label: t('aiworkspace.data.field.addEdit', { name: text }), operations: addFieldOps(source.entity_id, cell, text, type) })
      .then((outcome) => { if (outcome.status === 'accepted' || outcome.status === 'saved_locally') onClose() })
  }
  return (
    <form ref={ref} className="aiws-popover aiws-quick-field" style={{ top: at.top, left: at.left }} data-testid="aiws-quick-field" onSubmit={(event) => { event.preventDefault(); submit() }} onPointerDown={(event) => event.stopPropagation()}>
      <input aria-label={t('aiworkspace.data.field.name')} data-testid="aiws-quick-field-name" autoFocus placeholder={t('aiworkspace.data.field.name')} value={name} onChange={(event) => setName(event.target.value)} />
      <div className="aiws-quick-field-types" role="radiogroup" aria-label={t('aiworkspace.data.field.type')}>
        {CREATABLE_FIELD_TYPES.map((item) => {
          const Icon = FIELD_ICONS[item] ?? Type
          return (
            <button key={item} type="button" role="radio" aria-checked={type === item} className="aiws-quick-field-type" data-testid={`aiws-quick-field-type-${item}`} onClick={() => setType(item)}>
              <Icon size={14} aria-hidden="true" />{FIELD_TYPE_LABEL[item]}
            </button>
          )
        })}
      </div>
      <button type="submit" className="is-primary" data-testid="aiws-quick-field-add" disabled={!name.trim()}>{t('aiworkspace.data.field.addColumn')}</button>
    </form>
  )
}

/** "+ 新建" under the rows (Notion): type the title, Enter adds the record and keeps the line open for the next one.
 * A table with other required fields asks for them in the form instead. */
function NewRecordRow({ store, source, cellId, onNeedForm }: { store: WorkspaceStore; source: SourceRead; cellId: string; onNeedForm: () => void }) {
  const { t } = useAiwsText()
  const fields = source.content.fields
  const titleField = fields.find((field) => field.field_id === source.content.payload.title_field_id) ?? fields.find((field) => field.type === 'text')
  const otherRequired = fields.some((field) => field.required && field.field_id !== titleField?.field_id)
  const [open, setOpen] = useState(false)
  const [text, setText] = useState('')
  const [error, setError] = useState<string | null>(null)
  const insert = async (title: string) => {
    if (titleField?.required && !title) { setError(t('aiworkspace.data.record.required', { name: titleField.name })); return }
    const values: Record<string, Json> = titleField && title ? { [titleField.field_id]: title } : {}
    const recordId = randomId('r')
    const outcome = await store.submit({
      editId: `record:${source.entity_id}:${recordId}`, label: t('aiworkspace.data.record.add'), mine: values, hasMine: true,
      operations: [{ op: 'table.insert_records', source_id: source.entity_id, field_type_revs: titleField && title ? { [titleField.field_id]: titleField.type_rev } : {}, records: [{ record_id: recordId, values }] }],
    })
    if (outcome.status === 'accepted' || outcome.status === 'saved_locally') { setText(''); setError(null) }
    else setError(t('aiworkspace.data.record.addFailed'))
  }
  const start = () => {
    if (otherRequired) { onNeedForm(); return }
    if (!titleField) { void insert(''); return }
    setOpen(true)
  }
  if (!open) {
    return <button type="button" className="aiws-table-new" data-testid={`aiws-add-record-${cellId}`} onClick={start}><Plus size={14} aria-hidden="true" />{t('aiworkspace.data.record.new')}</button>
  }
  return (
    <form className="aiws-table-new-row" onSubmit={(event) => { event.preventDefault(); void insert(text.trim()) }}>
      <Plus size={14} aria-hidden="true" className="aiws-muted" />
      <input aria-label={t('aiworkspace.data.record.newAria', { name: titleField?.name ?? t('aiworkspace.data.record.titleFallback') })} data-testid="aiws-new-record-input" autoFocus value={text}
        placeholder={t('aiworkspace.data.record.newPlaceholder', { name: titleField?.name ?? t('aiworkspace.data.record.titleFallback') })}
        onChange={(event) => { setText(event.target.value); setError(null) }}
        onBlur={() => { if (!text.trim()) { setOpen(false); setError(null) } }}
        onKeyDown={(event) => { if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); setOpen(false); setText(''); setError(null) } }} />
      <button type="submit" className="aiws-link" data-testid="aiws-add-record-submit">{t('aiworkspace.data.record.addButton')}</button>
      {error && <span className="aiws-error" role="alert">{error}</span>}
    </form>
  )
}

function DeleteRecordButton({ sourceId, row }: { sourceId: string; row: QueryRow }) {
  const { t } = useAiwsText()
  const store = useStore()
  return (
    <button
      type="button"
      className="aiws-row-delete"
      title={t('aiworkspace.data.record.delete')}
      aria-label={t('aiworkspace.data.record.deleteNamed', { id: row.record_id })}
      data-testid={`aiws-delete-record-${row.record_id}`}
      onClick={() => {
        void store.submit({
          editId: `record:${sourceId}:${row.record_id}`, label: t('aiworkspace.data.record.deleteNamed', { id: row.record_id }),
          operations: [{ op: 'table.delete_records', source_id: sourceId, records: [{ record_id: row.record_id, expect: { rev: row.rev } }] }],
        })
      }}
    ><Trash2 size={14} aria-hidden="true" /></button>
  )
}

function cellEditId(sourceId: string, recordId: string, fieldId: string) {
  return `cell:${sourceId}:${recordId}:${fieldId}`
}

function writeOperation(sourceId: string, field: FieldDef, recordId: string, value: Input, rev: number): Operation {
  return value === UNSET
    ? { op: 'table.unset_values', source_id: sourceId, values: [{ record_id: recordId, field_id: field.field_id, expect: { rev } }] }
    : { op: 'table.set_values', source_id: sourceId, field_type_revs: { [field.field_id]: field.type_rev }, values: [{ record_id: recordId, field_id: field.field_id, value, expect: { rev } }] }
}

interface TableCellProps {
  pager: TablePager
  sourceId: string
  field: FieldDef
  row: QueryRow
  rowIndex: number
  colIndex: number
  /** Focus another cell (arrow keys, Tab). */
  onNavigate: (row: number, col: number) => void
  width: number
  editable: boolean
  /** The table is itself a wish result: per-cell provenance would mark every cell. */
  resultTable: boolean
  annotation?: AnnotationMark
  onAnnotate?: (anchor: CapturedAnchor) => void
  onActivateAnnotation?: (entityId: string | null) => void
}

const ARROWS: Record<string, [number, number]> = { ArrowUp: [-1, 0], ArrowDown: [1, 0], ArrowLeft: [0, -1], ArrowRight: [0, 1] }

/** The value as a page shows it: a tick box, option pills, or text. */
function CellDisplay({ field, value, shown }: { field: FieldDef; value: Json | undefined; shown: string }) {
  const { t } = useAiwsText()
  if ((field.type === 'select' || field.type === 'multi_select') && value !== undefined && value !== null) {
    const ids = Array.isArray(value) ? value.map(String) : [String(value)]
    const known = (field.options ?? []).filter((option) => ids.includes(option.option_id))
    const unknown = ids.filter((id) => !(field.options ?? []).some((option) => option.option_id === id))
    return (
      <span className="aiws-pills">
        {known.map((option) => <span key={option.option_id} className="aiws-pill" data-tone={toneOf(option.option_id)}>{option.label}</span>)}
        {unknown.map((id) => <span key={id} className="aiws-pill" data-tone="0">{t('aiworkspace.data.value.unknownOption', { id })}</span>)}
      </span>
    )
  }
  return <>{shown || <span className="aiws-muted">&nbsp;</span>}</>
}

/** A stable colour per option (eight soft tones). */
function toneOf(id: string): string {
  let h = 0
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) >>> 0
  return String(h % 8)
}

function TableCell({ pager, sourceId, field, row, rowIndex, colIndex, onNavigate, width, editable, resultTable, annotation, onAnnotate, onActivateAnnotation }: TableCellProps) {
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
  const shown = pending ? (entry.mine === null ? '' : formatValue(field, entry.mine)) : formatValue(field, value)
  const startEdit = (text?: string) => {
    if (!editable || editing) return
    setEditing({ baseRev: row.revs[field.field_id] ?? 0, ...(text !== undefined ? { text } : {}) })
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
      className="aiws-td"
      role="gridcell"
      style={{ width }}
      data-testid={`aiws-cell-${row.record_id}-${field.field_id}`}
      data-state={entry?.state ?? 'clean'}
      data-editable={editable ? 'true' : 'false'}
      data-type={field.type}
      onDoubleClick={() => { if (field.type !== 'boolean') startEdit() }}
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
      ) : field.type === 'boolean' ? (
        <label className="aiws-cell-check">
          <input ref={valueRef as RefObject<HTMLInputElement>} type="checkbox" aria-label={`${field.name} ${row.record_id}`} checked={ticked ?? current === true} disabled={!editable} {...cellAttrs}
            onChange={(event) => { const next = event.target.checked; setTicked(next); void write(next, row.revs[field.field_id] ?? 0).finally(() => setTicked(null)) }} onKeyDown={onCellKey} />
        </label>
      ) : (
        <button ref={valueRef as RefObject<HTMLButtonElement>} type="button" className="aiws-cell-value" disabled={!editable} onClick={() => startEdit()} onKeyDown={onCellKey} title={editable ? shown || undefined : undefined} {...cellAttrs}>
          <CellDisplay field={field} value={current} shown={shown} />
        </button>
      )}
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
      {!editing && onAnnotate && !annotation && (
        <button type="button" className="aiws-cell-annotate" title={t('aiworkspace.data.cell.annotate')} aria-label={t('aiworkspace.data.cell.annotateAria', { field: field.name, record: row.record_id })}
          onClick={() => {
            const label = `${field.name} · ${row.record_id}`
            const exact = Array.from(formatValue(field, value)).slice(0, 200).join('')
            onAnnotate({
              target: { entity_id: sourceId, selector: { kind: 'table_cell', record_id: row.record_id, field_id: field.field_id } },
              context: exact.trim() ? { quote: { exact }, label } : { label },
              label,
            })
          }}>✎</button>
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

interface ConflictBoxProps {
  store: WorkspaceStore
  editId: string
  theirs: string | null
  mine: string | null
  detail: string
  onKeepMine?: () => void
  onEditAgain?: () => void
}

/** Conflict / refusal: both values stay visible and my input is kept until I choose (design §6.5). */
export function ConflictBox({ store, editId, theirs, mine, detail, onKeepMine, onEditAgain }: ConflictBoxProps) {
  const { t, tNode } = useAiwsText()
  return (
    <div className="aiws-conflict" role="alertdialog" aria-label={t('aiworkspace.data.conflict.label')} data-testid="aiws-conflict">
      <div className="aiws-conflict-detail">{detail}</div>
      {theirs !== null && <div>{tNode('aiworkspace.data.conflict.theirs', { value: <b data-testid="aiws-conflict-theirs">{theirs}</b> })}</div>}
      {mine !== null && <div>{tNode('aiworkspace.data.conflict.mine', { value: <b data-testid="aiws-conflict-mine">{mine}</b> })}</div>}
      <div className="aiws-conflict-actions">
        {onKeepMine && <button type="button" data-testid="aiws-conflict-keep-mine" onClick={onKeepMine}>{t('aiworkspace.data.conflict.keepMine')}</button>}
        {onEditAgain && <button type="button" onClick={onEditAgain}>{t('aiworkspace.data.conflict.editAgain')}</button>}
        <button type="button" data-testid="aiws-conflict-discard" onClick={() => { void store.dismissEdit(editId) }}>{theirs !== null ? t('aiworkspace.data.conflict.useTheirs') : t('aiworkspace.data.conflict.discardMine')}</button>
      </div>
    </div>
  )
}

function FilterSortPanel({ fields, session, hasSavedFilter, hasSavedSorts, canSave, onApply, onSave, onClearSaved }: {
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
  const current = session.filter && session.filter.op === 'cmp' ? session.filter : null
  const [fieldId, setFieldId] = useState(current?.field_id ?? fields[0]?.field_id ?? '')
  const field = fields.find((item) => item.field_id === fieldId)
  const operators = field ? operatorsFor(field.type) : []
  const [operator, setOperator] = useState(current?.operator ?? operators[0]?.id ?? 'eq')
  const [text, setText] = useState(current && current.value !== undefined ? String(current.value) : '')
  const sortable = fields.filter((item) => SORTABLE.has(item.type))
  const [sortField, setSortField] = useState(session.sorts?.[0]?.field_id ?? '')
  const [direction, setDirection] = useState<'asc' | 'desc'>(session.sorts?.[0]?.direction ?? 'asc')
  const activeOperator = operators.find((item) => item.id === operator) ?? operators[0]

  const apply = () => {
    let filter: FilterNode | null = null
    if (field && activeOperator && (!activeOperator.needsValue || text !== '')) {
      let value: Json | undefined
      if (activeOperator.needsValue) {
        if (field.type === 'number') value = Number(text)
        else if (field.type === 'boolean') value = text === 'true'
        else if (field.type === 'multi_select') value = text.split(',').map((item) => item.trim()).filter(Boolean)
        else value = text
      }
      filter = { op: 'cmp', field_id: field.field_id, operator: activeOperator.id, ...(value !== undefined ? { value } : {}) }
    }
    onApply({ filter, sorts: sortField ? [{ field_id: sortField, direction }] : null })
  }

  return (
    <div className="aiws-inline-form aiws-filter-panel">
      <div>
        <span>{t('aiworkspace.data.filter.temporaryFilter')}</span>
        <select aria-label={t('aiworkspace.data.filter.field')} value={fieldId} onChange={(event) => { setFieldId(event.target.value); const next = fields.find((item) => item.field_id === event.target.value); setOperator(next ? operatorsFor(next.type)[0].id : 'eq'); setText('') }}>
          {fields.map((item) => <option key={item.field_id} value={item.field_id}>{item.name}</option>)}
        </select>
        <select aria-label={t('aiworkspace.data.filter.operator')} value={activeOperator?.id ?? ''} onChange={(event) => setOperator(event.target.value)}>
          {operators.map((item) => <option key={item.id} value={item.id}>{item.label}</option>)}
        </select>
        {activeOperator?.needsValue && (field?.type === 'select' || field?.type === 'boolean' ? (
          <select aria-label={t('aiworkspace.data.filter.value')} value={text} onChange={(event) => setText(event.target.value)}>
            <option value="">{t('aiworkspace.data.filter.choose')}</option>
            {field.type === 'boolean'
              ? [<option key="t" value="true">{t('aiworkspace.common.yes')}</option>, <option key="f" value="false">{t('aiworkspace.common.no')}</option>]
              : (field.options ?? []).map((option) => <option key={option.option_id} value={option.option_id}>{option.label}</option>)}
          </select>
        ) : (
          <input aria-label={t('aiworkspace.data.filter.value')} type={field?.type === 'date' ? 'date' : 'text'} value={text} onChange={(event) => setText(event.target.value)} placeholder={field?.type === 'multi_select' ? t('aiworkspace.data.filter.optionIds') : t('aiworkspace.data.filter.valuePlaceholder')} />
        ))}
      </div>
      <div>
        <span>{t('aiworkspace.data.filter.temporarySort')}</span>
        <select aria-label={t('aiworkspace.data.filter.sortField')} value={sortField} onChange={(event) => setSortField(event.target.value)}>
          <option value="">{t('aiworkspace.data.filter.keepViewSort')}</option>
          {sortable.map((item) => <option key={item.field_id} value={item.field_id}>{item.name}</option>)}
        </select>
        <select aria-label={t('aiworkspace.data.filter.sortDirection')} value={direction} onChange={(event) => setDirection(event.target.value === 'desc' ? 'desc' : 'asc')}>
          <option value="asc">{t('aiworkspace.data.filter.ascending')}</option><option value="desc">{t('aiworkspace.data.filter.descending')}</option>
        </select>
      </div>
      <div>
        <button type="button" data-testid="aiws-filter-apply" onClick={apply}>{t('aiworkspace.data.filter.apply')}</button>
        <button type="button" onClick={() => { onApply({ filter: null, sorts: null }); setText(''); setSortField('') }}>{t('aiworkspace.data.filter.clear')}</button>
        {canSave && <button type="button" data-testid="aiws-view-save" disabled={!session.filter && !session.sorts} onClick={onSave} title={t('aiworkspace.data.filter.saveTitle')}>{t('aiworkspace.data.filter.save')}</button>}
        {canSave && hasSavedFilter && <button type="button" onClick={() => onClearSaved('filter')}>{t('aiworkspace.data.filter.removeSavedFilter')}</button>}
        {canSave && hasSavedSorts && <button type="button" onClick={() => onClearSaved('sorts')}>{t('aiworkspace.data.filter.removeSavedSort')}</button>}
      </div>
      <div className="aiws-muted">{t('aiworkspace.data.filter.hint')}</div>
    </div>
  )
}

function AddRecordForm({ store, source, onDone }: { store: WorkspaceStore; source: SourceRead; onDone: () => void }) {
  const { t } = useAiwsText()
  const fields = source.content.fields
  const asked = fields.filter((field) => field.required || field.field_id === source.content.payload.title_field_id)
  const [texts, setTexts] = useState<Record<string, string>>({})
  const [error, setError] = useState<string | null>(null)
  const submit = async () => {
    const values: Record<string, Json> = {}
    const typeRevs: Record<string, number> = {}
    for (const field of asked) {
      const text = texts[field.field_id] ?? ''
      if (text === '') {
        if (field.required) { setError(t('aiworkspace.data.record.required', { name: field.name })); return }
        continue
      }
      values[field.field_id] = field.type === 'number' ? Number(text) : field.type === 'boolean' ? text === 'true' : text
      typeRevs[field.field_id] = field.type_rev
    }
    const recordId = randomId('r')
    const outcome = await store.submit({
      editId: `record:${source.entity_id}:${recordId}`, label: t('aiworkspace.data.record.add'), mine: values, hasMine: true,
      operations: [{ op: 'table.insert_records', source_id: source.entity_id, field_type_revs: typeRevs, records: [{ record_id: recordId, values }] }],
    })
    if (outcome.status === 'accepted') onDone()
    else setError(t('aiworkspace.data.record.addFailed'))
  }
  return (
    <form className="aiws-inline-form" onSubmit={(event) => { event.preventDefault(); void submit() }}>
      {asked.length === 0 && <span className="aiws-muted">{t('aiworkspace.data.record.noRequired')}</span>}
      {asked.map((field, index) => (
        <label key={field.field_id}>{field.name}{field.required ? ' *' : ''}
          {field.type === 'select' ? (
            <select value={texts[field.field_id] ?? ''} onChange={(event) => setTexts({ ...texts, [field.field_id]: event.target.value })}>
              <option value="">{t('aiworkspace.data.value.notSet')}</option>
              {(field.options ?? []).map((option) => <option key={option.option_id} value={option.option_id}>{option.label}</option>)}
            </select>
          ) : (
            <input aria-label={t('aiworkspace.data.record.newAria', { name: field.name })} autoFocus={index === 0} type={field.type === 'date' ? 'date' : 'text'} value={texts[field.field_id] ?? ''} onChange={(event) => setTexts({ ...texts, [field.field_id]: event.target.value })} />
          )}
        </label>
      ))}
      <button type="submit" data-testid="aiws-add-record-submit">{t('aiworkspace.data.record.insert')}</button>
      <button type="button" onClick={onDone}>{t('aiworkspace.common.cancel')}</button>
      {error && <span className="aiws-error" role="alert">{error}</span>}
    </form>
  )
}
