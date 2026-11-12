/* The official to-do control's body (控件机制 TODO §0.2): one table of to-dos seen as a list or as a board, both from
 * the same records, projected as label and shape Blocks (G2) and laid out by the host's `list` / `columns` layouts (G3).
 *
 *   list    quick add (Enter keeps adding), tick to complete, drag to reorder (an insertion line), completed ones in a
 *           section that folds, several selected for one action; ↑ ↓ move, Space completes, Enter renames, Delete
 *   board   one column per status option in its order; a card dragged to another column changes its status (a
 *           placeholder shows where it lands), cards sort inside a column, columns fold, "+ column" adds a status
 *           option (needs `structure`; without it the button says why)
 *
 * Every drop is one Commit with the cells it changes guarded by their versions (two people moving two cards both
 * win; the same card: one wins, the other sees the service's result and a notice) and goes on the undo stack. On a
 * phone a long press starts the drag; lifting without moving opens the item's menu with "Move to…". */

import { useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent } from 'react'
import { ArrowDownToLine, ArrowUpToLine, Check, ChevronDown, ChevronRight, Columns3, CornerDownLeft, Ellipsis, Expand, FolderInput, Link2, List, MessageSquarePlus, PanelLeftClose, PanelLeftOpen, Pencil, Plus, Trash2, Undo2 } from 'lucide-react'
import { columnDropAt, insertionAt, layoutColumns, layoutList, type LaidColumn } from '@controls/datalayout'
import { instantiate, type ProjectedNode, type Rect } from '@controls/projection'
import { deleteRecordsOp, orderAt, orderKeysBetween, sortTodos, TODO_DONE, todoAddOps, todoCompleteOps, todoMoveOps, todoUpdateOps, type TodoItem } from 'buckyos'
import type { Json, Operation } from '../../../api/types'
import { useEntity, useReadOnlyReason, useStore, useWorkspaceUi } from '../../../state/hooks'
import { useAiwsText } from '../../../useAiwsText'
import { ControlMenu, ControlPopover, anchorOf, type AnchorRect, type MenuItem } from '../data/ControlPopover'
import { edgeScroll, localPoint, trackPress, type Point } from '../data/gestures'
import { ProjectedView, projectedAt } from '../data/ProjectedView'
import { useRecords } from '../data/records'
import { useControlView } from '../data/viewState'
import { submitData } from '../data/write'
import { UnboundSource } from '../data/SourceChooser'
import { registerControlKeys } from '../data/keys'
import { annotatedRecords, copyRecordLink, openControlPage, recordAnchor, useRecordFocus } from '../data/focus'
import type { ControlBodyProps } from '../registry'
import { TodoDetail } from './TodoDetail'
import { CARD_GAP, CARD_TEMPLATE, cardHeight, COLUMN_COLLAPSED_W, COLUMN_GAP, COLUMN_HEADER, COLUMN_W, ITEMS_BINDING, itemScope, itemsOf, LIST_GAP, LIST_ROW_H, LIST_TEMPLATE, todayLocal, TODO_VIEW_DEFAULTS, TODO_VIEW_KEYS, type TodoView } from './model'

const DONE_HEADER_H = 34
const MODE_LABEL = { list: 'aiworkspace.todo.mode.list', board: 'aiworkspace.todo.mode.board' } as const
const OVERSCAN = 400

type Drag =
  | { kind: 'list'; ids: string[]; done: boolean; pointer: Point; offset: Point; index: number | null }
  | { kind: 'board'; ids: string[]; pointer: Point; offset: Point; target: { column: string; index: number } | null }

type Popover =
  | { kind: 'detail'; id: string; anchor: AnchorRect }
  | { kind: 'menu'; ids: string[]; anchor: AnchorRect }
  | { kind: 'view'; anchor: AnchorRect }

export function TodoBody({ containerId, mode, active, size, phone, host }: ControlBodyProps) {
  const { t } = useAiwsText()
  const store = useStore()
  const container = useEntity(containerId)
  const sourceId = container?.bindings?.[ITEMS_BINDING] ?? null
  const source = useEntity(sourceId ?? '')
  const { set, snapshot } = useRecords(sourceId)
  const ctl = useControlView<TodoView>(containerId, TODO_VIEW_DEFAULTS, TODO_VIEW_KEYS)
  const view = ctl.view
  const readOnlyNow = useReadOnlyReason()
  const caps = source?.capabilities ?? []
  const canWrite = !readOnlyNow && mode !== 'presentation_edit' && caps.includes('update')
  const canAdd = !readOnlyNow && mode !== 'presentation_edit' && (caps.includes('append') || caps.includes('update'))
  const canStructure = !readOnlyNow && caps.includes('structure')
  const noWrite = readOnlyNow ?? (mode === 'presentation_edit' ? t('aiworkspace.datactl.notHere') : t('aiworkspace.datactl.noUpdate'))
  const statusField = snapshot.fields.find((f) => f.field_id === 'status')
  const options = useMemo(() => statusField?.options ?? [{ option_id: 'todo', label: t('aiworkspace.todo.status.todo') }, { option_id: TODO_DONE, label: t('aiworkspace.todo.status.done') }], [statusField, t])
  const doneId = options.some((o) => o.option_id === TODO_DONE) ? TODO_DONE : options[options.length - 1]?.option_id ?? TODO_DONE
  const items = useMemo(() => sortTodos(itemsOf(snapshot.rows)), [snapshot.rows])
  const byId = useMemo(() => new Map(items.map((i) => [i.record_id, i])), [items])
  const [rawSelection, setSelection] = useState<string[]>([])
  const [anchorSel, setAnchorSel] = useState<string | null>(null)
  const [drag, setDrag] = useState<Drag | null>(null)
  // the drop reads the drag as it was last drawn (the write happens outside any state update)
  const dragRef = useRef<Drag | null>(null)
  useEffect(() => { dragRef.current = drag }, [drag])
  const [editing, setEditing] = useState<{ id: string; text: string } | null>(null)
  const [popover, setPopover] = useState<Popover | null>(null)
  const [adding, setAdding] = useState<{ column: string | null; text: string } | null>(null)
  const [newColumn, setNewColumn] = useState<string | null>(null)
  const [scroll, setScroll] = useState({ top: 0, left: 0, h: 600, w: 600 })
  const rootRef = useRef<HTMLDivElement>(null)
  const scrollRef = useRef<HTMLDivElement | null>(null)
  const [scrollEl, setScrollEl] = useState<HTMLDivElement | null>(null)
  const ui = useWorkspaceUi()
  const noted = useMemo(() => annotatedRecords(ui.annotations, sourceId), [ui.annotations, sourceId])
  const today = todayLocal()
  // a record that left (deleted elsewhere) leaves the selection
  const selection = useMemo(() => rawSelection.filter((id) => byId.has(id)), [rawSelection, byId])
  const selected = useMemo(() => new Set(selection), [selection])

  const contentW = Math.max(120, size.w - (phone ? 16 : 24))

  // ---- layout (list)
  const open = items.filter((i) => !i.done)
  const done = items.filter((i) => i.done)
  const list = useMemo(() => {
    const top = layoutList(open.map((i) => ({ key: i.record_id, h: LIST_ROW_H })), { width: contentW, gap: LIST_GAP })
    const doneY = top.height + (open.length ? 10 : 0)
    const bottom = view.hide_done ? null : layoutList(done.map((i) => ({ key: i.record_id, h: LIST_ROW_H })), { y: doneY + DONE_HEADER_H, width: contentW, gap: LIST_GAP })
    const rects = new Map([...top.rects, ...(bottom?.rects ?? [])])
    return { rects, openKeys: open.map((i) => i.record_id), doneKeys: done.map((i) => i.record_id), doneY, height: doneY + (done.length ? DONE_HEADER_H : 0) + (bottom?.height ?? 0) + 16 }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [items, contentW, view.hide_done])

  // ---- layout (board)
  const columnW = phone ? Math.max(200, size.w - 40) : COLUMN_W
  const board = useMemo(() => {
    const collapsed = new Set(view.collapsed)
    const cols = options.map((o) => ({ id: o.option_id, collapsed: collapsed.has(o.option_id) }))
    let cardItems = items.map((i) => ({ key: i.record_id, column: options.some((o) => o.option_id === i.status) ? i.status : options[0]?.option_id ?? '', h: cardHeight(i, columnW - 16) }))
    // while a card is dragged, the board makes room where it would land
    if (drag?.kind === 'board' && drag.target) {
      const moving = new Set(drag.ids)
      const moved = cardItems.filter((c) => moving.has(c.key))
      const rest = cardItems.filter((c) => !moving.has(c.key))
      const inColumn = rest.filter((c) => c.column === drag.target!.column)
      const at = inColumn[drag.target.index]
      const slot = { key: '__slot', column: drag.target.column, h: moved.reduce((h, c) => h + c.h + CARD_GAP, -CARD_GAP) }
      const index = at ? rest.indexOf(at) : rest.length
      cardItems = [...rest.slice(0, index), slot, ...rest.slice(index), ...moved.map((c) => ({ ...c, column: '__dragging' }))]
    }
    const height = Math.max(size.h - 60, 200)
    return layoutColumns(cols, cardItems, { height, columnWidth: columnW, collapsedWidth: COLUMN_COLLAPSED_W, gap: COLUMN_GAP, header: COLUMN_HEADER, padding: 8, cardGap: CARD_GAP })
  }, [items, options, view.collapsed, columnW, drag, size.h])
  const boardHeight = Math.max(...board.columns.map((c) => COLUMN_HEADER + c.contentHeight + 56), size.h - 60)

  // ---- projection: only what is near the visible part is instantiated (G7)
  const nodes = useMemo(() => {
    const out: ProjectedNode[] = []
    const visible = (r: Rect) => r.y + r.h > scroll.top - OVERSCAN && r.y < scroll.top + scroll.h + OVERSCAN && r.x + r.w > scroll.left - OVERSCAN && r.x < scroll.left + scroll.w + OVERSCAN
    const rects = view.mode === 'list' ? list.rects : board.rects
    const template = view.mode === 'list' ? LIST_TEMPLATE : CARD_TEMPLATE
    for (const item of items) {
      const rect = rects.get(item.record_id)
      if (!rect || !visible(rect)) continue
      const statusLabel = options.find((o) => o.option_id === item.status)?.label ?? item.status
      out.push(...instantiate(containerId, template, { key: item.record_id, scope: itemScope(item, { selected: selected.has(item.record_id), today, statusLabel, doneId, noted: noted.has(item.record_id) }) }, rect))
    }
    return out
  }, [items, list, board, view.mode, scroll, selected, today, options, doneId, containerId, noted])

  // ---- writes
  const write = (label: string, operations: Operation[], effects: { overlay?: Record<string, Record<string, Json | undefined>>; created?: Record<string, Record<string, Json | undefined>>; removed?: string[] } = {}) => {
    if (!sourceId) return Promise.resolve(false)
    return submitData(store, set, { editId: `todo:${sourceId}:${Date.now()}`, label, operations, ...effects })
  }

  const toggleDone = (ids: string[], value?: boolean) => {
    if (!sourceId || !canWrite) { if (!canWrite) store.notify('info', noWrite); return }
    const targets = ids.map((id) => byId.get(id)).filter((i): i is TodoItem => Boolean(i))
    const to = value ?? !targets.every((i) => i.done)
    const now = new Date().toISOString()
    const ops: Operation[] = []
    const overlay: Record<string, Record<string, Json | undefined>> = {}
    for (const item of targets) {
      if (item.done === to) continue
      ops.push(...todoCompleteOps(sourceId, item, to, { now, reopenStatus: options[0]?.option_id }) as Operation[])
      overlay[item.record_id] = to ? { status: doneId, done_at: now } : { status: options[0]?.option_id ?? 'todo', done_at: undefined }
    }
    void write(t(to ? 'aiworkspace.todo.completeN' : 'aiworkspace.todo.reopenN', { n: Object.keys(overlay).length }), ops, { overlay })
  }

  const addItem = async (title: string, status?: string) => {
    if (!sourceId || !title.trim()) return
    if (!canAdd) { store.notify('info', noWrite); return }
    const column = status ?? options[0]?.option_id ?? 'todo'
    const inStatus = sortTodos(items.filter((i) => (status ? i.status === status : !i.done)))
    const { record_id, ops, order } = todoAddOps(sourceId, { title: title.trim(), status: column }, { items: inStatus })
    await write(t('aiworkspace.todo.addOne', { title: title.trim() }), ops as Operation[], { created: { [record_id]: { title: title.trim(), status: column, order } } })
    // the new item scrolls into sight
    window.requestAnimationFrame(() => scrollRef.current?.querySelector(`[data-item="${record_id}"]`)?.scrollIntoView({ block: 'nearest' }))
  }

  const removeItems = (ids: string[]) => {
    if (!sourceId || !canWrite) { if (!canWrite) store.notify('info', noWrite); return }
    const rows = ids.map((id) => byId.get(id)).filter((i): i is TodoItem => Boolean(i))
    if (!rows.length) return
    void write(t('aiworkspace.todo.deleteN', { n: rows.length }), [deleteRecordsOp(sourceId, rows) as Operation], { removed: rows.map((r) => r.record_id) })
    setSelection([])
  }

  /** Move items to `status` (null: keep) at `index` among the items of that place (the moved ones excluded). */
  const moveItems = (ids: string[], to: { status: string | null; index: number; doneSection?: boolean }) => {
    if (!sourceId || !canWrite) { if (!canWrite) store.notify('info', noWrite); return }
    const moving = ids.map((id) => byId.get(id)).filter((i): i is TodoItem => Boolean(i))
    if (!moving.length) return
    const ids_ = new Set(ids)
    const place = view.mode === 'board' && to.status !== null
      ? items.filter((i) => i.status === to.status && !ids_.has(i.record_id))
      : items.filter((i) => (to.doneSection ? i.done : !i.done) && !ids_.has(i.record_id))
    const before = to.index > 0 ? place[to.index - 1]?.order ?? null : null
    const after = to.index < place.length ? place[to.index]?.order ?? null : null
    let keys: string[]
    try {
      keys = moving.length === 1 ? [orderAt(place, to.index)] : orderKeysBetween(before, after && before !== null && after <= before ? null : after, moving.length)
    } catch {
      store.notify('error', t('aiworkspace.todo.orderExhausted'))
      return
    }
    const now = new Date().toISOString()
    const ops: Operation[] = []
    const overlay: Record<string, Record<string, Json | undefined>> = {}
    moving.forEach((item, i) => {
      const status = to.status ?? item.status
      ops.push(...todoMoveOps(sourceId, item, { status, order: keys[i] }, { now }) as Operation[])
      overlay[item.record_id] = { order: keys[i], status, ...(status !== item.status ? { done_at: status === doneId ? now : undefined } : {}) }
    })
    const statusLabel = to.status ? options.find((o) => o.option_id === to.status)?.label ?? to.status : null
    void write(statusLabel ? t('aiworkspace.todo.moveToStatus', { n: moving.length, status: statusLabel }) : t('aiworkspace.todo.reorderN', { n: moving.length }), ops, { overlay })
  }

  const rename = (id: string, text: string) => {
    const item = byId.get(id)
    if (!item || !sourceId || text.trim() === item.title || !text.trim()) return
    void write(t('aiworkspace.todo.renameOne'), todoUpdateOps(sourceId, item, { title: text.trim() }) as Operation[], { overlay: { [id]: { title: text.trim() } } })
  }

  const addColumn = (label: string) => {
    if (!sourceId || !label.trim()) return
    const base = label.trim().toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 24) || 's'
    let id = /^[a-z0-9]/.test(base) ? base : `s${base}`
    const ids = new Set(options.map((o) => o.option_id))
    for (let n = 2; ids.has(id); n++) id = `${base}-${n}`
    // a new status goes before "done" (completed stays the last column)
    const doneIndex = options.findIndex((o) => o.option_id === doneId)
    const index = doneIndex >= 0 ? doneIndex : options.length
    void store.submit({ editId: `todo:${sourceId}:option`, label: t('aiworkspace.todo.addColumnEdit', { name: label.trim() }), operations: [{ op: 'table.add_option', source_id: sourceId, field_id: 'status', option: { option_id: id, label: label.trim() }, index, expect: { rev: statusField?.def_rev ?? 0 } }] })
      .then((outcome) => { if (outcome.status === 'accepted') void set?.reload() })
  }

  // ---- selection and keys
  const select = (id: string, event?: { shiftKey?: boolean; metaKey?: boolean; ctrlKey?: boolean }) => {
    const order = view.mode === 'list' ? [...list.openKeys, ...(view.hide_done ? [] : list.doneKeys)] : board.columns.flatMap((c) => c.keys.filter((k) => !k.startsWith('__')))
    if (event?.shiftKey && anchorSel && order.includes(anchorSel)) {
      const a = order.indexOf(anchorSel), b = order.indexOf(id)
      setSelection(order.slice(Math.min(a, b), Math.max(a, b) + 1))
      return
    }
    if (event?.metaKey || event?.ctrlKey) {
      setSelection((s) => (s.includes(id) ? s.filter((x) => x !== id) : [...s, id]))
      setAnchorSel(id)
      return
    }
    setSelection([id])
    setAnchorSel(id)
  }

  const visibleOrder = () => (view.mode === 'list' ? [...list.openKeys, ...(view.hide_done ? [] : list.doneKeys)] : board.columns.flatMap((c) => c.keys.filter((k) => !k.startsWith('__'))))

  const onKey = (event: KeyboardEvent | ReactKeyboardEvent): boolean => {
    if (editing || adding || popover) return false
    const mod = event.metaKey || event.ctrlKey
    const order = visibleOrder()
    const current = selection[selection.length - 1]
    const at = current ? order.indexOf(current) : -1
    if ((event.key === 'ArrowDown' || event.key === 'ArrowUp') && !mod) {
      if (event.altKey && current && canWrite) {
        // Alt+↑ / ↓ moves the item
        const item = byId.get(current)
        if (!item) return true
        const peers = view.mode === 'board' ? items.filter((i) => i.status === item.status) : items.filter((i) => i.done === item.done)
        const i = peers.findIndex((p) => p.record_id === current)
        const next = event.key === 'ArrowDown' ? Math.min(peers.length - 1, i + 1) : Math.max(0, i - 1)
        if (next !== i) moveItems([current], { status: view.mode === 'board' ? item.status : null, index: next, doneSection: item.done })
        return true
      }
      const next = order[Math.max(0, Math.min(order.length - 1, at < 0 ? 0 : at + (event.key === 'ArrowDown' ? 1 : -1)))]
      if (next) {
        select(next, { shiftKey: event.shiftKey })
        scrollRef.current?.querySelector(`[data-item="${next}"]`)?.scrollIntoView({ block: 'nearest' })
      }
      return true
    }
    if ((event.key === 'ArrowLeft' || event.key === 'ArrowRight') && view.mode === 'board' && current) {
      const item = byId.get(current)
      const ci = options.findIndex((o) => o.option_id === item?.status)
      const target = options[ci + (event.key === 'ArrowRight' ? 1 : -1)]
      if (!item || !target) return true
      if (event.altKey && canWrite) { moveItems([current], { status: target.option_id, index: 0 }); return true }
      const first = board.columns.find((c) => c.id === target.option_id)?.keys.find((k) => !k.startsWith('__'))
      if (first) select(first)
      return true
    }
    if (event.key === ' ' && selection.length) { toggleDone(selection); return true }
    if (event.key === 'Enter' && current && !mod) { setEditing({ id: current, text: byId.get(current)?.title ?? '' }); return true }
    if ((event.key === 'Delete' || event.key === 'Backspace') && selection.length) { removeItems(selection); return true }
    if (mod && event.key.toLowerCase() === 'a') { setSelection(order); return true }
    if (event.key === 'Escape' && selection.length) { setSelection([]); return true }
    if (event.key === 'n' && !mod && canAdd) { setAdding({ column: view.mode === 'board' ? options[0]?.option_id ?? null : null, text: '' }); return true }
    return false
  }

  // a link to one of the records (G12): show it (its section unfolded, its column open) and select it
  useRecordFocus(containerId, (focus) => {
    const item = byId.get(focus.record)
    if (item?.done && view.mode === 'list' && view.hide_done) ctl.set({ hide_done: false })
    if (item && view.mode === 'board' && view.collapsed.includes(item.status)) ctl.set({ collapsed: view.collapsed.filter((c) => c !== item.status) })
    setSelection([focus.record])
    setAnchorSel(focus.record)
    window.setTimeout(() => scrollRef.current?.querySelector(`[data-item="${focus.record}"]`)?.scrollIntoView({ block: 'center' }), 80)
  })

  // the canvas hands the keys of a selected control to it (the handler of the latest render)
  const keyRef = useRef(onKey)
  useEffect(() => { keyRef.current = onKey })
  useEffect(() => registerControlKeys(containerId, (event) => keyRef.current(event)), [containerId])

  // ---- pointer
  const toLocal = (x: number, y: number) => localPoint(scrollRef.current, x, y)
  const onPointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (!active) return
    const hit = projectedAt(event.target, scrollRef.current)
    if (!hit) {
      if ((event.target as HTMLElement).closest('.aiws-dc-canvas') && !(event.target as HTMLElement).closest('button, input')) setSelection([])
      return
    }
    const id = hit.item
    const item = byId.get(id)
    if (!item) return
    rootRef.current?.focus({ preventScroll: true })
    trackPress(event.nativeEvent, `${id}:${hit.role}`, toLocal, {
      onTap: (_p, e) => {
        if (hit.role === 'check') { toggleDone(selected.has(id) && selection.length > 1 ? selection : [id]); return }
        if (phone && !e.shiftKey) { openDetail(id); return }
        select(id, e)
      },
      onDoubleTap: () => {
        if (hit.role === 'title' && canWrite) setEditing({ id, text: item.title })
        else openDetail(id)
      },
      onLongPress: () => openMenu(selected.has(id) ? selection : [id], id),
      onDragStart: canWrite && hit.role !== 'check' ? (start) => {
        const ids = selected.has(id) ? selection.filter((s) => byId.get(s)?.done === item.done || view.mode === 'board') : [id]
        if (!selected.has(id)) { setSelection([id]); setAnchorSel(id) }
        const rect = (view.mode === 'list' ? list.rects : board.rects).get(id)
        const offset = rect ? { x: start.x - rect.x, y: start.y - rect.y } : { x: 0, y: 0 }
        setDrag(view.mode === 'list' ? { kind: 'list', ids, done: item.done, pointer: start, offset, index: null } : { kind: 'board', ids, pointer: start, offset, target: null })
        return true
      } : undefined,
      onDragMove: (p) => {
        const client = event.nativeEvent
        void client
        setDrag((d) => {
          if (!d) return d
          if (d.kind === 'list') {
            const keys = (d.done ? list.doneKeys : list.openKeys).filter((k) => !d.ids.includes(k))
            const at = insertionAt(keys, list.rects, p.y)
            return { ...d, pointer: p, index: at.index }
          }
          const target = columnDropAt(board, p.x, p.y, undefined)
          if (!target) return { ...d, pointer: p }
          const keys = (board.columns.find((c) => c.id === target.column)?.keys ?? []).filter((k) => !k.startsWith('__') && !d.ids.includes(k))
          const at = insertionAt(keys, board.rects, p.y)
          return { ...d, pointer: p, target: board.columns.find((c) => c.id === target.column)?.collapsed ? { column: target.column, index: 0 } : { column: target.column, index: at.index } }
        })
      },
      onDragEnd: (_p, _s, cancelled) => {
        const d = dragRef.current
        setDrag(null)
        if (!d || cancelled) return
        if (d.kind === 'list' && d.index !== null) moveItems(d.ids, { status: null, index: d.index, doneSection: d.done })
        if (d.kind === 'board' && d.target) moveItems(d.ids, { status: d.target.column, index: d.target.index })
      },
    })
  }
  // a drag near the edge scrolls
  useEffect(() => {
    if (!drag) return
    const onMove = (e: PointerEvent) => edgeScroll(scrollRef.current, e.clientX, e.clientY, view.mode === 'board' ? 'both' : 'y')
    window.addEventListener('pointermove', onMove)
    return () => window.removeEventListener('pointermove', onMove)
  }, [drag, view.mode])

  const openDetail = (id: string) => {
    const rect = (view.mode === 'list' ? list.rects : board.rects).get(id)
    setPopover({ kind: 'detail', id, anchor: anchorOf(scrollEl, rect) })
  }
  const openMenu = (ids: string[], at: string) => {
    const rect = (view.mode === 'list' ? list.rects : board.rects).get(at)
    setPopover({ kind: 'menu', ids, anchor: anchorOf(scrollEl, rect) })
  }

  // keep the visible window for the projection
  useLayoutEffect(() => {
    const el = scrollRef.current
    if (!el) return
    const update = () => setScroll({ top: el.scrollTop, left: el.scrollLeft, h: el.clientHeight, w: el.clientWidth })
    update()
    el.addEventListener('scroll', update, { passive: true })
    return () => el.removeEventListener('scroll', update)
  }, [view.mode, size.w, size.h])

  if (!container) return null
  if (!sourceId) return <UnboundSource containerId={containerId} />
  if (snapshot.error) return <div className="aiws-dc-empty aiws-error" role="alert">{t('aiworkspace.datactl.readFailed', { error: snapshot.error })}</div>

  const dragging = drag !== null
  const editingRect = editing ? (() => {
    const rect = (view.mode === 'list' ? list.rects : board.rects).get(editing.id)
    if (!rect) return null
    return view.mode === 'list' ? { x: rect.x + 42, y: rect.y + 4, w: rect.w - 146, h: rect.h - 8 } : { x: rect.x + 34, y: rect.y + 6, w: rect.w - 42, h: Math.max(26, rect.h - 32) }
  })() : null

  // ---- menus
  const menuItems = (ids: string[]): (MenuItem | 'separator')[] => {
    const first = byId.get(ids[0])
    const allDone = ids.every((id) => byId.get(id)?.done)
    const items_: (MenuItem | 'separator')[] = [
      { id: 'detail', label: t('aiworkspace.todo.openDetail'), icon: <Pencil size={14} />, disabled: ids.length !== 1 ? t('aiworkspace.todo.oneOnly') : false, run: () => openDetail(ids[0]) },
      { id: 'toggle', label: allDone ? t('aiworkspace.todo.reopen') : t('aiworkspace.todo.complete'), icon: allDone ? <Undo2 size={14} /> : <Check size={14} />, disabled: canWrite ? false : noWrite, run: () => toggleDone(ids, !allDone) },
      'separator',
      { id: 'top', label: t('aiworkspace.todo.moveTop'), icon: <ArrowUpToLine size={14} />, disabled: canWrite ? false : noWrite, run: () => moveItems(ids, { status: view.mode === 'board' && first ? first.status : null, index: 0, doneSection: first?.done }) },
      { id: 'bottom', label: t('aiworkspace.todo.moveBottom'), icon: <ArrowDownToLine size={14} />, disabled: canWrite ? false : noWrite, run: () => moveItems(ids, { status: view.mode === 'board' && first ? first.status : null, index: 1e9, doneSection: first?.done }) },
      ...options.filter((o) => ids.some((id) => byId.get(id)?.status !== o.option_id)).map((o): MenuItem => ({
        id: `status-${o.option_id}`, label: t('aiworkspace.todo.moveToNamed', { status: o.label }), icon: <FolderInput size={14} />, disabled: canWrite ? false : noWrite,
        run: () => moveItems(ids, { status: o.option_id, index: 1e9 }),
      })),
      'separator',
      { id: 'link', label: t('aiworkspace.datactl.copyLink'), icon: <Link2 size={14} />, disabled: ids.length !== 1 ? t('aiworkspace.todo.oneOnly') : false, run: () => copyRecordLink(store, containerId, ids[0], null, t('aiworkspace.datactl.linkCopied')) },
      ...(ui.annotate && sourceId ? [{ id: 'annotate', label: t('aiworkspace.datactl.annotate'), icon: <MessageSquarePlus size={14} />, disabled: ids.length !== 1 ? t('aiworkspace.todo.oneOnly') : false, run: () => ui.annotate?.(recordAnchor(sourceId, ids[0], byId.get(ids[0])?.title ?? ids[0])) } as MenuItem] : []),
      'separator',
      { id: 'delete', label: t('aiworkspace.todo.deleteLabel', { n: ids.length }), icon: <Trash2 size={14} />, danger: true, disabled: canWrite ? false : noWrite, run: () => removeItems(ids) },
    ]
    return items_
  }

  const header = (
    <div className="aiws-dc-head" data-testid={`aiws-todo-head-${containerId}`}>
      <span className="aiws-dc-title" title={container.title ?? undefined}>{container.title || t('aiworkspace.todo.title')}</span>
      <span className="aiws-dc-count" data-testid={`aiws-todo-count-${containerId}`}>{t('aiworkspace.todo.openCount', { n: open.length })}</span>
      <span className="aiws-dc-spacer" />
      <div className="aiws-dc-seg" role="radiogroup" aria-label={t('aiworkspace.todo.mode')}>
        {(['list', 'board'] as const).map((m) => (
          <button key={m} type="button" role="radio" aria-checked={view.mode === m} className={view.mode === m ? 'is-on' : undefined} data-testid={`aiws-todo-mode-${m}`}
            title={t(MODE_LABEL[m])} disabled={!active} onClick={() => ctl.set({ mode: m })}>
            {m === 'list' ? <List size={14} aria-hidden="true" /> : <Columns3 size={14} aria-hidden="true" />}<span>{t(MODE_LABEL[m])}</span>
          </button>
        ))}
      </div>
      <button type="button" className="aiws-dc-iconbtn" aria-label={t('aiworkspace.datactl.viewMenu')} title={t('aiworkspace.datactl.viewMenu')} disabled={!active} data-testid={`aiws-todo-menu-${containerId}`}
        onClick={(e) => setPopover({ kind: 'view', anchor: anchorOf(e.currentTarget) })}><Ellipsis size={16} aria-hidden="true" /></button>
    </div>
  )

  return (
    <div ref={rootRef} className={`aiws-dc aiws-todo is-${view.mode}${active ? ' is-active' : ''}${dragging ? ' is-dragging' : ''}${phone ? ' is-phone' : ''}`}
      data-testid={`aiws-todo-${containerId}`} data-host={host} data-role={active ? 'editor' : undefined} tabIndex={active ? 0 : -1}
      onKeyDown={(event) => { if (event.target === rootRef.current && onKey(event)) { event.preventDefault(); event.stopPropagation() } }}
      onContextMenu={(event) => {
        const hit = projectedAt(event.target, scrollRef.current)
        if (!hit || !active) return
        event.preventDefault()
        openMenu(selected.has(hit.item) ? selection : [hit.item], hit.item)
      }}>
      {header}
      {view.mode === 'list' && (
        <div className="aiws-todo-add">
          <Plus size={16} aria-hidden="true" />
          <input value={adding?.column === null ? adding.text : ''} placeholder={canAdd ? t('aiworkspace.todo.addPlaceholder') : noWrite} disabled={!canAdd || !active} data-testid={`aiws-todo-add-${containerId}`}
            onFocus={() => { if (!adding) setAdding({ column: null, text: '' }) }}
            onChange={(e) => setAdding({ column: null, text: e.target.value })}
            onBlur={() => { if (!adding?.text) setAdding(null) }}
            onKeyDown={(e) => {
              e.stopPropagation()
              if (e.nativeEvent.isComposing) return
              if (e.key === 'Enter' && adding?.text.trim()) { void addItem(adding.text); setAdding({ column: null, text: '' }) }
              if (e.key === 'Escape') { setAdding(null); (e.target as HTMLInputElement).blur(); rootRef.current?.focus() }
            }} />
          <CornerDownLeft size={14} className="aiws-todo-add-hint" aria-hidden="true" />
        </div>
      )}
      <div ref={(el) => { scrollRef.current = el; if (el !== scrollEl) setScrollEl(el) }} className={`aiws-dc-scroll${view.mode === 'board' ? ' is-board' : ''}`} onPointerDown={onPointerDown} data-testid={`aiws-todo-scroll-${containerId}`}>
        <div className="aiws-dc-canvas" style={view.mode === 'list' ? { height: list.height, width: contentW } : { height: boardHeight, width: board.width + (canStructure || caps.length ? 220 : 0) }}>
          {view.mode === 'board' && board.columns.map((c) => (
            <BoardColumn key={c.id} column={c} label={options.find((o) => o.option_id === c.id)?.label ?? c.id} count={c.keys.filter((k) => !k.startsWith('__')).length} height={boardHeight}
              dropping={drag?.kind === 'board' && drag.target?.column === c.id} active={active}
              onToggle={() => ctl.set({ collapsed: c.collapsed ? view.collapsed.filter((x) => x !== c.id) : [...view.collapsed, c.id] })}
              adding={adding?.column === c.id ? adding.text : null} canAdd={canAdd} noWrite={noWrite}
              onAddStart={() => setAdding({ column: c.id, text: '' })} onAddText={(text) => setAdding({ column: c.id, text })}
              onAddDone={(commit) => { if (commit && adding?.text.trim()) { void addItem(adding.text, c.id); setAdding({ column: c.id, text: '' }) } else setAdding(null) }} />
          ))}
          {view.mode === 'board' && (
            <div className="aiws-board-newcol" style={{ left: board.width + COLUMN_GAP, width: 200 }}>
              {newColumn === null ? (
                <button type="button" className="aiws-dc-ghostbtn" disabled={!canStructure || !active} title={canStructure ? undefined : t('aiworkspace.todo.needStructure')} data-testid={`aiws-todo-addcol-${containerId}`}
                  onClick={() => setNewColumn('')}><Plus size={14} aria-hidden="true" /> {t('aiworkspace.todo.addColumn')}</button>
              ) : (
                <input autoFocus value={newColumn} placeholder={t('aiworkspace.todo.columnName')} data-testid={`aiws-todo-colname-${containerId}`}
                  onChange={(e) => setNewColumn(e.target.value)} onBlur={() => setNewColumn(null)}
                  onKeyDown={(e) => { e.stopPropagation(); if (e.key === 'Enter') { addColumn(newColumn); setNewColumn(null) } if (e.key === 'Escape') setNewColumn(null) }} />
              )}
              {!canStructure && <div className="aiws-dc-note">{t('aiworkspace.todo.needStructure')}</div>}
            </div>
          )}
          {view.mode === 'list' && (done.length > 0) && (
            <button type="button" className="aiws-todo-donehead" style={{ top: list.doneY, width: contentW }} disabled={!active} data-testid={`aiws-todo-donehead-${containerId}`}
              onClick={() => ctl.set({ hide_done: !view.hide_done })}>
              {view.hide_done ? <ChevronRight size={14} aria-hidden="true" /> : <ChevronDown size={14} aria-hidden="true" />}
              {t('aiworkspace.todo.doneSection', { n: done.length })}
            </button>
          )}
          {nodes.map((node) => (
            <ProjectedView key={node.id} node={node} className={drag && drag.ids.includes(node.item) ? 'is-drag-source' : undefined} />
          ))}
          {drag?.kind === 'list' && drag.index !== null && (() => {
            const keys = (drag.done ? list.doneKeys : list.openKeys).filter((k) => !drag.ids.includes(k))
            const at = insertionAt(keys, list.rects, drag.pointer.y)
            const y = keys.length ? at.lineY : drag.done ? list.doneY + DONE_HEADER_H : 0
            return <div className="aiws-dc-insert" style={{ top: y - 1, width: contentW }} data-testid="aiws-todo-insert" />
          })()}
          {drag?.kind === 'board' && drag.target && board.rects.get('__slot') && (() => {
            const r = board.rects.get('__slot')!
            return <div className="aiws-dc-slot" style={{ left: r.x, top: r.y, width: r.w, height: r.h }} data-testid="aiws-todo-slot" />
          })()}
          {drag && (() => {
            const first = byId.get(drag.ids[0])
            if (!first) return null
            const w = view.mode === 'list' ? contentW : columnW - 16
            const h = view.mode === 'list' ? LIST_ROW_H : cardHeight(first, w)
            const ghost = instantiate(`${containerId}-ghost`, view.mode === 'list' ? LIST_TEMPLATE : CARD_TEMPLATE,
              { key: first.record_id, scope: itemScope(first, { selected: true, today, statusLabel: '', doneId }) },
              { x: drag.pointer.x - drag.offset.x, y: drag.pointer.y - drag.offset.y, w, h })
            return (
              <div className="aiws-dc-ghost" aria-hidden="true">
                {ghost.map((n) => <ProjectedView key={n.id} node={n} />)}
                {drag.ids.length > 1 && <span className="aiws-dc-ghost-count" style={{ left: drag.pointer.x - drag.offset.x + w - 12, top: drag.pointer.y - drag.offset.y - 10 }}>{drag.ids.length}</span>}
              </div>
            )
          })()}
          {editing && editingRect && (
            <input className="aiws-dc-inline-edit" style={{ left: editingRect.x, top: editingRect.y, width: editingRect.w, height: Math.min(editingRect.h, 30) }} autoFocus value={editing.text}
              aria-label={t('aiworkspace.todo.renameOne')} data-testid={`aiws-todo-edit-${containerId}`}
              onChange={(e) => setEditing({ ...editing, text: e.target.value })}
              onBlur={() => { rename(editing.id, editing.text); setEditing(null) }}
              onKeyDown={(e) => {
                e.stopPropagation()
                if (e.nativeEvent.isComposing) return
                if (e.key === 'Enter') { rename(editing.id, editing.text); setEditing(null); rootRef.current?.focus() }
                if (e.key === 'Escape') { setEditing(null); rootRef.current?.focus() }
              }} />
          )}
          {snapshot.loading && items.length === 0 && <div className="aiws-dc-empty">{t('aiworkspace.blocks.loading')}</div>}
          {!snapshot.loading && items.length === 0 && view.mode === 'list' && <div className="aiws-dc-empty is-hint">{t('aiworkspace.todo.empty')}</div>}
        </div>
      </div>
      {selection.length > 1 && (
        <div className="aiws-dc-bulk" role="toolbar" aria-label={t('aiworkspace.todo.bulk', { n: selection.length })} data-testid={`aiws-todo-bulk-${containerId}`}>
          <span>{t('aiworkspace.todo.bulk', { n: selection.length })}</span>
          <button type="button" disabled={!canWrite} title={canWrite ? undefined : noWrite} onClick={() => toggleDone(selection, true)}><Check size={14} aria-hidden="true" /> {t('aiworkspace.todo.complete')}</button>
          <button type="button" disabled={!canWrite} title={canWrite ? undefined : noWrite} onClick={(e) => setPopover({ kind: 'menu', ids: selection, anchor: anchorOf(e.currentTarget) })}><FolderInput size={14} aria-hidden="true" /> {t('aiworkspace.todo.moveTo')}</button>
          <button type="button" disabled={!canWrite} title={canWrite ? undefined : noWrite} onClick={() => removeItems(selection)}><Trash2 size={14} aria-hidden="true" /> {t('aiworkspace.common.delete')}</button>
          <button type="button" onClick={() => setSelection([])}>{t('aiworkspace.common.cancel')}</button>
        </div>
      )}
      {popover?.kind === 'detail' && byId.get(popover.id) && sourceId && (
        <ControlPopover anchor={popover.anchor} onClose={() => setPopover(null)} label={t('aiworkspace.todo.detail')} sheet={phone} testId={`aiws-todo-detail-${containerId}`}>
          <TodoDetail sourceId={sourceId} item={byId.get(popover.id)!} options={options} canWrite={canWrite} noWrite={noWrite} onDelete={() => { removeItems([popover.id]); setPopover(null) }} />
        </ControlPopover>
      )}
      {popover?.kind === 'menu' && (
        <ControlPopover anchor={popover.anchor} onClose={() => setPopover(null)} label={t('aiworkspace.todo.itemMenu')} sheet={phone} width={240} side="below">
          <ControlMenu items={menuItems(popover.ids)} onClose={() => setPopover(null)} label={t('aiworkspace.todo.itemMenu')} />
        </ControlPopover>
      )}
      {popover?.kind === 'view' && (
        <ControlPopover anchor={popover.anchor} onClose={() => setPopover(null)} label={t('aiworkspace.datactl.viewMenu')} width={260} side="below">
          <ControlMenu label={t('aiworkspace.datactl.viewMenu')} onClose={() => setPopover(null)} items={[
            { id: 'hide-done', label: t('aiworkspace.todo.hideDone'), checked: view.hide_done, icon: view.hide_done ? <Check size={14} /> : null, run: () => ctl.set({ hide_done: !view.hide_done }) },
            ...(view.mode === 'board' ? [{ id: 'expand', label: t('aiworkspace.todo.expandColumns'), icon: <PanelLeftOpen size={14} />, disabled: view.collapsed.length ? false : t('aiworkspace.todo.noneFolded'), run: () => ctl.set({ collapsed: [] }) } as MenuItem] : []),
            ...(host !== 'page' ? [{ id: 'page', label: t('aiworkspace.datactl.openAsPage'), icon: <Expand size={14} />, run: () => openControlPage(containerId) } as MenuItem] : []),
            'separator',
            { id: 'set-default', label: t('aiworkspace.datactl.setDefault'), icon: <PanelLeftClose size={14} />, disabled: ctl.setDefault ? false : ctl.canChange ? t('aiworkspace.datactl.alreadyDefault') : t('aiworkspace.datactl.noUpdate'), run: () => ctl.setDefault?.() },
            { id: 'reset', label: t('aiworkspace.datactl.resetView'), icon: <Undo2 size={14} />, disabled: Object.keys(ctl.personal).length ? false : t('aiworkspace.datactl.alreadyDefault'), run: () => ctl.reset() },
          ]} />
        </ControlPopover>
      )}
    </div>
  )
}

function BoardColumn({ column, label, count, height, dropping, active, onToggle, adding, canAdd, noWrite, onAddStart, onAddText, onAddDone }: {
  column: LaidColumn; label: string; count: number; height: number; dropping: boolean; active: boolean; onToggle: () => void
  adding: string | null; canAdd: boolean; noWrite: string; onAddStart: () => void; onAddText: (text: string) => void; onAddDone: (commit: boolean) => void
}) {
  const { t } = useAiwsText()
  const r = column.rect
  const addTop = column.body.y + column.contentHeight + (count ? CARD_GAP : 0)
  return (
    <>
      <div className={`aiws-board-col${column.collapsed ? ' is-collapsed' : ''}${dropping ? ' is-dropping' : ''}`} style={{ left: r.x, top: r.y, width: r.w, height }} data-testid={`aiws-board-col-${column.id}`} />
      <button type="button" className={`aiws-board-colhead${column.collapsed ? ' is-collapsed' : ''}`} style={{ left: r.x, top: r.y, width: r.w, height: column.collapsed ? height : COLUMN_HEADER }} disabled={!active}
        title={column.collapsed ? t('aiworkspace.todo.expandColumn', { name: label }) : t('aiworkspace.todo.foldColumn', { name: label })} data-testid={`aiws-board-colhead-${column.id}`} onClick={onToggle}>
        {column.collapsed ? <ChevronRight size={14} aria-hidden="true" /> : <ChevronDown size={14} aria-hidden="true" />}
        <span className="aiws-board-colname">{label}</span>
        <span className="aiws-board-colcount">{count}</span>
      </button>
      {!column.collapsed && (adding === null ? (
        <button type="button" className="aiws-dc-ghostbtn aiws-board-add" style={{ left: column.body.x, top: addTop, width: column.body.w }} disabled={!canAdd || !active} title={canAdd ? undefined : noWrite}
          data-testid={`aiws-board-add-${column.id}`} onClick={onAddStart}><Plus size={14} aria-hidden="true" /> {t('aiworkspace.todo.addCard')}</button>
      ) : (
        <input className="aiws-board-addinput" style={{ left: column.body.x, top: addTop, width: column.body.w }} autoFocus value={adding} placeholder={t('aiworkspace.todo.addPlaceholder')}
          data-testid={`aiws-board-addinput-${column.id}`}
          onChange={(e) => onAddText(e.target.value)} onBlur={() => onAddDone(false)}
          onKeyDown={(e) => { e.stopPropagation(); if (e.nativeEvent.isComposing) return; if (e.key === 'Enter') onAddDone(true); if (e.key === 'Escape') onAddDone(false) }} />
      ))}
    </>
  )
}
