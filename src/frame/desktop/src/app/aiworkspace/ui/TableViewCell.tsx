/* TableView cell (design §3.5): a virtualised, paged view of a TableSource with inline editors.
 * Every write carries the rev the user was looking at when the edit began. */

import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'
import { Columns3, ListFilter, Plus } from 'lucide-react'
import { randomId } from '../api/ids'
import type { ReadOk } from '../api/session'
import type {
  CellPayload, FieldDef, FilterNode, Json, KeyedContent, Operation, QueryRow, SortSpec, TableSourceContent,
} from '../api/types'
import { EDIT_STATE_LABEL } from '../state/edits'
import type { CapturedAnchor } from '../anchors/registry'
import { useEdit, useLoad, useStore, useUserState, useVersion, type AnnotationMark } from '../state/hooks'
import type { WorkspaceStore } from '../state/store'
import { consumeIntent, useEditorToolbar, useIntent } from './blocks/editorToolbar'
import type { ToolbarItem } from './blocks/registry'
import { FieldManager } from './FieldManager'
import { TablePager, type PagerQuery } from './tablePager'
import { ValueEditor } from './ValueEditor'
import { UNSET, formatValue, operatorsFor, SORTABLE, type Input } from './values'

const ROW_HEIGHT = 34
const DEFAULT_WIDTH = 150

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
  if (cell.error && !cell.data) return <div className="aiws-error" role="alert">无法读取视图：{cell.error}</div>
  if (!cell.data) return <div className="aiws-muted">正在载入视图…</div>
  const sourceRef = cell.data.content.payload.source_ref
  if (!sourceRef) return <div className="aiws-warning">这个表格视图没有绑定数据。</div>
  return <TableViewBody key={sourceRef.entity_id} cell={cell.data} readOnly={readOnly} compact={compact} annotations={annotations} onAnnotate={onAnnotate} onActivateAnnotation={onActivateAnnotation} />
}

function TableViewBody({ cell, sourceMode, readOnly, compact, annotations, onAnnotate, onActivateAnnotation }: Omit<Props, 'cellId' | 'sourceId'> & { cell: CellRead; sourceMode?: boolean }) {
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
    { kind: 'button', id: 'table-filter', icon: ListFilter, label: `筛选 / 排序${sessionView.filter || sessionView.sorts ? '（本会话有未保存的临时条件）' : payload.filter || payload.sorts?.length ? '（视图已保存条件）' : ''}`, active: panel === 'filter' || Boolean(sessionView.filter || sessionView.sorts), run: () => toggle('filter') },
    ...(canStructure ? [{ kind: 'button' as const, id: 'table-fields', icon: Columns3, label: '字段', active: panel === 'fields', run: () => toggle('fields') }] : []),
    ...(canAppend ? [{ kind: 'button' as const, id: 'table-add', icon: Plus, label: '新增记录', active: panel === 'add', run: () => toggle('add') }] : []),
  ]
  const inNearToolbar = useEditorToolbar('table', tools)

  const scrollRef = useRef<HTMLDivElement>(null)
  // eslint-disable-next-line react-hooks/incompatible-library -- TanStack Virtual returns unstable functions by design; nothing here is memoised on them.
  const virtualizer = useVirtualizer({ count: rows.total, getScrollElement: () => scrollRef.current, estimateSize: () => ROW_HEIGHT, overscan: 8 })
  const items = virtualizer.getVirtualItems()
  const lastIndex = items.length > 0 ? items[items.length - 1].index : 0
  useEffect(() => { pager.ensure(lastIndex) }, [pager, lastIndex])

  if (source.error && !source.data) return <div className="aiws-error" role="alert">无法读取数据源：{source.error}</div>
  if (!source.data || !fields) return <div className="aiws-muted">正在载入数据源…</div>

  const totalWidth = columns.reduce((sum, column) => sum + column.width, 0) + (canDelete ? 56 : 0)
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
    const outcome = await store.submit({ editId: `view:${cellId}`, label: `保存视图「${payload.title ?? cellId}」`, operations: [{ op: 'entity.set_keys', entity_id: cellId, keys }] })
    if (outcome.status === 'accepted') setSessionView({ filter: null, sorts: null })
  }
  const clearSaved = (key: 'filter' | 'sorts') => store.submit({
    editId: `view:${cellId}`, label: `移除视图的${key === 'filter' ? '筛选' : '排序'}`,
    operations: [{ op: 'entity.unset_keys', entity_id: cellId, keys: [{ key, expect: { rev: cell.content.key_revs[key] ?? 0 } }] }],
  })

  return (
    <div className="aiws-table" data-testid={`aiws-table-${cellId}`} data-total={rows.total}>
      {!compact && !inNearToolbar && (
        <div className="aiws-table-bar">
          <span className="aiws-muted" data-testid={`aiws-table-count-${cellId}`}>{rows.total} 条记录{rows.loading ? ' · 载入中' : ''}</span>
          {payload.filter && <span className="aiws-chip">视图已保存筛选</span>}
          {payload.sorts && payload.sorts.length > 0 && <span className="aiws-chip">视图已保存排序</span>}
          {(sessionView.filter || sessionView.sorts) && <span className="aiws-chip aiws-chip-warn">本会话临时条件（未保存）</span>}
          <span className="aiws-grow" />
          <button type="button" onClick={() => setPanel(panel === 'filter' ? 'none' : 'filter')}>筛选 / 排序</button>
          {canStructure && <button type="button" data-testid={`aiws-fields-${cellId}`} onClick={() => setPanel(panel === 'fields' ? 'none' : 'fields')}>字段</button>}
          {canAppend && <button type="button" data-testid={`aiws-add-record-${cellId}`} onClick={() => setPanel(panel === 'add' ? 'none' : 'add')}>+ 记录</button>}
        </div>
      )}
      {diagnostics.length > 0 && (
        <div className="aiws-warning" role="status" data-testid={`aiws-diagnostics-${cellId}`}>
          {diagnostics.map((item, index) => (
            <div key={index}>此视图的{item.key === 'filter' ? '筛选' : item.key === 'sorts' ? '排序' : item.key === 'group' ? '分组' : '列配置'}引用了{item.code === 'FIELD_DELETED' ? '已删除的字段' : item.code === 'OPTION_DELETED' ? '已删除的选项' : item.code}{item.field_id ? `（${item.field_id}）` : ''}。</div>
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
          {brokenFilter ? '此视图的筛选引用了已删除的字段或选项，无法查询（VIEW_BROKEN）。不会忽略该条件显示更多行。' : `查询失败：${rows.error.text}`}
          {brokenFilter && canEditView && <button type="button" onClick={() => { void clearSaved('filter') }}>移除失效筛选</button>}
          {!brokenFilter && <button type="button" onClick={() => pager.refresh()}>重试</button>}
        </div>
      ) : (
        <div ref={scrollRef} className="aiws-table-scroll" style={{ maxHeight: compact ? 220 : 380 }} data-testid={`aiws-table-scroll-${cellId}`}>
          <div className="aiws-table-head" style={{ width: totalWidth }}>
            {columns.map(({ field, width }) => (
              <div key={field.field_id} className="aiws-th" style={{ width }} title={field.field_id}>
                {field.name}
                {field.maintained_by === 'program' && <span className="aiws-chip" title="通常由加工程序维护">程序</span>}
              </div>
            ))}
            {canDelete && <div className="aiws-th" style={{ width: 56 }} />}
          </div>
          <div style={{ height: virtualizer.getTotalSize(), width: totalWidth, position: 'relative' }}>
            {items.map((item) => {
              const row = rows.rows[item.index]
              return (
                <div key={item.key} className="aiws-tr" role="row" data-testid="aiws-row" data-record-id={row?.record_id} style={{ transform: `translateY(${item.start}px)`, height: ROW_HEIGHT }}>
                  {row ? (
                    <>
                      {columns.map(({ field, width }) => (
                        <TableCell
                          key={field.field_id}
                          pager={pager}
                          sourceId={sourceId}
                          field={field}
                          row={row}
                          width={width}
                          editable={canUpdate}
                          resultTable={Boolean(source.data?.derived)}
                          annotation={annotations.find((mark) => mark.payload.target?.entity_id === sourceId && mark.payload.target.selector?.kind === 'table_cell'
                            && mark.payload.target.selector.record_id === row.record_id && mark.payload.target.selector.field_id === field.field_id)}
                          onAnnotate={onAnnotate}
                          onActivateAnnotation={onActivateAnnotation}
                        />
                      ))}
                      {canDelete && <div className="aiws-td" style={{ width: 56 }}><DeleteRecordButton sourceId={sourceId} row={row} /></div>}
                    </>
                  ) : <div className="aiws-td aiws-muted" style={{ width: totalWidth }}>…</div>}
                </div>
              )
            })}
          </div>
        </div>
      )}
    </div>
  )
}

function DeleteRecordButton({ sourceId, row }: { sourceId: string; row: QueryRow }) {
  const store = useStore()
  return (
    <button
      type="button"
      className="aiws-link"
      title="删除记录"
      onClick={() => {
        void store.submit({
          editId: `record:${sourceId}:${row.record_id}`, label: `删除记录 ${row.record_id}`,
          operations: [{ op: 'table.delete_records', source_id: sourceId, records: [{ record_id: row.record_id, expect: { rev: row.rev } }] }],
        })
      }}
    >删除</button>
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
  width: number
  editable: boolean
  /** The table is itself a wish result: per-cell provenance would mark every cell. */
  resultTable: boolean
  annotation?: AnnotationMark
  onAnnotate?: (anchor: CapturedAnchor) => void
  onActivateAnnotation?: (entityId: string | null) => void
}

function TableCell({ pager, sourceId, field, row, width, editable, resultTable, annotation, onAnnotate, onActivateAnnotation }: TableCellProps) {
  const store = useStore()
  const editId = cellEditId(sourceId, row.record_id, field.field_id)
  const entry = useEdit(editId)
  // The rev the user saw when the edit began: a change that arrives while typing must conflict, not be overwritten.
  const [editing, setEditing] = useState<{ baseRev: number; text?: string } | null>(null)
  const value = row.values[field.field_id]
  const meta = row.meta?.[field.field_id]
  const pending = entry && entry.state !== 'committed' && entry.hasMine
  // When the inline editor closes by Enter/Escape focus would fall to <body>: give it back to the cell.
  const valueRef = useRef<HTMLButtonElement>(null)
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

  const shown = pending ? (entry.mine === null ? '' : formatValue(field, entry.mine)) : formatValue(field, value)
  const startEdit = () => {
    if (!editable || editing) return
    setEditing({ baseRev: row.revs[field.field_id] ?? 0 })
  }

  return (
    <div
      className="aiws-td"
      role="gridcell"
      style={{ width }}
      data-testid={`aiws-cell-${row.record_id}-${field.field_id}`}
      data-state={entry?.state ?? 'clean'}
      data-editable={editable ? 'true' : 'false'}
      onDoubleClick={startEdit}
    >
      {editing ? (
        <ValueEditor
          def={field}
          value={value}
          initialText={editing.text}
          ariaLabel={`${field.name} ${row.record_id}`}
          onCancel={() => setEditing(null)}
          onCommit={(input) => { setEditing(null); void write(input, editing.baseRev) }}
        />
      ) : (
        <button ref={valueRef} type="button" className="aiws-cell-value" disabled={!editable} onClick={startEdit} title={editable ? '点击编辑' : undefined}>
          {shown || <span className="aiws-muted">&nbsp;</span>}
        </button>
      )}
      {!editing && meta?.derived && !resultTable && (
        <span className="aiws-chip aiws-chip-derived" data-testid="aiws-derived" title={`由 ${meta.derived.program ?? '加工程序'} 写入${meta.derived.program?.startsWith('mock.') ? '（模拟结果）' : ''}`}>
          {meta.derived.program?.startsWith('mock.') ? '模拟' : '派生'}
        </span>
      )}
      {!editing && meta?.manual_override && <span className="aiws-chip aiws-chip-warn" data-testid="aiws-manual-override" title="人工修改覆盖了程序写入的值">人工覆盖</span>}
      {!editing && annotation && (
        <button type="button" className="aiws-chip aiws-chip-note" title={annotation.payload.body} data-testid="aiws-cell-annotation"
          {...{ [`data-anno-${annotation.entityId}`]: '' }} onClick={() => onActivateAnnotation?.(annotation.entityId)}>注</button>
      )}
      {!editing && onAnnotate && !annotation && (
        <button type="button" className="aiws-cell-annotate" title="添加批注" aria-label={`批注 ${field.name} ${row.record_id}`}
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
          theirs={entry.hasTheirs ? formatValue(field, entry.theirs) || '（未设置）' : null}
          mine={entry.hasMine ? (entry.mine === null ? '（清空）' : formatValue(field, entry.mine ?? undefined)) : null}
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
        }}>重试</button>
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
  return (
    <div className="aiws-conflict" role="alertdialog" aria-label="需要处理的修改" data-testid="aiws-conflict">
      <div className="aiws-conflict-detail">{detail}</div>
      {theirs !== null && <div>当前值：<b data-testid="aiws-conflict-theirs">{theirs}</b></div>}
      {mine !== null && <div>我的输入：<b data-testid="aiws-conflict-mine">{mine}</b></div>}
      <div className="aiws-conflict-actions">
        {onKeepMine && <button type="button" data-testid="aiws-conflict-keep-mine" onClick={onKeepMine}>用我的输入覆盖</button>}
        {onEditAgain && <button type="button" onClick={onEditAgain}>继续编辑</button>}
        <button type="button" data-testid="aiws-conflict-discard" onClick={() => { void store.dismissEdit(editId) }}>{theirs !== null ? '采用当前值' : '放弃我的输入'}</button>
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
        <span>临时筛选：</span>
        <select aria-label="筛选字段" value={fieldId} onChange={(event) => { setFieldId(event.target.value); const next = fields.find((item) => item.field_id === event.target.value); setOperator(next ? operatorsFor(next.type)[0].id : 'eq'); setText('') }}>
          {fields.map((item) => <option key={item.field_id} value={item.field_id}>{item.name}</option>)}
        </select>
        <select aria-label="筛选运算符" value={activeOperator?.id ?? ''} onChange={(event) => setOperator(event.target.value)}>
          {operators.map((item) => <option key={item.id} value={item.id}>{item.label}</option>)}
        </select>
        {activeOperator?.needsValue && (field?.type === 'select' || field?.type === 'boolean' ? (
          <select aria-label="筛选值" value={text} onChange={(event) => setText(event.target.value)}>
            <option value="">请选择…</option>
            {field.type === 'boolean'
              ? [<option key="t" value="true">是</option>, <option key="f" value="false">否</option>]
              : (field.options ?? []).map((option) => <option key={option.option_id} value={option.option_id}>{option.label}</option>)}
          </select>
        ) : (
          <input aria-label="筛选值" type={field?.type === 'date' ? 'date' : 'text'} value={text} onChange={(event) => setText(event.target.value)} placeholder={field?.type === 'multi_select' ? '选项 ID，逗号分隔' : '值'} />
        ))}
      </div>
      <div>
        <span>临时排序：</span>
        <select aria-label="排序字段" value={sortField} onChange={(event) => setSortField(event.target.value)}>
          <option value="">（沿用视图）</option>
          {sortable.map((item) => <option key={item.field_id} value={item.field_id}>{item.name}</option>)}
        </select>
        <select aria-label="排序方向" value={direction} onChange={(event) => setDirection(event.target.value === 'desc' ? 'desc' : 'asc')}>
          <option value="asc">升序</option><option value="desc">降序</option>
        </select>
      </div>
      <div>
        <button type="button" data-testid="aiws-filter-apply" onClick={apply}>应用到本会话</button>
        <button type="button" onClick={() => { onApply({ filter: null, sorts: null }); setText(''); setSortField('') }}>清除临时条件</button>
        {canSave && <button type="button" data-testid="aiws-view-save" disabled={!session.filter && !session.sorts} onClick={onSave} title="把本会话的筛选（与已保存筛选取“且”）和排序写入视图">保存视图</button>}
        {canSave && hasSavedFilter && <button type="button" onClick={() => onClearSaved('filter')}>移除已保存筛选</button>}
        {canSave && hasSavedSorts && <button type="button" onClick={() => onClearSaved('sorts')}>移除已保存排序</button>}
      </div>
      <div className="aiws-muted">临时条件只在本窗口生效，不产生提交；“保存视图”才写入文档。</div>
    </div>
  )
}

function AddRecordForm({ store, source, onDone }: { store: WorkspaceStore; source: SourceRead; onDone: () => void }) {
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
        if (field.required) { setError(`「${field.name}」必填`); return }
        continue
      }
      values[field.field_id] = field.type === 'number' ? Number(text) : field.type === 'boolean' ? text === 'true' : text
      typeRevs[field.field_id] = field.type_rev
    }
    const recordId = randomId('r')
    const outcome = await store.submit({
      editId: `record:${source.entity_id}:${recordId}`, label: `新增记录`, mine: values, hasMine: true,
      operations: [{ op: 'table.insert_records', source_id: source.entity_id, field_type_revs: typeRevs, records: [{ record_id: recordId, values }] }],
    })
    if (outcome.status === 'accepted') onDone()
    else setError('未能新增记录，详情见“需要处理”列表；输入已保留。')
  }
  return (
    <form className="aiws-inline-form" onSubmit={(event) => { event.preventDefault(); void submit() }}>
      {asked.length === 0 && <span className="aiws-muted">此表没有必填字段，将插入一条空记录。</span>}
      {asked.map((field, index) => (
        <label key={field.field_id}>{field.name}{field.required ? ' *' : ''}
          {field.type === 'select' ? (
            <select value={texts[field.field_id] ?? ''} onChange={(event) => setTexts({ ...texts, [field.field_id]: event.target.value })}>
              <option value="">（未设置）</option>
              {(field.options ?? []).map((option) => <option key={option.option_id} value={option.option_id}>{option.label}</option>)}
            </select>
          ) : (
            <input aria-label={`新记录 ${field.name}`} autoFocus={index === 0} type={field.type === 'date' ? 'date' : 'text'} value={texts[field.field_id] ?? ''} onChange={(event) => setTexts({ ...texts, [field.field_id]: event.target.value })} />
          )}
        </label>
      ))}
      <button type="submit" data-testid="aiws-add-record-submit">插入</button>
      <button type="button" onClick={onDone}>取消</button>
      {error && <span className="aiws-error" role="alert">{error}</span>}
    </form>
  )
}
