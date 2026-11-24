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
import { buckyos, TaskPhase, type TaskOutcome } from 'buckyos'
import { getMessageHubStore, isMessageHubMock } from '../../app/messagehub/store'
import { getNotifyWatch } from '../../app/messagehub/notify'
import { fetchAccountUsername } from '../../app/messagehub/datamodel/sessionApi'
import type { MessageHubContext } from '../../app/messagehub/types'
import { shellChannel } from '../../api/channel'
import { isMockRuntime } from '../../runtime'
import { loop } from '../../shared/activity/loop.ts'
import type { UnreadConversations } from '../../shared/activity/messages.ts'
import { orderTasks, type ActivityNotifyItem, type ActivityTaskItem } from '../../shared/activity/model.ts'
import { notifyItem } from '../../shared/activity/notifies.ts'
import { watchSummary, type SummaryWatch } from '../../shared/activity/summary.ts'
import { createTaskReader, taskState, TRAY_TASK_WINDOW, type TaskReader } from '../../shared/activity/tasks.ts'
import { messageRows } from './messageRows'
import type { TrayRuntimeSink } from './trayModel'

const MAX_MESSAGE_ROWS = 30
const MAX_NOTIFY_ROWS = 50
const NOTIFY_POLL_MS = 30_000
/** Task reads while one is running (progress) / while none is. */
const TASK_ACTIVE_POLL_MS = 2_000
const TASK_IDLE_POLL_MS = 20_000

function visible(): boolean {
  return typeof document === 'undefined' || document.visibilityState !== 'hidden'
}

/* ── Counts ── */

/** The mock runtime counts what is listed. */
function startSummary(onChange: () => void): SummaryWatch {
  if (isMessageHubMock()) return { current: () => null, kick: () => undefined, stop: () => undefined }
  return watchSummary(shellChannel, onChange)
}

/* ── Messages ── */

/** The system notification sessions changed: something arrived or was read. */
function notifySignature(context: MessageHubContext): string {
  const store = getMessageHubStore()
  return store
    .sessions(context)
    .filter((session) => store.findEntity(context, session.entityId)?.systemNotify)
    .map((session) => `${session.id}:${session.lastActiveAt}:${session.unreadCount}`)
    .join('|')
}

function startMessages(sink: TrayRuntimeSink, summary: SummaryWatch, onNotifySessionsChanged: () => void) {
  const store = getMessageHubStore()
  let stopped = false
  let stopSync: (() => void) | null = null
  let unsubscribe: (() => void) | null = null
  let queued = false
  let lastSignature = ''
  let lastUnread = ''
  let rows: UnreadConversations | null = null

  const show = () => {
    if (!rows || stopped) return
    const { unread, items } = rows
    sink.setMessages({ status: 'ready', unread: summary.current()?.messages.unread ?? unread, items: items.slice(0, MAX_MESSAGE_ROWS) })
  }
  const publish = (context: MessageHubContext) => {
    queued = false
    if (stopped) return
    rows = messageRows(store, context)
    show()
    const unreadRows = rows.items.map((item) => `${item.sessionId}:${item.unreadCount}`).join('|')
    if (unreadRows !== lastUnread) {
      lastUnread = unreadRows
      summary.kick()
    }
    // Muted state lives in per-session preferences, loaded on demand.
    void store.loadPreferences(context, store.sessions(context).filter((session) => session.unreadCount > 0).map((session) => session.id)).catch(() => undefined)
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

  return {
    show,
    stop: () => {
      stopped = true
      stopSync?.()
      unsubscribe?.()
    },
  }
}

/* ── Notifications ── */

function startNotifies(sink: TrayRuntimeSink, summary: SummaryWatch) {
  const watch = getNotifyWatch()
  let items: ActivityNotifyItem[] = []
  let status: 'loading' | 'ready' | 'error' = 'loading'
  const show = () => {
    if (status !== 'loading') sink.setNotifies(status, items, summary.current()?.notify.open ?? items.length)
  }
  const reader = loop(async () => {
    summary.kick()
    try {
      const views = await watch.list({ status: ['open'], limit: MAX_NOTIFY_ROWS })
      items = views.map(notifyItem).sort((a, b) => b.timestamp - a.timestamp)
      status = 'ready'
    } catch (error) {
      console.warn('status tray: notify.list failed', error)
      status = items.length ? 'ready' : 'error'
    }
    show()
  }, () => NOTIFY_POLL_MS, visible)
  // Answered / acknowledged in MessageHub: read the list again.
  let pending: ReturnType<typeof setTimeout> | null = null
  const unsubscribe = watch.subscribe(() => {
    if (pending) return
    pending = setTimeout(() => { pending = null; reader.kick() }, 300)
  })
  return {
    kick: reader.kick,
    show,
    stop: () => {
      reader.stop()
      unsubscribe()
      if (pending) clearTimeout(pending)
    },
  }
}

/* ── Tasks ── */

async function mockTaskReader(): Promise<TaskReader> {
  const { listMockDownloads, mockDownloadState } = await import('../../api/downloads_mock')
  return {
    async read() {
      const now = Date.now()
      return listMockDownloads().flatMap((download): ActivityTaskItem[] => {
        const state = mockDownloadState(download, now)
        if (state.phase === 'Terminal' && now - (state.completedAt ?? now) > TRAY_TASK_WINDOW.finishedTtlMs) return []
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
          reader = createTaskReader({ client: buckyos.getTaskManagerClient(), username: fetchAccountUsername })
        }
      }
      const items = orderTasks(await reader.read())
      active = items.some((task) => task.state === 'running')
      sink.setTasks(items)
    } catch (error) {
      console.warn('status tray: task read failed', error)
    }
  }, () => (active ? TASK_ACTIVE_POLL_MS : TASK_IDLE_POLL_MS), visible)
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
  const summary = startSummary(() => {
    notifies.show()
    messages.show()
  })
  const notifies = startNotifies(sink, summary)
  const tasks = startTasks(sink)
  const messages = startMessages(sink, summary, notifies.kick)
  const kickAll = () => {
    notifies.kick()
    tasks.kick()
  }
  const onVisible = () => { if (visible()) kickAll() }
  document.addEventListener('visibilitychange', onVisible)
  window.addEventListener('online', kickAll)
  const stopRefresh = sink.onRefreshRequest(kickAll)
  return () => {
    messages.stop()
    notifies.stop()
    tasks.stop()
    summary.stop()
    stopRefresh()
    document.removeEventListener('visibilitychange', onVisible)
    window.removeEventListener('online', kickAll)
  }
}
