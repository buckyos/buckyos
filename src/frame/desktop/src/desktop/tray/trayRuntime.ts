/**
 * Readers behind the status-bar trays (loaded on demand by `trayModel.ts`):
 *
 * - messages: the shared MessageHub store, synchronised like MessageHub does
 *   (msg-center kevents plus a slow poll);
 * - notifications: `notify.list {status: [open]}` through MessageHub's
 *   notification cache, read again when a notification changes or the system
 *   notification sessions receive something;
 * - tasks: TaskMgr tasks of the schemas whose `presentation_schema` asks for
 *   system notifications, created by the viewer in the last days.
 *
 * Mock mode reads the MessageHub mock store, the mock notifications and the
 * mock downloads instead.
 */
import { buckyos, TaskOutcome, TaskPhase } from 'buckyos'
import type { Task as TaskMgrTask, TaskSchemaDefinition, TaskSummary } from 'buckyos'
import { getMessageHubStore } from '../../app/messagehub/store'
import { getNotifyWatch } from '../../app/messagehub/notify'
import { fetchAccountUsername } from '../../app/messagehub/datamodel/sessionApi'
import type { NotifyView } from '../../app/messagehub/notify/model'
import type { MessageHubContext } from '../../app/messagehub/types'
import { toProgressPercent } from '../../api/task_progress'
import { isMockRuntime } from '../../runtime'
import type { TrayMessageItem, TrayNotifyItem, TrayRuntimeSink, TrayTaskItem, TrayTaskState } from './trayModel'

const MAX_MESSAGE_ROWS = 30
const MAX_NOTIFY_ROWS = 50
const NOTIFY_POLL_MS = 30_000
/** Task reads while one is running (progress) / while none is. */
const TASK_ACTIVE_POLL_MS = 2_000
const TASK_IDLE_POLL_MS = 20_000
/** Tasks created this long ago are no longer looked at. */
const TASK_LOOKBACK_MS = 7 * 24 * 60 * 60 * 1000
/** A finished task leaves the bell after this long, dismissed or not. */
const FINISHED_TASK_TTL_MS = 3 * 24 * 60 * 60 * 1000
/** `presentation_schema` key of a schema whose tasks show in the bell (buckyos-api `TASK_PRESENTATION_SYSTEM_NOTIFY`). */
const SYSTEM_NOTIFY_KEY = 'system_notify'

type SystemNotifyMode = 'progress' | 'completion'

function visible(): boolean {
  return typeof document === 'undefined' || document.visibilityState !== 'hidden'
}

/** Runs `run` now and then every `delay()` ms while the page is visible; `kick` runs it again at once. */
function loop(run: () => Promise<void>, delay: () => number) {
  let timer: ReturnType<typeof setTimeout> | null = null
  let running = false
  let again = false
  let stopped = false
  const schedule = () => {
    if (stopped) return
    if (timer) clearTimeout(timer)
    timer = setTimeout(() => { timer = null; if (visible()) void tick(); else schedule() }, delay())
  }
  const tick = async () => {
    if (stopped) return
    if (running) { again = true; return }
    running = true
    try { await run() } finally {
      running = false
      if (again) { again = false; void tick() } else schedule()
    }
  }
  void tick()
  return {
    kick: () => { void tick() },
    stop: () => { stopped = true; if (timer) clearTimeout(timer) },
  }
}

/* ── Messages ── */

function messageRows(context: MessageHubContext): { unread: number; items: TrayMessageItem[]; unloadedPrefs: string[] } {
  const store = getMessageHubStore()
  const rows: TrayMessageItem[] = []
  let unread = 0
  const sessionCounts = new Map<string, number>()
  const candidates = store.sessions(context).filter((session) => session.unreadCount > 0)
  for (const session of candidates) {
    const entity = store.findEntity(context, session.entityId)
    if (entity?.systemNotify) continue
    if (store.preferences(context, session.id).muted) continue
    unread += session.unreadCount
    const name = entity?.name ?? store.title(context, session)
    let count = sessionCounts.get(session.entityId)
    if (count === undefined) {
      count = store.sessions(context, session.entityId).length
      sessionCounts.set(session.entityId, count)
    }
    rows.push({
      sessionId: session.id,
      entityId: session.entityId,
      title: count > 1 ? `${name} · ${store.title(context, session)}` : name,
      avatar: entity?.avatar,
      isGroup: entity?.type === 'group',
      senderName: session.lastMessage?.senderName,
      text: session.lastMessage?.text ?? '',
      unreadCount: session.unreadCount,
      timestamp: session.lastMessage?.timestamp ?? session.lastActiveAt,
    })
  }
  rows.sort((a, b) => b.timestamp - a.timestamp)
  return { unread, items: rows.slice(0, MAX_MESSAGE_ROWS), unloadedPrefs: candidates.map((session) => session.id) }
}

/** The system notification sessions changed: something arrived or was read. */
function notifySignature(context: MessageHubContext): string {
  const store = getMessageHubStore()
  return store
    .sessions(context)
    .filter((session) => store.findEntity(context, session.entityId)?.systemNotify)
    .map((session) => `${session.id}:${session.lastActiveAt}:${session.unreadCount}`)
    .join('|')
}

function startMessages(sink: TrayRuntimeSink, onNotifySessionsChanged: () => void): () => void {
  const store = getMessageHubStore()
  let stopped = false
  let stopSync: (() => void) | null = null
  let unsubscribe: (() => void) | null = null
  let queued = false
  let lastSignature = ''

  const publish = (context: MessageHubContext) => {
    queued = false
    if (stopped) return
    const { unread, items, unloadedPrefs } = messageRows(context)
    sink.setMessages({ status: 'ready', unread, items })
    // Muted state lives in per-session preferences, loaded on demand.
    void store.loadPreferences(context, unloadedPrefs).catch(() => undefined)
    const signature = notifySignature(context)
    if (signature !== lastSignature) {
      if (lastSignature) onNotifySessionsChanged()
      lastSignature = signature
    }
  }

  store.initialize().then(() => {
    if (stopped) return
    const context = store.defaultContext()
    stopSync = store.startSync(context, null)
    void store.ensureOwner(context)
    unsubscribe = store.subscribe(() => {
      if (queued) return
      queued = true
      queueMicrotask(() => publish(context))
    })
    publish(context)
  }, (error: unknown) => {
    console.warn('status tray: MessageHub unavailable', error)
    if (!stopped) sink.setMessages({ status: 'error', unread: 0, items: [] })
  })

  return () => {
    stopped = true
    stopSync?.()
    unsubscribe?.()
  }
}

/* ── Notifications ── */

function notifyItem(view: NotifyView): TrayNotifyItem {
  return {
    notifyId: view.notify_id,
    title: view.prompt.title?.trim() || view.prompt.message?.trim() || view.notify_id,
    ...(view.prompt.title?.trim() && view.prompt.message?.trim() ? { message: view.prompt.message.trim() } : {}),
    category: view.category,
    needsAnswer: view.kind === 'input',
    timestamp: view.created_at_ms,
  }
}

function startNotifies(sink: TrayRuntimeSink) {
  const watch = getNotifyWatch()
  let items: TrayNotifyItem[] = []
  const reader = loop(async () => {
    try {
      const views = await watch.list({ status: ['open'], limit: MAX_NOTIFY_ROWS })
      items = views.map(notifyItem).sort((a, b) => b.timestamp - a.timestamp)
      sink.setNotifies('ready', items)
    } catch (error) {
      console.warn('status tray: notify.list failed', error)
      sink.setNotifies(items.length ? 'ready' : 'error', items)
    }
  }, () => NOTIFY_POLL_MS)
  // Answered / acknowledged in MessageHub: read the list again.
  let pending: ReturnType<typeof setTimeout> | null = null
  const unsubscribe = watch.subscribe(() => {
    if (pending) return
    pending = setTimeout(() => { pending = null; reader.kick() }, 300)
  })
  return {
    kick: reader.kick,
    stop: () => {
      reader.stop()
      unsubscribe()
      if (pending) clearTimeout(pending)
    },
  }
}

/* ── Tasks ── */

function toMillis(value: number | undefined): number {
  if (!value) return 0
  return value < 10_000_000_000 ? value * 1000 : value
}

function taskState(phase: TaskPhase, outcome: TaskOutcome | undefined): TrayTaskState {
  switch (phase) {
    case TaskPhase.Terminal:
      return outcome === TaskOutcome.Failed ? 'failed' : outcome === TaskOutcome.Canceled ? 'canceled' : 'completed'
    case TaskPhase.Waiting:
      return 'waiting'
    case TaskPhase.Paused:
      return 'paused'
    default:
      return 'running'
  }
}

function systemNotifyMode(schema: TaskSchemaDefinition): SystemNotifyMode | null {
  const presentation = schema.presentation_schema
  if (!presentation || typeof presentation !== 'object') return null
  const mode = (presentation as Record<string, unknown>)[SYSTEM_NOTIFY_KEY]
  return mode === 'progress' || mode === 'completion' ? mode : null
}

interface TaskReader {
  /** Every task to list, running ones included. */
  read(): Promise<TrayTaskItem[]>
}

function rpcTaskReader(): TaskReader {
  const client = buckyos.getTaskManagerClient()
  /** schema_id → schema_version → mode, read once per tray start. */
  let marked: Promise<Map<string, Map<number, SystemNotifyMode>>> | null = null
  let username: Promise<string> | null = null
  const details = new Map<string, { revision: number; task: TaskMgrTask }>()

  const loadMarked = () => {
    marked ??= client.listTaskSchemas({ include_disabled: true }).then((schemas) => {
      const map = new Map<string, Map<number, SystemNotifyMode>>()
      for (const schema of schemas) {
        const mode = systemNotifyMode(schema)
        if (!mode) continue
        const versions = map.get(schema.schema_id) ?? new Map<number, SystemNotifyMode>()
        versions.set(schema.schema_version, mode)
        map.set(schema.schema_id, versions)
      }
      return map
    }).catch((error: unknown) => { marked = null; throw error })
    return marked
  }

  const detailOf = async (summary: TaskSummary): Promise<TaskMgrTask | null> => {
    const cached = details.get(summary.task_id)
    if (cached && cached.revision === summary.revision) return cached.task
    try {
      const task = await client.getTask(summary.task_id)
      details.set(task.task_id, { revision: task.revision, task })
      return task
    } catch {
      return cached?.task ?? null
    }
  }

  return {
    async read() {
      const schemas = await loadMarked()
      username ??= fetchAccountUsername().catch((error: unknown) => { username = null; throw error })
      const user = await username
      const now = Date.now()
      const items: TrayTaskItem[] = []
      const live = new Set<string>()
      for (const [schemaId, versions] of schemas) {
        let cursor: string | undefined
        for (let page = 0; page < 3; page += 1) {
          const result = await client.listTasks({
            schema_id: schemaId,
            ...(user ? { creator_user_id: user } : {}),
            created_after: now - TASK_LOOKBACK_MS,
            limit: 200,
            cursor,
          })
          for (const summary of result.tasks ?? []) {
            const mode = versions.get(summary.schema_version)
            if (!mode) continue
            const terminal = summary.phase === TaskPhase.Terminal
            if (!terminal && mode === 'completion') continue
            const finishedAt = toMillis(summary.completed_at ?? summary.updated_at)
            if (terminal && now - finishedAt > FINISHED_TASK_TTL_MS) continue
            live.add(summary.task_id)
            const detail = terminal ? null : await detailOf(summary)
            items.push({
              taskId: summary.task_id,
              title: summary.name || summary.task_id,
              message: summary.message ?? detail?.message ?? detail?.error?.message,
              state: taskState(summary.phase, summary.outcome),
              progress: terminal ? null : toProgressPercent(detail?.progress),
              timestamp: terminal ? finishedAt : toMillis(summary.created_at),
            })
          }
          cursor = result.next_cursor
          if (!cursor) break
        }
      }
      for (const taskId of [...details.keys()]) if (!live.has(taskId)) details.delete(taskId)
      return items
    },
  }
}

async function mockTaskReader(): Promise<TaskReader> {
  const { listMockDownloads, mockDownloadState } = await import('../../api/downloads_mock')
  return {
    async read() {
      const now = Date.now()
      return listMockDownloads().flatMap((download): TrayTaskItem[] => {
        const state = mockDownloadState(download, now)
        if (state.phase === 'Terminal' && now - (state.completedAt ?? now) > FINISHED_TASK_TTL_MS) return []
        const terminal = state.phase === 'Terminal'
        return [{
          taskId: download.taskId,
          title: `Download ${download.filename}`,
          message: state.error,
          state: terminal ? taskState(TaskPhase.Terminal, state.outcome as TaskOutcome | undefined) : 'running',
          progress: terminal ? null : Math.round((state.completedBytes / download.totalBytes) * 100),
          timestamp: terminal ? state.completedAt ?? now : download.createdAt,
        }]
      })
    },
  }
}

/** Running tasks first (newest first), then finished ones by when they finished. */
function orderTasks(tasks: TrayTaskItem[]): TrayTaskItem[] {
  const running = tasks.filter((task) => task.state === 'running' || task.state === 'waiting' || task.state === 'paused')
  const finished = tasks.filter((task) => !running.includes(task))
  return [...running.sort((a, b) => b.timestamp - a.timestamp), ...finished.sort((a, b) => b.timestamp - a.timestamp)]
}

function startTasks(sink: TrayRuntimeSink) {
  let active = false
  let reader: TaskReader | null = null
  let detachMock: (() => void) | null = null
  const tasks = loop(async () => {
    try {
      if (!reader) {
        if (isMockRuntime()) {
          reader = await mockTaskReader()
          const { subscribeMockDownloads } = await import('../../api/downloads_mock')
          detachMock = subscribeMockDownloads(() => tasks.kick())
        } else {
          reader = rpcTaskReader()
        }
      }
      const items = orderTasks(await reader.read())
      active = items.some((task) => task.state === 'running')
      sink.setTasks(items)
    } catch (error) {
      console.warn('status tray: task read failed', error)
    }
  }, () => (active ? TASK_ACTIVE_POLL_MS : TASK_IDLE_POLL_MS))
  return {
    kick: tasks.kick,
    stop: () => {
      tasks.stop()
      detachMock?.()
    },
  }
}

/* ── Runtime ── */

export function startTrayRuntime(sink: TrayRuntimeSink): () => void {
  const notifies = startNotifies(sink)
  const tasks = startTasks(sink)
  const stopMessages = startMessages(sink, notifies.kick)
  const kickAll = () => {
    notifies.kick()
    tasks.kick()
  }
  const onVisible = () => { if (visible()) kickAll() }
  document.addEventListener('visibilitychange', onVisible)
  window.addEventListener('online', kickAll)
  const stopRefresh = sink.onRefreshRequest(kickAll)
  return () => {
    stopMessages()
    notifies.stop()
    tasks.stop()
    stopRefresh()
    document.removeEventListener('visibilitychange', onVisible)
    window.removeEventListener('online', kickAll)
  }
}
