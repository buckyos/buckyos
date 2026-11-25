/// <reference types="vite/client" />
import { buckyos, TaskOutcome, TaskPhase, type NotifyStatus, type NotifyView } from 'buckyos'
import { MessageHubApiStore } from '../../src/app/messagehub/api/store.ts'
import { messageRows } from '../../src/desktop/tray/messageRows.ts'
import type { ZoneChannel } from '../../src/shared/channel.ts'
import { ActivityFeed } from '../../src/shared/activity/feed.ts'
import { ConversationReader } from '../../src/shared/activity/conversations.ts'
import {
  activityPath,
  dismissTasks,
  mergeActivity,
  messagePreview,
  orderTasks,
  taskStateLabel,
  undismissedTasks,
  type ActivityMessageItem,
  type ActivityTaskItem,
  type DismissedTasks,
} from '../../src/shared/activity/model.ts'
import { notifyItem } from '../../src/shared/activity/notifies.ts'
import { SummaryReader, watchSummary } from '../../src/shared/activity/summary.ts'
import { createTaskReader } from '../../src/shared/activity/tasks.ts'
import type { ProjectionLabels } from '../../src/shared/messagehub/projection.ts'
import type { MailboxRecord, SessionSummary } from '../../src/shared/messagehub/wire.ts'

const OWNER = 'did:bns:me'
const ALICE = 'did:bns:alice'
const BOB = 'did:bns:bob'
const GROUP = 'did:bns:team'
const ZONE = 'did:bns:zone'
const AGENT = 'did:bns:jarvis.me'

function equal(actual: unknown, expected: unknown, what = '') {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw Error(`${what}\nexpected ${JSON.stringify(expected)}\nreceived ${JSON.stringify(actual)}`)
}

// The Desktop's English words, as MessageHub passes them (i18n falls back to them without a page).
const labels: ProjectionLabels = {
  direct: 'Direct Message', groupMain: 'Group chat', untitled: 'Untitled session', unassigned: 'Unassigned sessions', unassignedDescription: '',
  you: 'You', previewImage: '[Image]', previewAttachment: '[Attachment]', previewVoice: '[Voice message]', previewUnavailable: 'Message unavailable',
  systemNotify: 'System notifications', notifyCategories: { todo: 'To-do', agent: 'Agent', app: 'App notifications' },
}

// ── A Zone answering both the Shell's store and a channel ──

interface FakeSession { id: string; from: string; text: string; at: number; unread: number; kind?: 'chat' | 'group_msg'; title?: string; origin?: string; peer?: string }

function zone(sessions: FakeSession[], options: { muted?: string[]; titles?: Record<string, string>; notifies?: NotifyView[]; summary?: 'ok' | 'missing' } = {}) {
  const calls: Array<{ method: string; params: Record<string, unknown> }> = []
  const notifies = new Map((options.notifies ?? []).map((view) => [view.notify_id, view]))
  const records = new Map(sessions.map((session) => [session.id, Array.from({ length: session.unread }, (_, index) => ({ id: `${session.id}:${index}`, at: session.at - index, read: false }))]))
  const unreadOf = (session: FakeSession) => records.get(session.id)!.filter((record) => !record.read).length
  const summaryOf = (session: FakeSession): SessionSummary => {
    const group = session.kind === 'group_msg'
    const record: MailboxRecord = { record_id: `${session.id}:0`, mailbox: OWNER, owner: OWNER, box_kind: 'INBOX', msg_id: `msg:${session.id}`, state: 'UNREAD', from: session.from, to: OWNER, session_id: session.id, sort_key: session.at, created_at_ms: session.at, updated_at_ms: session.at }
    return {
      session_id: session.id, unread_count: unreadOf(session), updated_at_ms: session.at, last_activity_ms: session.at, request_count: 0, lifecycle: 'active',
      last_record: { record, msg: { from: session.from, to: [group ? GROUP : OWNER], kind: session.kind ?? 'chat', created_at_ms: session.at, content: { content: session.text } } },
      ...(session.peer ? { state: { owner: OWNER, session_id: session.id, lifecycle: 'active', registered: true, origin: session.origin, peer_did: session.peer, title: session.title, created_at_ms: 1, updated_at_ms: 1 } } : {}),
    }
  }
  const call = (method: string, params: Record<string, unknown>): unknown => {
    calls.push({ method, params })
    switch (method) {
      case 'user.get': return { local_profile: { did: OWNER } }
      case 'agent.list': return { agents: [{ agent_did: AGENT }] }
      case 'contact.list_contacts': return [
        { did: ALICE, name: 'Alice', avatar: 'data:image/png;base64,aaa', source: 'manual_create', is_verified: true, access_level: 'allow', created_at: 1, updated_at: 1 },
        { did: BOB, name: 'Bob', source: 'manual_create', is_verified: true, access_level: 'allow', created_at: 1, updated_at: 1 },
      ]
      case 'group.list_by_member': return { items: [{ obj_id: 'x', doc: { id: GROUP, owner: OWNER, controller: OWNER, host: ZONE, profile: { name: 'Team' }, revision: '1', lifecycle: 'active' } }] }
      case 'ui_session.list_state': {
        const id = String(params.session_id)
        return [
          ...(options.muted?.includes(id) ? [{ session_id: id, key: 'ui.muted', value: true, updated_at_ms: 1 }] : []),
          ...(options.titles?.[id] ? [{ session_id: id, key: 'ui.title', value: options.titles[id], updated_at_ms: 1 }] : []),
        ]
      }
      case 'msg.list_sessions': return { items: [...sessions].sort((a, b) => b.at - a.at).map(summaryOf) }
      case 'msg.list_session': return { items: records.get(String(params.session_id))!.map((record) => ({ record_id: record.id, msg_id: record.id, direction: 'in', box_kind: 'INBOX', sort_key: record.at, from: ALICE, to: OWNER, recipient_state: record.read ? 'READ' : 'UNREAD' })) }
      case 'msg.update_record_state': {
        const record = [...records.values()].flat().find((item) => item.id === params.record_id)!
        record.read = params.new_state === 'READ'
        return { record_id: record.id, state: params.new_state }
      }
      case 'notify.summary':
        if (options.summary === 'missing') throw Error('RPC error: Unknown method: notify.summary')
        return { notify: { open: 7, todo: 1, agent: 2, app: 4 }, messages: { unread: 9 } }
      case 'notify.list': {
        const wanted = params.status as NotifyStatus[] | undefined
        const [at, id] = typeof params.cursor === 'string' ? params.cursor.split('|') : []
        const matching = [...notifies.values()]
          .filter((view) => (!wanted || wanted.includes(view.status)) && (!params.kind || view.kind === params.kind))
          .sort((a, b) => b.created_at_ms - a.created_at_ms)
          .filter((view) => at === undefined || view.created_at_ms < Number(at) || (view.created_at_ms === Number(at) && view.notify_id < id))
        const items = matching.slice(0, Number(params.limit ?? 50))
        const last = items[items.length - 1]
        return { items, ...(matching.length > items.length ? { next_cursor: `${last.created_at_ms}|${last.notify_id}` } : {}) }
      }
      case 'notify.get': return notifies.get(String(params.notify_id))
      case 'notify.ack':
      case 'notify.dismiss': {
        const view = { ...notifies.get(String(params.notify_id))!, status: method === 'notify.ack' ? 'read' : 'dismissed' } as NotifyView
        notifies.set(view.notify_id, view)
        return view
      }
      case 'list_task_schemas': return { schemas: [] }
      case 'list_tasks': return { tasks: [] }
      default: throw Error(`Unexpected RPC ${method}`)
    }
  }
  const channel: ZoneChannel = {
    rpc: async <T>(_service: string, method: string, params: Record<string, unknown> = {}) => call(method, params) as T,
    fetch: () => Promise.reject(Error('no fetch')),
  }
  return { call, channel, calls, notifies, records }
}

const CHATS: FakeSession[] = [
  { id: `dm:${ALICE}`, from: ALICE, text: 'hello\nthere', at: 5_000, unread: 2 },
  { id: 'topic-1', from: ALICE, text: 'about the trip', at: 6_000, unread: 1, peer: ALICE, origin: 'manual', title: 'Trip' },
  { id: `dm:${BOB}`, from: BOB, text: 'muted', at: 7_000, unread: 4 },
  { id: GROUP, from: BOB, text: 'standup?', at: 4_000, unread: 3, kind: 'group_msg' },
  { id: `dm:${AGENT}`, from: AGENT, text: 'done', at: 3_000, unread: 0 },
  { id: 'sysnotify:app', from: ZONE, text: 'a notification', at: 9_000, unread: 5, peer: ZONE, origin: 'manual' },
]

// ── Messages ──

Deno.test('an unread conversation reads the same from msg-center alone as from the MessageHub store', async () => {
  const backend = zone(CHATS, { muted: [`dm:${BOB}`], titles: { 'topic-1': 'Trip to Lisbon' } })
  const rows = await new ConversationReader({ channel: backend.channel, userId: 'me', owner: OWNER, labels: () => labels }).read()
  equal(rows.unread, 6, 'muted and notification sessions do not count')
  equal(rows.items, [
    { sessionId: 'topic-1', entityId: ALICE, title: 'Alice · Trip to Lisbon', avatar: 'data:image/png;base64,aaa', isGroup: false, text: 'about the trip', unreadCount: 1, timestamp: 6_000 },
    { sessionId: `dm:${ALICE}`, entityId: ALICE, title: 'Alice · Direct Message', avatar: 'data:image/png;base64,aaa', isGroup: false, text: 'hello', unreadCount: 2, timestamp: 5_000 },
    { sessionId: GROUP, entityId: GROUP, title: 'Team', isGroup: true, senderName: 'Bob', text: 'standup?', unreadCount: 3, timestamp: 4_000 },
  ])

  const saved = { getCurrentIdentity: buckyos.getCurrentIdentity, getAccountInfo: buckyos.getAccountInfo, getServiceRpcClient: buckyos.getServiceRpcClient }
  buckyos.getCurrentIdentity = async () => ({ kind: 'anonymous' })
  buckyos.getAccountInfo = async () => ({ user_id: 'me', user_name: 'Me', session_token: 'test-only' })
  buckyos.getServiceRpcClient = (() => ({ call: async (method: string, params: Record<string, unknown>) => backend.call(method, params) })) as unknown as typeof buckyos.getServiceRpcClient
  try {
    const store = new MessageHubApiStore()
    await store.initialize()
    const context = store.defaultContext()
    await store.loadPreferences(context, CHATS.map((session) => session.id))
    equal(messageRows(store, context), rows, 'the Shell tray')
  } finally {
    Object.assign(buckyos, saved)
  }
})

Deno.test('names, groups and session settings are reused for a while; a conversation is marked read up to what was listed', async () => {
  const backend = zone(CHATS)
  let now = 0
  const reader = new ConversationReader({ channel: backend.channel, userId: 'me', owner: OWNER, labels: () => labels, now: () => now })
  await reader.read()
  await reader.read()
  const count = (method: string) => backend.calls.filter((call) => call.method === method).length
  equal([count('contact.list_contacts'), count('ui_session.list_state')], [1, 4])
  now = 60_000
  await reader.read()
  equal([count('contact.list_contacts'), count('ui_session.list_state')], [2, 8])

  backend.records.get(`dm:${ALICE}`)!.push({ id: `dm:${ALICE}:new`, at: 8_000, read: false })
  await reader.markRead(`dm:${ALICE}`)
  equal(backend.records.get(`dm:${ALICE}`)!.map((record) => record.read), [true, true, false], 'a message that arrived since stays unread')
})

// ── Rows and labels ──

const task = (taskId: string, state: ActivityTaskItem['state'], timestamp: number): ActivityTaskItem => ({ taskId, title: taskId, state, progress: null, timestamp })
const message = (sessionId: string, timestamp: number): ActivityMessageItem => ({ sessionId, entityId: ALICE, title: 'Alice', isGroup: false, text: 'hi', unreadCount: 1, timestamp })

function notice(id: string, at: number, patch: Partial<NotifyView> = {}): NotifyView {
  return { notify_id: id, url: `/messagehub?notify=${id}`, to_user: 'me', kind: 'notice', category: 'app', level: 'normal', status: 'open', prompt: { title: id, controls: [] }, source: {}, created_at_ms: at, ...patch }
}

Deno.test('rows of the three sources merge newest first and lead to their pages', () => {
  const entries = mergeActivity({ messages: [message('dm:a', 30)], notifies: [notifyItem(notice('n 1', 50)), notifyItem(notice('n2', 10))], tasks: [task('t1', 'running', 40)] })
  equal(entries.map((entry) => entry.key), ['notify-n 1', 'task-t1', 'message-dm:a', 'notify-n2'])
  equal(entries.map(activityPath), ['/messagehub?notify=n%201', '/taskcenter?taskid=t1', '/messagehub?entityId=did%3Abns%3Aalice&sessionId=dm%3Aa', '/messagehub?notify=n2'])
})

Deno.test('a notification row is read from untrusted data', () => {
  equal(notifyItem(notice('n', 1, { prompt: { title: ' Title ', message: ' Body ', controls: [] }, source: { app_id: 'notes', link: 'https://notes.zone/x' } })), {
    notifyId: 'n', title: 'Title', message: 'Body', category: 'app', kind: 'notice', level: 'normal', status: 'open', needsAnswer: false, appId: 'notes', link: 'https://notes.zone/x', timestamp: 1,
  })
  equal(notifyItem(notice('n', 1, { source: { link: 'javascript:alert(1)' } })).link, undefined, 'not a page')
  equal(notifyItem(notice('n', 1, { source: { link: '//evil.example' } })).link, undefined, 'not a path of the Zone')
  equal(notifyItem(notice('n', 1, { kind: 'input' })).needsAnswer, true)
  equal(notifyItem(notice('n', 1, { kind: 'input', status: 'answered' })).needsAnswer, false)
})

Deno.test('labels: what a task or a conversation row says', () => {
  equal(taskStateLabel({ state: 'failed', message: 'disk full' }), { key: 'tray.task.failedWith', text: 'Failed: {{detail}}', values: { detail: 'disk full' } })
  equal(taskStateLabel({ state: 'running', message: 'half way' }), 'half way')
  equal(messagePreview({ text: '', senderName: 'Bob', unreadCount: 1 }), { sender: 'Bob', text: { key: 'tray.messages.noPreview', text: 'New message' } })
  equal(messagePreview({ text: 'secret', senderName: 'Bob', unreadCount: 3 }, false), { text: { key: 'tray.messages.newCount', text: '{{count}} new messages', values: { count: 3 } } })
})

Deno.test('finished tasks can be dismissed; running ones stay whatever is stored', () => {
  const tasks = [task('done', 'completed', 10), task('busy', 'running', 20), task('old', 'failed', 5)]
  equal(orderTasks(tasks).map((item) => item.taskId), ['busy', 'done', 'old'])
  const dismissed = dismissTasks({ forgotten: 0 }, ['done', 'busy'], 11 * 86_400_000)
  equal(Object.keys(dismissed), ['done', 'busy'], 'an old dismissal is forgotten')
  equal(undismissedTasks(tasks, dismissed).map((item) => item.taskId), ['busy', 'old'])
})

// ── Tasks ──

Deno.test('tasks: marked schemas only, completion-only ones once finished, within the window', async () => {
  const now = 1_790_000_000_000
  const base = { schema_version: 1, created_at: now - 1000, updated_at: now - 500, revision: 1 }
  const tasks: Record<string, unknown[]> = {
    download: [
      { ...base, task_id: 'running', name: 'Download', phase: TaskPhase.Running },
      { ...base, task_id: 'failed', name: 'Broken', phase: TaskPhase.Terminal, outcome: TaskOutcome.Failed, completed_at: now - 100, message: 'no space' },
      { ...base, task_id: 'stale', name: 'Stale', phase: TaskPhase.Terminal, outcome: TaskOutcome.Succeeded, completed_at: now - 4 * 86_400_000 },
      { ...base, task_id: 'v2', schema_version: 2, name: 'Unmarked version', phase: TaskPhase.Running },
    ],
    export: [{ ...base, task_id: 'exporting', phase: TaskPhase.Running }],
  }
  const asked: unknown[] = []
  const reader = createTaskReader({
    client: {
      listTaskSchemas: async () => [
        { schema_id: 'download', schema_version: 1, presentation_schema: { system_notify: 'progress' } },
        { schema_id: 'export', schema_version: 1, presentation_schema: { system_notify: 'completion' } },
        { schema_id: 'quiet', schema_version: 1, presentation_schema: {} },
      ] as never,
      listTasks: async (request: { schema_id?: string }) => { asked.push(request); return { tasks: tasks[request.schema_id!] ?? [] } as never },
      getTask: async (taskId: string) => ({ task_id: taskId, revision: 1, progress: { completed: 1, total: 4 }, message: 'downloading' }) as never,
    },
    username: async () => 'me',
    now: () => now,
  })
  equal(await reader.read(), [
    { taskId: 'running', title: 'Download', message: 'downloading', state: 'running', progress: 25, timestamp: now - 1000 },
    { taskId: 'failed', title: 'Broken', message: 'no space', state: 'failed', progress: null, timestamp: now - 100 },
  ])
  equal(asked.map((request) => (request as { schema_id: string; creator_user_id: string }).schema_id), ['download', 'export'])
})

// ── Counts ──

Deno.test('a Zone without notify.summary is asked once; other failures are not taken for that', async () => {
  let failure: Error | null = Error('RPC error: Unknown method: notify.summary')
  let calls = 0
  const channel: ZoneChannel = { rpc: async () => { calls += 1; if (failure) throw failure; return { notify: { open: 2, todo: 0, agent: 1, app: 1 }, messages: { unread: 3 } } as never }, fetch: () => Promise.reject(Error()) }
  const old = new SummaryReader(channel)
  equal([await old.read(), await old.read(), calls], [null, null, 1])
  failure = Error('network down')
  const current = new SummaryReader(channel)
  let thrown = ''
  try { await current.read() } catch (error) { thrown = (error as Error).message }
  equal(thrown, 'network down')
  failure = null
  equal(await current.read(), { notify: { open: 2, todo: 0, agent: 1, app: 1 }, messages: { unread: 3 } })
})

Deno.test('the tray counts follow the changes it hears of, once per burst, and survive a failed read', async () => {
  let unread = 3
  let calls = 0
  let failing = false
  const channel: ZoneChannel = {
    rpc: async () => { calls += 1; if (failing) throw Error('network down'); return { notify: { open: 0, todo: 0, agent: 0, app: 0 }, messages: { unread } } as never },
    fetch: () => Promise.reject(Error()),
  }
  let changes = 0
  const watch = watchSummary(channel, () => { changes += 1 }, 1)
  const settle = () => new Promise((resolve) => setTimeout(resolve, 10))
  equal(watch.current(), null)
  watch.kick(); watch.kick(); watch.kick()
  await settle()
  equal([calls, changes, watch.current()?.messages.unread], [1, 1, 3])
  watch.kick(); await settle()
  equal([calls, changes], [2, 1], 'the same counts are no change')
  failing = true
  watch.kick(); await settle()
  equal([changes, watch.current()?.messages.unread], [1, 3])
  failing = false; unread = 0
  watch.kick(); await settle()
  equal([changes, watch.current()?.messages.unread], [2, 0])
  watch.kick(); watch.stop(); await settle()
  equal(calls, 4)
})

// ── The feed ──

function feed(backend: ReturnType<typeof zone>, options: { pageSize?: number } = {}) {
  let dismissed: DismissedTasks = {}
  return new ActivityFeed({ channel: backend.channel, userId: 'me', labels: () => labels, dismissed: { read: () => dismissed, write: (next) => { dismissed = next } }, pageSize: options.pageSize })
}
const keys = (activity: ActivityFeed) => activity.getSnapshot().entries.map((entry) => entry.key)

Deno.test('feed: rows older than the last loaded notification wait for the next page', async () => {
  const backend = zone([CHATS[0]], { notifies: [notice('a', 9_000), notice('b', 8_000, { status: 'read' }), notice('c', 2_000), notice('d', 1_000)] })
  const activity = feed(backend, { pageSize: 2 })
  await activity.refresh()
  equal(keys(activity), ['notify-a', 'notify-b'], 'the conversation of 5000 is older than what is loaded')
  equal(activity.getSnapshot().hasMore, true)
  equal(activity.getSnapshot().status, { message: 'ready', notify: 'ready', task: 'ready' })
  await activity.loadMore()
  equal(keys(activity), ['notify-a', 'notify-b', `message-dm:${ALICE}`, 'notify-c', 'notify-d'])
  equal(activity.getSnapshot().hasMore, false)
  equal(activity.getSnapshot().summary, { notify: { open: 7, todo: 1, agent: 2, app: 4 }, messages: { unread: 9 } })
})

Deno.test('feed: a refresh adds what is new, keeps the loaded pages and follows what became of the open ones', async () => {
  const backend = zone([], { notifies: [notice('a', 5), notice('b', 4), notice('c', 3), notice('d', 2), notice('e', 1)] })
  const activity = feed(backend, { pageSize: 2 })
  await activity.refresh('notify')
  await activity.loadMore()
  equal(keys(activity), ['notify-a', 'notify-b', 'notify-c', 'notify-d'])
  backend.notifies.set('new', notice('new', 6))
  backend.notifies.set('d', notice('d', 2, { status: 'dismissed' }))
  await activity.refresh('notify')
  equal(keys(activity), ['notify-new', 'notify-a', 'notify-b', 'notify-c', 'notify-d'])
  const last = activity.getSnapshot().entries[4]
  equal(last.kind === 'notify' && last.item.status, 'dismissed')
  await activity.loadMore()
  equal(keys(activity).slice(-1), ['notify-e'], 'paging goes on where it was')

  for (let index = 0; index < 20; index++) backend.notifies.set(`burst-${index}`, notice(`burst-${index}`, 100 + index))
  await activity.refresh('notify')
  equal([keys(activity).length, keys(activity)[0], activity.getSnapshot().hasMore], [8, 'notify-burst-19', true], 'too much arrived: the list starts over')
})

Deno.test('feed: without notify.summary it counts what is loaded', async () => {
  const backend = zone(CHATS, { summary: 'missing', notifies: [notice('a', 2, { category: 'todo' }), notice('b', 1), notice('c', 3, { status: 'read' })] })
  const activity = feed(backend)
  await activity.refresh()
  equal(activity.getSnapshot().summary, { notify: { open: 2, todo: 1, agent: 0, app: 1 }, messages: { unread: 10 } })
})

Deno.test('feed: one row or everything can be marked read; what needs an answer stays', async () => {
  const backend = zone(CHATS, { muted: [`dm:${BOB}`], notifies: [notice('a', 50_000), notice('b', 40_000), notice('ask', 30_000, { kind: 'input', task_id: 't' })] })
  const activity = feed(backend)
  await activity.refresh()
  await activity.ackNotify('a')
  const statusOf = (id: string) => activity.getSnapshot().entries.flatMap((entry) => (entry.kind === 'notify' && entry.item.notifyId === id ? [entry.item.status] : []))[0]
  equal([statusOf('a'), statusOf('b')], ['read', 'open'])
  await activity.dismissNotify('b')
  equal(statusOf('b'), 'dismissed')
  await activity.markSessionRead('topic-1')
  equal(keys(activity).includes('message-topic-1'), false)

  backend.notifies.set('late', notice('late', 60_000))
  await activity.markAllRead()
  equal([statusOf('late'), statusOf('ask')], ['read', 'open'])
  equal(keys(activity).filter((key) => key.startsWith('message-')), [])
  equal(backend.records.get(`dm:${BOB}`)!.some((record) => record.read), false, 'a muted conversation is not listed and not touched')
})

Deno.test('feed: one unreachable source does not hide the others', async () => {
  const backend = zone([], { notifies: [notice('a', 1)] })
  const rpc = backend.channel.rpc
  backend.channel.rpc = (service, method, params) => method === 'msg.list_sessions' ? Promise.reject(Error('down')) : rpc(service, method, params)
  const activity = feed(backend)
  const warn = console.warn
  console.warn = () => undefined
  try { await activity.refresh() } finally { console.warn = warn }
  equal(activity.getSnapshot().status, { message: 'error', notify: 'ready', task: 'ready' })
  equal(keys(activity), ['notify-a'])
})
