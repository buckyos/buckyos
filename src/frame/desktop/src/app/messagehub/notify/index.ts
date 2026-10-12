import { useCallback, useEffect, useMemo, useSyncExternalStore } from 'react'
import { foldMessageRelations, messageObjId } from '../conversation/history/relations'
import type { ConversationMessageReader } from '../conversation/history/types'
import { getMockNotifyBackend } from '../mock/notify'
import { messageNotify, type MessageObject, type NotifyStatus } from '../protocol/msgobj'
import { isMessageHubMock } from '../store'
import { createApiNotifyBackend } from './api'
import { notifyFilterMatches, type NotifyFilter } from './model'
import { NotifyWatch, type NotifyEntry } from './watch'

let instance: NotifyWatch | null = null

/** One notification cache for every MessageHub view: all cards of a notification read the same entry. */
export function getNotifyWatch(): NotifyWatch {
  if (instance) return instance
  const watch = new NotifyWatch(isMessageHubMock() ? getMockNotifyBackend() : createApiNotifyBackend())
  instance = watch
  if (typeof window !== 'undefined') {
    window.addEventListener('online', () => watch.resync())
    document.addEventListener('visibilitychange', () => { if (document.visibilityState === 'visible') watch.resync() })
    if (import.meta.env.DEV) Object.assign(window, { __messageHubNotifyWatch: watch })
  }
  return watch
}

/** Watches a notification while the caller is mounted. */
export function useNotifyEntry(notifyId: string | undefined): NotifyEntry | undefined {
  const watch = getNotifyWatch()
  useEffect(() => notifyId ? watch.watch(notifyId) : undefined, [watch, notifyId])
  return useSyncExternalStore(watch.subscribe, () => notifyId ? watch.entry(notifyId) : undefined)
}

/** Reads every listed notification (once) and re-renders on any change; for views that filter by status. */
export function useNotifyStates(notifyIds: readonly string[]): number {
  const watch = getNotifyWatch()
  const key = useMemo(() => [...new Set(notifyIds)].sort().join('\n'), [notifyIds])
  useEffect(() => { if (key) watch.ensure(key.split('\n')) }, [watch, key])
  // Nothing listed (no filter applied): no re-render on every notification change.
  const subscribe = useCallback((listener: () => void) => key ? watch.subscribe(listener) : () => {}, [watch, key])
  return useSyncExternalStore(subscribe, () => key ? watch.version() : 0)
}

function hash(text: string): number {
  let value = 5381
  for (let index = 0; index < text.length; index++) value = ((value << 5) + value + text.charCodeAt(index)) | 0
  return value
}

/**
 * The rows of a notification session under a client-side filter: the cards
 * whose notification is pending (`open` / `read`) or done. A card whose state
 * is not read yet shows only under "all". `all` is the reader itself.
 */
export function useNotifyFilteredReader(reader: ConversationMessageReader, filter: NotifyFilter): ConversationMessageReader {
  const timeline = reader.timeline
  const ids = useMemo(() => filter === 'all' ? [] : (timeline ?? []).flatMap(message => messageNotify(message)?.notify_id ?? []), [timeline, filter])
  useNotifyStates(ids)
  const watch = getNotifyWatch()
  // `id=status` of every card: the rows change exactly when one of these does.
  const states = ids.map(id => `${id}=${watch.entry(id)?.view?.status ?? ''}`).join('\n')
  return useMemo(() => {
    if (filter === 'all') return reader
    const status = new Map(states.split('\n').map(item => [item.slice(0, item.lastIndexOf('=')), item.slice(item.lastIndexOf('=') + 1)]))
    const rows: MessageObject[] = foldMessageRelations(timeline ?? []).filter(message => {
      const data = messageNotify(message)
      return !!data && notifyFilterMatches((status.get(data.notify_id) || undefined) as NotifyStatus | undefined, filter)
    })
    return {
      readerKey: `${reader.readerKey}:notify:${filter}`,
      // Rows come and go in the middle as states arrive: every change is a new revision, never an append.
      revision: hash(rows.map(message => messageObjId(message) ?? String(message.ui_message_id)).join('\n')),
      totalCount: rows.length,
      timeline: rows,
      readRange: async (start: number, count: number) => rows.slice(Math.max(0, start), Math.max(0, start) + Math.max(0, count)),
    } as ConversationMessageReader
  }, [reader, timeline, filter, states])
}
