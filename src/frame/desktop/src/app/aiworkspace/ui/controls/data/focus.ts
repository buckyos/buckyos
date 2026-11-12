/* Going to a record of a data control (控件机制 TODO G12): a link, a notification or an Agent's reply names the control,
 * the record and, for a repeating event, the occurrence (its original start: the occurrence's identity apart from when
 * it is planned, D4 §5.5). The shell asks; the control's body, once mounted, turns to that day or column and selects
 * it. A request made before the body exists waits for it. Also the link and the annotation anchor of a record. */

import { useEffect, useRef } from 'react'
import type { CapturedAnchor } from '../../../anchors/registry'
import { targetQuery, workspacePageUrl } from '../../../links'
import type { WorkspaceStore } from '../../../state/store'

export interface RecordFocus { record: string; occurrence?: string | null; nonce: number }

const pending = new Map<string, RecordFocus>()
const listeners = new Map<string, (focus: RecordFocus) => void>()
let nonce = 0

/** Ask the control to show a record (delivered now, or when its body mounts). */
export function requestRecordFocus(containerId: string, record: string, occurrence?: string | null) {
  const focus = { record, occurrence: occurrence ?? null, nonce: ++nonce }
  const listener = listeners.get(containerId)
  if (listener) listener(focus)
  else pending.set(containerId, focus)
}

/** A body receives the requests for its control (the latest handler is called; a waiting request at once). */
export function useRecordFocus(containerId: string, handler: (focus: RecordFocus) => void) {
  const ref = useRef(handler)
  useEffect(() => { ref.current = handler })
  useEffect(() => {
    const listener = (focus: RecordFocus) => ref.current(focus)
    listeners.set(containerId, listener)
    const waiting = pending.get(containerId)
    if (waiting) { pending.delete(containerId); listener(waiting) }
    return () => { if (listeners.get(containerId) === listener) listeners.delete(containerId) }
  }, [containerId])
}

/** The address of a record in a control: the Workspace page when the service names one, else this page's address. */
export function recordLink(store: WorkspaceStore, containerId: string, record: string, occurrence?: string | null): string {
  const surface = store.outline.ancestors(containerId).map((id) => store.outline.get(id)).find((e) => e?.kind === 'surface')
  const info = store.session.info()
  const inner = { record, occurrence: occurrence ?? null }
  const page = surface ? workspacePageUrl(info.public_base, info.title, { id: surface.entity_id, title: surface.title ?? surface.name ?? '' }, containerId, inner) : null
  if (page) return page
  const url = new URL(window.location.href)
  url.search = targetQuery(surface?.entity_id ?? '', containerId, inner).toString()
  return url.toString()
}

/** Copy a record's link (with a notice). */
export function copyRecordLink(store: WorkspaceStore, containerId: string, record: string, occurrence: string | null, done: string) {
  const link = recordLink(store, containerId, record, occurrence)
  void navigator.clipboard?.writeText(link).then(() => store.notify('info', done), () => store.notify('info', link))
}

/** What annotating a record anchors to: the table's record selector (02), whatever control shows it. */
export function recordAnchor(tableId: string, record: string, label: string): CapturedAnchor {
  return { target: { entity_id: tableId, selector: { kind: 'table_record', record_id: record } }, label }
}

/** The records of a table that have annotations, from the annotations in scope. */
export function annotatedRecords(annotations: readonly { payload: { target?: { entity_id?: string; selector?: unknown } } }[], tableId: string | null): Set<string> {
  const out = new Set<string>()
  if (!tableId) return out
  for (const a of annotations) {
    const target = a.payload.target
    const sel = target?.selector as { kind?: string; record_id?: string } | undefined
    if (target?.entity_id === tableId && sel?.kind === 'table_record' && sel.record_id) out.add(sel.record_id)
  }
  return out
}

// ---- a control as a page of its own (G14): asked by a control's menu, served by the shell

const pageListeners = new Set<(containerId: string | null) => void>()

/** Show a data control as the whole page (null: back to the canvas). */
export function openControlPage(containerId: string | null) {
  for (const listener of [...pageListeners]) listener(containerId)
}

export function onControlPage(listener: (containerId: string | null) => void): () => void {
  pageListeners.add(listener)
  return () => { pageListeners.delete(listener) }
}
