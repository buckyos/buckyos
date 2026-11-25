/* The standard table editor (UI 框架 PRD §7.1–§7.3): one editor for a TableSource, used by a table Block (on a
 * canvas or a page), by a data-mode tab and by a phone's full screen. Its title row has the filter, the fields and ⋯;
 * the conditions in force are chips under it; the content is a grid or cards (a phone starts with cards, a computer
 * with the grid, the choice is each person's); the footer counts the loaded records. A
 * record opens in its form (a side panel, or the whole phone screen); records are selected by ticking (computer) or
 * a long press (phone) and the bar at the bottom deletes or copies them. Every write keeps the rules of the grid:
 * capability, the read-only reason, undo, the save state. */

import { createPortal } from 'react-dom'
import { useContext, useEffect, useEffectEvent, useMemo, useRef, useState, useSyncExternalStore, type ReactNode } from 'react'
import { ClipboardCopy, Columns3, LayoutGrid, ListChecks, ListFilter, Lock, MoreHorizontal, Plus, Rows3, X } from 'lucide-react'
import { LockScope } from '../blocks/editors'
import { BlockTitle } from '../blocks/BlockTitle'
import { useFlowBlock } from '../blocks/flowContext'
import { TableBlockContext } from './TableBlockContext'
import { TableHeading } from './TableHeading'
import { tableAppearance, cleanRowHeights } from './appearance'
import { placeRow, rowOrderOp } from './rowOrder'
import { insertRecordOp, missingRecordFields } from './insert'
import { randomId } from '../../api/ids'
import type { ReadOk } from '../../api/session'
import type { CellPayload, CommitOutcome, EntityEnvelope, RowOrder, FilterNode, Json, KeyedContent, QueryRow, SortSpec, TableSourceContent } from '../../api/types'
import type { SubmitOptions } from '../../state/store'
import type { CapturedAnchor } from '../../anchors/registry'
import { useEntity, useLoad, useReadOnlyReason, useStore, useUserState, useVersion, type AnnotationMark } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { consumeIntent, requestIntent, useEditorToolbar, useIntent, type TableIntent } from '../blocks/editorToolbar'
import type { ToolbarItem } from '../blocks/registry'
import { annotatedRecords, useRecordFocus } from '../controls/data/focus'
import { FieldManager } from '../FieldManager'
import { quickColumnOps, insertColumnId } from '../fieldOps'
import { COPY_LIMIT, tableClipboard } from '../import/copyTable'
import { usePhone } from '../shell/shellContext'
import { TablePager, type PagerQuery } from '../tablePager'
import { PhoneEditBar } from './EditBar'
import { FilterSortPanel } from './FilterPanel'
import {
  blockColumns, conditionText, conditionsOf, listedOf, personalColumns, personalOf, recordTitle, sortText, titleFieldOf, withoutCondition,
  type Column, type ColumnEntry, type PersonalColumns, type SessionView, type TableEditorSize, type TableLayout,
} from './model'
import { publishTableCurrent } from './current'
import { TableRecordPanel, type RecordPanelState } from './RecordPanel'
import { TableCards } from './TableCards'
import { TableGrid, type CellRef, type TableGridProps } from './TableGrid'
import { ConfirmPop, TableMenu, TablePop, anchorOf, usePop, type MenuItem } from './TablePop'
import './table.css'

export type CellRead = ReadOk<KeyedContent<CellPayload>>
type SourceRead = ReadOk<TableSourceContent>
/** "Show the first N rows" of a table view (PRD §4.3, §6 Q9). */
const DEFAULT_ROW_LIMIT = 100

export interface TableEditorBodyProps {
  /** The table-view Block, or the table's synthetic view (`sourceMode`). */
  cell: CellRead
  lockSource?: EntityEnvelope
  sourceMode?: boolean
  size: TableEditorSize
  awaitingLock?: boolean
  readOnly: boolean
  readOnlyReason?: string | null
  compact?: boolean
  annotations: AnnotationMark[]
  onAnnotate?: (anchor: CapturedAnchor) => void
  onActivateAnnotation?: (entityId: string | null) => void
}

/** What this person chose for this view (user work state): the presentation per device; without a Block also the
 * temporary filter and sort and the columns. */
interface TablePref extends PersonalColumns {
  row_order?: RowOrder
  config?: Record<string, Json>
  layout?: { phone?: TableLayout; desktop?: TableLayout }
  filter?: FilterNode | null
  sorts?: SortSpec[] | null
}

type Panel = 'none' | 'filter' | 'fields'

function useTableData({ cell, sourceMode, compact }: TableEditorBodyProps) {
  const store = useStore()
  const cellId = cell.entity_id
  const payload = cell.content.payload
  const sourceId = payload.source_ref?.entity_id ?? ''
  const sourceVersion = useVersion(`e:${sourceId}`)
  const loadSource = useMemo(() => () => store.session.read<TableSourceContent>(sourceId), [store, sourceId])
  const source = useLoad<SourceRead>(loadSource, sourceVersion)

  // ---- personal state: a Block's `block:<id>/table`; the table itself keeps its old `tableview:<id>` entry (phase two §6.1)
  const prefKey = sourceMode ? `tableview:${sourceId}` : `block:${cellId}/table`
  const pref = useUserState<Json>(prefKey) as TablePref | undefined
  const writePref = (change: Partial<TablePref>) => {
    const next: Record<string, unknown> = { ...((store.userState.get<Json>(prefKey) as TablePref | undefined) ?? {}), ...change }
    for (const key of Object.keys(next)) if (next[key] === null || next[key] === undefined) delete next[key]
    store.userState.set(prefKey, Object.keys(next).length > 0 ? (next as Json) : null)
  }
  // session-level filter and sort (design §3.5.2): not a commit until "save view"; without a Block in the user state
  const [localView, setLocalView] = useState<SessionView>({ filter: null, sorts: null })
  if (compact && (localView.filter || localView.sorts)) setLocalView({ filter: null, sorts: null })
  const sessionView: SessionView = sourceMode ? { filter: pref?.filter ?? null, sorts: pref?.sorts ?? null } : localView
  const setSessionView = (view: SessionView) => { if (sourceMode) writePref({ filter: view.filter, sorts: view.sorts }); else setLocalView(view) }

  // ---- rows
  const rowOrder = sourceMode ? pref?.row_order : payload.row_order
  const savedKey = JSON.stringify([payload.filter ?? null, payload.sorts ?? null, payload.fields ?? null, rowOrder ?? null])
  const sessionKey = JSON.stringify(sessionView)
  const { pager, viewportRef } = useMemo(() => {
    const saved = JSON.parse(savedKey) as [FilterNode | null, SortSpec[] | null, unknown, RowOrder | null]
    const session = JSON.parse(sessionKey) as SessionView
    const query: PagerQuery = {
      viewId: sourceMode ? undefined : cellId, sourceId, rowOrder: sourceMode ? saved[3] ?? undefined : undefined, filter: session.filter, sorts: session.sorts,
      orderDependsOnData: Boolean(saved[0] || (saved[1] && saved[1].length > 0) || session.filter || (session.sorts && session.sorts.length > 0)),
    }
    return { pager: new TablePager(store.session, query), viewportRef: { current: { left: 0, top: 0 } } }
  }, [store, cellId, sourceId, savedKey, sessionKey, sourceMode])
  const rows = useSyncExternalStore(pager.subscribe, pager.snapshot)
  // a Block may show only the first rows of its view (B7-3): a display limit, the data keeps every row
  const savedLimit = (payload as { row_limit?: unknown }).row_limit
  const rowLimit = !sourceMode && typeof savedLimit === 'number' ? savedLimit : null
  const limited = rowLimit !== null && rows.total > rowLimit
  const shown = limited ? { ...rows, rows: rows.rows.slice(0, rowLimit), total: rowLimit, done: rows.done || rows.rows.length >= rowLimit } : rows
  // A schema change (field added, renamed, deleted, migrated) changes what a row means: read again.
  const schemaKey = (source.data?.content.fields ?? []).map((field) => `${field.field_id}:${field.def_rev}`).join(',')
  const lastSchema = useRef<string | null>(null)
  useEffect(() => {
    if (lastSchema.current !== null && lastSchema.current !== schemaKey) pager.refresh()
    lastSchema.current = schemaKey
  }, [schemaKey, pager])

  return {
    cellId, payload, sourceId, source, prefKey, pref, writePref, sessionView, setSessionView, sessionKey,
    rowOrder, pager, viewportRef, rows, rowLimit, limited, shown,
  }
}

export function TableEditorBody(props: TableEditorBodyProps) {
  const data = useTableData(props)
  const session = <TableEditorSession {...props} data={data} />
  return props.compact ? session : <LockScope entity={props.lockSource} intentKey={props.cell.entity_id}>{session}</LockScope>
}

function TableEditorSession({ cell, sourceMode, size, awaitingLock, readOnly, readOnlyReason, compact, data, annotations, onAnnotate, onActivateAnnotation }: TableEditorBodyProps & { data: ReturnType<typeof useTableData> }) {
  const { t } = useAiwsText()
  const store = useStore()
  const phone = usePhone()
  const block = useContext(TableBlockContext)
  const flow = useFlowBlock()
  const titleRelease = useRef<number | undefined>(undefined)
  useEffect(() => () => window.clearTimeout(titleRelease.current), [])
  const interactiveStatic = Boolean(compact && block?.mode === 'edit' && block.depth === 0)
  const activeBlock = (action: TableIntent) => {
    if (!interactiveStatic || !block) return
    requestIntent(block.cell.entity_id, action)
    block.activateEditor()
  }
  const {
    cellId, payload, sourceId, source, prefKey, pref, writePref, sessionView, setSessionView, sessionKey,
    rowOrder, pager, viewportRef, rows, rowLimit, limited, shown,
  } = data
  const entity = useEntity(sourceId)
  const directReason = useReadOnlyReason()
  const device = phone ? 'phone' : 'desktop'
  const layout: TableLayout = pref?.layout?.[device] ?? (phone ? 'cards' : 'grid')
  const setLayout = (next: TableLayout) => writePref({ layout: { ...(pref?.layout ?? {}), [device]: next } })

  // ---- what is open
  const [panel, setPanel] = useState<Panel>('none')
  const [record, setRecord] = useState<RecordPanelState | null>(null)
  const [selection, setSelection] = useState<Set<string> | null>(null)
  const [phoneCell, setPhoneCell] = useState<CellRef | null>(null)
  const [editBar, setEditBar] = useState<CellRef | null>(null)
  const pops = usePop<'menu' | 'delete-selected'>()
  // a toolbar button pressed while the Block was only selected names what to open (标准对象的交互改进 §5.2)
  const intentKey = compact ? '' : cellId
  const intent = useIntent(intentKey)
  const [intentSeen, setIntentSeen] = useState(0)
  const [askedCopy, setAskedCopy] = useState(0)
  if (intent && !intent.table && intent.value !== 'add' && intent.seq !== intentSeen) {
    setIntentSeen(intent.seq)
    if (intent.value === 'copy') setAskedCopy(intent.seq)
    else if (intent.value === 'select') setSelection(new Set())
    else if (intent.value === 'filter' || intent.value === 'fields') setPanel(intent.value)
  }

  // a record's link (控件机制 G12) opens its form
  useRecordFocus(intentKey, (focus) => setRecord({ kind: 'edit', recordId: focus.record }))

  // ---- columns: a Block's view lists them; the table itself, this person
  const fields = source.data?.content.fields
  const entries: ColumnEntry[] = useMemo(() => (!fields ? [] : sourceMode ? personalColumns(fields, pref ?? {}) : blockColumns(fields, payload.fields)), [fields, sourceMode, pref, payload.fields])
  const columns: Column[] = entries.filter((entry) => entry.shown).map(({ field, width }) => ({ field, width }))
  const titleField = fields ? titleFieldOf(fields, source.data?.content.payload.title_field_id) : undefined

  // ---- what may be done here, and why not
  const caps = source.data?.capabilities ?? []
  const external = Boolean(entity?.source ?? source.data?.source)
  const canWriteContent = !external && !readOnly
  const canUpdate = canWriteContent && caps.includes('update')
  const canAppend = canWriteContent && caps.some((capability) => capability === 'append' || capability === 'update')
  const canDelete = canWriteContent && caps.includes('delete')
  const canStructure = canWriteContent && caps.includes('structure')
  const canEditView = (interactiveStatic ? block?.readOnlyReason === null : !readOnly) && !sourceMode && cell.capabilities.includes('update')
  const reason = readOnlyReason ?? directReason ?? (external ? t('aiworkspace.table.readOnly.external')
    : source.data && !caps.some((capability) => capability === 'update' || capability === 'append') ? t('aiworkspace.table.readOnly.noPermission') : null)
  const columnsReason = sourceMode ? null : canEditView ? null : t('aiworkspace.table.readOnly.noView')
  const structureReason = canStructure ? null : reason ?? t('aiworkspace.table.readOnly.noStructure')
  const config = sourceMode ? pref?.config : payload.config
  const appearance = useMemo(() => tableAppearance(config), [config])
  const accepted = (outcome: CommitOutcome) => outcome.status === 'accepted' || outcome.status === 'saved_locally'
  const saveAppearance = async (patch: Record<string, Json>, height?: { id: string; px: number }) => {
    if (!sourceMode && !canEditView) return
    const visible = new Set(rows.rows.map((r) => r.record_id))
    let current = cell
    for (let attempt = 0; attempt < 2; attempt++) {
      const config = sourceMode ? (store.userState.get<Json>(prefKey) as TablePref | undefined)?.config : current.content.payload.config
      const next = { ...config, ...patch }
      const heights = tableAppearance(next).rowHeights
      if (height) heights[height.id] = height.px
      if (Object.keys(heights).length) next.row_heights = await cleanRowHeights(store.session, sourceId, heights, visible)
      if (sourceMode) { writePref({ config: next }); return }
      const result = await store.submit({ editId: `view:${cellId}:config`, label: t('aiworkspace.table.properties'), operations: [
        { op: 'entity.set_keys', entity_id: cellId, keys: [{ key: 'config', value: next, expect: { rev: current.content.key_revs.config ?? 0 } }] },
      ] })
      if (result.status !== 'conflict') return
      current = await store.session.read<KeyedContent<CellPayload>>(cellId)
    }
  }
  const saveColumns = (next: ColumnEntry[]) => {
    if (sourceMode) { writePref(personalOf(next)); return }
    if (!canEditView) { store.notify('info', columnsReason ?? ''); return }
    void store.submit({ editId: `view:${cellId}`, label: t('aiworkspace.data.view.columnsEdit'), operations: [{ op: 'entity.set_keys', entity_id: cellId, keys: [{ key: 'fields', value: listedOf(next) as unknown as Json, expect: { rev: cell.content.key_revs.fields ?? 0 } }] }] })
      .then((result) => { if (result.status === 'conflict') store.versions.bump([`e:${cellId}`]) })
  }

  /** "Copy as table" (表格数据导入 TODO §4.1): the shown columns, every row of the view's filter and order (≤ 20 000),
   * or the selected rows. The clipboard item is written at once (the click's user activation) with its data still being read. */
  const copyRows = (only: ReadonlySet<string> | null) => {
    const shown = columns.map((column) => column.field)
    const data = only
      ? Promise.resolve(rows.rows.filter((row) => row && only.has(row.record_id))).then((picked) => ({ ...tableClipboard(shown, picked), count: picked.length, total: picked.length }))
      : pager.fetchAll(COPY_LIMIT).then(({ rows: all, total }) => ({ ...tableClipboard(shown, all), count: all.length, total }))
    const blob = (type: string, pick: (d: { text: string; html: string }) => string) => data.then((d) => new Blob([pick(d)], { type }))
    let written: Promise<void>
    try {
      written = navigator.clipboard.write([new ClipboardItem({ 'text/plain': blob('text/plain', (d) => d.text), 'text/html': blob('text/html', (d) => d.html) })])
    } catch (error) {
      written = Promise.reject(error)
    }
    void written.then(() => data).then(
      (d) => store.notify('info', d.total > d.count ? t('aiworkspace.data.table.copiedPart', { count: d.count, total: d.total }) : t('aiworkspace.data.table.copied', { count: d.count })),
      (error: unknown) => store.notify('error', t('aiworkspace.data.table.copyFailed', { reason: error instanceof Error ? error.message : String(error) })))
  }
  // the Block toolbar's "copy as table" reaches the editor as an intent (once per request: effects run twice on a development mount)
  const copyRef = useRef(copyRows)
  useEffect(() => { copyRef.current = copyRows })
  const copiedFor = useRef(0)
  const columnsReady = columns.length > 0
  useEffect(() => { if (askedCopy && columnsReady && copiedFor.current !== askedCopy) { copiedFor.current = askedCopy; copyRef.current(null) } }, [askedCopy, columnsReady])

  // ---- selection
  const toggle = (id: string, on: boolean) => setSelection((current) => { const next = new Set(current ?? []); if (on) next.add(id); else next.delete(id); return next })
  const toggleAll = (on: boolean) => setSelection(on ? new Set(rows.rows.filter(Boolean).map((row) => row.record_id)) : new Set())
  const deleteSelected = async () => {
    if (!selection || selection.size === 0) return
    const picked = rows.rows.filter((row) => row && selection.has(row.record_id))
    const outcome = await store.submit({
      editId: `records:${sourceId}:${randomId().slice(0, 6)}`, label: t('aiworkspace.table.selection.deleteEdit', { count: picked.length }),
      operations: [{ op: 'table.delete_records', source_id: sourceId, records: picked.map((row) => ({ record_id: row.record_id, expect: { rev: row.rev } })) }],
    })
    if (outcome.status === 'accepted' || outcome.status === 'saved_locally') setSelection(null)
  }

  const setRowLimit = (limit: number | null) => store.submit({
    editId: `view:${cellId}`, label: t('aiworkspace.datasource.rows.edit'),
    operations: [limit === null
      ? { op: 'entity.unset_keys', entity_id: cellId, keys: [{ key: 'row_limit', expect: { rev: cell.content.key_revs.row_limit ?? 0 } }] }
      : { op: 'entity.set_keys', entity_id: cellId, keys: [{ key: 'row_limit', value: limit, expect: { rev: cell.content.key_revs.row_limit ?? 0 } }] }],
  })

  type Side = 'before' | 'after'
  const sorted = Boolean((sessionView.sorts?.length ?? 0) || payload.sorts?.length)
  const schemaId = source.data?.schema_id ?? entity?.schema_id
  const staticWrite = interactiveStatic && block?.readOnlyReason === null && !external
  const allowRow = canAppend || (staticWrite && caps.some((cap) => cap === 'append' || cap === 'update'))
  const canChangeFields = canStructure || (staticWrite && caps.includes('structure'))
  const allowColumn = canChangeFields && (!payload.fields?.length || canEditView || sourceMode)
  const allowMove = (!compact && !readOnly || staticWrite) && !external && (sourceMode || canEditView) && !sorted
  const [focusRequest, setFocusRequest] = useState<TableGridProps['focusRequest']>()
  const [addedOutside, setAddedOutside] = useState<string | null>(null)
  const anchor = useRef<{ id: string; side: Side; index?: number } | null>(null)
  const inserting = useRef(false)
  type PendingInsert = { id: string; submission: SubmitOptions; expectedIndex: number; personalRank?: string }
  const unresolved = useRef<(PendingInsert & { key: string }) | null>(null)
  const [pendingRow, setPendingRow] = useState<{ id: string; before: typeof rows.rows; expectedIndex: number } | null>(null)
  useEffect(() => {
    if (pendingRow) pager.ensure(Math.min(pendingRow.expectedIndex, rowLimit === null ? Infinity : rowLimit - 1))
  }, [pager, pendingRow, rowLimit])
  if (pendingRow && !rows.loading && (shown.done || shown.rows.length > pendingRow.expectedIndex) && pendingRow.before !== rows.rows) {
    setPendingRow(null)
    const actual = shown.rows.findIndex((r) => r.record_id === pendingRow.id)
    if (actual >= 0 && (!sorted || actual === pendingRow.expectedIndex)) setFocusRequest((f) => ({ recordId: pendingRow.id, action: 'focus', seq: (f?.seq ?? 0) + 1 }))
    else setAddedOutside(pendingRow.id)
  }
  const prepareInsert = async (recordId: string, values: Record<string, Json>, definitions: NonNullable<typeof fields>): Promise<PendingInsert> => {
    const ops = [insertRecordOp(sourceId, definitions, recordId, values)]
    const target = anchor.current
    let personalRank: string | undefined
    if (target && !sorted) {
      if (!sourceMode && !canEditView) throw new Error(t('aiworkspace.table.readOnly.noView'))
      const row = rows.rows.find((r) => r.record_id === target.id) ?? (await store.session.read<QueryRow>(sourceId, { kind: 'table_record', record_id: target.id })).content
      const next = placeRow(store.core, rowOrder, recordId, row, target.side)
      if (sourceMode) personalRank = next[recordId]
      else ops.push(rowOrderOp(cell, next))
    }
    const submission = { editId: `record:${sourceId}:${recordId}`, label: t('aiworkspace.data.record.add'), operations: ops, mine: values, hasMine: true }
    const expectedIndex = target ? (target.index ?? rows.rows.findIndex((r) => r.record_id === target.id)) + Number(target.side === 'after')
      : rows.rows.length || rows.done ? rows.total : source.data?.content.record_count ?? 0
    return { id: recordId, submission, personalRank, expectedIndex }
  }
  const insert = async (recordId: string, values: Record<string, Json>) => {
    if (!fields || !canAppend) throw new Error(reason ?? t('aiworkspace.blocks.noPermission'))
    const prior = unresolved.current
    const request = prior ?? await prepareInsert(recordId, values, fields)
    const outcome = prior ? await store.retry(prior.submission, prior.key) : await store.submit(request.submission)
    if (outcome.status === 'unknown') {
      unresolved.current = { ...request, key: outcome.idempotency_key }
      store.notify('error', outcome.detail)
      return outcome
    }
    unresolved.current = null
    if (accepted(outcome)) {
      if (request.personalRank) {
        const current = store.userState.get<Json>(prefKey) as TablePref | undefined
        writePref({ row_order: { ...current?.row_order, [request.id]: request.personalRank } })
      }
      setAddedOutside(null)
      setPendingRow({ id: request.id, before: pager.snapshot().rows, expectedIndex: request.expectedIndex })
      pager.refresh()
    } else if (outcome.status === 'conflict') store.versions.bump([`e:${cellId}`])
    return outcome
  }
  const insertRow = async (anchorId?: string, side: Side = 'after', rowIndex?: number) => {
    const index = rowIndex ?? (anchorId ? rows.rows.findIndex((r) => r.record_id === anchorId) : undefined)
    if (interactiveStatic) { activeBlock({ action: 'insertRow', recordId: anchorId, side, rowIndex: index }); return }
    if (!canAppend || !fields || inserting.current || record?.kind === 'new') return
    anchor.current = anchorId ? { id: anchorId, side, index } : null
    if (missingRecordFields(store.core, fields, schemaId).length) { setRecord({ kind: 'new' }); return }
    inserting.current = true
    try { await insert(unresolved.current?.id ?? randomId('r'), {}) }
    catch (e) { store.notify('error', String(e)) }
    finally { inserting.current = false }
  }
  const insertColumn = async (anchorId?: string, side: Side = 'after') => {
    if (interactiveStatic) { activeBlock({ action: 'insertColumn', fieldId: anchorId, side }); return }
    if (!allowColumn || !fields) return
    const { fieldId, operations } = quickColumnOps(store.core, sourceId, cell, fields, anchorId, side)
    const result = await store.submit({ editId: `schema:${sourceId}:${fieldId}`, label: t('aiworkspace.data.field.addColumn'), operations })
    if (accepted(result) && sourceMode) writePref({ order: insertColumnId(entries.map((e) => e.field.field_id), fieldId, anchorId, side) })
    if (result.status === 'conflict') store.versions.bump([`e:${cellId}`, `e:${sourceId}`])
  }
  const moveRow = async (id: string, targetId: string, side: Side) => {
    const row = pager.snapshot().rows.find((r) => r.record_id === targetId)
    if (!row || !allowMove) return
    const next = placeRow(store.core, rowOrder, id, row, side)
    if (sourceMode) writePref({ row_order: next })
    else {
      const result = await store.submit({ editId: `view:${cellId}:row_order`, label: t('aiworkspace.table.moveRow'), operations: [rowOrderOp(cell, next)] })
      if (result.status === 'conflict') store.versions.bump([`e:${cellId}`])
    }
  }
  const readyIntent = fields && (!readOnly || (!intent?.table && intent?.value !== 'add')) ? intent : undefined
  if (readyIntent?.table && readyIntent.seq !== intentSeen) {
    const task = readyIntent.table
    setIntentSeen(readyIntent.seq)
    if (task.action === 'edit' || task.action === 'rename') {
      setFocusRequest({ action: task.action, recordId: task.recordId ?? '', fieldId: task.fieldId, seq: (focusRequest?.seq ?? 0) + 1 })
    }
    if (task.action === 'openRecord' && task.recordId) setRecord({ kind: 'edit', recordId: task.recordId })
  }
  const addByIntent = useEffectEvent(async (task: TableIntent) => {
    if (task.action === 'insertColumn') await insertColumn(task.fieldId, task.side)
    else await insertRow(task.recordId, task.side, task.rowIndex)
  })
  const consumed = useRef(0)
  useEffect(() => {
    if (!readyIntent || consumed.current === readyIntent.seq) return
    consumed.current = readyIntent.seq
    consumeIntent(intentKey, readyIntent)
    const task = readyIntent.value === 'add' ? { action: 'insertRow' as const } : readyIntent.table
    if (task?.rowIndex !== undefined) pager.ensure(task.rowIndex)
    // eslint-disable-next-line react-hooks/set-state-in-effect -- deliver the pending canvas click once the editor holds its lock
    if (task?.action === 'insertColumn' || task?.action === 'insertRow') void addByIntent(task).catch((e) => store.notify('error', String(e)))
  }, [readyIntent, intentKey, pager, store])
  // ---- the title row's tools: in the near toolbar on a free canvas, drawn here elsewhere
  const togglePanel = (next: Panel) => setPanel(panel === next ? 'none' : next)
  const filtered = Boolean(sessionView.filter || sessionView.sorts)
  const canFields = canStructure || columnsReason === null
  const filterLabel = filtered ? t('aiworkspace.data.table.filterSortSession') : payload.filter || payload.sorts?.length ? t('aiworkspace.data.table.filterSortSaved') : t('aiworkspace.data.table.filterSort')
  const menuItems: (MenuItem | 'separator')[] = [
    layout === 'grid'
      ? { id: 'table-layout-cards', label: t('aiworkspace.table.layout.showCards'), icon: <LayoutGrid size={14} />, run: () => setLayout('cards') }
      : { id: 'table-layout-grid', label: t('aiworkspace.table.layout.showGrid'), icon: <Rows3 size={14} />, run: () => setLayout('grid') },
    ...(allowRow ? [{ id: 'table-add', label: t('aiworkspace.table.newRecord'), icon: <Plus size={14} />, run: () => { void insertRow() } }] : []),
    ...(phone && canFields ? [{ id: 'table-fields', label: t('aiworkspace.data.table.fields'), icon: <Columns3 size={14} />, run: () => togglePanel('fields') }] : []),
    { id: 'table-select', label: t('aiworkspace.table.selection.start'), icon: <ListChecks size={14} />, run: () => setSelection(new Set()) },
    { id: 'table-copy', label: t('aiworkspace.data.table.copyAsTable'), icon: <ClipboardCopy size={14} />, run: () => copyRows(null) },
    ...(canEditView ? [{ id: 'table-rows', label: rowLimit ? t('aiworkspace.datasource.rows.all') : t('aiworkspace.datasource.rows.first', { count: DEFAULT_ROW_LIMIT }), icon: <Rows3 size={14} />, run: () => { void setRowLimit(rowLimit ? null : DEFAULT_ROW_LIMIT) } }] : []),
  ]
  const tools: ToolbarItem[] | null = compact || !source.data ? null : [
    { kind: 'button', id: 'table-filter', icon: ListFilter, label: filterLabel, active: panel === 'filter' || filtered, run: () => togglePanel('filter') },
    ...(canFields ? [{ kind: 'button' as const, id: 'table-fields', icon: Columns3, label: t('aiworkspace.data.table.fields'), active: panel === 'fields', run: () => togglePanel('fields') }] : []),
    ...(canAppend ? [{ kind: 'button' as const, id: 'table-add', icon: Plus, label: t('aiworkspace.data.record.add'), active: record?.kind === 'new', run: () => { void insertRow() } }] : []),
    { kind: 'button', id: 'table-copy', icon: ClipboardCopy, label: t('aiworkspace.data.table.copyAsTable'), run: () => copyRows(null) },
    { kind: 'menu', id: 'table-more', icon: MoreHorizontal, label: t('aiworkspace.table.more'), onPick: (value) => (menuItems.find((item) => item !== 'separator' && item.id === value) as MenuItem | undefined)?.run(),
      // eslint-disable-next-line react-hooks/refs -- only labels and IDs are read here; commands run on activation
      items: menuItems.flatMap((item) => (item === 'separator' || item.id === 'table-copy' || item.id === 'table-fields' || item.disabled ? [] : [{ value: item.id, label: item.label }])) },
  ]
  const inNearToolbar = useEditorToolbar('table', tools)

  const scrollRef = useRef<HTMLDivElement>(null)
  const [root, bindRoot] = useState<HTMLDivElement | null>(null)
  const host = (root?.closest('.aiws-root') as HTMLElement | null) ?? null
  const annotatedSet = useMemo(() => annotatedRecords(annotations, sourceId), [annotations, sourceId])
  // an open record follows its row (a refused write keeps it; a deleted record closes it once it is gone from the view)
  const editBarRow = editBar ? rows.rows.find((row) => row?.record_id === editBar.recordId) : undefined
  const editBarField = editBar ? fields?.find((field) => field.field_id === editBar.fieldId) : undefined

  // ---- the record this editor is on (§5.5): the one open in the form, else the row of the current cell
  const [focusedRecord, setFocusedRecord] = useState<string | null>(null)
  // the form's record stays current once the form is closed (its previous / next move the table along)
  const openRecord = record?.kind === 'edit' ? record.recordId : null
  if (openRecord && openRecord !== focusedRecord) setFocusedRecord(openRecord)
  const rowOf = (id: string | null | undefined) => (id ? rows.rows.findIndex((row) => row?.record_id === id) : -1)
  const currentId = [openRecord, phoneCell?.recordId, focusedRecord].find((id) => rowOf(id) >= 0) ?? null
  const currentIndex = rowOf(currentId)
  const currentTitle = currentIndex >= 0 ? recordTitle(titleField, rows.rows[currentIndex], '') || null : null
  const publishes = size === 'tab' && !compact
  useEffect(() => {
    if (!publishes) return
    publishTableCurrent(sourceId, currentId ? { recordId: currentId, index: currentIndex + 1, total: rows.total, title: currentTitle } : null)
  }, [publishes, sourceId, currentId, currentIndex, currentTitle, rows.total])
  useEffect(() => (publishes ? () => publishTableCurrent(sourceId, null) : undefined), [publishes, sourceId])

  if (source.error && !source.data) return <div className="aiws-error" role="alert">{t('aiworkspace.data.table.sourceReadError', { error: source.error })}</div>
  if (!source.data || !fields) return <div className="aiws-muted">{t('aiworkspace.data.table.sourceLoading')}</div>
  // a data source defined but not synced yet (B0-3) is no empty table
  const sourcePayload = source.data.content.payload as { source?: { kind?: string } | null; data_version?: unknown }
  const pending = Boolean(sourcePayload.source) && sourcePayload.source?.kind !== 'session' && !sourcePayload.data_version

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
  /** One "and" condition of the saved filter removed (the rest stays saved). */
  const removeSavedCondition = (index: number) => {
    const rest = payload.filter ? withoutCondition(payload.filter, index) : null
    if (!rest) { void clearSaved('filter'); return }
    void store.submit({ editId: `view:${cellId}`, label: t('aiworkspace.data.view.removeFilterEdit'), operations: [{ op: 'entity.set_keys', entity_id: cellId, keys: [{ key: 'filter', value: rest as unknown as Json, expect: { rev: cell.content.key_revs.filter ?? 0 } }] }] })
  }

  const chips: { key: string; text: string; kind: 'saved' | 'session'; remove: (() => void) | null }[] = []
  conditionsOf(payload.filter).forEach((condition, index) => chips.push({ key: `sf${index}`, kind: 'saved', text: condition ? conditionText(fields, condition) : t('aiworkspace.data.table.savedFilter'), remove: canEditView ? () => removeSavedCondition(index) : null }))
  for (const [index, sort] of (payload.sorts ?? []).entries()) chips.push({ key: `ss${index}`, kind: 'saved', text: sortText(fields, sort, t), remove: canEditView ? () => { void clearSaved('sorts') } : null })
  conditionsOf(sessionView.filter).forEach((condition, index) => chips.push({ key: `tf${index}`, kind: 'session', text: condition ? conditionText(fields, condition) : t('aiworkspace.data.table.sessionConditions'),
    remove: () => setSessionView({ filter: sessionView.filter && sessionView.filter.op === 'and' ? withoutCondition(sessionView.filter, index) : null, sorts: sessionView.sorts }) }))
  for (const [index, sort] of (sessionView.sorts ?? []).entries()) chips.push({ key: `ts${index}`, kind: 'session', text: sortText(fields, sort, t), remove: () => setSessionView({ filter: sessionView.filter, sorts: null }) })
  const noEditing = !canUpdate && !canAppend
  const showReason = !compact && reason !== null && noEditing

  const canvasGrid = inNearToolbar && block?.depth === 0 && layout === 'grid'
  const maxHeight = size !== 'embedded' || canvasGrid ? null : compact ? 220 : phone ? 520 : 420
  const filterPanel = (
    <FilterSortPanel
      key={sessionKey}
      fields={fields}
      session={sessionView}
      hasSavedFilter={Boolean(payload.filter)}
      hasSavedSorts={Boolean(payload.sorts && payload.sorts.length > 0)}
      canSave={canEditView}
      onApply={(view) => { setSessionView(view); if (phone) setPanel('none') }}
      onSave={() => { void saveView() }}
      onClearSaved={(key) => { void clearSaved(key) }}
    />
  )
  const fieldManager = <FieldManager source={source.data} cell={cell} entries={entries} columnsReason={columnsReason} onColumns={saveColumns} structureReason={structureReason} />
  const inSheet = phone && !compact
  const tableId = cellId

  const title = cell.title ?? payload.title ?? entity?.title ?? entity?.name ?? t('aiworkspace.blocks.table.title')
  const leave = () => {
    if (flow) flow.select()
    else if (block?.selectBlock) { block.deactivateEditor(); block.selectBlock() }
    else root?.focus()
    const owner = document.activeElement
    if (!(owner instanceof HTMLElement) || owner === document.body) return
    const controller = new AbortController()
    owner.addEventListener('blur', () => controller.abort(), { once: true })
    owner.addEventListener('keydown', (event) => {
      if (event.key !== 'Tab' || event.target !== owner) return
      const children = Array.from(owner.children).filter((el): el is HTMLElement => el instanceof HTMLElement && !el.inert)
      for (const child of children) child.inert = true
      requestAnimationFrame(() => { for (const child of children) child.inert = false })
      controller.abort()
    }, { signal: controller.signal })
  }
  const canRename = Boolean(block?.mode === 'edit' && block.depth === 0 && directReason === null && block.cell.capabilities.includes('update'))
  const selectTitle = (clickCount: number) => {
    if (!block) return
    block.selectBlock?.()
    if (!block.editorActive) {
      if (root?.dataset.pointer === 'touch') block.activateEditor()
      return
    }
    if (root?.dataset.pointer === 'touch' || !canRename || clickCount === 0) {
      block.deactivateEditor()
      return
    }
    // Selection and cell blur happen on the first click. Keep the title mounted long enough to receive a double click.
    if (clickCount === 1) {
      titleRelease.current = window.setTimeout(() => {
        if (!root?.contains(document.activeElement)) block.deactivateEditor()
      }, 500)
    }
  }
  const heading = <TableHeading layout={layout} setLayout={setLayout} fields={fields.length} records={source.data.content.record_count} source={entity?.title ?? entity?.name ?? sourceId}
    reason={reason} showHeader={appearance.showHeader} style={appearance.style}
    setAppearance={canEditView || sourceMode ? (patch) => { void saveAppearance(patch).catch((e) => store.notify('error', String(e))) } : undefined}
    title={block ? <BlockTitle cell={block.cell} title={title} canRename={canRename}
      select={flow ? undefined : selectTitle} finishRename={!flow && block.editorActive ? block.deactivateEditor : undefined} /> : <strong>{title}</strong>}>
    {(compact || layout === 'cards' || !rows.total) && !flow && <button type="button" className="aiws-table-tool" aria-label={t('aiworkspace.table.more')} data-testid={`aiws-table-heading-menu-${tableId}`}
      onClick={(e) => pops.open('menu', e.currentTarget)}><MoreHorizontal size={16} /></button>}
  </TableHeading>
  return (
    <div ref={bindRoot} tabIndex={-1} data-grid-style={appearance.style} data-table-interactive={block?.mode === 'edit' && block.depth === 0 ? '' : undefined}
      onPointerDownCapture={(e) => { window.clearTimeout(titleRelease.current); e.currentTarget.dataset.pointer = e.pointerType }}
      onClickCapture={interactiveStatic || awaitingLock ? (e) => {
        const target = e.target as HTMLElement
        if (target.closest('a[href], .aiws-cell-reference button')) return
        const td = target.closest<HTMLElement>('[role="gridcell"], [role="columnheader"]')
        if (!td?.dataset.fieldId) return
        e.stopPropagation()
        const dispatch = interactiveStatic ? activeBlock : (action: TableIntent) => requestIntent(intentKey, action)
        const row = td.closest<HTMLElement>('[data-row-index]')
        dispatch({ action: td.getAttribute('role') === 'columnheader' ? 'rename' : 'edit', fieldId: td.dataset.fieldId,
          recordId: td.dataset.recordId ?? row?.dataset.recordId, rowIndex: row ? Number(row.dataset.rowIndex) : undefined })
      } : undefined} className="aiws-table" data-testid={`aiws-table-${tableId}`} data-total={rows.total} data-loaded={rows.rows.length} data-compact={compact ? 'true' : 'false'}
      data-layout={layout} data-size={canvasGrid ? 'canvas' : size} data-phone={phone ? 'true' : undefined} data-selecting={selection ? 'true' : undefined}
      onFocusCapture={publishes ? (event) => { const id = (event.target as HTMLElement).closest<HTMLElement>('.aiws-tr[data-record-id]')?.dataset.recordId; if (id) setFocusedRecord(id) } : undefined}>
      {flow ? flow.tableHeading && createPortal(heading, flow.tableHeading) : heading}
      {addedOutside && <div role="status">{t('aiworkspace.table.insertedElsewhere')} <button type="button" onClick={() => setRecord({ kind: 'edit', recordId: addedOutside })}>{t('aiworkspace.table.openInserted')}</button></div>}
      {!compact && !inNearToolbar && (
        <div className="aiws-table-bar" data-testid={`aiws-table-bar-${tableId}`}>
          <button type="button" className="aiws-table-tool" aria-label={t('aiworkspace.data.table.filterSort')} title={filterLabel} aria-pressed={panel === 'filter' || filtered} data-testid={`aiws-filter-open-${tableId}`}
            onClick={() => togglePanel('filter')}><ListFilter size={15} aria-hidden="true" /></button>
          {canFields && !phone && <button type="button" className="aiws-table-tool" aria-label={t('aiworkspace.data.table.fields')} title={t('aiworkspace.data.table.fieldsTitle')} aria-pressed={panel === 'fields'} data-testid={`aiws-fields-${tableId}`}
            onClick={() => togglePanel('fields')}><Columns3 size={15} aria-hidden="true" /></button>}
          <button type="button" className="aiws-table-tool" aria-label={t('aiworkspace.table.more')} title={t('aiworkspace.table.more')} aria-haspopup="menu" data-testid={`aiws-table-menu-${tableId}`}
            onClick={(event) => pops.open('menu', event.currentTarget)}><MoreHorizontal size={15} aria-hidden="true" /></button>
        </div>
      )}
      {!compact && (showReason || chips.length > 0) && (
        <div className="aiws-table-chips" data-testid={`aiws-table-chips-${tableId}`}>
          {showReason && <span className="aiws-chip aiws-table-reason" data-testid={`aiws-table-readonly-${tableId}`} title={reason ?? undefined}><Lock size={12} aria-hidden="true" />{reason}</span>}
          {chips.map((chip) => (
            <span key={chip.key} className={`aiws-chip aiws-table-chip${chip.kind === 'session' ? ' aiws-chip-warn' : ''}`} data-testid="aiws-table-chip" data-kind={chip.kind} title={chip.kind === 'session' ? t('aiworkspace.data.table.sessionConditions') : t('aiworkspace.table.chip.saved')}>
              <span>{chip.text}</span>
              {chip.remove && <button type="button" className="aiws-table-chip-remove" aria-label={t('aiworkspace.table.chip.remove', { text: chip.text })} title={t('aiworkspace.table.chip.remove', { text: chip.text })} data-testid="aiws-table-chip-remove" onClick={chip.remove}><X size={12} aria-hidden="true" /></button>}
            </span>
          ))}
        </div>
      )}
      {diagnostics.length > 0 && (
        <div className="aiws-warning" role="status" data-testid={`aiws-diagnostics-${tableId}`}>
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
      {panel === 'filter' && !inSheet && filterPanel}
      {panel === 'fields' && !inSheet && fieldManager}
      {panel !== 'none' && inSheet && (
        <SheetPanel label={panel === 'filter' ? t('aiworkspace.data.table.filterSort') : t('aiworkspace.data.table.fields')} testId={`aiws-table-sheet-${panel}`} onClose={() => setPanel('none')}>
          {panel === 'filter' ? filterPanel : fieldManager}
        </SheetPanel>
      )}
      {pending ? (
        <div className="aiws-muted aiws-table-empty" role="status" data-testid={`aiws-table-pending-${tableId}`}>{t('aiworkspace.datasource.pendingTable')}</div>
      ) : rows.error ? (
        <div className="aiws-error" role="alert" data-testid={`aiws-table-error-${tableId}`}>
          {brokenFilter ? t('aiworkspace.data.table.viewBroken') : t('aiworkspace.data.table.queryFailed', { error: rows.error.text })}
          {brokenFilter && canEditView && <button type="button" onClick={() => { void clearSaved('filter') }}>{t('aiworkspace.data.table.removeBrokenFilter')}</button>}
          {!brokenFilter && <button type="button" onClick={() => pager.refresh()}>{t('aiworkspace.common.retry')}</button>}
        </div>
      ) : layout === 'cards' ? (
        <TableCards tableId={tableId} fields={fields} columns={columns} titleField={titleField} rows={shown} pager={pager} scrollRef={scrollRef} maxHeight={maxHeight}
          selection={selection} onToggle={toggle} onStartSelect={(id) => setSelection(new Set([id]))} onOpen={(id) => { if (compact) activeBlock({ action: 'openRecord', recordId: id }); else setRecord({ kind: 'edit', recordId: id }) }} annotatedRecords={annotatedSet} />
      ) : (
        <TableGrid tableId={tableId} sourceId={sourceId} fields={fields} columns={columns} rows={shown} pager={pager} scrollRef={scrollRef} maxHeight={maxHeight} compact={Boolean(compact)} phone={phone}
          editable={canUpdate} resultTable={Boolean(source.data.derived)} titleField={titleField} annotations={annotations} onAnnotate={onAnnotate} onActivateAnnotation={onActivateAnnotation}
          viewportRef={canvasGrid ? viewportRef : undefined} showHandles={!compact || interactiveStatic} selection={selection} onToggle={toggle} onToggleAll={toggleAll}
          onOpen={(id) => { if (interactiveStatic) activeBlock({ action: 'openRecord', recordId: id }); else setRecord({ kind: 'edit', recordId: id }) }}
          annotatedRecords={annotatedSet} phoneCell={phoneCell} onPhoneCell={setPhoneCell} onPhoneEdit={(c) => { setPhoneCell(c); setEditBar(c) }}
          columnsEditable={(!compact || interactiveStatic) && columnsReason === null}
          onResize={(fieldId, width) => saveColumns(entries.map((entry) => (entry.field.field_id === fieldId ? { ...entry, width } : entry)))}
          onMove={(fieldId, to) => {
            // `to` counts shown columns: place it before the shown column now at that index
            const shown = entries.filter((entry) => entry.shown)
            const moving = entries.find((entry) => entry.field.field_id === fieldId)
            if (!moving) return
            const rest = entries.filter((entry) => entry.field.field_id !== fieldId)
            const before = shown.filter((entry) => entry.field.field_id !== fieldId)[to]
            const at = before ? rest.indexOf(before) : rest.length
            saveColumns([...rest.slice(0, at), moving, ...rest.slice(at)])
          }}
          onInsertRow={allowRow ? insertRow : undefined} onInsertColumn={allowColumn ? insertColumn : undefined}
          activateCells={Boolean((interactiveStatic && !block?.readOnlyReason || awaitingLock) && !external && caps.includes('update'))} allowNearRow={Boolean(sourceMode || canEditView || sorted)}
          onMoveRow={allowMove ? moveRow : undefined} sorted={sorted} limited={limited}
          rowHeights={appearance.rowHeights} showHeader={appearance.showHeader} canRenameField={(!compact || interactiveStatic) && canChangeFields}
          onResizeRow={(canEditView || sourceMode) ? (id, px) => { void saveAppearance({}, { id, px }).catch((e) => store.notify('error', String(e))) } : undefined}
          onExitGrid={leave} focusRequest={focusRequest} />
      )}
      {!compact && !rows.error && !pending && rows.done && rows.total === 0 && <div className="aiws-muted aiws-table-empty" data-testid={`aiws-table-empty-${tableId}`}>{canAppend ? t('aiworkspace.table.emptyAdd') : t('aiworkspace.table.empty')}</div>}
      {selection ? (
        <div className="aiws-table-foot aiws-table-selection" role="toolbar" aria-label={t('aiworkspace.table.selection.label')} data-testid={`aiws-table-selection-${tableId}`}>
          <span data-testid="aiws-table-selection-count">{t('aiworkspace.table.selection.count', { count: selection.size })}</span>
          <span className="aiws-grow" />
          <button type="button" className="aiws-table-foot-btn" data-testid="aiws-table-selection-copy" disabled={selection.size === 0} onClick={() => copyRows(selection)}><ClipboardCopy size={15} aria-hidden="true" />{t('aiworkspace.data.table.copyAsTable')}</button>
          {canDelete && <button type="button" className="aiws-table-foot-btn is-danger" data-testid="aiws-table-selection-delete" disabled={selection.size === 0} onClick={(event) => pops.open('delete-selected', event.currentTarget)}>{t('aiworkspace.common.delete')}</button>}
          <button type="button" className="aiws-table-foot-btn" data-testid="aiws-table-selection-cancel" aria-label={t('aiworkspace.table.selection.cancel')} onClick={() => setSelection(null)}><X size={15} aria-hidden="true" />{t('aiworkspace.table.selection.cancel')}</button>
        </div>
      ) : (!compact || (canvasGrid && interactiveStatic)) && (
        <div className="aiws-table-foot">
          <span className="aiws-grow" />
          <span className="aiws-table-count" data-testid={`aiws-table-count-${tableId}`} data-limited={limited ? 'true' : undefined}>{pending ? '' : limited
            ? t('aiworkspace.datasource.rows.limited', { count: rowLimit ?? 0, total: rows.total })
            : rows.rows.length < rows.total
            ? t('aiworkspace.table.countLoaded', { loaded: rows.rows.length, count: rows.total })
            : rows.loading ? t('aiworkspace.data.table.countLoading', { count: rows.total }) : t('aiworkspace.data.table.count', { count: rows.total })}</span>
        </div>
      )}
      {pops.pop?.kind === 'menu' && <TableMenu anchor={pops.pop.anchor} onClose={pops.close} label={t('aiworkspace.table.more')} items={menuItems} testId={`aiws-table-menu-pop-${tableId}`} />}
      {pops.pop?.kind === 'delete-selected' && selection && (
        <ConfirmPop anchor={pops.pop.anchor} title={t('aiworkspace.table.selection.deleteAsk', { count: selection.size })} detail={t('aiworkspace.table.undoHint')} confirm={t('aiworkspace.common.delete')}
          onConfirm={() => { void deleteSelected() }} onClose={pops.close} />
      )}
      {record && (
        <TableRecordPanel host={host} sourceId={sourceId} schemaId={schemaId} onInsert={insert} cellId={sourceMode ? null : cellId} fields={fields} titleField={titleField} state={record} rows={rows.rows} pager={pager}
          canUpdate={canUpdate} canAppend={canAppend} canDelete={canDelete} readOnlyReason={noEditing ? reason : null}
          onOpen={(recordId) => setRecord({ kind: 'edit', recordId })} onClose={() => setRecord(null)} />
      )}
      {editBar && editBarRow && editBarField && (
        <PhoneEditBar key={`${editBar.recordId}/${editBar.fieldId}`} host={host} sourceId={sourceId} field={editBarField} row={editBarRow} pager={pager} onClose={() => setEditBar(null)} onAnnotate={onAnnotate} />
      )}
    </div>
  )
}

/** A phone's sheet for the filter and the fields: up from the bottom, over the page. */
function SheetPanel({ label, testId, onClose, children }: { label: string; testId: string; onClose: () => void; children: ReactNode }) {
  const anchor = anchorOf(null)
  return <TablePop anchor={anchor} onClose={onClose} label={label} testId={testId}>{children}</TablePop>
}
