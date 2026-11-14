/* The standard table editor (UI 框架 PRD §7.1–§7.3): one editor for a TableSource, used by a table Block (on a
 * canvas or a page), by a data-mode tab and by a phone's full screen. Its title row has the filter, the fields and ⋯;
 * the conditions in force are chips under it; the content is a grid or cards (a phone starts with cards, a computer
 * with the grid, the choice is each person's); the foot holds "+ new record" and how many records are loaded. A
 * record opens in its form (a side panel, or the whole phone screen); records are selected by ticking (computer) or
 * a long press (phone) and the bar at the bottom deletes or copies them. Every write keeps the rules of the grid:
 * capability, the read-only reason, undo, the save state. */

import { useEffect, useMemo, useRef, useState, useSyncExternalStore, type ReactNode } from 'react'
import { ClipboardCopy, Columns3, LayoutGrid, ListChecks, ListFilter, Lock, MoreHorizontal, Plus, Rows3, Upload, X } from 'lucide-react'
import { TABULAR_ACCEPT } from '@table/files'
import { languageCurrency } from '@table/currency'
import { randomId } from '../../api/ids'
import type { ReadOk } from '../../api/session'
import type { CellPayload, FieldDef, FilterNode, Json, KeyedContent, SortSpec, TableSourceContent } from '../../api/types'
import type { CapturedAnchor } from '../../anchors/registry'
import { useEntity, useLoad, useReadOnlyReason, useStore, useUserState, useVersion, type AnnotationMark } from '../../state/hooks'
import type { WorkspaceStore } from '../../state/store'
import { useAiwsText } from '../../useAiwsText'
import { consumeIntent, useEditorToolbar, useIntent } from '../blocks/editorToolbar'
import type { ToolbarItem } from '../blocks/registry'
import { annotatedRecords, useRecordFocus } from '../controls/data/focus'
import { FieldManager } from '../FieldManager'
import { addFieldOps, CREATABLE_FIELD_TYPES } from '../fieldOps'
import { COPY_LIMIT, tableClipboard } from '../import/copyTable'
import { tableImports, type TableDestination } from '../import/tableImportRequest'
import { useFilePicker } from '../shell/filePicker'
import { usePhone } from '../shell/shellContext'
import { TablePager, type PagerQuery } from '../tablePager'
import { FIELD_TYPE_LABEL } from '../values'
import { PhoneEditBar } from './EditBar'
import { FIELD_ICONS } from './fieldIcons'
import { FilterSortPanel } from './FilterPanel'
import {
  blockColumns, conditionText, conditionsOf, listedOf, personalColumns, personalOf, sortText, titleFieldOf, withoutCondition,
  type Column, type ColumnEntry, type PersonalColumns, type SessionView, type TableEditorSize, type TableLayout,
} from './model'
import { TableRecordPanel, type RecordPanelState } from './RecordPanel'
import { TableCards } from './TableCards'
import { TableGrid, type CellRef } from './TableGrid'
import { ConfirmPop, TableMenu, TablePop, anchorOf, usePop, type MenuItem } from './TablePop'
import './table.css'

export type CellRead = ReadOk<KeyedContent<CellPayload>>
type SourceRead = ReadOk<TableSourceContent>

export interface TableEditorBodyProps {
  /** The table-view Block, or the table's synthetic view (`sourceMode`). */
  cell: CellRead
  sourceMode?: boolean
  size: TableEditorSize
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
  layout?: { phone?: TableLayout; desktop?: TableLayout }
  filter?: FilterNode | null
  sorts?: SortSpec[] | null
}

type Panel = 'none' | 'filter' | 'fields'

export function TableEditorBody({ cell, sourceMode, size, readOnly, readOnlyReason, compact, annotations, onAnnotate, onActivateAnnotation }: TableEditorBodyProps) {
  const { t, locale } = useAiwsText()
  const store = useStore()
  const phone = usePhone()
  const cellId = cell.entity_id
  const payload = cell.content.payload
  const sourceId = payload.source_ref?.entity_id ?? ''
  const sourceVersion = useVersion(`e:${sourceId}`)
  const loadSource = useMemo(() => () => store.session.read<TableSourceContent>(sourceId), [store, sourceId])
  const source = useLoad<SourceRead>(loadSource, sourceVersion)
  const entity = useEntity(sourceId)
  const directReason = useReadOnlyReason()

  // ---- personal state: a Block's `block:<id>/table`; the table itself keeps its old `tableview:<id>` entry (phase two §6.1)
  const prefKey = sourceMode ? `tableview:${sourceId}` : `block:${cellId}/table`
  const pref = useUserState<Json>(prefKey) as TablePref | undefined
  const writePref = (change: Partial<TablePref>) => {
    const next: Record<string, unknown> = { ...(pref ?? {}), ...change }
    for (const key of Object.keys(next)) if (next[key] === null || next[key] === undefined) delete next[key]
    store.userState.set(prefKey, Object.keys(next).length > 0 ? (next as Json) : null)
  }
  // session-level filter and sort (design §3.5.2): not a commit until "save view"; without a Block in the user state
  const [localView, setLocalView] = useState<SessionView>({ filter: null, sorts: null })
  const sessionView: SessionView = sourceMode ? { filter: pref?.filter ?? null, sorts: pref?.sorts ?? null } : localView
  const setSessionView = (view: SessionView) => { if (sourceMode) writePref({ filter: view.filter, sorts: view.sorts }); else setLocalView(view) }
  const device = phone ? 'phone' : 'desktop'
  const layout: TableLayout = compact ? 'grid' : pref?.layout?.[device] ?? (phone ? 'cards' : 'grid')
  const setLayout = (next: TableLayout) => writePref({ layout: { ...(pref?.layout ?? {}), [device]: next } })

  // ---- what is open
  const [panel, setPanel] = useState<Panel>('none')
  const [record, setRecord] = useState<RecordPanelState | null>(null)
  const [selection, setSelection] = useState<Set<string> | null>(null)
  const [phoneCell, setPhoneCell] = useState<CellRef | null>(null)
  const [editBar, setEditBar] = useState<CellRef | null>(null)
  const pops = usePop<'menu' | 'delete-selected' | 'add-field'>()
  // a toolbar button pressed while the Block was only selected names what to open (标准对象的交互改进 §5.2)
  const intentKey = sourceMode || compact ? '' : cellId
  const intent = useIntent(intentKey)
  const [intentSeen, setIntentSeen] = useState(0)
  const [asked, setAsked] = useState<{ copy: number; import: number }>({ copy: 0, import: 0 })
  if (intent && intent.seq !== intentSeen) {
    setIntentSeen(intent.seq)
    if (intent.value === 'copy' || intent.value === 'import') setAsked((a) => ({ ...a, [intent.value]: intent.seq }))
    else if (intent.value === 'add') setRecord({ kind: 'new' })
    else if (intent.value === 'select') setSelection(new Set())
    else if (intent.value === 'filter' || intent.value === 'fields') setPanel(intent.value)
  }
  useEffect(() => { if (intent) consumeIntent(intentKey, intent) }, [intent, intentKey])
  // a record's link (控件机制 G12) opens its form
  useRecordFocus(intentKey, (focus) => setRecord({ kind: 'edit', recordId: focus.record }))

  // ---- rows
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

  // ---- columns: a Block's view lists them; the table itself, this person
  const fields = source.data?.content.fields
  const entries: ColumnEntry[] = useMemo(() => (!fields ? [] : sourceMode ? personalColumns(fields, pref ?? {}) : blockColumns(fields, payload.fields)), [fields, sourceMode, pref, payload.fields])
  const columns: Column[] = entries.filter((entry) => entry.shown).map(({ field, width }) => ({ field, width }))
  const titleField = fields ? titleFieldOf(fields, source.data?.content.payload.title_field_id) : undefined

  // ---- what may be done here, and why not
  const caps = source.data?.capabilities ?? []
  const external = Boolean(entity?.source ?? source.data?.source)
  const canUpdate = !readOnly && caps.includes('update')
  const canAppend = !readOnly && caps.some((capability) => capability === 'append' || capability === 'update')
  const canDelete = !readOnly && caps.includes('delete')
  const canStructure = !readOnly && caps.includes('structure')
  const canEditView = !readOnly && !sourceMode && cell.capabilities.includes('update')
  const reason = readOnlyReason ?? directReason ?? (external ? t('aiworkspace.table.readOnly.external')
    : source.data && !caps.some((capability) => capability === 'update' || capability === 'append') ? t('aiworkspace.table.readOnly.noPermission') : null)
  const columnsReason = sourceMode ? null : canEditView ? null : t('aiworkspace.table.readOnly.noView')
  const structureReason = canStructure ? null : reason ?? t('aiworkspace.table.readOnly.noStructure')
  const saveColumns = (next: ColumnEntry[]) => {
    if (sourceMode) { writePref(personalOf(next)); return }
    if (!canEditView) { store.notify('info', columnsReason ?? ''); return }
    void store.submit({ editId: `view:${cellId}`, label: t('aiworkspace.data.view.columnsEdit'), operations: [{ op: 'entity.set_keys', entity_id: cellId, keys: [{ key: 'fields', value: listedOf(next) as unknown as Json, expect: { rev: cell.content.key_revs.fields ?? 0 } }] }] })
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
  useEffect(() => { if (asked.copy && columnsReady && copiedFor.current !== asked.copy) { copiedFor.current = asked.copy; copyRef.current(null) } }, [asked.copy, columnsReady])

  // ---- import (§7.3 "导入数据…"): the table wizard, the file chosen from the Zone first; the result is a new table next to this one
  const pickFile = useFilePicker((message) => store.notify('error', message))
  const surface = sourceMode ? undefined : store.outline.ancestors(cellId).map((id) => store.outline.get(id)).find((e) => e?.kind === 'surface')
  const folderId = entity?.parent_id
  const importTarget: { destination: TableDestination; reason: string | null } | null = surface
    ? {
        reason: surface.capabilities.includes('structure') ? null : t('aiworkspace.table.import.noPermission'),
        destination: { canvas: {
          surfaceId: surface.entity_id, isFree: surface.layout?.mode === 'free', contentFolderId: surface.content_folder_id ?? '',
          at: (() => { const p = store.outline.get(cellId)?.placement as { x?: number; y?: number; h?: number } | undefined; return surface.layout?.mode === 'free' && p && typeof p.x === 'number' ? { x: p.x, y: (p.y ?? 0) + (p.h ?? 0) + 40 } : null })(),
          center: { x: 0, y: 0 }, afterId: cellId,
        } },
      }
    : folderId ? { reason: store.outline.get(folderId)?.capabilities.includes('structure') ? null : t('aiworkspace.table.import.noPermission'), destination: { folderId } } : null
  const importReason = directReason ?? importTarget?.reason ?? (importTarget ? null : t('aiworkspace.table.import.noPlace'))
  const runImport = async () => {
    if (!importTarget || importReason) { if (importReason) store.notify('info', importReason); return }
    if (tableImports.busy) { store.notify('info', t('aiworkspace.import.busy')); return }
    const file = await pickFile.file(TABULAR_ACCEPT)
    if (!file) return
    const bytes = new Uint8Array(await file.arrayBuffer())
    tableImports.open({ source: { kind: 'file', name: file.name, bytes }, step: 'wizard', destination: importTarget.destination })
  }
  const importRef = useRef(runImport)
  useEffect(() => { importRef.current = runImport })
  const importedFor = useRef(0)
  useEffect(() => { if (asked.import && importedFor.current !== asked.import) { importedFor.current = asked.import; void importRef.current() } }, [asked.import])

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

  // ---- the title row's tools: in the near toolbar on a free canvas, drawn here elsewhere
  const togglePanel = (next: Panel) => setPanel(panel === next ? 'none' : next)
  const filtered = Boolean(sessionView.filter || sessionView.sorts)
  const canFields = canStructure || columnsReason === null
  const filterLabel = filtered ? t('aiworkspace.data.table.filterSortSession') : payload.filter || payload.sorts?.length ? t('aiworkspace.data.table.filterSortSaved') : t('aiworkspace.data.table.filterSort')
  const menuItems = (inline: boolean): (MenuItem | 'separator')[] => [
    layout === 'grid'
      ? { id: 'table-layout-cards', label: t('aiworkspace.table.layout.showCards'), icon: <LayoutGrid size={14} />, run: () => setLayout('cards') }
      : { id: 'table-layout-grid', label: t('aiworkspace.table.layout.showGrid'), icon: <Rows3 size={14} />, run: () => setLayout('grid') },
    ...(inline && phone && canFields ? [{ id: 'table-fields', label: t('aiworkspace.data.table.fields'), icon: <Columns3 size={14} />, run: () => togglePanel('fields') }] : []),
    { id: 'table-select', label: t('aiworkspace.table.selection.start'), icon: <ListChecks size={14} />, run: () => setSelection(new Set()) },
    { id: 'table-copy', label: t('aiworkspace.data.table.copyAsTable'), icon: <ClipboardCopy size={14} />, run: () => copyRows(null) },
    'separator',
    { id: 'table-import', label: t('aiworkspace.table.import.label'), icon: <Upload size={14} />, disabled: importReason ?? false, run: () => { void runImport() } },
  ]
  const tools: ToolbarItem[] | null = compact || !source.data ? null : [
    { kind: 'button', id: 'table-filter', icon: ListFilter, label: filterLabel, active: panel === 'filter' || filtered, run: () => togglePanel('filter') },
    ...(canFields ? [{ kind: 'button' as const, id: 'table-fields', icon: Columns3, label: t('aiworkspace.data.table.fields'), active: panel === 'fields', run: () => togglePanel('fields') }] : []),
    ...(canAppend ? [{ kind: 'button' as const, id: 'table-add', icon: Plus, label: t('aiworkspace.data.record.add'), active: record?.kind === 'new', run: () => setRecord({ kind: 'new' }) }] : []),
    { kind: 'button', id: 'table-copy', icon: ClipboardCopy, label: t('aiworkspace.data.table.copyAsTable'), run: () => copyRows(null) },
    { kind: 'menu', id: 'table-more', icon: MoreHorizontal, label: t('aiworkspace.table.more'), onPick: (value) => (menuItems(false).find((item) => item !== 'separator' && item.id === value) as MenuItem | undefined)?.run(),
      items: menuItems(false).flatMap((item) => (item === 'separator' || item.id === 'table-copy' || item.disabled ? [] : [{ value: item.id, label: item.label }])) },
  ]
  const inNearToolbar = useEditorToolbar('table', tools)

  const scrollRef = useRef<HTMLDivElement>(null)
  const [root, bindRoot] = useState<HTMLDivElement | null>(null)
  const host = (root?.closest('.aiws-root') as HTMLElement | null) ?? null
  const annotatedSet = useMemo(() => annotatedRecords(annotations, sourceId), [annotations, sourceId])
  // an open record follows its row (a refused write keeps it; a deleted record closes it once it is gone from the view)
  const editBarRow = editBar ? rows.rows.find((row) => row?.record_id === editBar.recordId) : undefined
  const editBarField = editBar ? fields?.find((field) => field.field_id === editBar.fieldId) : undefined

  if (source.error && !source.data) return <div className="aiws-error" role="alert">{t('aiworkspace.data.table.sourceReadError', { error: source.error })}</div>
  if (!source.data || !fields) return <div className="aiws-muted">{t('aiworkspace.data.table.sourceLoading')}</div>

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

  const maxHeight = size !== 'embedded' ? null : compact ? 220 : phone ? 520 : 420
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

  return (
    <div ref={bindRoot} className="aiws-table" data-testid={`aiws-table-${tableId}`} data-total={rows.total} data-loaded={rows.rows.length} data-compact={compact ? 'true' : 'false'}
      data-layout={layout} data-size={size} data-phone={phone ? 'true' : undefined} data-selecting={selection ? 'true' : undefined}>
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
      {rows.error ? (
        <div className="aiws-error" role="alert" data-testid={`aiws-table-error-${tableId}`}>
          {brokenFilter ? t('aiworkspace.data.table.viewBroken') : t('aiworkspace.data.table.queryFailed', { error: rows.error.text })}
          {brokenFilter && canEditView && <button type="button" onClick={() => { void clearSaved('filter') }}>{t('aiworkspace.data.table.removeBrokenFilter')}</button>}
          {!brokenFilter && <button type="button" onClick={() => pager.refresh()}>{t('aiworkspace.common.retry')}</button>}
        </div>
      ) : layout === 'cards' ? (
        <TableCards tableId={tableId} fields={fields} columns={columns} titleField={titleField} rows={rows} pager={pager} scrollRef={scrollRef} maxHeight={maxHeight}
          selection={selection} onToggle={toggle} onStartSelect={(id) => setSelection(new Set([id]))} onOpen={(id) => setRecord({ kind: 'edit', recordId: id })} annotatedRecords={annotatedSet} />
      ) : (
        <TableGrid tableId={tableId} sourceId={sourceId} fields={fields} columns={columns} rows={rows} pager={pager} scrollRef={scrollRef} maxHeight={maxHeight} compact={Boolean(compact)} phone={phone}
          editable={canUpdate} resultTable={Boolean(source.data.derived)} titleField={titleField} annotations={annotations} onAnnotate={onAnnotate} onActivateAnnotation={onActivateAnnotation}
          rowHead={!compact} selection={selection} onToggle={toggle} onToggleAll={toggleAll} onStartSelect={(id) => setSelection(new Set([id]))} onOpen={(id) => setRecord({ kind: 'edit', recordId: id })}
          annotatedRecords={annotatedSet} phoneCell={phoneCell} onPhoneCell={setPhoneCell} onPhoneEdit={(c) => { setPhoneCell(c); setEditBar(c) }}
          columnsEditable={!compact && columnsReason === null}
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
          onAddField={canStructure && !compact && !phone ? (button) => pops.open('add-field', button) : null} addFieldOpen={pops.pop?.kind === 'add-field'} />
      )}
      {!compact && !rows.error && rows.done && rows.total === 0 && <div className="aiws-muted aiws-table-empty" data-testid={`aiws-table-empty-${tableId}`}>{canAppend ? t('aiworkspace.table.emptyAdd') : t('aiworkspace.table.empty')}</div>}
      {!compact && (selection ? (
        <div className="aiws-table-foot aiws-table-selection" role="toolbar" aria-label={t('aiworkspace.table.selection.label')} data-testid={`aiws-table-selection-${tableId}`}>
          <span data-testid="aiws-table-selection-count">{t('aiworkspace.table.selection.count', { count: selection.size })}</span>
          <span className="aiws-grow" />
          <button type="button" className="aiws-table-foot-btn" data-testid="aiws-table-selection-copy" disabled={selection.size === 0} onClick={() => copyRows(selection)}><ClipboardCopy size={15} aria-hidden="true" />{t('aiworkspace.data.table.copyAsTable')}</button>
          {canDelete && <button type="button" className="aiws-table-foot-btn is-danger" data-testid="aiws-table-selection-delete" disabled={selection.size === 0} onClick={(event) => pops.open('delete-selected', event.currentTarget)}>{t('aiworkspace.common.delete')}</button>}
          <button type="button" className="aiws-table-foot-btn" data-testid="aiws-table-selection-cancel" aria-label={t('aiworkspace.table.selection.cancel')} onClick={() => setSelection(null)}><X size={15} aria-hidden="true" />{t('aiworkspace.table.selection.cancel')}</button>
        </div>
      ) : !inNearToolbar && (
        <div className="aiws-table-foot">
          {canAppend && !rows.error && (phone || layout === 'cards'
            ? <button type="button" className="aiws-table-new" data-testid={`aiws-add-record-${tableId}`} onClick={() => setRecord({ kind: 'new' })}><Plus size={16} aria-hidden="true" />{t('aiworkspace.table.newRecord')}</button>
            : <NewRecordRow store={store} source={source.data} cellId={tableId} onNeedForm={() => setRecord({ kind: 'new' })} />)}
          <span className="aiws-grow" />
          <span className="aiws-table-count" data-testid={`aiws-table-count-${tableId}`}>{rows.rows.length < rows.total
            ? t('aiworkspace.table.countLoaded', { loaded: rows.rows.length, count: rows.total })
            : rows.loading ? t('aiworkspace.data.table.countLoading', { count: rows.total }) : t('aiworkspace.data.table.count', { count: rows.total })}</span>
        </div>
      ))}
      {pops.pop?.kind === 'menu' && <TableMenu anchor={pops.pop.anchor} onClose={pops.close} label={t('aiworkspace.table.more')} items={menuItems(true)} testId={`aiws-table-menu-pop-${tableId}`} />}
      {pops.pop?.kind === 'delete-selected' && selection && (
        <ConfirmPop anchor={pops.pop.anchor} title={t('aiworkspace.table.selection.deleteAsk', { count: selection.size })} detail={t('aiworkspace.table.undoHint')} confirm={t('aiworkspace.common.delete')}
          onConfirm={() => { void deleteSelected() }} onClose={pops.close} />
      )}
      {pops.pop?.kind === 'add-field' && <QuickAddField source={source.data} cell={cell} anchor={pops.pop.anchor} onClose={pops.close} locale={locale} />}
      {record && (
        <TableRecordPanel host={host} sourceId={sourceId} cellId={sourceMode ? null : cellId} fields={fields} titleField={titleField} state={record} rows={rows.rows} pager={pager}
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
  return <TablePop anchor={anchor} onClose={onClose} label={label} testId={testId}><div className="aiws-table-pop-title">{label}</div>{children}</TablePop>
}

/** "+" of the header: a field by name and type, nothing else asked (the field manager has the rest). */
function QuickAddField({ source, cell, anchor, onClose, locale }: { source: SourceRead; cell: CellRead; anchor: ReturnType<typeof anchorOf>; onClose: () => void; locale: string }) {
  const { t } = useAiwsText()
  const store = useStore()
  const [name, setName] = useState('')
  const [type, setType] = useState<FieldDef['type']>('text')
  const submit = () => {
    const text = name.trim()
    if (!text) return
    // a currency column takes the interface language's currency (zh → CNY, else USD); the field manager can change it
    void store.submit({ editId: `schema:${source.entity_id}:${randomId()}`, label: t('aiworkspace.data.field.addEdit', { name: text }), operations: addFieldOps(source.entity_id, cell, text, type, 2, languageCurrency(locale)) })
      .then((outcome) => { if (outcome.status === 'accepted' || outcome.status === 'saved_locally') onClose() })
  }
  return (
    <TablePop anchor={anchor} onClose={onClose} label={t('aiworkspace.data.field.addColumn')} width={280}>
      <form className="aiws-quick-field" data-testid="aiws-quick-field" onSubmit={(event) => { event.preventDefault(); submit() }}>
        <input aria-label={t('aiworkspace.data.field.name')} data-testid="aiws-quick-field-name" autoFocus placeholder={t('aiworkspace.data.field.name')} value={name} onChange={(event) => setName(event.target.value)} />
        <div className="aiws-quick-field-types" role="radiogroup" aria-label={t('aiworkspace.data.field.type')}>
          {CREATABLE_FIELD_TYPES.map((item) => {
            const Icon = FIELD_ICONS[item] ?? Columns3
            return (
              <button key={item} type="button" role="radio" aria-checked={type === item} className="aiws-quick-field-type" data-testid={`aiws-quick-field-type-${item}`} onClick={() => setType(item)}>
                <Icon size={14} aria-hidden="true" />{FIELD_TYPE_LABEL[item]}
              </button>
            )
          })}
        </div>
        <button type="submit" className="is-primary" data-testid="aiws-quick-field-add" disabled={!name.trim()}>{t('aiworkspace.data.field.addColumn')}</button>
      </form>
    </TablePop>
  )
}

/** "+ 新建" under the rows (Notion): type the title, Enter adds the record and keeps the line open for the next one.
 * A table with other required fields asks for them in the record's form instead. */
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
    return <button type="button" className="aiws-table-new" data-testid={`aiws-add-record-${cellId}`} onClick={start}><Plus size={14} aria-hidden="true" />{t('aiworkspace.table.newRecord')}</button>
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
