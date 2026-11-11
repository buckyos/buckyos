import { useMemo, useSyncExternalStore } from 'react'
import type { ConversationMessageReader } from '../conversation/history/types'
import type { MessageObject } from '../protocol/msgobj'

const STORAGE_KEY = 'buckyos.messagehub.agentBreaks.v1'
const MAX_SESSIONS = 200

let memory: Record<string, number[]> | null = null
const listeners = new Set<() => void>()

function load(): Record<string, number[]> {
  if (memory) return memory
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '{}')
    memory = parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed as Record<string, number[]> : {}
  } catch {
    memory = {}
  }
  return memory
}

const keyOf = (viewerDid: string, sessionId: string) => `${viewerDid}\n${sessionId}`
const EMPTY: number[] = []

/**
 * When the Agent side of a conversation started over ("Start a new
 * conversation"): the times its earlier UI Session generations were
 * archived. Known from `ui.conversation`, which is only read while the
 * panel is open, so they are remembered per viewer in this browser and the
 * separators stay in the timeline after a reload.
 */
export function rememberGenerationBreaks(viewerDid: string, sessionId: string, breaks: readonly number[]) {
  const all = load()
  const key = keyOf(viewerDid, sessionId)
  const next = [...new Set(breaks)].sort((a, b) => a - b)
  if (JSON.stringify(all[key] ?? []) === JSON.stringify(next)) return
  const rest = Object.fromEntries(Object.entries(all).filter(([name]) => name !== key).slice(-(MAX_SESSIONS - 1)))
  memory = next.length ? { ...rest, [key]: next } : rest
  try { localStorage.setItem(STORAGE_KEY, JSON.stringify(memory)) } catch { /* kept for this page only */ }
  listeners.forEach(listener => listener())
}

const subscribe = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener) } }

export function useGenerationBreaks(viewerDid: string, sessionId: string | undefined): readonly number[] {
  return useSyncExternalStore(subscribe, () => sessionId ? load()[keyOf(viewerDid, sessionId)] ?? EMPTY : EMPTY)
}

/** `rows` with a status pill where each break falls. */
export function rowsWithBreaks(rows: readonly MessageObject[], breaks: readonly number[], label: string, sessionId: string): MessageObject[] {
  if (rows.length === 0 || breaks.length === 0) return [...rows]
  const pills = breaks.map((at): MessageObject => ({ from: rows[0].from, to: rows[0].to, kind: 'notify', created_at_ms: at, content: { format: 'text/plain', content: label }, ui_item_kind: 'status', ui_message_id: `generation-break:${at}`, ui_session_id: sessionId, ui_status_type: 'info' }))
  const merged: MessageObject[] = []
  let next = 0
  for (const row of rows) {
    while (next < pills.length && pills[next].created_at_ms < row.created_at_ms) merged.push(pills[next++])
    merged.push(row)
  }
  return [...merged, ...pills.slice(next)]
}

/**
 * `reader` with the generation breaks of the conversation shown in its
 * timeline. Breaks older than the loaded history are left out; without any
 * (or without a loaded timeline) the reader is returned as it is.
 */
export function useReaderWithBreaks(reader: ConversationMessageReader, breaks: readonly number[], label: string, sessionId: string | undefined): ConversationMessageReader {
  const first = reader.timeline?.[0]?.created_at_ms
  return useMemo(() => {
    const shown = first === undefined ? [] : breaks.filter(at => at >= first)
    if (!sessionId || shown.length === 0) return reader
    let rows: Promise<MessageObject[]> | null = null
    const base = (reader as { revision?: unknown }).revision
    return {
      readerKey: reader.readerKey,
      revision: (typeof base === 'number' ? base : 0) * 1009 + shown.length,
      totalCount: reader.totalCount + shown.length,
      timeline: reader.timeline,
      readRange: async (start: number, count: number) => {
        rows ??= reader.readRange(0, reader.totalCount).then(loaded => rowsWithBreaks(loaded, shown, label, sessionId))
        return (await rows).slice(Math.max(0, start), Math.max(0, start) + Math.max(0, count))
      },
    } as ConversationMessageReader
  }, [reader, first, breaks, label, sessionId])
}
