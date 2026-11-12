/* The official to-do control's model (控件机制 TODO §0.2, G2–G8): its kind, its view (list or board, hidden completed
 * items, folded columns — the instance's default and each person's own, G6), the templates its items are projected
 * with (label, shape; Q1: no Block types of its own) and the item sizes the layouts use. */

import { CARD_TEMPLATE, LIST_TEMPLATE } from '@controls/schedule'
import { todoItem, type TodoItem } from 'buckyos'
import type { Json, QueryRow } from '../../../api/types'

export { CARD_TEMPLATE, LIST_TEMPLATE }

export const TODO_KIND = 'todo'
/** The binding naming the to-do table. */
export const ITEMS_BINDING = 'items'

export interface TodoView extends Record<string, Json | undefined> {
  mode: 'list' | 'board'
  hide_done: boolean
  /** Folded board columns (status option ids). */
  collapsed: string[]
}

export const TODO_VIEW_DEFAULTS: TodoView = { mode: 'list', hide_done: false, collapsed: [] }
export const TODO_VIEW_KEYS = ['mode', 'hide_done', 'collapsed'] as const

export const LIST_ROW_H = 38
export const LIST_GAP = 2
export const CARD_GAP = 8
export const COLUMN_W = 260
export const COLUMN_COLLAPSED_W = 44
export const COLUMN_HEADER = 40
export const COLUMN_GAP = 12

export type Priority = 'low' | 'normal' | 'high' | 'urgent'
export const PRIORITY_COLOR: Record<Priority, string> = { low: 'muted', normal: 'accent', high: 'warning', urgent: 'danger' }

/** What a projected item's placeholders read: the record's values as the control shows them. */
export function itemScope(item: TodoItem, opts: { selected: boolean; today: string; statusLabel: string; doneId: string; noted?: boolean }): Record<string, unknown> {
  const overdue = !item.done && item.due !== null && item.due < opts.today
  const today = !item.done && item.due === opts.today
  const priority = typeof item.ext.priority === 'string' ? item.ext.priority as Priority : null
  return {
    item: {
      id: item.record_id, title: item.title || null, due: item.due, done: item.done, status: item.status, status_label: opts.statusLabel,
      check_fill: item.done ? 'accent' : 'none', check_stroke: item.done ? 'accent' : 'muted',
      title_color: item.done ? 'muted' : 'text', due_color: overdue ? 'danger' : today ? 'accent' : 'muted',
      row_fill: opts.selected ? 'selected' : 'none', row_stroke: opts.selected ? 'selected-border' : 'none',
      card_stroke: opts.selected ? 'accent' : 'border',
      priority, priority_color: priority ? PRIORITY_COLOR[priority] : null,
      doing: item.status !== opts.doneId && item.status !== 'todo' ? true : null,
      tags: Array.isArray(item.ext.tags) && item.ext.tags.length ? (item.ext.tags as string[]).join(' · ') : null,
      noted: opts.noted ? true : null,
    },
  }
}

/** Rough text width (CJK about one em, Latin about half): enough to size a wrapping card without measuring the DOM. */
export function textWidth(text: string, size: number): number {
  let w = 0
  for (const ch of text) w += /[ᄀ-￿]/.test(ch) ? size : /[A-Z0-9mwMW]/.test(ch) ? size * 0.62 : size * 0.52
  return w
}

/** A card's height: its title's lines, and the footer line when it has a due date or tags. */
export function cardHeight(item: TodoItem, width: number): number {
  const lines = Math.min(6, Math.max(1, Math.ceil(textWidth(item.title || ' ', 13) / Math.max(40, width - 46))))
  const footer = item.due || (Array.isArray(item.ext.tags) && item.ext.tags.length) ? 24 : 0
  return 10 + lines * 18 + footer + 8
}

export function itemsOf(rows: readonly QueryRow[]): TodoItem[] {
  return rows.map((r) => todoItem(r as never))
}

/** Today on the reader's calendar (`YYYY-MM-DD`). */
export function todayLocal(now = new Date()): string {
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`
}
