/* The official calendar control's body (控件机制 TODO §0.1): the events of the bound calendar tables — repeating ones
 * expanded by buckyos-websdk, as the Agent commands expand them — projected as shape and label Blocks (Q1) and laid out
 * by the host's `month`, `time_grid` and `agenda` layouts (G3).
 *
 *   views       day, week, month, agenda; today, back / forward, a small month to jump (T / D / W / M / A, ← →)
 *   showing     the all-day row, spans as bars, overlapping events side by side, "+N more", the current time,
 *               each calendar's colour, the due dates of a bound to-do table (read only)
 *   creating    click an empty slot, or drag out a time range / a run of days: type the title in place; "More options"
 *               opens the whole form (repeat, location, reminders, colour)
 *   changing    drag to reschedule (15-minute steps, across days), drag the bottom edge to change the end; the preview
 *               moves while dragging, the drop is one Commit with start and end together, undoable; a repeating
 *               event asks "only this one / this and following / all" first
 *   touch       hold, then drag; or open the event and edit its time
 *
 * The person's view and date are theirs (`block:<id>/view`, G6); "Set as default" writes the instance config. */

import { useEffect, useLayoutEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import { CalendarCheck2, ChevronLeft, ChevronRight, Ellipsis, Expand, Plus, Undo2 } from 'lucide-react'
import { instantiate, type ProjectedNode, type Rect } from '@controls/projection'
import { layoutAgenda, layoutMonth, layoutTimeGrid, minuteY, monthCell, monthDayAt, type MonthOptions, type TimedPiece } from '@controls/datalayout'
import { CALENDAR_SCHEMA_ID, calendarOccurrences, canonicalOriginal, eventAddOps, eventDeleteOps, eventEditOps, eventSkipOps, todoItem, TODO_SCHEMA_ID, viewerTimeZone, type EventChange, type EventTimes, type Json as SdkJson, type RecurrenceScope } from 'buckyos'
import { randomId } from '../../../api/ids'
import type { Json, Operation } from '../../../api/types'
import { useReadOnlyReason, useStore, useEntity, useWorkspaceUi } from '../../../state/hooks'
import { useAiwsText } from '../../../useAiwsText'
import { LabelArt } from '../../blocks/labelBlock'
import { ControlMenu, ControlPopover, anchorOf, type AnchorRect, type MenuItem } from '../data/ControlPopover'
import { localPoint, trackPress, type Point } from '../data/gestures'
import { registerControlKeys } from '../data/keys'
import { copyRecordLink, openControlPage, recordAnchor, useRecordFocus } from '../data/focus'
import { ProjectedView, projectedAt } from '../data/ProjectedView'
import { useRecordSets } from '../data/records'
import { useControlView } from '../data/viewState'
import { submitData } from '../data/write'
import { SourceChooser, UnboundSource } from '../data/SourceChooser'
import { bindOps, freeBindingName, newSourceOps, sourceTables, surfaceFolderOf } from '../data/source'
import type { ControlBodyProps } from '../registry'
import { EventDetail } from './EventDetail'
import { draftOf, draftTimes, EventForm, hhmm, locale, MiniMonth, QuickCreate, ScopeAsk, type EventDraft } from './EventForm'
import {
  AGENDA_TEMPLATE, BAR_TEMPLATE, BLOCK_TEMPLATE, CAL_VIEW_KEYS, CAL_VIEWS, calendarDefaults, colorFor, dateOfDay, dayOfDate, DAY_MS, eventScope, isLong, localDayOf,
  localInstant, localMidnight, minuteOf, place, rangeOf, stepAnchor, TODO_BINDING, EVENTS_BINDING, isTodoBinding, weekdayOf, type CalendarView, type CalView, type Placed,
} from './model'

const SNAP = 15
const BAR_H = 20
const BAR_GAP = 2
const AGENDA_ROW = 40
const VIEW_LABEL: Record<CalView, string> = { day: 'aiworkspace.calendar.view.day', week: 'aiworkspace.calendar.view.week', month: 'aiworkspace.calendar.view.month', agenda: 'aiworkspace.calendar.view.agenda' }
const VIEW_KEY: Record<CalView, string> = { day: 'D', week: 'W', month: 'M', agenda: 'A' }

type Drag =
  | { kind: 'move'; p: Placed; start: Point; dayDelta: number; minDelta: number; copy: boolean; area: 'grid' | 'allday' | 'month' }
  | { kind: 'resize'; p: Placed; endMin: number }
  | { kind: 'create'; area: 'grid' | 'allday' | 'month'; day0: number; day1: number; min0: number; min1: number }

type Popover =
  | { kind: 'detail'; p: Placed; anchor: AnchorRect }
  | { kind: 'form'; draft: EventDraft; p: Placed | null; anchor: AnchorRect }
  | { kind: 'quick'; draft: EventDraft; anchor: AnchorRect }
  | { kind: 'scope'; verb: 'edit' | 'delete'; p: Placed; change: EventChange | null; anchor: AnchorRect }
  | { kind: 'more'; day: number; anchor: AnchorRect }
  | { kind: 'jump'; anchor: AnchorRect }
  | { kind: 'menu'; anchor: AnchorRect }

const snap = (min: number, step = SNAP) => Math.round(min / step) * step
let edits = 0
const nextEdit = () => ++edits

/** Re-render every minute (the current-time line, "today"). */
function useMinute(): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 60_000)
    return () => window.clearInterval(timer)
  }, [])
  return now
}

export function CalendarBody({ containerId, mode, active, size, phone, host }: ControlBodyProps) {
  const { t } = useAiwsText()
  const store = useStore()
  const container = useEntity(containerId)
  const readOnlyNow = useReadOnlyReason()
  const now = useMinute()
  const loc = locale()
  const ctl = useControlView<CalendarView>(containerId, calendarDefaults(loc, phone), CAL_VIEW_KEYS)
  const view = ctl.view
  const bindings = useMemo(() => container?.bindings ?? {}, [container?.bindings])
  const calendars = useMemo(() => Object.entries(bindings).filter(([name]) => !isTodoBinding(name)).map(([binding, sourceId], i) => ({ binding, sourceId, color: colorFor(binding, i, container?.config) })), [bindings, container?.config])
  // each to-do list its own colour, after the calendars' (`config.colors` names one per binding)
  const todos = useMemo(() => Object.entries(bindings).filter(([name]) => isTodoBinding(name)).map(([binding, sourceId], i) => ({ binding, sourceId, color: colorFor(binding, calendars.length + i, container?.config) })), [bindings, calendars.length, container?.config])
  const sourceIds = useMemo(() => [...calendars.map((c) => c.sourceId), ...todos.map((x) => x.sourceId)], [calendars, todos])
  const { sets, snapshots, version } = useRecordSets(sourceIds)
  const today = localDayOf(now)
  /** The day shown, as this window last moved to it (the person's saved date, `view.date`, follows; a saved date read
   * from the service late must not move the view back). */
  const [shownDate, setShownDate] = useState<string | null | undefined>(undefined)
  const anchor = shownDate !== undefined ? (shownDate ? dayOfDate(shownDate) : today) : view.date ? dayOfDate(view.date) : today
  const range = rangeOf(view.view, anchor, view.week_start)
  const viewerTz = useMemo(() => viewerTimeZone(), [])
  const capsOf = (sourceId: string) => store.outline.get(sourceId)?.capabilities ?? []
  const writable = (sourceId: string) => !readOnlyNow && mode !== 'presentation_edit' && capsOf(sourceId).includes('update')
  const noWrite = readOnlyNow ?? (mode === 'presentation_edit' ? t('aiworkspace.datactl.notHere') : t('aiworkspace.datactl.noUpdate'))
  const mainCalendar = calendars[0]
  const canCreate = Boolean(mainCalendar) && !readOnlyNow && mode !== 'presentation_edit' && (capsOf(mainCalendar.sourceId).includes('append') || capsOf(mainCalendar.sourceId).includes('update'))

  // ---- what is shown
  const placed = useMemo(() => {
    const window = { fromMs: localMidnight(range.first), toMs: localMidnight(range.last + 1) }
    const out: Placed[] = []
    const hidden = new Set(view.hidden ?? [])
    for (const c of calendars) {
      if (hidden.has(c.binding)) continue
      const rows = snapshots.get(c.sourceId)?.rows ?? []
      for (const occ of calendarOccurrences(rows as never, window, { viewerTz })) {
        const own = typeof occ.ext.color === 'string' && /^#[0-9a-fA-F]{6}$/.test(occ.ext.color) ? occ.ext.color : c.color
        out.push(place(occ, c.binding, own))
      }
    }
    for (const list of view.show_todo ? todos : []) {
      if (hidden.has(list.binding)) continue
      for (const row of snapshots.get(list.sourceId)?.rows ?? []) {
        const item = todoItem(row as never)
        if (!item.due) continue
        const day = dayOfDate(item.due)
        if (day < range.first || day > range.last) continue
        out.push({
          occ: { id: `${list.binding}:${item.record_id}`, row: row as never, title: item.title, allDay: true, startDate: item.due, endDate: item.due, status: null, ext: {}, recurring: false, override: false, agentWritten: false },
          calendar: list.binding, color: list.color, startDay: day, endDay: day, allDay: true, startMin: 0, endMin: 1440, todo: { recordId: item.record_id, done: item.done, sourceId: list.sourceId },
        })
      }
    }
    return out
  }, [snapshots, calendars, todos, view.show_todo, view.hidden, range.first, range.last, viewerTz])
  const byId = useMemo(() => new Map(placed.map((p) => [p.occ.id, p])), [placed])
  const sourceOf = (binding: string) => bindings[binding] ?? ''
  // ---- which tables it shows (a document change): more calendars, to-do tables' due dates, or one fewer
  const [bindAsk, setBindAsk] = useState<'calendar' | 'todo' | 'remove' | null>(null)
  const canBind = !readOnlyNow && mode !== 'presentation_edit' && (container?.capabilities ?? []).includes('update')
  const unbound = (schemaId: string) => sourceTables(store, schemaId).filter((table) => !Object.values(bindings).includes(table.entity_id))
  const removable = Object.entries(bindings).filter(([name]) => name !== EVENTS_BINDING).map(([, id]) => store.outline.get(id)).filter((e): e is NonNullable<typeof e> => Boolean(e))
  const bindTable = async (kind: 'calendar' | 'todo' | 'remove', choice: string) => {
    setBindAsk(null)
    const name = container?.title ?? t('aiworkspace.calendar.title')
    let operations: Operation[]
    if (kind === 'remove') {
      const binding = Object.entries(bindings).find(([b, id]) => b !== EVENTS_BINDING && id === choice)?.[0]
      if (!binding) return
      operations = bindOps(containerId, { [binding]: null })
    } else {
      const binding = freeBindingName(bindings, kind === 'todo' ? TODO_BINDING : EVENTS_BINDING)
      const tableId = choice === 'new' ? randomId('t') : choice
      operations = []
      if (choice === 'new') {
        const folder = surfaceFolderOf(store, containerId)
        if (!folder) return
        operations.push(...newSourceOps(store, { binding, schemaId: CALENDAR_SCHEMA_ID, table: 'calendar' }, folder, tableId, t('aiworkspace.calendar.title')))
      }
      operations.push(...bindOps(containerId, { [binding]: tableId }))
    }
    const outcome = await store.submit({ editId: `bind:${containerId}`, label: t(kind === 'remove' ? 'aiworkspace.calendar.unbindLabel' : 'aiworkspace.datactl.source.bindLabel', { name }), operations })
    if (outcome.status === 'rejected') store.notify('error', t('aiworkspace.datactl.rejected', { reason: outcome.errors?.[0]?.detail ?? outcome.detail ?? outcome.code }))
  }
  const askBind = (kind: 'calendar' | 'todo' | 'remove') => {
    setPopover(null)
    if (kind === 'calendar' && unbound(CALENDAR_SCHEMA_ID).length === 0) void bindTable('calendar', 'new')
    else if (kind === 'todo' && unbound(TODO_SCHEMA_ID).length === 1) void bindTable('todo', unbound(TODO_SCHEMA_ID)[0].entity_id)
    else setBindAsk(kind)
  }
  const rowsOf = (sourceId: string) => (snapshots.get(sourceId)?.rows ?? []) as never[]

  // ---- session state
  const ui = useWorkspaceUi()
  const [selected, setSelected] = useState<string | null>(null)
  /** An occurrence a link asked for: its details open once it is drawn. */
  const [pendingDetail, setPendingDetail] = useState<string | null>(null)
  const [drag, setDrag] = useState<Drag | null>(null)
  const dragRef = useRef<Drag | null>(null)
  useEffect(() => { dragRef.current = drag }, [drag])
  const [popover, setPopover] = useState<Popover | null>(null)
  /** A drop being written: its preview stays until the records show the result. */
  const [settling, setSettling] = useState<{ drag: Drag; version: number | null } | null>(null)
  const settled = settling !== null && settling.version !== null && version !== settling.version
  const rootRef = useRef<HTMLDivElement>(null)
  const gridRef = useRef<HTMLDivElement>(null)
  const alldayRef = useRef<HTMLDivElement>(null)
  const monthRef = useRef<HTMLDivElement>(null)
  const agendaRef = useRef<HTMLDivElement>(null)
  const shown = drag ?? (settling && !settled ? settling.drag : null)

  // ---- writes
  const write = (sourceId: string, label: string, ops: SdkJson[], overlay?: Record<string, Record<string, Json | undefined>>) =>
    submitData(store, sets.get(sourceId) ?? null, { editId: `cal:${sourceId}:${nextEdit()}`, label, operations: ops as unknown as Operation[], overlay })
  const applyEdit = async (p: Placed, change: EventChange, scope: RecurrenceScope, dropped?: Drag) => {
    const sourceId = sourceOf(p.calendar)
    if (!writable(sourceId)) { store.notify('info', noWrite); return }
    let ops: SdkJson[]
    try { ops = eventEditOps(sourceId, p.occ, change, scope, { rows: rowsOf(sourceId) }).ops } catch (error) { store.notify('error', String(error instanceof Error ? error.message : error)); return }
    const overlay = !p.occ.recurring && change.times ? { [p.occ.row.record_id]: timesValues(change.times) } : undefined
    if (dropped) setSettling({ drag: dropped, version: null })
    const ok = await write(sourceId, t('aiworkspace.calendar.editOne', { title: p.occ.title }), ops, overlay)
    setSettling((s) => (s && s.drag === dropped ? (ok ? { ...s, version } : null) : s))
  }
  /** A change to an event: a repeating one asks for its scope first. */
  const requestEdit = (p: Placed, change: EventChange, anchorRect: AnchorRect, dropped?: Drag) => {
    if (p.occ.recurring) { if (dropped) setSettling({ drag: dropped, version: null }); setPopover({ kind: 'scope', verb: 'edit', p, change, anchor: anchorRect }); return }
    void applyEdit(p, change, 'one', dropped)
  }
  const removeEvent = (p: Placed, scope: RecurrenceScope) => {
    const sourceId = sourceOf(p.calendar)
    if (!writable(sourceId)) { store.notify('info', noWrite); return }
    const ops = eventDeleteOps(sourceId, p.occ, scope, { rows: rowsOf(sourceId) })
    void write(sourceId, t('aiworkspace.calendar.deleteOne', { title: p.occ.title }), ops)
    setSelected(null)
  }
  const createEvent = (draft: EventDraft) => {
    const cal = calendars.find((c) => c.binding === draft.calendar) ?? mainCalendar
    if (!cal) return
    const times = draftTimes(draft)
    const ext: Record<string, SdkJson> = {}
    if (draft.rrule) { ext.rrule = draft.rrule; ext.tz = viewerTz }
    if (draft.location.trim()) ext.location = draft.location.trim()
    if (draft.color) ext.color = draft.color
    if (draft.reminders.length) ext.reminders = draft.reminders.map((m) => ({ before_min: m }))
    const { ops, record_id } = eventAddOps(cal.sourceId, times.allDay
      ? { title: draft.title.trim(), all_day: true, start_date: times.startDate, end_date: times.endDate, ext }
      : { title: draft.title.trim(), start: times.start, end: times.end, ext })
    void write(cal.sourceId, t('aiworkspace.calendar.addOne', { title: draft.title.trim() }), ops)
    setSelected(record_id)
  }
  const saveForm = (draft: EventDraft, p: Placed | null, anchorRect: AnchorRect) => {
    setPopover(null)
    if (!p) { createEvent(draft); return }
    const ext: Record<string, SdkJson | null> = { location: draft.location.trim() || null, color: draft.color, reminders: draft.reminders.length ? draft.reminders.map((m) => ({ before_min: m })) : null }
    const before = p.occ.master ? (p.occ.master.values.ext as Record<string, unknown> | undefined)?.rrule ?? null : (p.occ.ext.rrule ?? null)
    const change: EventChange = { times: draftTimes(draft), values: { title: draft.title.trim() }, ext_merge: ext, ...(draft.rrule !== before ? { rrule: draft.rrule } : {}) }
    if (draft.rrule && !p.occ.recurring) change.ext_merge = { ...ext, tz: viewerTz }
    requestEdit(p, change, anchorRect)
  }
  const duplicate = (p: Placed) => {
    const d = draftOf(p.occ, p.calendar)
    createEvent({ ...d, rrule: null, calendar: calendars.some((c) => c.binding === p.calendar) ? p.calendar : mainCalendar?.binding ?? '' })
  }

  // ---- geometry
  const contentW = Math.max(200, size.w - 2)
  const gutter = phone ? 42 : 54
  const hourH = phone ? 44 : 48
  const days = view.view === 'day' ? 1 : 7
  const colW = (contentW - gutter) / days
  const longs = useMemo(() => placed.filter(isLong), [placed])
  const timed = useMemo(() => placed.filter((p) => !isLong(p)), [placed])
  /** The all-day row of the time grid: a one-week month layout. */
  const alldayLayout = useMemo(() => {
    const events = longs.map((p) => ({ key: p.occ.id, start: p.startDay - range.first, end: p.endDay - range.first, allDay: p.allDay }))
    const lanesNeeded = Math.max(1, layoutMonth(events, { weeks: 1, width: contentW - gutter, height: 400, header: 0, dayHeader: 3, barHeight: BAR_H, barGap: BAR_GAP }).bars.reduce((m, b) => Math.max(m, b.lane + 1), 0))
    const lanes = Math.min(lanesNeeded, 3)
    const height = 3 + lanes * (BAR_H + BAR_GAP) + 4
    return { ...layoutMonth(events.filter((e) => e.end >= 0 && e.start < days), alldayOpts(contentW, gutter, days, height)), height }
  }, [longs, range.first, contentW, gutter, days])
  const grid = useMemo(() => {
    const pieces: TimedPiece[] = timed.map((p) => ({ key: p.occ.id, day: p.startDay - range.first, start: p.startMin, end: Math.max(p.endMin, p.startMin + SNAP) }))
    return layoutTimeGrid(pieces.filter((pc) => pc.day >= 0 && pc.day < days), { days, width: contentW - gutter, x: gutter, y: 0, hourHeight: hourH, minHeight: 20, rightGap: phone ? 6 : 12 })
  }, [timed, range.first, days, contentW, gutter, hourH, phone])
  const monthH = Math.max(240, size.h - 50)
  const monthOpts = useMemo((): MonthOptions => ({ weeks: range.weeks, width: contentW, height: monthH, header: 26, dayHeader: 24, barHeight: BAR_H, barGap: BAR_GAP }), [range.weeks, contentW, monthH])
  const month = useMemo(() => layoutMonth(placed.map((p) => ({ key: p.occ.id, start: p.startDay - range.first, end: p.endDay - range.first, allDay: p.allDay || isLong(p), order: p.startMin })), monthOpts), [placed, range.first, monthOpts])

  // the grid opens at the start of the working day
  useLayoutEffect(() => {
    const el = gridRef.current
    if (el && (view.view === 'week' || view.view === 'day')) el.scrollTop = Math.max(0, (view.day_start ?? 8) * hourH - 8)
  }, [view.view, view.day_start, hourH])

  // ---- labels for events
  const timeFmt = useMemo(() => new Intl.DateTimeFormat(loc === 'zh' ? 'zh-CN' : 'en', { hour: '2-digit', minute: '2-digit' }), [loc])
  const timeText = (p: Placed) => (p.allDay ? t('aiworkspace.calendar.allDay') : `${timeFmt.format(new Date(p.occ.startMs!))} – ${timeFmt.format(new Date(p.occ.endMs!))}`)
  const barText = (p: Placed) => (p.allDay || isLong(p) ? p.occ.title || t('aiworkspace.calendar.untitled') : `${timeFmt.format(new Date(p.occ.startMs!))} ${p.occ.title || t('aiworkspace.calendar.untitled')}`)

  // ---- projected nodes of each view
  const dragKey = shown && shown.kind !== 'create' ? shown.p.occ.id : null
  const monthNodes = (() => {
    if (view.view !== 'month') return []
    const out: ProjectedNode[] = []
    for (const bar of month.bars) {
      const p = byId.get(bar.key)
      if (!p) continue
      out.push(...instantiate(containerId, BAR_TEMPLATE, { key: `${bar.key}#${bar.week}.${bar.startDay}`, scope: eventScope(p, { selected: selected === p.occ.id, dragging: dragKey === p.occ.id, text: barText(p), time: '', mode: 'bar' }) }, bar.rect).map((n) => ({ ...n, item: p.occ.id })))
    }
    return out
  })()
  const gridNodes = (() => {
    if (view.view !== 'week' && view.view !== 'day') return []
    const out: ProjectedNode[] = []
    for (const p of timed) {
      const r = grid.rects.get(p.occ.id)
      if (!r) continue
      out.push(...instantiate(containerId, BLOCK_TEMPLATE, { key: p.occ.id, scope: eventScope(p, { selected: selected === p.occ.id, dragging: dragKey === p.occ.id, text: '', time: timeText(p), mode: 'block', height: r.h }) }, r))
    }
    return out
  })()
  const alldayNodes = (() => {
    if (view.view !== 'week' && view.view !== 'day') return []
    const out: ProjectedNode[] = []
    for (const bar of alldayLayout.bars) {
      const p = byId.get(bar.key)
      if (!p) continue
      out.push(...instantiate(containerId, BAR_TEMPLATE, { key: `${bar.key}#${bar.startDay}`, scope: eventScope(p, { selected: selected === p.occ.id, dragging: dragKey === p.occ.id, text: barText(p), time: '', mode: 'bar' }) }, bar.rect).map((n) => ({ ...n, item: p.occ.id })))
    }
    return out
  })()
  const agenda = (() => {
    if (view.view !== 'agenda') return null
    const groups = new Map<number, Placed[]>()
    for (const p of placed) for (let d = Math.max(p.startDay, range.first); d <= Math.min(p.endDay, range.last); d++) { const list = groups.get(d) ?? []; list.push(p); groups.set(d, list) }
    const sorted = [...groups.entries()].sort((a, b) => a[0] - b[0]).map(([day, list]) => ({ day, list: list.sort((a, b) => Number(!a.allDay) - Number(!b.allDay) || a.startMin - b.startMin) }))
    const laid = layoutAgenda(sorted.map((g) => ({ day: g.day, items: g.list.map((p) => ({ key: `${p.occ.id}@${g.day}`, h: AGENDA_ROW })) })), { width: contentW - 24, x: 12, y: 4, dayHeader: 30, gap: 2, groupGap: 10 })
    const nodes: ProjectedNode[] = []
    for (const g of sorted) for (const p of g.list) {
      const r = laid.rects.get(`${p.occ.id}@${g.day}`)!
      const time = p.allDay ? t('aiworkspace.calendar.allDay') : p.startDay === p.endDay ? `${timeFmt.format(new Date(p.occ.startMs!))}–${timeFmt.format(new Date(p.occ.endMs!))}` : g.day === p.startDay ? timeFmt.format(new Date(p.occ.startMs!)) : t('aiworkspace.calendar.allDay')
      nodes.push(...instantiate(containerId, AGENDA_TEMPLATE, { key: `${p.occ.id}@${g.day}`, scope: eventScope(p, { selected: selected === p.occ.id, dragging: false, text: '', time, mode: 'agenda' }) }, r).map((n) => ({ ...n, item: p.occ.id })))
    }
    return { laid, nodes, groups: sorted }
  })()

  // ---- navigation and keys
  const go = (day: number | null) => {
    const date = day === null || day === today ? null : dateOfDay(day)
    setShownDate(date)
    ctl.set({ date })
  }
  const setView = (v: CalView) => ctl.set({ view: v })
  const step = (dir: -1 | 1) => go(stepAnchor(view.view, anchor, dir))
  const newEventDraft = (day: number, startMin: number | null, endDay = day, endMin?: number): EventDraft => ({
    title: '', allDay: startMin === null, startDate: dateOfDay(day), endDate: dateOfDay(endDay),
    startMin: startMin ?? 9 * 60, endMin: endMin ?? Math.min(24 * 60 - 1, (startMin ?? 9 * 60) + 60), rrule: null, location: '', color: null, reminders: [], calendar: mainCalendar?.binding ?? '',
  })
  const onKey = (event: KeyboardEvent): boolean => {
    if (popover || drag) return false
    const mod = event.metaKey || event.ctrlKey || event.altKey
    if (mod) return false
    const k = event.key.toLowerCase()
    if (k === 't') { go(null); return true }
    if (k === 'd') { setView('day'); return true }
    if (k === 'w') { setView('week'); return true }
    if (k === 'm') { setView('month'); return true }
    if (k === 'a') { setView('agenda'); return true }
    if (event.key === 'ArrowLeft') { step(-1); return true }
    if (event.key === 'ArrowRight') { step(1); return true }
    if (k === 'n' && canCreate) {
      const start = snap(minuteOf(now, today) + 30, 30)
      setPopover({ kind: 'form', draft: newEventDraft(today, Math.min(start, 23 * 60)), p: null, anchor: anchorOf(rootRef.current) })
      return true
    }
    const p = selected ? byId.get(selected) : undefined
    if ((event.key === 'Delete' || event.key === 'Backspace') && p && !p.todo) {
      if (p.occ.recurring) setPopover({ kind: 'scope', verb: 'delete', p, change: null, anchor: anchorOf(rootRef.current) })
      else removeEvent(p, 'one')
      return true
    }
    if (event.key === 'Enter' && p) { openDetail(p); return true }
    if (event.key === 'Escape' && selected) { setSelected(null); return true }
    return false
  }

  // ---- pointer
  const openDetail = (p: Placed, el?: Element | null) => {
    setSelected(p.occ.id)
    const rect = (el as HTMLElement | null | undefined)?.getBoundingClientRect()
    setPopover({ kind: 'detail', p, anchor: rect ? { left: rect.left, top: rect.top, width: rect.width, height: rect.height } : anchorOf(rootRef.current) })
  }
  const canDrag = (p: Placed) => !p.todo && writable(sourceOf(p.calendar))
  const clientRectOf = (target: EventTarget | null): AnchorRect | null => {
    const el = (target as HTMLElement | null)?.closest?.('.aiws-proj') as HTMLElement | null
    if (!el) return null
    const r = el.getBoundingClientRect()
    return { left: r.left, top: r.top, width: r.width, height: r.height }
  }
  /** A press in the time grid (scrolling area). */
  const onGridDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (!active) return
    rootRef.current?.focus({ preventScroll: true })
    const toLocal = (x: number, y: number) => localPoint(gridRef.current, x, y)
    const at = (pt: Point) => ({ col: Math.max(0, Math.min(days - 1, Math.floor((pt.x - gutter) / colW))), min: Math.max(0, Math.min(24 * 60, (pt.y / hourH) * 60)) })
    const hit = projectedAt(event.target, gridRef.current)
    const p = hit ? byId.get(hit.item) : undefined
    if (p) {
      const start = toLocal(event.clientX, event.clientY)
      trackPress(event.nativeEvent, `${p.occ.id}:${hit!.role}`, toLocal, {
        onTap: () => { setSelected(p.occ.id); if (phone) openDetail(p, event.target as Element) },
        onDoubleTap: () => openDetail(p, event.target as Element),
        onLongPress: () => openDetail(p, event.target as Element),
        onDragStart: canDrag(p) ? (_s, e) => {
          setSelected(p.occ.id)
          setDrag(hit!.role === 'resize-end' ? { kind: 'resize', p, endMin: p.endMin } : { kind: 'move', p, start, dayDelta: 0, minDelta: 0, copy: e.altKey, area: 'grid' })
          return true
        } : undefined,
        onDragMove: (pt) => setDrag((d) => {
          if (!d) return d
          if (d.kind === 'resize') return { ...d, endMin: Math.max(d.p.startMin + SNAP, Math.min(24 * 60, snap(at(pt).min))) }
          if (d.kind === 'move') return { ...d, dayDelta: at(pt).col - at(d.start).col, minDelta: snap(((pt.y - d.start.y) / hourH) * 60) }
          return d
        }),
        onDragEnd: (_pt, _s, cancelled) => {
          const d = dragRef.current
          setDrag(null)
          if (!d || cancelled) return
          if (d.kind === 'resize' && d.endMin !== d.p.endMin) {
            const end = localInstant(d.p.endDay, d.endMin)
            requestEdit(d.p, { times: { allDay: false, start: new Date(d.p.occ.startMs!).toISOString(), end: new Date(end).toISOString() } }, clientRectOf(event.target) ?? anchorOf(gridRef.current), d)
          }
          if (d.kind === 'move' && (d.dayDelta || d.minDelta)) {
            const shift = d.dayDelta * DAY_MS + d.minDelta * 60_000
            const times: EventTimes = { allDay: false, start: new Date(d.p.occ.startMs! + shift).toISOString(), end: new Date(d.p.occ.endMs! + shift).toISOString() }
            if (d.copy) createEvent({ ...draftOf({ ...d.p.occ, startMs: d.p.occ.startMs! + shift, endMs: d.p.occ.endMs! + shift }, d.p.calendar), rrule: null })
            else requestEdit(d.p, { times }, clientRectOf(event.target) ?? anchorOf(gridRef.current), d)
          }
        },
      })
      return
    }
    // an empty slot: click for an hour there, or drag out a range
    if (!canCreate) return
    const p0 = toLocal(event.clientX, event.clientY)
    const a0 = at(p0)
    const day0 = range.first + a0.col
    trackPress(event.nativeEvent, 'grid-empty', toLocal, {
      onTap: () => {
        const start = Math.min(23 * 60, Math.floor(a0.min / 30) * 30)
        openQuick(newEventDraft(day0, start), { x: gutter + a0.col * colW, y: minuteY(start, { hourHeight: hourH }), w: colW, h: hourH }, gridRef.current)
      },
      onDragStart: () => { setDrag({ kind: 'create', area: 'grid', day0, day1: day0, min0: snap(a0.min), min1: snap(a0.min) + SNAP }); return true },
      onDragMove: (pt) => setDrag((d) => (d && d.kind === 'create' ? { ...d, min1: Math.max(0, Math.min(24 * 60, snap(at(pt).min))) } : d)),
      onDragEnd: (_pt, _s, cancelled) => {
        const d = dragRef.current
        setDrag(null)
        if (!d || cancelled || d.kind !== 'create') return
        const s = Math.min(d.min0, d.min1), e = Math.max(d.min0, d.min1, s + SNAP)
        openQuick(newEventDraft(d.day0, s, d.day0, Math.min(e, 24 * 60 - 1)), { x: gutter + (d.day0 - range.first) * colW, y: minuteY(s, { hourHeight: hourH }), w: colW, h: Math.max(20, ((e - s) / 60) * hourH) }, gridRef.current)
      },
    })
  }
  /** A press in the month grid or the all-day row: days. */
  const onDaysDown = (event: ReactPointerEvent<HTMLDivElement>, area: 'month' | 'allday') => {
    if (!active) return
    rootRef.current?.focus({ preventScroll: true })
    const el = area === 'month' ? monthRef.current : alldayRef.current
    const toLocal = (x: number, y: number) => localPoint(el, x, y)
    const dayAt = (pt: Point) => {
      if (area === 'month') return monthDayAt(pt.x, pt.y, monthOpts)
      const col = Math.floor((pt.x - gutter) / colW)
      return col >= 0 && col < days ? col : null
    }
    const hit = projectedAt(event.target, el)
    const p = hit ? byId.get(hit.item) : undefined
    const start = toLocal(event.clientX, event.clientY)
    if (hit?.node === 'more') return
    if (p) {
      trackPress(event.nativeEvent, `${p.occ.id}:${area}`, toLocal, {
        onTap: () => { if (phone) openDetail(p, event.target as Element); else setSelected(p.occ.id) },
        onDoubleTap: () => openDetail(p, event.target as Element),
        onLongPress: () => openDetail(p, event.target as Element),
        onDragStart: canDrag(p) ? (_s, e) => { setSelected(p.occ.id); setDrag({ kind: 'move', p, start, dayDelta: 0, minDelta: 0, copy: e.altKey, area }); return true } : undefined,
        onDragMove: (pt) => setDrag((d) => {
          if (!d || d.kind !== 'move') return d
          const a = dayAt(d.start), b = dayAt(pt)
          return a === null || b === null ? d : { ...d, dayDelta: b - a }
        }),
        onDragEnd: (_pt, _s, cancelled) => {
          const d = dragRef.current
          setDrag(null)
          if (!d || cancelled || d.kind !== 'move' || !d.dayDelta) return
          const o = d.p.occ
          const times: EventTimes = o.allDay
            ? { allDay: true, startDate: dateOfDay(dayOfDate(o.startDate!) + d.dayDelta), endDate: dateOfDay(dayOfDate(o.endDate!) + d.dayDelta) }
            : { allDay: false, start: new Date(o.startMs! + d.dayDelta * DAY_MS).toISOString(), end: new Date(o.endMs! + d.dayDelta * DAY_MS).toISOString() }
          if (d.copy) createEvent({ ...draftOf(times.allDay ? { ...o, startDate: times.startDate, endDate: times.endDate } : { ...o, startMs: Date.parse(times.start), endMs: Date.parse(times.end) }, d.p.calendar), rrule: null })
          else requestEdit(d.p, { times }, clientRectOf(event.target) ?? anchorOf(el), d)
        },
      })
      return
    }
    if (!canCreate) return
    const d0 = dayAt(start)
    if (d0 === null) return
    trackPress(event.nativeEvent, `${area}-empty`, toLocal, {
      onTap: () => openQuick(newEventDraft(range.first + d0, null), area === 'month' ? monthCell(d0, monthOpts) : { x: gutter + d0 * colW, y: 0, w: colW, h: alldayLayout.height }, el),
      onDragStart: () => { setDrag({ kind: 'create', area, day0: range.first + d0, day1: range.first + d0, min0: 0, min1: 0 }); return true },
      onDragMove: (pt) => setDrag((d) => { const day = dayAt(pt); return d && d.kind === 'create' && day !== null ? { ...d, day1: range.first + day } : d }),
      onDragEnd: (_pt, _s, cancelled) => {
        const d = dragRef.current
        setDrag(null)
        if (!d || cancelled || d.kind !== 'create') return
        const a = Math.min(d.day0, d.day1), b = Math.max(d.day0, d.day1)
        openQuick(newEventDraft(a, null, b), area === 'month' ? monthCell(a - range.first, monthOpts) : { x: gutter + (a - range.first) * colW, y: 0, w: colW * (b - a + 1), h: alldayLayout.height }, el)
      },
    })
  }
  const openQuick = (draft: EventDraft, local: Rect, el: HTMLElement | null) => {
    setSelected(null)
    setPopover({ kind: 'quick', draft, anchor: anchorOf(el, local) })
  }
  const onAgendaDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (!active) return
    const hit = projectedAt(event.target, agendaRef.current)
    const p = hit ? byId.get(hit.item) : undefined
    if (p) openDetail(p, (event.target as HTMLElement).closest('.aiws-proj'))
  }

  // a link to a record or an occurrence (G12): turn to its day, select it, open its details
  useRecordFocus(containerId, (focus) => {
    const original = focus.occurrence ? canonicalOriginal(focus.occurrence) : null
    let day: number | null = null
    if (original) day = original.length === 10 ? dayOfDate(original) : localDayOf(Date.parse(original))
    else {
      for (const c of calendars) {
        const row = (snapshots.get(c.sourceId)?.rows ?? []).find((r) => r.record_id === focus.record)
        if (!row) continue
        const v = row.values
        day = v.all_day === true && typeof v.start_date === 'string' ? dayOfDate(v.start_date) : typeof v.start === 'string' ? localDayOf(Date.parse(v.start)) : null
      }
    }
    if (day !== null) go(day)
    const id = original ? `${focus.record}@${original}` : focus.record
    setSelected(id)
    setPendingDetail(id)
  })
  useEffect(() => {
    if (!pendingDetail) return
    const p = byId.get(pendingDetail)
    if (!p) return
    const timer = window.setTimeout(() => {
      setPendingDetail(null)
      const el = rootRef.current?.querySelector(`[data-item="${CSS.escape(p.occ.id)}"]`)
      el?.scrollIntoView({ block: 'center' })
      openDetail(p, el)
    }, 60)
    return () => window.clearTimeout(timer)
  })

  const keyRef = useRef<(event: KeyboardEvent) => boolean>(() => false)
  useEffect(() => { keyRef.current = onKey })
  useEffect(() => registerControlKeys(containerId, (event) => keyRef.current(event)), [containerId])

  if (!container) return null
  if (!calendars.length) return <div className="aiws-dc aiws-cal"><UnboundSource containerId={containerId} /></div>
  const errors = sourceIds.map((id) => snapshots.get(id)?.error).filter(Boolean)

  // ---- preview of a drag in progress (or a drop being written)
  const preview: ProjectedNode[] = []
  if (shown && (shown.kind === 'move' || shown.kind === 'resize')) {
    const p = shown.p
    if (shown.kind === 'resize' || (shown.area === 'grid' && !isLong(p))) {
      const dayCol = p.startDay - range.first + (shown.kind === 'move' ? shown.dayDelta : 0)
      const s = p.startMin + (shown.kind === 'move' ? shown.minDelta : 0)
      const e = shown.kind === 'resize' ? shown.endMin : p.endMin + shown.minDelta
      const r = { x: gutter + dayCol * colW, y: minuteY(s, { hourHeight: hourH }), w: colW - (phone ? 6 : 12), h: Math.max(20, ((e - s) / 60) * hourH) }
      preview.push(...instantiate(`${containerId}-drag`, BLOCK_TEMPLATE, { key: p.occ.id, scope: eventScope(p, { selected: true, dragging: false, text: '', time: `${hhmm(((s % 1440) + 1440) % 1440)} – ${hhmm(((e % 1440) + 1440) % 1440)}`, mode: 'block', height: r.h }) }, r))
    } else {
      const ev = [{ key: p.occ.id, start: p.startDay - range.first + shown.dayDelta, end: p.endDay - range.first + shown.dayDelta }]
      const laid = shown.area === 'month' ? layoutMonth(ev, monthOpts) : layoutMonth(ev, alldayOpts(contentW, gutter, days, alldayLayout.height))
      for (const bar of laid.bars) preview.push(...instantiate(`${containerId}-drag`, BAR_TEMPLATE, { key: `${p.occ.id}#${bar.startDay}`, scope: eventScope(p, { selected: true, dragging: false, text: barText(p), time: '', mode: 'bar' }) }, bar.rect))
    }
  }
  const createRect = (area: 'grid' | 'allday' | 'month'): Rect[] => {
    if (!shown || shown.kind !== 'create' || shown.area !== area) return []
    if (area === 'grid') {
      const s = Math.min(shown.min0, shown.min1), e = Math.max(shown.min0, shown.min1, s + SNAP)
      return [{ x: gutter + (shown.day0 - range.first) * colW, y: minuteY(s, { hourHeight: hourH }), w: colW - 4, h: ((e - s) / 60) * hourH }]
    }
    const a = Math.min(shown.day0, shown.day1) - range.first, b = Math.max(shown.day0, shown.day1) - range.first
    if (area === 'allday') return [{ x: gutter + a * colW, y: 2, w: colW * (b - a + 1), h: alldayLayout.height - 4 }]
    return Array.from({ length: b - a + 1 }, (_, i) => monthCell(a + i, monthOpts))
  }

  const rangeLabel = (() => {
    const f = (o: Intl.DateTimeFormatOptions) => new Intl.DateTimeFormat(loc === 'zh' ? 'zh-CN' : 'en', { ...o, timeZone: 'UTC' })
    const d = (day: number) => new Date(day * DAY_MS)
    if (view.view === 'month' && range.month) return f({ year: 'numeric', month: 'long' }).format(new Date(Date.UTC(range.month.y, range.month.m - 1, 1)))
    if (view.view === 'day') return f({ year: 'numeric', month: 'long', day: 'numeric', weekday: 'short' }).format(d(anchor))
    return `${f({ month: 'short', day: 'numeric' }).format(d(range.first))} – ${f({ month: 'short', day: 'numeric', year: 'numeric' }).format(d(range.last))}`
  })()
  const wdFmt = new Intl.DateTimeFormat(loc === 'zh' ? 'zh-CN' : 'en', { weekday: 'short', timeZone: 'UTC' })

  const header = (
    <div className="aiws-dc-head aiws-cal-head" data-testid={`aiws-cal-head-${containerId}`}>
      <span className="aiws-dc-title">{container.title || t('aiworkspace.calendar.title')}</span>
      <button type="button" className="aiws-cal-today" disabled={!active} data-testid={`aiws-cal-today-${containerId}`} onClick={() => go(null)} title="T">{t('aiworkspace.calendar.today')}</button>
      <button type="button" className="aiws-dc-iconbtn" disabled={!active} aria-label={t('aiworkspace.calendar.prev')} title={`${t('aiworkspace.calendar.prev')} (←)`} data-testid={`aiws-cal-prev-${containerId}`} onClick={() => step(-1)}><ChevronLeft size={16} aria-hidden="true" /></button>
      <button type="button" className="aiws-dc-iconbtn" disabled={!active} aria-label={t('aiworkspace.calendar.next')} title={`${t('aiworkspace.calendar.next')} (→)`} data-testid={`aiws-cal-next-${containerId}`} onClick={() => step(1)}><ChevronRight size={16} aria-hidden="true" /></button>
      <button type="button" className="aiws-cal-range" disabled={!active} data-testid={`aiws-cal-range-${containerId}`} onClick={(e) => setPopover({ kind: 'jump', anchor: anchorOf(e.currentTarget) })}>{rangeLabel}</button>
      <span className="aiws-dc-spacer" />
      <div className="aiws-dc-seg" role="radiogroup" aria-label={t('aiworkspace.calendar.view')}>
        {CAL_VIEWS.map((v) => (
          <button key={v} type="button" role="radio" aria-checked={view.view === v} className={view.view === v ? 'is-on' : undefined} disabled={!active}
            data-testid={`aiws-cal-view-${v}`} title={`${t(VIEW_LABEL[v])} (${VIEW_KEY[v]})`} onClick={() => setView(v)}><span>{t(VIEW_LABEL[v])}</span></button>
        ))}
      </div>
      {canCreate && !phone && <button type="button" className="aiws-dc-iconbtn" disabled={!active} aria-label={t('aiworkspace.calendar.newEvent')} title={`${t('aiworkspace.calendar.newEvent')} (N)`} data-testid={`aiws-cal-new-${containerId}`}
        onClick={(e) => setPopover({ kind: 'form', draft: newEventDraft(anchor, 9 * 60), p: null, anchor: anchorOf(e.currentTarget) })}><Plus size={16} aria-hidden="true" /></button>}
      <button type="button" className="aiws-dc-iconbtn" disabled={!active} aria-label={t('aiworkspace.datactl.viewMenu')} title={t('aiworkspace.datactl.viewMenu')} data-testid={`aiws-cal-menu-${containerId}`}
        onClick={(e) => setPopover({ kind: 'menu', anchor: anchorOf(e.currentTarget) })}><Ellipsis size={16} aria-hidden="true" /></button>
    </div>
  )

  const todayCol = today - range.first
  const nowMin = minuteOf(now, today)

  return (
    <div ref={rootRef} className={`aiws-dc aiws-cal is-${view.view}${active ? ' is-active' : ''}${drag ? ' is-dragging' : ''}${phone ? ' is-phone' : ''}`}
      data-testid={`aiws-cal-${containerId}`} data-host={host} data-view={view.view} data-role={active ? 'editor' : undefined} tabIndex={active ? 0 : -1}
      onKeyDown={(event) => { if (event.target === rootRef.current && keyRef.current(event.nativeEvent)) { event.preventDefault(); event.stopPropagation() } }}>
      {header}
      {errors.length > 0 && <div className="aiws-dc-note aiws-error" role="alert">{t('aiworkspace.datactl.readFailed', { error: errors[0]! })}</div>}
      {(view.view === 'week' || view.view === 'day') && (
        <>
          <div className="aiws-cal-dayhead" style={{ paddingLeft: gutter }}>
            {Array.from({ length: days }, (_, i) => {
              const day = range.first + i
              return (
                <button key={day} type="button" className={`aiws-cal-dayname${day === today ? ' is-today' : ''}`} style={{ width: colW }} disabled={!active} onClick={() => { go(day); setView('day') }}>
                  <LabelArt text={wdFmt.format(new Date(day * DAY_MS))} look={{ font_size: 11, muted: true, align: 'center' }} />
                  <span className="aiws-cal-daynum"><LabelArt text={String(new Date(day * DAY_MS).getUTCDate())} look={{ font_size: 18, weight: day === today ? 'bold' : 'normal', align: 'center' }} /></span>
                </button>
              )
            })}
          </div>
          <div ref={alldayRef} className="aiws-cal-allday" style={{ height: alldayLayout.height }} onPointerDown={(e) => onDaysDown(e, 'allday')} data-testid={`aiws-cal-allday-${containerId}`}>
            <span className="aiws-cal-allday-label" style={{ width: gutter }}><LabelArt text={t('aiworkspace.calendar.allDay')} look={{ font_size: 10, muted: true, align: 'center' }} /></span>
            {createRect('allday').map((r, i) => <div key={i} className="aiws-cal-create" style={{ left: r.x, top: r.y, width: r.w, height: r.h }} />)}
            {alldayNodes.map((n) => <ProjectedView key={n.id} node={n} />)}
            {alldayLayout.more.map((m) => (
              <button key={m.day} type="button" className="aiws-cal-more" style={{ left: m.rect.x, top: m.rect.y, width: m.rect.w, height: m.rect.h }}
                onClick={(e) => setPopover({ kind: 'more', day: range.first + m.day, anchor: anchorOf(e.currentTarget) })}>{t('aiworkspace.calendar.more', { n: m.count })}</button>
            ))}
            {preview.length > 0 && shown?.kind === 'move' && shown.area === 'allday' && preview.map((n) => <ProjectedView key={n.id} node={n} className="is-preview" />)}
          </div>
          <div ref={gridRef} className="aiws-dc-scroll aiws-cal-grid" onPointerDown={onGridDown} data-testid={`aiws-cal-grid-${containerId}`}>
            <div className="aiws-dc-canvas" style={{ height: 24 * hourH, width: contentW }}>
              <div className="aiws-cal-lines" style={{ left: gutter, backgroundSize: `${colW}px ${hourH}px` }} />
              {Array.from({ length: 23 }, (_, h) => (
                <span key={h} className="aiws-cal-hour" style={{ top: (h + 1) * hourH - 7, width: gutter - 8 }}><LabelArt text={hhmm((h + 1) * 60)} look={{ font_size: 10, muted: true, align: 'right' }} /></span>
              ))}
              {todayCol >= 0 && todayCol < days && (
                <div className="aiws-cal-now" style={{ left: gutter + todayCol * colW, top: minuteY(nowMin, { hourHeight: hourH }), width: colW }} data-testid={`aiws-cal-now-${containerId}`} />
              )}
              {createRect('grid').map((r, i) => <div key={i} className="aiws-cal-create" style={{ left: r.x, top: r.y, width: r.w, height: r.h }} data-testid="aiws-cal-creating" />)}
              {gridNodes.map((n) => <ProjectedView key={n.id} node={n} />)}
              {preview.length > 0 && !(shown?.kind === 'move' && shown.area === 'allday') && preview.map((n) => <ProjectedView key={n.id} node={n} className="is-preview" />)}
            </div>
          </div>
        </>
      )}
      {view.view === 'month' && (
        <div ref={monthRef} className="aiws-cal-month" style={{ height: monthOpts.height, width: contentW }} onPointerDown={(e) => onDaysDown(e, 'month')} data-testid={`aiws-cal-month-${containerId}`}>
          {Array.from({ length: 7 }, (_, i) => (
            <span key={i} className="aiws-cal-mwd" style={{ left: (i * contentW) / 7, width: contentW / 7 }}><LabelArt text={wdFmt.format(new Date((range.first + i) * DAY_MS))} look={{ font_size: 11, muted: true, align: 'center' }} /></span>
          ))}
          {month.cells.map((c) => {
            const day = range.first + c.day
            const out = range.month ? new Date(day * DAY_MS).getUTCMonth() + 1 !== range.month.m : false
            return (
              <div key={c.day} className={`aiws-cal-cell${out ? ' is-out' : ''}${day === today ? ' is-today' : ''}${weekdayOf(day) >= 5 ? ' is-weekend' : ''}`} style={{ left: c.rect.x, top: c.rect.y, width: c.rect.w, height: c.rect.h }}>
                <span className="aiws-cal-cellnum"><LabelArt text={String(new Date(day * DAY_MS).getUTCDate())} look={{ font_size: 12, weight: day === today ? 'bold' : 'normal', align: 'center' }} /></span>
              </div>
            )
          })}
          {createRect('month').map((r, i) => <div key={i} className="aiws-cal-create" style={{ left: r.x, top: r.y, width: r.w, height: r.h }} />)}
          {monthNodes.map((n) => <ProjectedView key={n.id} node={n} />)}
          {month.more.map((m) => (
            <button key={m.day} type="button" className="aiws-cal-more" style={{ left: m.rect.x, top: m.rect.y, width: m.rect.w, height: m.rect.h }} data-testid={`aiws-cal-more-${dateOfDay(range.first + m.day)}`}
              onPointerDown={(e) => e.stopPropagation()} onClick={(e) => setPopover({ kind: 'more', day: range.first + m.day, anchor: anchorOf(e.currentTarget) })}>{t('aiworkspace.calendar.more', { n: m.count })}</button>
          ))}
          {preview.map((n) => <ProjectedView key={n.id} node={n} className="is-preview" />)}
        </div>
      )}
      {view.view === 'agenda' && agenda && (
        <div ref={agendaRef} className="aiws-dc-scroll aiws-cal-agenda" onPointerDown={onAgendaDown} data-testid={`aiws-cal-agenda-${containerId}`}>
          <div className="aiws-dc-canvas" style={{ height: agenda.laid.height + 16, width: contentW - 2 }}>
            {agenda.laid.headers.map((h) => {
              const day = h.day
              const f = new Intl.DateTimeFormat(loc === 'zh' ? 'zh-CN' : 'en', { month: 'long', day: 'numeric', weekday: 'long', timeZone: 'UTC' })
              return <span key={day} className={`aiws-cal-agenda-day${day === today ? ' is-today' : ''}`} style={{ left: h.rect.x, top: h.rect.y, width: h.rect.w, height: h.rect.h }}><LabelArt text={day === today ? `${t('aiworkspace.calendar.today')} · ${f.format(new Date(day * DAY_MS))}` : f.format(new Date(day * DAY_MS))} look={{ font_size: 13, weight: 'bold' }} /></span>
            })}
            {agenda.nodes.map((n) => <ProjectedView key={n.id} node={n} />)}
            {agenda.groups.length === 0 && <div className="aiws-dc-empty">{t('aiworkspace.calendar.noEvents')}</div>}
          </div>
          {canCreate && phone && <button type="button" className="aiws-cal-fab" aria-label={t('aiworkspace.calendar.newEvent')} onClick={(e) => setPopover({ kind: 'form', draft: newEventDraft(anchor, 9 * 60), p: null, anchor: anchorOf(e.currentTarget) })}><Plus size={22} aria-hidden="true" /></button>}
        </div>
      )}

      {popover?.kind === 'detail' && (
        <ControlPopover anchor={popover.anchor} onClose={() => setPopover(null)} label={t('aiworkspace.calendar.detail')} sheet={phone} testId={`aiws-cal-popover-${containerId}`}>
          <EventDetail p={popover.p} sourceId={sourceOf(popover.p.calendar)} canWrite={!popover.p.todo && writable(sourceOf(popover.p.calendar))} noWrite={noWrite}
            onEdit={() => setPopover({ kind: 'form', draft: draftOf(popover.p.occ, popover.p.calendar), p: popover.p, anchor: popover.anchor })}
            onDelete={() => { if (popover.p.occ.recurring) setPopover({ kind: 'scope', verb: 'delete', p: popover.p, change: null, anchor: popover.anchor }); else { removeEvent(popover.p, 'one'); setPopover(null) } }}
            onSkip={popover.p.occ.recurring ? () => {
              const sourceId = sourceOf(popover.p.calendar)
              void write(sourceId, t('aiworkspace.calendar.skipOne', { title: popover.p.occ.title }), eventSkipOps(sourceId, popover.p.occ, { rows: rowsOf(sourceId) }))
              setPopover(null)
            } : null}
            onDuplicate={() => { duplicate(popover.p); setPopover(null) }}
            onCopyLink={() => copyRecordLink(store, containerId, (popover.p.occ.master ?? popover.p.occ.row).record_id, popover.p.occ.original ?? null, t('aiworkspace.datactl.linkCopied'))}
            onAnnotate={ui.annotate && !popover.p.todo ? () => { const p = popover.p; setPopover(null); ui.annotate?.(recordAnchor(sourceOf(p.calendar), p.occ.row.record_id, p.occ.title)) } : null} />
        </ControlPopover>
      )}
      {popover?.kind === 'quick' && (
        <ControlPopover anchor={popover.anchor} onClose={() => setPopover(null)} label={t('aiworkspace.calendar.newEvent')} sheet={phone} width={300} testId={`aiws-cal-popover-${containerId}`}>
          <QuickCreate summary={summaryOf(popover.draft, loc)} disabled={canCreate ? null : noWrite}
            onCreate={(title) => { createEvent({ ...popover.draft, title }); setPopover(null) }}
            onMore={(title) => setPopover({ kind: 'form', draft: { ...popover.draft, title }, p: null, anchor: popover.anchor })} />
        </ControlPopover>
      )}
      {popover?.kind === 'form' && (
        <ControlPopover anchor={popover.anchor} onClose={() => setPopover(null)} label={popover.p ? t('aiworkspace.calendar.editEvent') : t('aiworkspace.calendar.newEvent')} sheet={phone} width={380} testId={`aiws-cal-popover-${containerId}`}>
          <EventForm initial={popover.draft} isNew={!popover.p} calendars={calendars.map((c) => ({ id: c.binding, label: store.outline.get(c.sourceId)?.name ?? c.binding, color: c.color }))}
            disabled={popover.p ? (writable(sourceOf(popover.p.calendar)) ? null : noWrite) : canCreate ? null : noWrite}
            onSave={(draft) => saveForm(draft, popover.p, popover.anchor)} onCancel={() => setPopover(null)} />
        </ControlPopover>
      )}
      {popover?.kind === 'scope' && (
        <ControlPopover anchor={popover.anchor} onClose={() => { setPopover(null); setSettling(null) }} label={t('aiworkspace.calendar.scope.title')} sheet={phone} width={280} testId={`aiws-cal-popover-${containerId}`}>
          <ScopeAsk verb={popover.verb} onPick={(scope) => {
            const pending = popover
            setPopover(null)
            if (pending.verb === 'delete') { setSettling(null); removeEvent(pending.p, scope) } else void applyEdit(pending.p, pending.change!, scope, settling?.drag)
          }} />
        </ControlPopover>
      )}
      {popover?.kind === 'more' && (
        <ControlPopover anchor={popover.anchor} onClose={() => setPopover(null)} label={t('aiworkspace.calendar.dayEvents')} sheet={phone} width={280} testId={`aiws-cal-popover-${containerId}`}>
          <div className="aiws-cal-daylist">
            <div className="aiws-cal-daylist-title">{new Intl.DateTimeFormat(loc === 'zh' ? 'zh-CN' : 'en', { month: 'long', day: 'numeric', weekday: 'long', timeZone: 'UTC' }).format(new Date(popover.day * DAY_MS))}</div>
            {placed.filter((p) => p.startDay <= popover.day && p.endDay >= popover.day).sort((a, b) => Number(!a.allDay) - Number(!b.allDay) || a.startMin - b.startMin).map((p) => (
              <button key={p.occ.id} type="button" className="aiws-cal-daylist-item" onClick={(e) => openDetail(p, e.currentTarget)}>
                <span className="aiws-cal-dot" style={{ background: p.color }} /><span className="aiws-cal-daylist-time">{p.allDay || isLong(p) ? t('aiworkspace.calendar.allDay') : timeFmt.format(new Date(p.occ.startMs!))}</span>
                <span className="aiws-cal-daylist-name">{p.occ.title || t('aiworkspace.calendar.untitled')}</span>
              </button>
            ))}
          </div>
        </ControlPopover>
      )}
      {popover?.kind === 'jump' && (
        <ControlPopover anchor={popover.anchor} onClose={() => setPopover(null)} label={t('aiworkspace.calendar.jump')} width={260} side="below">
          <MiniMonth value={dateOfDay(anchor)} today={dateOfDay(today)} weekStart={view.week_start} onPick={(date) => { go(dayOfDate(date)); setPopover(null) }} />
        </ControlPopover>
      )}
      {bindAsk && (
        <SourceChooser title={t(bindAsk === 'calendar' ? 'aiworkspace.calendar.addCalendarTitle' : bindAsk === 'todo' ? 'aiworkspace.calendar.addTodoTitle' : 'aiworkspace.calendar.removeTitle')}
          note={bindAsk === 'remove' ? t('aiworkspace.calendar.removeNote') : undefined} allowNew={bindAsk === 'calendar'}
          tables={bindAsk === 'remove' ? removable : unbound(bindAsk === 'calendar' ? CALENDAR_SCHEMA_ID : TODO_SCHEMA_ID)}
          onPick={(choice) => void bindTable(bindAsk, choice)} onClose={() => setBindAsk(null)} />
      )}
      {popover?.kind === 'menu' && (
        <ControlPopover anchor={popover.anchor} onClose={() => setPopover(null)} label={t('aiworkspace.datactl.viewMenu')} width={260} side="below">
          <ControlMenu label={t('aiworkspace.datactl.viewMenu')} onClose={() => setPopover(null)} items={[
            { id: 'week-mon', label: t('aiworkspace.calendar.weekStartsMonday'), checked: view.week_start === 0, icon: view.week_start === 0 ? <CalendarCheck2 size={14} /> : null, run: () => ctl.set({ week_start: 0 }) },
            { id: 'week-sun', label: t('aiworkspace.calendar.weekStartsSunday'), checked: view.week_start === 6, icon: view.week_start === 6 ? <CalendarCheck2 size={14} /> : null, run: () => ctl.set({ week_start: 6 }) },
            ...(todos.length ? [{ id: 'todo', label: t('aiworkspace.calendar.showTodo'), checked: view.show_todo, icon: view.show_todo ? <CalendarCheck2 size={14} /> : null, run: () => ctl.set({ show_todo: !view.show_todo }) } as MenuItem] : []),
            ...(calendars.length + todos.length > 1 ? [...calendars, ...todos].map((c): MenuItem => {
              const hidden = (view.hidden ?? []).includes(c.binding)
              return { id: `cal-${c.binding}`, label: store.outline.get(c.sourceId)?.name ?? c.binding, checked: !hidden, icon: <span className="aiws-cal-dot" style={{ background: hidden ? 'transparent' : c.color, borderColor: c.color }} />,
                run: () => ctl.set({ hidden: hidden ? (view.hidden ?? []).filter((x) => x !== c.binding) : [...(view.hidden ?? []), c.binding] }) }
            }) : []),
            ...(canBind ? ['separator',
              { id: 'add-calendar', label: t('aiworkspace.calendar.addCalendar'), icon: <Plus size={14} />, run: () => askBind('calendar') },
              { id: 'add-todo', label: t('aiworkspace.calendar.addTodo'), icon: <Plus size={14} />, disabled: unbound(TODO_SCHEMA_ID).length ? false : t('aiworkspace.calendar.noTodoTables'), run: () => askBind('todo') },
              { id: 'remove-table', label: t('aiworkspace.calendar.removeTable'), disabled: removable.length ? false : t('aiworkspace.calendar.nothingToRemove'), run: () => askBind('remove') }] as (MenuItem | 'separator')[] : []),
            ...(host !== 'page' ? [{ id: 'page', label: t('aiworkspace.datactl.openAsPage'), icon: <Expand size={14} />, run: () => openControlPage(containerId) } as MenuItem] : []),
            'separator',
            { id: 'set-default', label: t('aiworkspace.datactl.setDefault'), disabled: ctl.setDefault ? false : ctl.canChange ? t('aiworkspace.datactl.alreadyDefault') : t('aiworkspace.datactl.noUpdate'), run: () => ctl.setDefault?.() },
            { id: 'reset', label: t('aiworkspace.datactl.resetView'), icon: <Undo2 size={14} />, disabled: Object.keys(ctl.personal).length ? false : t('aiworkspace.datactl.alreadyDefault'), run: () => ctl.reset() },
          ]} />
        </ControlPopover>
      )}
    </div>
  )
}

/** The all-day row is a one-week month grid (a day view: one of its seven columns is wide enough to fill the row). */
function alldayOpts(contentW: number, gutter: number, days: number, height: number): MonthOptions {
  return { weeks: 1, width: ((contentW - gutter) * 7) / days, height, header: 0, dayHeader: 3, barHeight: BAR_H, barGap: BAR_GAP, x: gutter }
}

function timesValues(times: EventTimes): Record<string, Json | undefined> {
  return times.allDay
    ? { all_day: true, start_date: times.startDate, end_date: times.endDate !== times.startDate ? times.endDate : undefined, start: undefined, end: undefined }
    : { all_day: undefined, start: times.start, end: times.end, start_date: undefined, end_date: undefined }
}

function summaryOf(d: EventDraft, loc: 'zh' | 'en'): string {
  const f = new Intl.DateTimeFormat(loc === 'zh' ? 'zh-CN' : 'en', { month: 'short', day: 'numeric', weekday: 'short', timeZone: 'UTC' })
  const day = (s: string) => f.format(new Date(`${s}T00:00:00Z`))
  if (d.allDay) return d.startDate === d.endDate ? day(d.startDate) : `${day(d.startDate)} – ${day(d.endDate)}`
  return `${day(d.startDate)} ${hhmm(d.startMin)} – ${hhmm(d.endMin)}`
}

