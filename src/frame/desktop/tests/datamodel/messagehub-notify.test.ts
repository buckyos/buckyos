import { isStatusMessageObject, messageNotify, notifySessionCategory, NOTIFY_INTENT, type MessageObject, type NotifyMessageData } from '../../src/app/messagehub/protocol/msgobj.ts'
import { creationReason, isMessageActivity, selectDefaultSession, sessionAccess, sortSessions, defaultPreferences } from '../../src/app/messagehub/sessionModel.ts'
import { conversationInputState } from '../../src/app/messagehub/conversation/input/inputState.ts'
import { projectOwner, summarizeMessage, type ProjectionLabels } from '../../src/app/messagehub/api/projection.ts'
import type { SessionSummary } from '../../src/app/messagehub/datamodel/sessionApi.ts'
import { committedApp, notifyFilterMatches, notifyTaskResult, resultOption, validateNotifyAnswer, type NotifyView } from '../../src/app/messagehub/notify/model.ts'
import { commitNotifyAnswer, NotifyWatch, type NotifyBackend, type NotifyTaskState } from '../../src/app/messagehub/notify/watch.ts'
import type { MessageHubContext, Session } from '../../src/app/messagehub/types.ts'

const ME = 'did:web:alice.zone.test', ZONE = 'did:web:zone.test', AGENT = 'did:web:jarvis.zone.test'
const OWN: MessageHubContext = { viewerDid: ME, ownerDid: ME, mode: 'self' }
const now = 1_780_000_000_000
function equal(actual: unknown, expected: unknown) { if (JSON.stringify(actual) !== JSON.stringify(expected)) throw Error(`Expected ${JSON.stringify(expected)}, received ${JSON.stringify(actual)}`) }
const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms))
async function until(check: () => boolean, label: string) {
  for (let attempt = 0; attempt < 200; attempt++) { if (check()) return; await sleep(5) }
  throw Error(`Timed out waiting for ${label}`)
}

const prompt = { title: 'Which branch?', message: 'Pick one', controls: [{ kind: 'choice' as const, id: 'option', options: [{ value: 'main', label: 'main' }, { value: 'release', label: 'release-2.4' }], required: false }, { kind: 'text_input' as const, id: 'text', multiline: true }] }
function data(id: string, home: boolean, extra: Partial<NotifyMessageData> = {}): NotifyMessageData {
  return { version: 1, notify_id: id, kind: 'input', category: 'agent', level: 'timely', home, source: { app_id: 'jarvis', agent_did: AGENT, session_id: 'work-1', operation: 'Deploy' }, prompt, task_id: `task-${id}`, url: `https://zone.test/messagehub?notify=${id}`, ...extra }
}
function notifyMsg(id: string, home: boolean, at = now, extra: Partial<MessageObject> = {}): MessageObject {
  return { from: home ? ZONE : AGENT, to: [ME], kind: 'notify', created_at_ms: at, ui_message_id: `${id}-${home ? 'home' : 'copy'}`, ui_record: { msgId: `obj-${id}-${home}`, recordId: `${id}-${home}` }, content: { title: 'Which branch?', format: 'text/plain', content: 'Which branch?\nPick one\n处理：https://zone.test/messagehub?notify=' + id, machine: { intent: NOTIFY_INTENT, data: data(id, home) as never } }, ...extra }
}

Deno.test('a buckyos.notify message is a card: parsed, counted as activity, never a status pill', () => {
  const home = notifyMsg('nt-1', true)
  const parsed = messageNotify(home)
  equal([parsed?.notify_id, parsed?.kind, parsed?.category, parsed?.level, parsed?.home, parsed?.task_id], ['nt-1', 'input', 'agent', 'timely', true, 'task-nt-1'])
  equal(parsed?.prompt.controls?.map(control => control.kind), ['choice', 'text_input'])
  equal([isStatusMessageObject(home), isMessageActivity(home)], [false, true])
  // A transient notify (typing / status) stays a pill; a placeholder for an unreadable record is no activity.
  const transient: MessageObject = { from: AGENT, to: [ME], kind: 'notify', created_at_ms: now, content: { content: 'typing' } }
  equal([isStatusMessageObject(transient), messageNotify(transient)], [true, undefined])
  equal(isMessageActivity({ ...transient, ui_unavailable: true }), false)
  // Wrong intent, unknown kind or category, an edit: not a card.
  equal(messageNotify({ ...home, content: { ...home.content, machine: { intent: 'other', data: data('x', true) as never } } }), undefined)
  equal(messageNotify({ ...home, content: { ...home.content, machine: { intent: NOTIFY_INTENT, data: { ...data('x', true), kind: 'approval' } as never } } }), undefined)
  equal(messageNotify({ ...home, content: { ...home.content, machine: { intent: NOTIFY_INTENT, data: { ...data('x', true), category: 'spam' } as never } } }), undefined)
  equal(messageNotify({ ...home, relates_to: { rel: 'edit', target: 'obj-x' } }), undefined)
  // Unknown controls and malformed options are dropped; an unknown level reads as normal.
  const odd = messageNotify({ ...home, content: { ...home.content, machine: { intent: NOTIFY_INTENT, data: { ...data('x', true), level: 'loud', prompt: { title: 'T', controls: [{ kind: 'slider', id: 'v' }, { kind: 'choice', id: 'option', options: [{ value: 'a' }, { value: 'b', label: 'B' }] }] } } as never } } })
  equal([odd?.level, odd?.prompt.controls], ['normal', [{ kind: 'choice', id: 'option', options: [{ value: 'b', label: 'B' }], multiple: false, required: false }]])
  equal([notifySessionCategory('sysnotify:todo'), notifySessionCategory('sysnotify:spam'), notifySessionCategory('dm:x')], ['todo', undefined, undefined])
  equal(summarizeMessage(home, { previewUnavailable: '?' } as ProjectionLabels), 'Which branch?')
})

const labels: ProjectionLabels = { direct: 'Direct', groupMain: 'Group chat', untitled: 'Untitled', unassigned: 'Unassigned', unassignedDescription: '', you: 'You', previewImage: '[Image]', previewAttachment: '[Attachment]', previewUnavailable: 'Unavailable', systemNotify: 'System notifications', notifyCategories: { todo: 'To-do', agent: 'Agent', app: 'App notifications' } }

function notifySummary(category: string, at: number | null, unread = 0, title = '应用通知'): SessionSummary {
  const id = `sysnotify:${category}`
  return {
    session_id: id, unread_count: unread, updated_at_ms: at ?? now, last_activity_ms: at ?? now, request_count: 0, lifecycle: 'active',
    state: { owner: ME, session_id: id, lifecycle: 'active', registered: true, origin: 'sysnotify', peer_did: ZONE, title, created_at_ms: now - 1000, updated_at_ms: now - 1000 },
    ...(at ? { last_record: { record: { mailbox: ME, record_id: `r-${category}`, owner: ME, box_kind: 'INBOX', msg_id: `m-${category}`, state: unread ? 'UNREAD' : 'READ', from: ZONE, to: ME, session_id: id, sort_key: at, created_at_ms: at, updated_at_ms: at }, msg: notifyMsg(`nt-${category}`, true, at) } } : {}),
  } as SessionSummary
}

Deno.test('the notification sessions project into one System notifications entity, ordered by category, empty ones hidden', () => {
  const summaries = [notifySummary('app', now - 10, 2), notifySummary('agent', now - 50, 1, 'Agent'), notifySummary('todo', null, 0, '待办')]
  // The zone's username would hide a did:web account; the notification sender never is.
  const projected = projectOwner({ ownerDid: ME, summaries, contacts: [], agentDids: [AGENT], personalTitles: {}, groups: {}, groupSessionTitles: {}, policies: {}, labels, hiddenAccounts: { dids: [ME], usernames: ['zone'] } })
  const entity = projected.entities.find(item => item.id === ZONE)
  equal([entity?.systemNotify, entity?.type, entity?.name, entity?.domain, entity?.unreadCount, entity?.sessionCount], [true, 'service', 'System notifications', 'managed', 3, 2])
  const sessions = projected.sessionsByEntity.get(ZONE) ?? []
  // The empty to-do session is not shown; titles come from the viewer's labels, not the registered title.
  equal(sessions.map(session => [session.id, session.title, session.shared.title]), [['sysnotify:app', 'App notifications', ''], ['sysnotify:agent', 'Agent', '']])
  equal(sortSessions(sessions, () => ({ ...defaultPreferences, pinned: true })).map(session => session.id), ['sysnotify:agent', 'sysnotify:app'])
  equal(entity?.lastMessage?.text, 'Which branch?')
  equal(selectDefaultSession(entity!, sessions)?.id, 'sysnotify:agent')
  // Nothing can be sent or created there.
  equal([sessionAccess(OWN, sessions[0], false).mode, sessionAccess(OWN, sessions[0], false).readOnlyReason, sessionAccess(OWN, sessions[0], false).canEditSharedState], ['read_only', 'system_notify', false])
  equal(creationReason(OWN, entity!, 'allow', { kind: 'native', targetDid: ZONE }), 'system_notify')
  equal(sessionAccess({ viewerDid: ME, ownerDid: AGENT, mode: 'observe' }, { ...sessions[0], ownerDid: AGENT }, false).readOnlyReason, 'agent_observer')
})

Deno.test('a notification copy in an Agent conversation never changes the composer', () => {
  const chat = (id: string, from: string, at: number, extra: Partial<MessageObject> = {}): MessageObject => ({ from, to: [from === ME ? AGENT : ME], kind: 'chat', created_at_ms: at, ui_message_id: id, ui_record: { msgId: `obj-${id}`, recordId: id }, content: { format: 'text/plain', content: id }, ...extra })
  const question = chat('q', AGENT, now + 2, { agent_ask: { call_id: 'call-1', input: { kind: 'text', options: [], allow_text: true } } })
  // Newest is the copy: the open question still owns the composer.
  equal(conversationInputState([chat('a', ME, now), question, notifyMsg('nt-1', false, now + 5)], ME).mode, 'answer')
  // Without a question the composer stays normal (typeable).
  equal(conversationInputState([chat('a', ME, now), notifyMsg('nt-1', false, now + 5)], ME).mode, 'normal')
})

Deno.test('answers, results and filters follow the msg_notify.rs contract', () => {
  equal(notifyTaskResult({ option: 'main' }, 'alice', 7), { kind: 'human', action: 'answer', values: { option: 'main' }, submitted_by: 'alice', submitted_at: 7 })
  equal(notifyTaskResult({ text: '  ' }, 'alice', 7), { kind: 'human', action: 'answer', submitted_by: 'alice', submitted_at: 7 })
  equal(notifyTaskResult({ dismiss: true }, 'alice', 7), { kind: 'human', action: 'dismiss', submitted_by: 'alice', submitted_at: 7 })
  equal([validateNotifyAnswer({ option: 'main' }, prompt), validateNotifyAnswer({ option: 'nope' }, prompt), validateNotifyAnswer({}, prompt), validateNotifyAnswer({ dismiss: true }, prompt)], [null, 'unknown_option', 'empty', null])
  equal(validateNotifyAnswer({ text: 'free' }, { title: 'q', controls: [prompt.controls[0]] }), 'text_not_accepted')
  equal([resultOption({ action: 'answer', values: { option: 'main' } }), committedApp({ action: 'answer', committed_by: 'alice|jarvis' }), committedApp({ action: 'answer' })], ['main', 'jarvis', undefined])
  equal((['open', 'read', 'answered', 'dismissed', 'expired', 'canceled', undefined] as const).map(status => [notifyFilterMatches(status, 'all'), notifyFilterMatches(status, 'pending'), notifyFilterMatches(status, 'done')]),
    [[true, true, false], [true, true, false], [true, false, true], [true, false, true], [true, false, true], [true, false, true], [true, false, false]])
})

/** One input notification: a TaskMgr task with a CAS on its revision, and `notify.get` derived from it. */
class FakeBackend implements NotifyBackend {
  task: NotifyTaskState & { outcome?: string; committedBy?: string } = { revision: 4, phase: 'Waiting' }
  reads = 0
  commitDelayMs = 0
  hints = new Set<() => void>()
  view(): NotifyView {
    const result = this.task.result as { action: string; values?: Record<string, unknown>; submitted_by?: string; submitted_at?: number } | undefined
    const status = this.task.phase !== 'Terminal' ? 'open' : result?.action === 'dismiss' ? 'dismissed' : result ? 'answered' : 'canceled'
    return { notify_id: 'nt-1', url: '/messagehub?notify=nt-1', to_user: 'alice', kind: 'input', category: 'agent', level: 'timely', status, prompt, source: { app_id: 'jarvis', agent_did: AGENT }, created_at_ms: now, task_id: 'task-1', ...(result ? { result: { ...result, committed_by: this.task.committedBy } } : {}), home: { owner: ME, session_id: 'sysnotify:agent', record_id: 'r', msg_id: 'm', from: ZONE } }
  }
  async get() { this.reads++; return this.view() }
  async list() { return [this.view()] }
  async ack() { return this.view() }
  async dismiss() { return this.view() }
  async getTask() { return { ...this.task } }
  async commitResult(_taskId: string, result: unknown, expectedRevision: number) {
    if (this.commitDelayMs) await sleep(this.commitDelayMs)
    if (this.task.phase === 'Terminal') throw Object.assign(Error('done'), { code: 'task_already_completed' })
    if (this.task.revision !== expectedRevision) throw Object.assign(Error('stale'), { code: 'revision_conflict' })
    this.task = { revision: this.task.revision + 1, phase: 'Terminal', result, committedBy: 'alice|control-panel' }
    this.hints.forEach(hint => hint())
  }
  taskErrorCode(error: unknown) { return (error as { code?: string }).code ?? null }
  failure() { return 'error' as const }
  subscribe(_notifyId: string, _taskId: string | undefined, hint: () => void) { this.hints.add(hint); return () => { this.hints.delete(hint) } }
  async submitter() { return 'alice' }
  now() { return now }
}

Deno.test('every card of one notification reads one entry; a hint brings the final state to all of them', async () => {
  const backend = new FakeBackend()
  const watch = new NotifyWatch(backend, { pollDelaysMs: [60_000] })
  const releaseHome = watch.watch('nt-1'), releaseCopy = watch.watch('nt-1')
  await until(() => watch.entry('nt-1')?.status === 'ready', 'first read')
  equal([backend.reads, watch.entry('nt-1')?.view?.status, backend.hints.size], [1, 'open', 1])
  // Answered somewhere else (another device): the task's kevent hint re-reads once for both cards.
  backend.task = { revision: 5, phase: 'Terminal', result: { kind: 'human', action: 'answer', values: { option: 'release' }, submitted_by: 'alice', submitted_at: now }, committedBy: 'alice|jarvis' }
  backend.hints.forEach(hint => hint())
  await until(() => watch.entry('nt-1')?.view?.status === 'answered', 'hinted read')
  equal([backend.reads, resultOption(watch.entry('nt-1')?.view?.result), committedApp(watch.entry('nt-1')?.view?.result)], [2, 'release', 'jarvis'])
  // A closed notification is no longer subscribed.
  equal(backend.hints.size, 0)
  releaseHome(); releaseCopy()
})

Deno.test('two cards submitting at once: the first commit wins, the other sees the final result', async () => {
  const backend = new FakeBackend()
  backend.commitDelayMs = 20
  const watch = new NotifyWatch(backend, { pollDelaysMs: [60_000] })
  const release = watch.watch('nt-1')
  await until(() => watch.entry('nt-1')?.status === 'ready', 'first read')
  const outcomes = await Promise.all([watch.answer('nt-1', { option: 'main' }), watch.answer('nt-1', { text: 'use release' })])
  equal(outcomes.sort(), ['already_closed', 'committed'])
  equal([watch.entry('nt-1')?.view?.status, resultOption(watch.entry('nt-1')?.view?.result), backend.task.revision], ['answered', 'main', 5])
  equal(backend.task.result, { kind: 'human', action: 'answer', values: { option: 'main' }, submitted_by: 'alice', submitted_at: now })
  // A late answer to a closed notification does not commit anything.
  equal(await watch.answer('nt-1', { option: 'release' }), 'already_closed')
  equal(backend.task.revision, 5)
  release()
})

Deno.test('a stale snapshot loses the CAS, re-reads and still commits while the task is open', async () => {
  const backend = new FakeBackend()
  let stale = true
  const racy = { ...backend, getTask: async () => { const task = await backend.getTask(); if (stale) { stale = false; return { ...task, revision: task.revision - 1 } } return task }, commitResult: backend.commitResult.bind(backend), taskErrorCode: backend.taskErrorCode.bind(backend) }
  equal(await commitNotifyAnswer(racy, 'task-1', { dismiss: true }, 'alice', now), 'committed')
  equal([backend.task.phase, (backend.task.result as { action: string }).action], ['Terminal', 'dismiss'])
  await assertRejects(() => commitNotifyAnswer({ ...racy, getTask: async () => ({ revision: 1, phase: 'Waiting' }), commitResult: async () => { throw Object.assign(Error('down'), { code: 'internal' }) } }, 'task-1', { option: 'main' }, 'alice', now))
})

async function assertRejects(run: () => Promise<unknown>) {
  try { await run() } catch { return }
  throw Error('expected a rejection')
}

Deno.test('an invalid answer is refused before anything is committed', async () => {
  const backend = new FakeBackend()
  const watch = new NotifyWatch(backend, { pollDelaysMs: [60_000] })
  await watch.load('nt-1')
  await assertRejects(() => watch.answer('nt-1', { option: 'nope' }))
  equal(backend.task.phase, 'Waiting')
})

Deno.test('sessions of other entities keep their own order', () => {
  const plain = (id: string, at: number): Session => ({ id, entityId: AGENT, ownerDid: ME, binding: { kind: 'native', targetDid: AGENT }, origin: 'manual', lifecycle: 'active', createdAt: 0, shared: { title: '', description: '', updatedAt: 0 }, members: {}, title: id, type: 'chat', lastActiveAt: at, unreadCount: 0 })
  equal(sortSessions([plain('a', 1), plain('b', 3), plain('c', 2)], () => defaultPreferences).map(session => session.id), ['b', 'c', 'a'])
})
