/**
 * System Notify in mock mode: the notifications of the three category
 * sessions and the copies in a CodeAssistant conversation, plus a scripted
 * msg-center / TaskMgr. The notification and task states live in
 * localStorage, so two tabs of one browser share them like two devices of
 * one user: a change in one tab reaches the other as a `storage` event, the
 * way a TaskMgr kevent would.
 */
import { NOTIFY_CHOICE_CONTROL_ID, NOTIFY_INTENT, NOTIFY_TEXT_CONTROL_ID, notifySessionId, type CanonValue, type HumanTaskPrompt, type MessageObject, type NotifyCategory, type NotifyKind, type NotifyLevel, type NotifyMessageData, type NotifySource, type NotifyStatus } from '../protocol/msgobj'
import { isNotifyClosed, NOTIFY_ACTION_DISMISS, TASK_ERR_ALREADY_COMPLETED, TASK_ERR_REVISION_CONFLICT, type NotifyAnswer, type NotifyResult, type NotifyView } from '../notify/model'
import type { NotifyBackend, NotifyTaskState } from '../notify/watch'
import { sessionKey } from '../sessionModel'
import type { Entity, Session } from '../types'
import { getMockEntityDid, MOCK_SELF_DID } from './data'

/** The system notify sender of the mock zone (its Zone DID). */
export const MOCK_NOTIFY_SENDER = 'did:web:zone.buckyos.mock'
/** The mock viewer as TaskMgr names a submitter. */
export const MOCK_SELF_USER = 'self'
/** The CodeAssistant conversation that holds copies of its notifications. */
export const MOCK_NOTIFY_AGENT_SESSION = 'session-coder-notify'
/** Ids of the scripted notifications, for tests and the Task Center mock. */
export const MOCK_NOTIFY = {
  todo: 'nt-todo-budget',
  todoDone: 'nt-todo-contract',
  work: 'nt-agent-work',
  goal: 'nt-agent-goal',
  reminder: 'nt-agent-reminder',
  answered: 'nt-agent-tests',
  timedOut: 'nt-agent-timeout',
  app: 'nt-app-backup',
  appInput: 'nt-app-sync',
  dismissed: 'nt-app-disk',
  expired: 'nt-app-import',
} as const

const CODER = getMockEntityDid('agent-coder')
const CODER_APP = 'codeassistant'
const STORAGE_KEY = 'buckyos.messagehub.mock-notify.v1'
const MINUTE = 60_000

interface MockTask {
  taskId: string
  revision: number
  phase: 'Waiting' | 'Terminal'
  outcome?: 'Succeeded' | 'Canceled'
  result?: { kind: 'human'; action?: string; values?: Record<string, unknown>; submitted_by?: string; submitted_at?: number }
  committedBy?: string
  completedAt?: number
}

interface MockNotify {
  view: Omit<NotifyView, 'status' | 'result'>
  /** The state of a `notice` (its records); an `input` reads its task. */
  notice?: NotifyStatus
  task?: MockTask
}

interface Seed {
  id: string
  kind: NotifyKind
  category: NotifyCategory
  level?: NotifyLevel
  title: string
  message?: string
  options?: Array<[string, string]>
  /** Takes free text (alone when there are no options). */
  text?: boolean
  source: NotifySource
  minutesAgo: number
  /** Copy in the CodeAssistant conversation (an Agent notification delivered there). */
  copy?: boolean
  notice?: NotifyStatus
  /** How the task of an `input` ended; open when absent. */
  done?: { answer?: NotifyAnswer; by?: string; via?: string; canceled?: string; minutesAgo: number }
  expiresMinutesAgo?: number
}

const agent = (extra: NotifySource): NotifySource => ({ app_id: CODER_APP, agent_did: CODER, ...extra })
const conversation = { to: MOCK_SELF_DID, to_session: MOCK_NOTIFY_AGENT_SESSION }

const SEEDS: Seed[] = [
  { id: MOCK_NOTIFY.todo, kind: 'input', category: 'todo', title: 'Review the Q4 budget draft', message: 'Finance asked you to review the budget draft before Friday.', options: [['approve', 'Looks good'], ['changes', 'Needs changes']], text: true, source: { app_id: 'workflow', operation: 'Quarterly budget review', link: '/taskcenter' }, minutesAgo: 50 },
  { id: MOCK_NOTIFY.todoDone, kind: 'input', category: 'todo', title: 'Sign off the vendor contract', message: 'Legal needs your sign-off on the renewed hosting contract.', options: [['signed', 'Signed'], ['later', 'Later']], source: { app_id: 'workflow', operation: 'Vendor renewal' }, minutesAgo: 30 * 60, done: { answer: { option: 'signed' }, by: MOCK_SELF_USER, minutesAgo: 26 * 60 } },
  { id: MOCK_NOTIFY.timedOut, kind: 'input', category: 'agent', title: 'Should I keep the old API docs?', options: [['keep', 'Keep them'], ['remove', 'Remove them']], source: agent({ session_id: 'work-api-docs', turn: 2, call_id: 'call-ask-2', operation: 'Clean up the API docs' }), minutesAgo: 52 * 60, done: { canceled: 'ask_timeout', minutesAgo: 49 * 60 } },
  { id: MOCK_NOTIFY.answered, kind: 'input', category: 'agent', title: 'Which test suite should run before the release?', options: [['smoke', 'Smoke tests'], ['full', 'Full suite']], source: agent({ session_id: 'work-release', turn: 6, call_id: 'call-ask-5', operation: 'Prepare release 2.4' }), minutesAgo: 27 * 60, done: { answer: { option: 'full' }, by: MOCK_SELF_USER, via: CODER_APP, minutesAgo: 26 * 60 } },
  { id: MOCK_NOTIFY.goal, kind: 'input', category: 'agent', title: 'Which day should the weekly report go out?', message: 'I am setting up the weekly project report and need to know when you read it.', text: true, source: agent({ goal_id: 'goal-weekly-report', operation: 'Weekly project report' }), minutesAgo: 70 },
  { id: MOCK_NOTIFY.reminder, kind: 'notice', category: 'agent', title: 'Reminder: dentist appointment tomorrow at 9:00', message: 'You asked me to remind you a day ahead.', source: agent({ goal_id: 'goal-reminders', operation: 'Personal reminders', conversation }), minutesAgo: 40, copy: true, notice: 'open' },
  { id: MOCK_NOTIFY.work, kind: 'input', category: 'agent', level: 'timely', title: 'Which branch should I deploy the docs preview from?', message: 'The preview build is ready. main has the latest docs; release-2.4 matches the shipped version.', options: [['main', 'main'], ['release', 'release-2.4']], source: agent({ session_id: 'work-docs-deploy', turn: 4, call_id: 'call-ask-7', operation: 'Deploy the docs preview', conversation }), minutesAgo: 25, copy: true },
  { id: MOCK_NOTIFY.expired, kind: 'input', category: 'app', title: 'Import 214 photos from the camera?', options: [['import', 'Import'], ['skip', 'Skip']], source: { app_id: 'photos', operation: 'Camera import' }, minutesAgo: 72 * 60, expiresMinutesAgo: 48 * 60, done: { canceled: 'expired', minutesAgo: 48 * 60 } },
  { id: MOCK_NOTIFY.dismissed, kind: 'notice', category: 'app', level: 'urgent', title: 'Disk space is low', message: 'Volume /data has 8% free space left.', source: { app_id: 'storage-monitor', operation: 'Storage health' }, minutesAgo: 24 * 60, notice: 'dismissed' },
  { id: MOCK_NOTIFY.appInput, kind: 'input', category: 'app', title: 'Sync conflict: report.docx', message: 'report.docx changed on this device and on your laptop.', options: [['both', 'Keep both'], ['laptop', 'Use the laptop version']], text: true, source: { app_id: 'sync', operation: 'Folder sync' }, minutesAgo: 35 },
  { id: MOCK_NOTIFY.app, kind: 'notice', category: 'app', title: 'Backup finished', message: 'Tonight’s backup of 12,408 files finished in 14 minutes.', source: { app_id: 'backup', operation: 'Nightly backup', link: 'https://www.buckyos.ai/' }, minutesAgo: 15, notice: 'open' },
]

/** Records the user has not read yet: where the unread of each notification landed (`System Notify.md` §3). */
const UNREAD_HOME = new Set<string>([MOCK_NOTIFY.todo, MOCK_NOTIFY.goal, MOCK_NOTIFY.appInput, MOCK_NOTIFY.app])

function prompt(seed: Seed): HumanTaskPrompt {
  const controls: NonNullable<HumanTaskPrompt['controls']> = []
  if (seed.options?.length) controls.push({ kind: 'choice', id: NOTIFY_CHOICE_CONTROL_ID, options: seed.options.map(([value, label]) => ({ value, label })), multiple: false, required: !seed.text })
  if (seed.kind === 'input' && (!seed.options?.length || seed.text)) controls.push({ kind: 'text_input', id: NOTIFY_TEXT_CONTROL_ID, multiline: true, required: !seed.options?.length })
  return { title: seed.title, ...(seed.message ? { message: seed.message } : {}), controls }
}

const homeId = (id: string) => `msg-${id}-home`
const copyId = (id: string) => `msg-${id}-copy`

function seedNotify(seed: Seed, now: number): MockNotify {
  const at = now - seed.minutesAgo * MINUTE
  const category = notifySessionId(seed.category)
  const view: MockNotify['view'] = {
    notify_id: seed.id,
    url: `/messagehub?notify=${seed.id}`,
    to_user: MOCK_SELF_USER,
    kind: seed.kind,
    category: seed.category,
    level: seed.level ?? 'normal',
    prompt: prompt(seed),
    source: seed.source,
    created_at_ms: at,
    ...(seed.expiresMinutesAgo !== undefined ? { expires_at_ms: now - seed.expiresMinutesAgo * MINUTE } : {}),
    ...(seed.kind === 'input' ? { task_id: `task-${seed.id}` } : {}),
    home: { owner: MOCK_SELF_DID, session_id: category, record_id: `rec-${seed.id}-home`, msg_id: homeId(seed.id), from: MOCK_NOTIFY_SENDER },
    ...(seed.copy ? { deliveries: [{ session_id: 'ui-session-1', platform: 'native', to: MOCK_SELF_DID, to_session: MOCK_NOTIFY_AGENT_SESSION, msg_id: copyId(seed.id), ok: true }] } : {}),
  }
  if (seed.kind === 'notice') {
    const closedAt = seed.notice && isNotifyClosed(seed.notice) ? { closed_at_ms: at + 5 * MINUTE } : {}
    return { view: { ...view, ...closedAt }, notice: seed.notice ?? 'open' }
  }
  const done = seed.done
  const task: MockTask = { taskId: `task-${seed.id}`, revision: 1, phase: 'Waiting' }
  if (done) {
    const completedAt = now - done.minutesAgo * MINUTE
    Object.assign(task, { revision: 2, phase: 'Terminal', completedAt })
    if (done.canceled) {
      task.outcome = 'Canceled'
      Object.assign(view, { closed_at_ms: completedAt, closed_reason: done.canceled })
    } else if (done.answer) {
      task.outcome = 'Succeeded'
      task.result = { kind: 'human', action: done.answer.dismiss ? NOTIFY_ACTION_DISMISS : 'answer', values: { ...(done.answer.option ? { option: done.answer.option } : {}), ...(done.answer.text ? { text: done.answer.text } : {}) }, submitted_by: done.by, submitted_at: completedAt }
      task.committedBy = `${done.by}|${done.via ?? 'control-panel'}`
    }
  }
  return { view, task }
}

function seedState(now: number): Record<string, MockNotify> {
  return Object.fromEntries(SEEDS.map(seed => [seed.id, seedNotify(seed, now)]))
}

function plainText(view: MockNotify['view']): string {
  const options = view.prompt.controls?.flatMap(control => control.kind === 'choice' ? control.options ?? [] : []) ?? []
  return [view.prompt.title, view.prompt.message, ...options.map(option => `- ${option.label}`), `处理：${view.url}`].filter(Boolean).join('\n')
}

function message(notify: MockNotify, home: boolean, now: number): MessageObject {
  const { view } = notify
  const data: NotifyMessageData = {
    version: 1, notify_id: view.notify_id, kind: view.kind, category: view.category, level: view.level, home, source: view.source, prompt: view.prompt,
    ...(view.task_id ? { task_id: view.task_id } : {}), url: view.url, ...(view.expires_at_ms ? { expires_at_ms: view.expires_at_ms } : {}),
  }
  const sessionId = home ? view.home!.session_id : MOCK_NOTIFY_AGENT_SESSION
  return {
    from: home ? MOCK_NOTIFY_SENDER : CODER,
    to: [MOCK_SELF_DID],
    to_session: sessionId,
    kind: 'notify',
    created_at_ms: Math.min(view.created_at_ms, now),
    content: { title: view.prompt.title, format: 'text/plain', content: plainText(view), machine: { intent: NOTIFY_INTENT, data: structuredClone(data) as unknown as Record<string, CanonValue> } },
    ui_message_id: home ? homeId(view.notify_id) : copyId(view.notify_id),
    ui_session_id: sessionId,
    ...(home ? {} : { ui_sender_name: 'CodeAssistant' }),
  }
}

function chat(id: string, from: string, content: string, at: number): MessageObject {
  return { from, to: [from === MOCK_SELF_DID ? CODER : MOCK_SELF_DID], kind: 'chat', created_at_ms: at, content: { format: 'text/plain', content }, ui_message_id: id, ui_session_id: MOCK_NOTIFY_AGENT_SESSION, ui_sender_name: from === MOCK_SELF_DID ? 'You' : 'CodeAssistant', ...(from === MOCK_SELF_DID ? { ui_delivery_status: 'read' } : {}) }
}

/** The mailbox side of the scripted notifications: records by session key, oldest first. */
export function mockNotifyMessages(now: number): Record<string, MessageObject[]> {
  const state = seedState(now)
  const result: Record<string, MessageObject[]> = {}
  const push = (sessionId: string, item: MessageObject) => { (result[sessionKey(MOCK_SELF_DID, sessionId)] ??= []).push(item) }
  push(MOCK_NOTIFY_AGENT_SESSION, chat('msg-nt-chat-1', MOCK_SELF_DID, 'Deploy the docs preview when the build is ready.', now - 46 * MINUTE))
  push(MOCK_NOTIFY_AGENT_SESSION, chat('msg-nt-chat-2', CODER, 'On it. I will ask you before picking the branch.', now - 45 * MINUTE))
  for (const seed of SEEDS) {
    const notify = state[seed.id]
    push(notify.view.home!.session_id, message(notify, true, now))
    if (seed.copy) push(MOCK_NOTIFY_AGENT_SESSION, message(notify, false, now))
  }
  for (const list of Object.values(result)) list.sort((a, b) => a.created_at_ms - b.created_at_ms)
  return result
}

/** The three category sessions and the CodeAssistant conversation with the copies. */
export function mockNotifySessions(now: number): Session[] {
  const messages = mockNotifyMessages(now)
  const titles: Record<NotifyCategory, string> = { todo: 'To-do', agent: 'Agent', app: 'App notifications' }
  const session = (id: string, entityId: string, title: string, unreadCount: number): Session => {
    const list = messages[sessionKey(MOCK_SELF_DID, id)] ?? []
    const last = list[list.length - 1]
    const lastText = last?.kind === 'notify' ? last.content.title ?? '' : last?.content.content ?? ''
    return {
      id, ownerDid: MOCK_SELF_DID, entityId, binding: { kind: 'native', targetDid: entityId }, origin: 'manual', lifecycle: 'active', createdAt: now - 30 * 86400_000,
      shared: { title: '', description: '', updatedAt: now - 30 * 86400_000 }, members: entityId === CODER ? { [MOCK_SELF_DID]: { nickname: 'Me', updatedAt: now }, [CODER]: { nickname: 'CodeAssistant', updatedAt: now } } : {},
      ...(last ? { lastMessage: { text: lastText, timestamp: last.created_at_ms } } : {}),
      title, type: 'chat', source: 'buckyos', lastActiveAt: last?.created_at_ms ?? now - 30 * 86400_000, unreadCount,
    }
  }
  const unread = (category: NotifyCategory) => SEEDS.filter(seed => seed.category === category && UNREAD_HOME.has(seed.id)).length
  return [
    ...(['todo', 'agent', 'app'] as const).map(category => session(notifySessionId(category), MOCK_NOTIFY_SENDER, titles[category], unread(category))),
    session(MOCK_NOTIFY_AGENT_SESSION, CODER, 'Docs Preview', SEEDS.filter(seed => seed.copy).length),
  ]
}

/** The entity of the system notify sender; `name` is the viewer's language. */
export function mockNotifyEntity(name: string): Entity {
  return { id: MOCK_NOTIFY_SENDER, type: 'service', name, systemNotify: true, tags: [], unreadCount: 0, lastActiveAt: 0, source: 'buckyos', sources: ['buckyos'], domain: 'managed' }
}

function taskError(code: string) {
  return Object.assign(Error(code), { code })
}

/**
 * Scripted msg-center `notify.*` + TaskMgr for the cards. Reads go through
 * localStorage every time, so a tab never trusts its own copy; commits check
 * `expected_revision` against the stored task like TaskMgr does.
 */
export class MockNotifyBackend implements NotifyBackend {
  private hints = new Map<string, Set<() => void>>()
  private commitDelayMs = 0
  private failNext?: string
  /** Reads served so far (tests assert that nothing is fetched needlessly). */
  readonly reads = { get: 0, getTask: 0 }

  constructor() {
    if (typeof window === 'undefined') return
    window.addEventListener('storage', event => {
      if (event.key !== STORAGE_KEY) return
      const before = this.parse(event.oldValue), after = this.parse(event.newValue)
      for (const id of new Set([...Object.keys(before), ...Object.keys(after)])) if (JSON.stringify(before[id]) !== JSON.stringify(after[id])) this.emit(id)
    })
  }

  private parse(value: string | null): Record<string, MockNotify> {
    try { return value ? JSON.parse(value) as Record<string, MockNotify> : {} } catch { return {} }
  }

  private load(): Record<string, MockNotify> {
    const stored = this.parse(localStorage.getItem(STORAGE_KEY))
    if (Object.keys(stored).length) return stored
    const seeded = seedState(Date.now())
    localStorage.setItem(STORAGE_KEY, JSON.stringify(seeded))
    return seeded
  }

  private save(state: Record<string, MockNotify>, changed: string) {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state))
    this.emit(changed)
  }

  private emit(notifyId: string) { this.hints.get(notifyId)?.forEach(hint => hint()) }

  private record(notifyId: string): [Record<string, MockNotify>, MockNotify] {
    const state = this.load()
    const notify = state[notifyId]
    if (!notify) throw Object.assign(Error('notify not found'), { kind: 'not_found' })
    return [state, notify]
  }

  private byTask(taskId: string): [Record<string, MockNotify>, MockNotify & { task: MockTask }] {
    const state = this.load()
    const notify = Object.values(state).find(item => item.task?.taskId === taskId)
    if (!notify?.task) throw taskError('task_not_found')
    return [state, notify as MockNotify & { task: MockTask }]
  }

  private viewOf(notify: MockNotify): NotifyView {
    if (notify.view.kind === 'notice') return { ...structuredClone(notify.view), status: notify.notice ?? 'open' }
    const task = notify.task!
    let status: NotifyStatus = 'open'
    let result: NotifyResult | undefined
    if (task.phase === 'Terminal') {
      if (task.outcome === 'Succeeded' && task.result) {
        result = { action: task.result.action ?? 'answer', ...(task.result.values && Object.keys(task.result.values).length ? { values: task.result.values } : {}), ...(task.result.submitted_by ? { submitted_by: task.result.submitted_by } : {}), ...(task.result.submitted_at ? { submitted_at: task.result.submitted_at } : {}), ...(task.committedBy ? { committed_by: task.committedBy } : {}) }
        status = result.action === NOTIFY_ACTION_DISMISS ? 'dismissed' : 'answered'
      } else status = notify.view.closed_reason === 'expired' ? 'expired' : 'canceled'
    }
    return { ...structuredClone(notify.view), status, ...(result ? { result } : {}), ...(task.completedAt && task.phase === 'Terminal' ? { closed_at_ms: notify.view.closed_at_ms ?? task.completedAt } : {}) }
  }

  private maybeFail(): void {
    if (!this.failNext) return
    const error = this.failNext
    this.failNext = undefined
    throw Error(error)
  }

  async get(notifyId: string) {
    this.reads.get++
    return this.viewOf(this.record(notifyId)[1])
  }

  async ack(notifyId: string) {
    this.maybeFail()
    const [state, notify] = this.record(notifyId)
    if (notify.view.kind === 'notice' && notify.notice === 'open') { notify.notice = 'read'; this.save(state, notifyId) }
    return this.viewOf(notify)
  }

  async dismiss(notifyId: string) {
    this.maybeFail()
    const [state, notify] = this.record(notifyId)
    if (notify.view.kind === 'notice' && !isNotifyClosed(notify.notice ?? 'open')) {
      notify.notice = 'dismissed'
      notify.view.closed_at_ms = Date.now()
      this.save(state, notifyId)
    }
    return this.viewOf(notify)
  }

  async getTask(taskId: string): Promise<NotifyTaskState> {
    this.reads.getTask++
    const { task } = this.byTask(taskId)[1]
    return { revision: task.revision, phase: task.phase, ...(task.result ? { result: task.result } : {}) }
  }

  async commitResult(taskId: string, result: unknown, expectedRevision: number) {
    if (this.commitDelayMs) await new Promise(resolve => setTimeout(resolve, this.commitDelayMs))
    this.maybeFail()
    const [state, notify] = this.byTask(taskId)
    const task = notify.task
    if (task.phase === 'Terminal') throw taskError(TASK_ERR_ALREADY_COMPLETED)
    if (task.revision !== expectedRevision) throw taskError(TASK_ERR_REVISION_CONFLICT)
    const value = result as MockTask['result']
    Object.assign(task, { phase: 'Terminal', outcome: 'Succeeded', result: value, revision: task.revision + 1, completedAt: Date.now(), committedBy: `${value?.submitted_by ?? MOCK_SELF_USER}|control-panel` })
    this.save(state, notify.view.notify_id)
  }

  taskErrorCode(error: unknown) {
    const code = (error as { code?: unknown } | null)?.code
    return typeof code === 'string' ? code : null
  }

  failure(error: unknown) {
    const record = error as { kind?: unknown; code?: unknown } | null
    return record?.kind === 'not_found' || record?.code === 'task_not_found' ? 'missing' as const : record?.code === 'permission_denied' ? 'denied' as const : 'error' as const
  }

  subscribe(notifyId: string, _taskId: string | undefined, onHint: () => void) {
    const set = this.hints.get(notifyId) ?? new Set<() => void>()
    this.hints.set(notifyId, set)
    set.add(onHint)
    return () => { set.delete(onHint); if (set.size === 0) this.hints.delete(notifyId) }
  }

  async submitter() { return MOCK_SELF_USER }
  now() { return Date.now() }

  /* ── Test hooks ── */

  /** `commitDelayMs`: wait before each commit (a slow network), so two submissions can race; `failNext`: the next write (ack / dismiss / commit) fails. */
  configure(options: { commitDelayMs?: number; failNext?: string }) {
    if (options.commitDelayMs !== undefined) this.commitDelayMs = options.commitDelayMs
    if (options.failNext !== undefined) this.failNext = options.failNext
  }

  /** Someone answers on another device (or an Agent on the user's behalf, `via` its App). */
  answerElsewhere(notifyId: string, answer: NotifyAnswer, by = MOCK_SELF_USER, via = 'control-panel') {
    const [state, notify] = this.record(notifyId)
    const task = notify.task
    if (!task || task.phase === 'Terminal') return
    const now = Date.now()
    Object.assign(task, { phase: 'Terminal', outcome: 'Succeeded', revision: task.revision + 1, completedAt: now, committedBy: `${by}|${via}`, result: { kind: 'human', action: answer.dismiss ? NOTIFY_ACTION_DISMISS : 'answer', values: { ...(answer.option ? { option: answer.option } : {}), ...(answer.text ? { text: answer.text } : {}) }, submitted_by: by, submitted_at: now } })
    this.save(state, notifyId)
  }

  /** The sender or the sweeper closes it: `expired`, or canceled with a reason. */
  close(notifyId: string, reason: string) {
    const [state, notify] = this.record(notifyId)
    const now = Date.now()
    if (notify.task) Object.assign(notify.task, { phase: 'Terminal', outcome: 'Canceled', revision: notify.task.revision + 1, completedAt: now })
    else notify.notice = reason === 'expired' ? 'expired' : 'canceled'
    Object.assign(notify.view, { closed_at_ms: now, closed_reason: reason })
    this.save(state, notifyId)
  }

  view(notifyId: string): NotifyView { return this.viewOf(this.record(notifyId)[1]) }
}

let backend: MockNotifyBackend | null = null

export function getMockNotifyBackend(): MockNotifyBackend {
  if (backend) return backend
  backend = new MockNotifyBackend()
  if (import.meta.env.DEV && typeof window !== 'undefined') Object.assign(window, { __messageHubMockNotify: backend })
  return backend
}
