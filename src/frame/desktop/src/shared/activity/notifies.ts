/* ── System Notify as rows ── */

import type { MsgCenterClient, NotifyView } from 'buckyos'
import { parseHumanTaskPrompt } from '../messagehub/msgobj.ts'
import type { ActivityNotifyItem } from './model.ts'

/** A page an App may send the user to: an address of its own, or a path of the Zone's root. */
function usableLink(value: unknown): string | undefined {
  const link = typeof value === 'string' ? value.trim() : ''
  return /^(https?:\/\/|\/(?!\/))/i.test(link) ? link : undefined
}

/** The row of a notification. Its prompt comes from an App and is read the way a card renders it. */
export function notifyItem(view: NotifyView): ActivityNotifyItem {
  const prompt = parseHumanTaskPrompt(view.prompt)
  const title = prompt.title?.trim()
  const message = prompt.message?.trim()
  const link = usableLink(view.source?.link)
  return {
    notifyId: view.notify_id,
    title: title || message || view.notify_id,
    ...(title && message ? { message } : {}),
    category: view.category,
    kind: view.kind,
    level: view.level,
    status: view.status,
    needsAnswer: view.kind === 'input' && view.status === 'open',
    ...(view.source?.app_id ? { appId: view.source.app_id } : {}),
    ...(view.source?.agent_did ? { agentDid: view.source.agent_did } : {}),
    ...(link ? { link } : {}),
    timestamp: view.created_at_ms,
  }
}

const MAX_JOIN_PAGES = 4
const OPEN_LIMIT = 200
const MAX_REREADS = 20

type NotifyClient = Pick<MsgCenterClient, 'notifyList' | 'notifyGet'>

/** Notifications of every status, newest first, read a page at a time and kept current. */
export class NotifyPages {
  private readonly client: NotifyClient
  private readonly pageSize: number
  private items = new Map<string, ActivityNotifyItem>()
  private cursor: string | undefined
  private loaded = false
  private queue: Promise<unknown> = Promise.resolve()

  constructor(client: NotifyClient, pageSize: number) {
    this.client = client
    this.pageSize = pageSize
  }

  list(): ActivityNotifyItem[] {
    return [...this.items.values()].sort((a, b) => b.timestamp - a.timestamp)
  }

  get hasMore(): boolean {
    return this.cursor !== undefined
  }

  put(view: NotifyView) {
    if (this.items.has(view.notify_id)) this.items.set(view.notify_id, notifyItem(view))
  }

  /** Reads what is new, and what became of the loaded ones that were still open. */
  refresh(): Promise<void> {
    return this.enqueue(async () => {
      const known = this.items
      const fresh = new Map<string, ActivityNotifyItem>()
      let cursor: string | undefined
      let joined = false
      for (let page = 0; page < (this.loaded ? MAX_JOIN_PAGES : 1); page++) {
        const result = await this.client.notifyList({ limit: this.pageSize, ...(cursor ? { cursor } : {}) })
        for (const view of result.items) fresh.set(view.notify_id, notifyItem(view))
        cursor = result.next_cursor || undefined
        joined = result.items.some((view) => known.has(view.notify_id))
        if (joined || !cursor) break
      }
      if (joined && cursor) {
        const older = [...known.values()].filter((item) => !fresh.has(item.notifyId))
        this.items = new Map([...fresh, ...older.map((item) => [item.notifyId, item] as const)])
        await this.rereadOpen(older.filter((item) => item.status === 'open'))
      } else {
        // The first read, the whole list, or too much arrived since: what was loaded further back is read again by paging.
        this.items = fresh
        this.cursor = cursor
      }
      this.loaded = true
    })
  }

  loadMore(): Promise<void> {
    return this.enqueue(async () => {
      if (!this.cursor) return
      const result = await this.client.notifyList({ limit: this.pageSize, cursor: this.cursor })
      for (const view of result.items) this.items.set(view.notify_id, notifyItem(view))
      this.cursor = result.next_cursor || undefined
    })
  }

  private async rereadOpen(items: ActivityNotifyItem[]) {
    if (items.length === 0) return
    const open = await this.client.notifyList({ status: ['open'], limit: OPEN_LIMIT })
    const stillOpen = new Set(open.items.map((view) => view.notify_id))
    const changed = items.filter((item) => !stillOpen.has(item.notifyId)).slice(0, MAX_REREADS)
    await Promise.all(changed.map(async (item) => {
      const view = await this.client.notifyGet(item.notifyId).catch(() => null)
      if (view) this.put(view)
    }))
  }

  /** One read at a time, so a page never lands on a list another read has replaced. */
  private enqueue(run: () => Promise<void>): Promise<void> {
    const next = this.queue.then(run, run)
    this.queue = next.catch(() => undefined)
    return next
  }
}
