/* The record a table editor is on (UI 框架 PRD §5.5), for the data mode's status bar: the record open in the form, else
 * the row of the grid's current cell — among the view's loaded rows. A data-mode tab's editor (size `tab`) publishes it
 * under its table's id; only the current tab's editor is mounted, so there is one at a time. */

import { useSyncExternalStore } from 'react'

export interface TableCurrent {
  recordId: string
  /** 1-based position among the view's rows. */
  index: number
  /** Rows of the view (its filter applied). */
  total: number
  /** The record's title, or null. */
  title: string | null
}

const current = new Map<string, TableCurrent>()
const listeners = new Set<() => void>()

function same(a: TableCurrent | undefined, b: TableCurrent | null): boolean {
  if (!a || !b) return !a && !b
  return a.recordId === b.recordId && a.index === b.index && a.total === b.total && a.title === b.title
}

export function publishTableCurrent(sourceId: string, value: TableCurrent | null) {
  if (same(current.get(sourceId), value)) return
  if (value) current.set(sourceId, value)
  else current.delete(sourceId)
  for (const listener of listeners) listener()
}

const subscribe = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener) } }

/** What the table `sourceId` is on, while a data-mode tab shows it. */
export function useTableCurrent(sourceId: string | null): TableCurrent | undefined {
  return useSyncExternalStore(subscribe, () => (sourceId ? current.get(sourceId) : undefined))
}
