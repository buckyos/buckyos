/* The official calendar control's model (控件机制 TODO §0.1, G2, G3, G6, G10): its view (day, week, month, agenda —
 * the instance's default and each person's own choice and date), the reader's calendar arithmetic (days on the
 * device's calendar, minutes of the day), the colours of the bound calendars and the templates events are projected
 * with (shape + label; Q1). Occurrences come from buckyos-websdk `calendarOccurrences` (one expansion for the control
 * and the Agent commands). */

import { AGENDA_TEMPLATE, BAR_TEMPLATE, BLOCK_TEMPLATE } from '@controls/schedule'
import type { EventOccurrence } from 'buckyos'
import type { Json } from '../../../api/types'

export { AGENDA_TEMPLATE, BAR_TEMPLATE, BLOCK_TEMPLATE }

export const CALENDAR_KIND = 'calendar'
/** The binding of the main calendar; any other binding naming a calendar table is a further calendar, `todo` a to-do
 * table whose due dates are shown. */
export const EVENTS_BINDING = 'events'
export const TODO_BINDING = 'todo'

export type CalView = 'day' | 'week' | 'month' | 'agenda'
export const CAL_VIEWS: CalView[] = ['day', 'week', 'month', 'agenda']

export interface CalendarView extends Record<string, Json | undefined> {
  view: CalView
  /** The date the view shows (`YYYY-MM-DD`); absent: today. Personal only. */
  date?: string
  /** First day of the week, 0 Monday … 6 Sunday. */
  week_start: number
  /** The hour the time grid scrolls to. */
  day_start: number
  show_todo: boolean
  hidden?: string[]
}

export const CAL_VIEW_KEYS = ['view', 'date', 'week_start', 'day_start', 'show_todo', 'hidden'] as const

export function calendarDefaults(locale: string, phone: boolean): CalendarView {
  return { view: phone ? 'agenda' : 'week', week_start: locale.startsWith('en') ? 6 : 0, day_start: 8, show_todo: true }
}

// ---- the reader's calendar (the device's time zone)

export const DAY_MS = 86_400_000
const pad = (n: number) => String(n).padStart(2, '0')

/** `YYYY-MM-DD` ↔ day number (days since 1970-01-01). */
export const dayOfDate = (date: string) => Math.floor(Date.UTC(+date.slice(0, 4), +date.slice(5, 7) - 1, +date.slice(8, 10)) / DAY_MS)
export function dateOfDay(day: number): string {
  const t = new Date(day * DAY_MS)
  return `${t.getUTCFullYear()}-${pad(t.getUTCMonth() + 1)}-${pad(t.getUTCDate())}`
}
/** The local day an instant falls on. */
export function localDayOf(ms: number): number {
  const d = new Date(ms)
  return Math.floor(Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()) / DAY_MS)
}
/** Minutes since local midnight of `day` (may exceed 1440 or be negative). */
export function minuteOf(ms: number, day: number): number {
  return Math.round((ms - localMidnight(day)) / 60_000)
}
export function localMidnight(day: number): number {
  const t = new Date(day * DAY_MS)
  return new Date(t.getUTCFullYear(), t.getUTCMonth(), t.getUTCDate()).getTime()
}
/** The instant of a local wall clock. */
export function localInstant(day: number, minute: number): number {
  const t = new Date(day * DAY_MS)
  return new Date(t.getUTCFullYear(), t.getUTCMonth(), t.getUTCDate(), Math.floor(minute / 60), minute % 60).getTime()
}
export const todayDay = () => localDayOf(Date.now())
/** Weekday of a day number, 0 Monday. */
export const weekdayOf = (day: number) => (((day + 3) % 7) + 7) % 7
export function weekStartOf(day: number, weekStart: number): number {
  return day - ((weekdayOf(day) - weekStart + 7) % 7)
}

export interface CalRange {
  /** First and last day shown (inclusive). */
  first: number
  last: number
  /** Month view: weeks of the grid; the month the anchor is in. */
  weeks: number
  month?: { y: number; m: number }
}

export function rangeOf(view: CalView, anchor: number, weekStart: number): CalRange {
  switch (view) {
    case 'day': return { first: anchor, last: anchor, weeks: 1 }
    case 'week': { const first = weekStartOf(anchor, weekStart); return { first, last: first + 6, weeks: 1 } }
    case 'month': {
      const t = new Date(anchor * DAY_MS)
      const y = t.getUTCFullYear(), m = t.getUTCMonth() + 1
      const firstOfMonth = Math.floor(Date.UTC(y, m - 1, 1) / DAY_MS)
      const lastOfMonth = Math.floor(Date.UTC(y, m, 0) / DAY_MS)
      const first = weekStartOf(firstOfMonth, weekStart)
      const weeks = Math.ceil((lastOfMonth - first + 1) / 7)
      return { first, last: first + weeks * 7 - 1, weeks, month: { y, m } }
    }
    case 'agenda': return { first: anchor, last: anchor + 29, weeks: 1 }
  }
}

/** One step back / forward in a view. */
export function stepAnchor(view: CalView, anchor: number, dir: -1 | 1): number {
  if (view === 'day') return anchor + dir
  if (view === 'week') return anchor + dir * 7
  if (view === 'agenda') return anchor + dir * 30
  const t = new Date(anchor * DAY_MS)
  const y = t.getUTCFullYear(), m = t.getUTCMonth() + dir
  const days = new Date(Date.UTC(y, m + 1, 0)).getUTCDate()
  return Math.floor(Date.UTC(y, m, Math.min(t.getUTCDate(), days)) / DAY_MS)
}

// ---- occurrences on the reader's calendar

/** Where an occurrence sits: its local days (end inclusive) and, timed, minutes of its first and last day. */
export interface Placed {
  occ: EventOccurrence
  /** The calendar it belongs to (binding name). */
  calendar: string
  color: string
  startDay: number
  endDay: number
  allDay: boolean
  startMin: number
  endMin: number
  /** A to-do's due date shown read-only. */
  todo?: { recordId: string; done: boolean; sourceId: string }
}

/** Timed events of a day or longer are drawn as bars with the all-day ones (as mainstream calendars do). */
export function place(occ: EventOccurrence, calendar: string, color: string): Placed {
  if (occ.allDay) {
    const s = dayOfDate(occ.startDate!), e = dayOfDate(occ.endDate!)
    return { occ, calendar, color, startDay: s, endDay: Math.max(s, e), allDay: true, startMin: 0, endMin: 1440 }
  }
  const s = localDayOf(occ.startMs!)
  const endMs = Math.max(occ.endMs!, occ.startMs!)
  // an event ending at midnight ends on the day before
  const e = Math.max(s, localDayOf(endMs - (endMs > occ.startMs! ? 1 : 0)))
  return { occ, calendar, color, startDay: s, endDay: e, allDay: false, startMin: minuteOf(occ.startMs!, s), endMin: minuteOf(endMs, e) }
}

export const isLong = (p: Placed) => p.allDay || p.endDay > p.startDay

// ---- colours

export const CALENDAR_COLORS = ['#2563eb', '#16a34a', '#d97706', '#db2777', '#7c3aed', '#0891b2', '#dc2626', '#4b5563']
export function colorFor(binding: string, index: number, config: Record<string, Json> | null | undefined): string {
  const set = (config?.colors as Record<string, Json> | undefined)?.[binding]
  return typeof set === 'string' && /^#[0-9a-fA-F]{6}$/.test(set) ? set : CALENDAR_COLORS[index % CALENDAR_COLORS.length]
}

/** How an event is painted: tentative ones paler, cancelled ones struck, a selected one outlined. */
export function eventScope(p: Placed, opts: { selected: boolean; dragging: boolean; text: string; time: string; mode: 'bar' | 'block' | 'agenda'; height?: number; continues?: boolean }) {
  const tentative = p.occ.status === 'tentative'
  const cancelled = p.occ.status === 'cancelled'
  const solid = opts.mode === 'bar' && (p.allDay || p.endDay > p.startDay)
  const color = p.todo ? (p.todo.done ? 'muted' : p.color) : p.color
  const h = opts.height ?? 0
  return {
    item: {
      color, title: p.occ.title || null, text: opts.text, time: opts.time,
      fill: solid ? `${color}/${tentative ? '0.45' : '0.88'}` : opts.mode === 'block' ? `${p.color}/${tentative ? '0.10' : '0.16'}` : opts.selected ? `${p.color}/0.16` : 'none',
      stroke: opts.selected ? p.color : opts.mode === 'block' && tentative ? `${p.color}/0.5` : 'none',
      ink: solid ? '#ffffff' : 'text', ink2: 'muted',
      opacity: opts.dragging ? 0.4 : cancelled ? 0.55 : 1,
      dot: !solid && opts.mode === 'bar' ? true : null, pad: !solid && opts.mode === 'bar' ? 16 : 6,
      cancelled: cancelled || Boolean(p.todo?.done),
      wrap: h > 34, title_h: h > 34 ? Math.max(16, h - 20) : h - 2, show_time: h > 34 ? true : null, time_y: Math.max(18, h - 18),
      resizable: opts.mode === 'block' && !p.todo ? true : null,
      row_fill: opts.selected ? 'selected' : 'none', title_ink: cancelled ? 'muted' : 'text',
    },
  }
}
