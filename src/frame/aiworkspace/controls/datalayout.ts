/* Layouts that place projected items by their values (控件机制 TODO G3), shipped with the host (K2): `list` (by the
 * sort field), `columns` (one column per option of a select field, sorted inside), `month` (a 7-column grid; spans are
 * bars, what does not fit is "+N more"), `time_grid` (a time axis per day; overlapping events side by side) and
 * `agenda` (a list grouped by day). They read sizes and the values the caller already turned into numbers (day
 * indices, minutes) and only produce positions: no dates, no time zones, no writes. */

import type { Rect } from './projection.ts'

// ---- list

export interface ListItem { key: string; h: number }

export function layoutList(items: ListItem[], opts: { x?: number; y?: number; width: number; gap?: number }): { rects: Map<string, Rect>; height: number } {
  const rects = new Map<string, Rect>()
  const gap = opts.gap ?? 0
  let y = opts.y ?? 0
  for (const item of items) {
    rects.set(item.key, { x: opts.x ?? 0, y, w: opts.width, h: item.h })
    y += item.h + gap
  }
  return { rects, height: Math.max(0, y - (opts.y ?? 0) - (items.length ? gap : 0)) }
}

/** Where a pointer at `y` would insert among a list's items (index into `keys`, 0…length), and the line to draw. */
export function insertionAt(keys: string[], rects: Map<string, Rect>, y: number): { index: number; lineY: number } {
  for (let i = 0; i < keys.length; i++) {
    const r = rects.get(keys[i])
    if (r && y < r.y + r.h / 2) return { index: i, lineY: r.y - 1 }
  }
  const last = keys.length ? rects.get(keys[keys.length - 1]) : undefined
  return { index: keys.length, lineY: last ? last.y + last.h + 1 : 0 }
}

// ---- columns (a board)

export interface ColumnSpec { id: string; collapsed?: boolean }
export interface ColumnItem { key: string; column: string; h: number }
export interface LaidColumn { id: string; rect: Rect; header: Rect; body: Rect; collapsed: boolean; keys: string[]; contentHeight: number }

export interface ColumnsOptions {
  x?: number
  y?: number
  height: number
  columnWidth: number
  collapsedWidth: number
  gap: number
  header: number
  padding: number
  cardGap: number
  /** Scroll offset of each column's body. */
  scroll?: (column: string) => number
}

export function layoutColumns(columns: ColumnSpec[], items: ColumnItem[], opts: ColumnsOptions): { columns: LaidColumn[]; rects: Map<string, Rect>; width: number } {
  const rects = new Map<string, Rect>()
  const out: LaidColumn[] = []
  let x = opts.x ?? 0
  const y = opts.y ?? 0
  for (const c of columns) {
    const collapsed = !!c.collapsed
    const w = collapsed ? opts.collapsedWidth : opts.columnWidth
    const rect = { x, y, w, h: opts.height }
    const header = { x, y, w, h: opts.header }
    const body = { x: x + opts.padding, y: y + opts.header, w: Math.max(0, w - opts.padding * 2), h: Math.max(0, opts.height - opts.header - opts.padding) }
    const keys: string[] = []
    let cy = body.y - (opts.scroll?.(c.id) ?? 0)
    for (const item of items) {
      if (item.column !== c.id) continue
      keys.push(item.key)
      if (!collapsed) rects.set(item.key, { x: body.x, y: cy, w: body.w, h: item.h })
      cy += item.h + opts.cardGap
    }
    out.push({ id: c.id, rect, header, body, collapsed, keys, contentHeight: Math.max(0, cy - body.y + (opts.scroll?.(c.id) ?? 0) - (keys.length ? opts.cardGap : 0)) })
    x += w + opts.gap
  }
  return { columns: out, rects, width: Math.max(0, x - (opts.x ?? 0) - (columns.length ? opts.gap : 0)) }
}

/** The column under `x` and the insertion index in it at `y`. */
export function columnDropAt(laid: { columns: LaidColumn[]; rects: Map<string, Rect> }, x: number, y: number, exclude?: string): { column: string; index: number; lineY: number } | null {
  const col = laid.columns.find((c) => x >= c.rect.x && x <= c.rect.x + c.rect.w)
  if (!col) return null
  const keys = col.keys.filter((k) => k !== exclude)
  const at = insertionAt(keys, laid.rects, y)
  return { column: col.id, index: at.index, lineY: keys.length ? at.lineY : col.body.y }
}

// ---- month

/** An event on the month grid: days are indices from the grid's first day (0 = top-left), `end` inclusive. */
export interface MonthEvent { key: string; start: number; end: number; allDay?: boolean
  /** Order among the events starting the same day with the same length (a timed event's start minute). */
  order?: number }

export interface MonthOptions {
  weeks: number
  width: number
  height: number
  /** Weekday name row above the grid. */
  header: number
  /** Day number band at the top of each cell. */
  dayHeader: number
  barHeight: number
  barGap: number
  x?: number
  y?: number
}

export interface MonthBar { key: string; week: number; lane: number; rect: Rect; startsBefore: boolean; endsAfter: boolean; startDay: number; endDay: number }
export interface MonthMore { day: number; count: number; rect: Rect; keys: string[] }

export function monthCell(day: number, opts: MonthOptions): Rect {
  const cw = opts.width / 7
  const ch = (opts.height - opts.header) / opts.weeks
  const week = Math.floor(day / 7)
  return { x: (opts.x ?? 0) + (day % 7) * cw, y: (opts.y ?? 0) + opts.header + week * ch, w: cw, h: ch }
}

/** Bars per week in lanes: events sorted by start, longer first (multi-day before single days, as mainstream calendars
 * draw them), each in the lowest lane free over its days. Lanes that do not fit a cell become "+N more" per day. */
export function layoutMonth(events: MonthEvent[], opts: MonthOptions): { cells: { day: number; rect: Rect }[]; bars: MonthBar[]; more: MonthMore[]; lanes: number } {
  const days = opts.weeks * 7
  const cells = Array.from({ length: days }, (_, day) => ({ day, rect: monthCell(day, opts) }))
  const ch = (opts.height - opts.header) / opts.weeks
  const lanes = Math.max(0, Math.floor((ch - opts.dayHeader + opts.barGap) / (opts.barHeight + opts.barGap)))
  const sorted = events.filter((e) => e.end >= 0 && e.start < days).sort((a, b) => a.start - b.start || (b.end - b.start) - (a.end - a.start) || (a.allDay === b.allDay ? 0 : a.allDay ? -1 : 1) || (a.order ?? 0) - (b.order ?? 0) || a.key.localeCompare(b.key))
  const bars: MonthBar[] = []
  const hidden = new Map<number, string[]>()
  for (let week = 0; week < opts.weeks; week++) {
    const w0 = week * 7, w1 = w0 + 6
    const used: boolean[][] = []
    for (const e of sorted) {
      if (e.end < w0 || e.start > w1) continue
      const s = Math.max(e.start, w0), t = Math.min(e.end, w1)
      let lane = 0
      for (;; lane++) {
        used[lane] ??= Array(7).fill(false)
        let free = true
        for (let d = s; d <= t; d++) if (used[lane][d - w0]) { free = false; break }
        if (free) break
      }
      for (let d = s; d <= t; d++) used[lane][d - w0] = true
      // the last lane is kept for "+N more" when something does not fit
      if (lane < lanes) {
        const a = monthCell(s, opts), b = monthCell(t, opts)
        bars.push({ key: e.key, week, lane, startDay: s, endDay: t, startsBefore: e.start < s, endsAfter: e.end > t,
          rect: { x: a.x + 2, y: a.y + opts.dayHeader + lane * (opts.barHeight + opts.barGap), w: b.x + b.w - a.x - 4, h: opts.barHeight } })
      } else {
        for (let d = s; d <= t; d++) { const list = hidden.get(d) ?? []; list.push(e.key); hidden.set(d, list) }
      }
    }
  }
  // a day with hidden events gives up its last shown lane to the "+N more" line
  const more: MonthMore[] = []
  for (const [day, keys] of [...hidden.entries()].sort((a, b) => a[0] - b[0])) {
    const lane = Math.max(0, lanes - 1)
    const displaced = bars.filter((b) => b.lane === lane && b.startDay <= day && b.endDay >= day)
    const extra = [...keys]
    for (const b of displaced) {
      extra.push(b.key)
      // a bar crossing other days is cut around this day
      const cut = splitBar(b, day, opts)
      bars.splice(bars.indexOf(b), 1, ...cut)
    }
    const cell = monthCell(day, opts)
    more.push({ day, count: extra.length, keys: extra, rect: { x: cell.x + 2, y: cell.y + opts.dayHeader + lane * (opts.barHeight + opts.barGap), w: cell.w - 4, h: opts.barHeight } })
  }
  return { cells, bars, more, lanes }
}

function splitBar(b: MonthBar, day: number, opts: MonthOptions): MonthBar[] {
  const out: MonthBar[] = []
  const piece = (s: number, t: number): MonthBar => {
    const a = monthCell(s, opts), z = monthCell(t, opts)
    return { ...b, startDay: s, endDay: t, startsBefore: b.startsBefore || s > b.startDay, endsAfter: b.endsAfter || t < b.endDay, rect: { ...b.rect, x: a.x + 2, w: z.x + z.w - a.x - 4 } }
  }
  if (b.startDay < day) out.push(piece(b.startDay, day - 1))
  if (b.endDay > day) out.push(piece(day + 1, b.endDay))
  return out
}

/** The day of the month grid under a point. */
export function monthDayAt(x: number, y: number, opts: MonthOptions): number | null {
  const lx = x - (opts.x ?? 0), ly = y - (opts.y ?? 0) - opts.header
  if (lx < 0 || ly < 0 || lx >= opts.width) return null
  const col = Math.floor(lx / (opts.width / 7))
  const row = Math.floor(ly / ((opts.height - opts.header) / opts.weeks))
  return row >= opts.weeks ? null : row * 7 + col
}

// ---- time grid (day / week)

/** One day's piece of a timed event: minutes from that day's midnight (an event crossing midnight comes as pieces). */
export interface TimedPiece { key: string; day: number; start: number; end: number }

export interface TimeGridOptions {
  days: number
  /** Width of the day columns together (the time gutter is left of `x`). */
  width: number
  x?: number
  /** Top of minute 0. */
  y?: number
  hourHeight: number
  /** A piece is at least this tall (a 5-minute event stays clickable). */
  minHeight: number
  /** Space kept free at the right of each day column (to click into it to create). */
  rightGap: number
}

export const minuteY = (minute: number, opts: Pick<TimeGridOptions, 'y' | 'hourHeight'>) => (opts.y ?? 0) + (minute / 60) * opts.hourHeight

/** Overlapping pieces of a day share its width: a cluster of overlapping pieces gets as many columns as it needs at
 * once, each piece the first column free at its start, and a piece stretches over free columns to its right. */
export function layoutTimeGrid(pieces: TimedPiece[], opts: TimeGridOptions): { rects: Map<string, Rect>; columns: Map<string, { column: number; of: number }> } {
  const rects = new Map<string, Rect>()
  const columns = new Map<string, { column: number; of: number }>()
  const dayW = opts.width / opts.days
  for (let day = 0; day < opts.days; day++) {
    const list = pieces.filter((p) => p.day === day).sort((a, b) => a.start - b.start || (b.end - b.start) - (a.end - a.start) || a.key.localeCompare(b.key))
    let cluster: TimedPiece[] = []
    let clusterEnd = -Infinity
    const flush = () => {
      if (!cluster.length) return
      const ends: number[] = []
      const col = new Map<string, number>()
      for (const p of cluster) {
        const visualEnd = Math.max(p.end, p.start + (opts.minHeight / opts.hourHeight) * 60)
        let c = ends.findIndex((e) => e <= p.start)
        if (c < 0) { c = ends.length; ends.push(visualEnd) } else ends[c] = visualEnd
        col.set(p.key, c)
      }
      const n = ends.length
      const colW = (dayW - opts.rightGap) / n
      for (const p of cluster) {
        const c = col.get(p.key)!
        // stretch right while the next columns are free over this piece's time
        let span = 1
        for (let k = c + 1; k < n; k++) {
          const blocked = cluster.some((q) => col.get(q.key) === k && q.start < p.end && Math.max(q.end, q.start + (opts.minHeight / opts.hourHeight) * 60) > p.start)
          if (blocked) break
          span++
        }
        const top = minuteY(Math.max(0, p.start), opts)
        const bottom = Math.max(top + opts.minHeight, minuteY(Math.min(24 * 60, p.end), opts))
        rects.set(p.key, { x: (opts.x ?? 0) + day * dayW + c * colW, y: top, w: colW * span - 2, h: bottom - top - 1 })
        columns.set(p.key, { column: c, of: n })
      }
      cluster = []
      clusterEnd = -Infinity
    }
    for (const p of list) {
      const visualEnd = Math.max(p.end, p.start + (opts.minHeight / opts.hourHeight) * 60)
      if (p.start >= clusterEnd) flush()
      cluster.push(p)
      clusterEnd = Math.max(clusterEnd, visualEnd)
    }
    flush()
  }
  return { rects, columns }
}

/** The day column and minute under a point, snapped to `snap` minutes. */
export function timeAt(x: number, y: number, opts: TimeGridOptions, snap = 15): { day: number; minute: number } | null {
  const lx = x - (opts.x ?? 0)
  if (lx < 0 || lx >= opts.width) return null
  const day = Math.floor(lx / (opts.width / opts.days))
  const raw = ((y - (opts.y ?? 0)) / opts.hourHeight) * 60
  const minute = Math.min(24 * 60, Math.max(0, Math.round(raw / snap) * snap))
  return { day, minute }
}

// ---- agenda

export interface AgendaGroup { day: number; items: ListItem[] }

export function layoutAgenda(groups: AgendaGroup[], opts: { x?: number; y?: number; width: number; dayHeader: number; gap: number; groupGap: number }): { headers: { day: number; rect: Rect }[]; rects: Map<string, Rect>; height: number } {
  const headers: { day: number; rect: Rect }[] = []
  const rects = new Map<string, Rect>()
  const x = opts.x ?? 0
  let y = opts.y ?? 0
  for (const g of groups) {
    headers.push({ day: g.day, rect: { x, y, w: opts.width, h: opts.dayHeader } })
    y += opts.dayHeader
    for (const item of g.items) {
      rects.set(item.key, { x, y, w: opts.width, h: item.h })
      y += item.h + opts.gap
    }
    y += opts.groupGap
  }
  return { headers, rects, height: y - (opts.y ?? 0) }
}
