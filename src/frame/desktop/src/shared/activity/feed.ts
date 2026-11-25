/* ── Activity: one list of notifications, unread conversations and tasks, newest first ──
 *
 * A plain store over a Zone channel: `start()` reads and keeps reading, `refresh()` is for a caller that learned of a
 * change (a kevent). Notifications are paged; while older ones are not loaded yet, rows of the other sources that are
 * older than the last loaded notification wait for `loadMore()`, so paging only ever adds rows at the end.
 */

import { answerNotify, type MsgCenterClient, type TaskManagerClient } from 'buckyos'
import { msgCenterOf, taskManagerOf, type ZoneChannel } from '../channel.ts'
import { readAccountDid } from '../controlPanel/account.ts'
import type { ProjectionLabels } from '../messagehub/projection.ts'
import { ConversationReader } from './conversations.ts'
import { loop } from './loop.ts'
import type { UnreadConversations } from './messages.ts'
import { dismissTasks, isTaskFinished, mergeActivity, undismissedTasks, type ActivityEntry, type ActivitySource, type ActivityTaskItem, type DismissedTasks } from './model.ts'
import { NotifyPages } from './notifies.ts'
import { SummaryReader, type ActivitySummary } from './summary.ts'
import { ACTIVITY_TASK_WINDOW, createTaskReader, type TaskReader } from './tasks.ts'

const PAGE_SIZE = 50
const NOTIFY_POLL_MS = 30_000
const MESSAGE_POLL_MS = 20_000
/** Task reads while one is running (progress) / while none is. */
const TASK_ACTIVE_POLL_MS = 2_000
const TASK_IDLE_POLL_MS = 20_000
const PARALLEL_ACKS = 8

export type ActivityLoadStatus = 'loading' | 'ready' | 'error'

export interface ActivitySnapshot {
  entries: ActivityEntry[]
  status: Record<ActivitySource, ActivityLoadStatus>
  /** Older notifications can be loaded. */
  hasMore: boolean
  loadingMore: boolean
  /** `notify.summary`; counted from what is loaded when the Zone does not have it. */
  summary: ActivitySummary
}

export interface ActivityFeedOptions {
  channel: ZoneChannel
  userId: string
  /** The words conversation rows need, in the viewer's language. */
  labels: () => ProjectionLabels
  /** Finished tasks the user dismissed, kept by the caller. */
  dismissed: { read(): DismissedTasks; write(dismissed: DismissedTasks): void }
  /** Polling pauses while false (the page is hidden). */
  active?: () => boolean
  pageSize?: number
  now?: () => number
}

const noSummary: ActivitySummary = { notify: { open: 0, todo: 0, agent: 0, app: 0 }, messages: { unread: 0 } }

export class ActivityFeed {
  private readonly options: ActivityFeedOptions
  private readonly msgCenter: MsgCenterClient
  private readonly taskManager: TaskManagerClient
  private readonly notifies: NotifyPages
  private readonly summaryReader: SummaryReader
  private readonly taskReader: TaskReader
  private conversations: Promise<ConversationReader> | null = null
  private messages: UnreadConversations = { unread: 0, items: [] }
  private tasks: ActivityTaskItem[] = []
  private summary: ActivitySummary | null = null
  private loops: Partial<Record<ActivitySource, ReturnType<typeof loop>>> = {}
  private listeners = new Set<() => void>()
  private snapshot: ActivitySnapshot = {
    entries: [],
    status: { message: 'loading', notify: 'loading', task: 'loading' },
    hasMore: false,
    loadingMore: false,
    summary: noSummary,
  }

  constructor(options: ActivityFeedOptions) {
    this.options = options
    this.msgCenter = msgCenterOf(options.channel)
    this.taskManager = taskManagerOf(options.channel)
    this.notifies = new NotifyPages(this.msgCenter, options.pageSize ?? PAGE_SIZE)
    this.summaryReader = new SummaryReader(options.channel)
    this.taskReader = createTaskReader({ client: this.taskManager, username: () => Promise.resolve(options.userId), window: ACTIVITY_TASK_WINDOW, now: options.now })
  }

  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }

  getSnapshot = () => this.snapshot

  /** Reads now and keeps the list current until the returned function is called. */
  start(): () => void {
    const { active } = this.options
    this.loops = {
      notify: loop(() => this.readNotifies(), () => NOTIFY_POLL_MS, active),
      message: loop(() => this.readMessages(), () => MESSAGE_POLL_MS, active),
      task: loop(() => this.readTasks(), () => (this.tasks.some(task => task.state === 'running') ? TASK_ACTIVE_POLL_MS : TASK_IDLE_POLL_MS), active),
    }
    const started = this.loops
    return () => {
      Object.values(started).forEach(running => running?.stop())
      if (this.loops === started) this.loops = {}
    }
  }

  /** Reads a source (all of them by default) again now. */
  async refresh(source?: ActivitySource): Promise<void> {
    const reads = {
      notify: () => this.readNotifies(),
      message: () => this.readMessages(),
      task: () => this.readTasks(),
    }
    await Promise.all(source ? [reads[source]()] : Object.values(reads).map(read => read()))
  }

  async loadMore(): Promise<void> {
    if (this.snapshot.loadingMore || !this.notifies.hasMore) return
    this.publish({ loadingMore: true })
    try {
      await this.notifies.loadMore()
    } finally {
      this.publish({ loadingMore: false })
    }
  }

  /** Marks a `notice` read. */
  async ackNotify(notifyId: string): Promise<void> {
    this.notifies.put(await this.msgCenter.notifyAck(notifyId))
    this.publish()
    await this.readSummary()
  }

  /** Dismisses a notification; an `input` is answered with the dismissal. */
  async dismissNotify(notifyId: string): Promise<void> {
    this.notifies.put(await answerNotify(this.msgCenter, this.taskManager, notifyId, { dismiss: true }))
    this.publish()
    await this.readSummary()
  }

  async markSessionRead(sessionId: string): Promise<void> {
    await (await this.conversationReader()).markRead(sessionId)
    await this.readMessages()
  }

  /** Takes a finished task off the list on this device. */
  dismissTask(taskId: string): void {
    this.dismiss([taskId])
  }

  /**
   * Marks every open `notice` and every listed conversation read and dismisses the finished tasks. What waits for an
   * answer stays. Rejects with the first failure after trying everything.
   */
  async markAllRead(): Promise<void> {
    const failures: unknown[] = []
    const attempt = async (work: () => Promise<unknown>) => {
      try { await work() } catch (error) { failures.push(error) }
    }
    this.dismiss(this.tasks.filter(task => isTaskFinished(task.state)).map(task => task.taskId))
    await attempt(() => this.ackOpenNotices(attempt))
    await attempt(async () => {
      const reader = await this.conversationReader()
      for (const item of this.messages.items) await attempt(() => reader.markRead(item.sessionId))
    })
    await this.refresh()
    if (failures.length > 0) throw failures[0]
  }

  private async ackOpenNotices(attempt: (work: () => Promise<unknown>) => Promise<void>) {
    let cursor: string | undefined
    do {
      const page = await this.msgCenter.notifyList({ status: ['open'], kind: 'notice', limit: 200, ...(cursor ? { cursor } : {}) })
      for (let offset = 0; offset < page.items.length; offset += PARALLEL_ACKS) {
        await Promise.all(page.items.slice(offset, offset + PARALLEL_ACKS).map(view => attempt(async () => this.notifies.put(await this.msgCenter.notifyAck(view.notify_id)))))
      }
      cursor = page.next_cursor || undefined
    } while (cursor)
  }

  private conversationReader(): Promise<ConversationReader> {
    const { channel, userId, labels, now } = this.options
    this.conversations ??= readAccountDid(channel, userId).then(owner => {
      if (!owner) throw new Error('the account has no DID')
      return new ConversationReader({ channel, userId, owner, labels, now })
    }).catch((error: unknown) => { this.conversations = null; throw error })
    return this.conversations
  }

  private async readNotifies() {
    await this.read('notify', () => this.notifies.refresh())
    await this.readSummary()
  }

  private async readMessages() {
    await this.read('message', async () => { this.messages = await (await this.conversationReader()).read() })
    await this.readSummary()
  }

  private async readTasks() {
    await this.read('task', async () => { this.tasks = await this.taskReader.read() })
  }

  /** A failed read keeps the rows that are shown and is only an error before the first good one. */
  private async read(source: ActivitySource, work: () => Promise<void>) {
    try {
      await work()
      this.publish({ status: { ...this.snapshot.status, [source]: 'ready' } })
    } catch (error) {
      console.warn(`activity: ${source} read failed`, error)
      if (this.snapshot.status[source] !== 'ready') this.publish({ status: { ...this.snapshot.status, [source]: 'error' } })
    }
  }

  private async readSummary() {
    // A read that failed for now leaves the counts as they were.
    this.summary = await this.summaryReader.read().catch(() => this.summary)
    this.publish()
  }

  private countedSummary(): ActivitySummary {
    const open = this.notifies.list().filter(item => item.status === 'open')
    const ofCategory = (category: string) => open.filter(item => item.category === category).length
    return {
      notify: { open: open.length, todo: ofCategory('todo'), agent: ofCategory('agent'), app: ofCategory('app') },
      messages: { unread: this.messages.unread },
    }
  }

  private dismiss(taskIds: string[]) {
    if (taskIds.length === 0) return
    this.options.dismissed.write(dismissTasks(this.options.dismissed.read(), taskIds, this.options.now?.()))
    this.publish()
  }

  private publish(patch: Partial<ActivitySnapshot> = {}) {
    const notifies = this.notifies.list()
    const merged = mergeActivity({ messages: this.messages.items, notifies, tasks: undismissedTasks(this.tasks, this.options.dismissed.read()) })
    const hasMore = this.notifies.hasMore
    const oldest = notifies[notifies.length - 1]?.timestamp ?? -Infinity
    this.snapshot = {
      ...this.snapshot,
      ...patch,
      entries: hasMore ? merged.filter(entry => entry.timestamp >= oldest) : merged,
      hasMore,
      summary: this.summary ?? this.countedSummary(),
    }
    this.listeners.forEach(listener => listener())
  }
}
