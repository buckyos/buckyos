import { displayedContent, editsOf, effectiveContent, foldMessageRelations, messageObjId, messageRelations, messageSummaryText, threadMessagesOf } from '../../src/app/messagehub/conversation/history/relations.ts'
import { agentAskQuestionText, buildAgentAskAnswer, messageAgentAsk, messageAgentAskAnswer, messageAgentTaskId, type AgentAsk, type MessageObject, type MsgContent } from '../../src/app/messagehub/protocol/msgobj.ts'
import { conversationInputState, isStopCommand, mergeMentions } from '../../src/app/messagehub/conversation/input/inputState.ts'
import { taskAreaView, taskNodeState } from '../../src/app/messagehub/conversation/tasks/taskView.ts'
import { TaskWatch, type TaskChildren, type TaskSnapshot } from '../../src/app/messagehub/conversation/tasks/taskWatch.ts'
import type { TaskWatchDetail, TaskWatchNode, TaskWatchSource } from '../../src/api/task_mgr.ts'

const ME = 'did:user:me', AGENT = 'did:agent:jarvis', BOB = 'did:user:bob'
const now = 1_780_000_000_000
function equal(actual: unknown, expected: unknown) { if (JSON.stringify(actual) !== JSON.stringify(expected)) throw Error(`Expected ${JSON.stringify(expected)}, received ${JSON.stringify(actual)}`) }
const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms))
async function until(check: () => boolean, label: string) {
  for (let attempt = 0; attempt < 200; attempt++) { if (check()) return; await sleep(5) }
  throw Error(`Timed out waiting for ${label}`)
}

function msg(id: string, from: string, content: MsgContent | string, extra: Partial<MessageObject> = {}, at = now): MessageObject {
  return { from, to: [ME], kind: 'chat', created_at_ms: at, ui_message_id: id, ui_record: { msgId: `obj-${id}`, recordId: id }, content: typeof content === 'string' ? { format: 'text/plain', content } : content, ...extra }
}

const finalContent: MsgContent = {
  title: 'Report',
  format: 'text/markdown',
  content: '## Done\n\n- **3** failures',
  machine: { intent: 'agent.report', data: { failures: 3 } },
  refs: [{ role: 'output', label: 'report.pdf', target: { type: 'data_obj', obj_id: 'cyfile:report' } }],
}
const question = msg('q', ME, 'Summarize the build', {}, now)
const placeholder = msg('p', AGENT, '收到，开始处理…', { agent_task: { task_id: 'task-1' } }, now + 1)
const final = msg('f', AGENT, finalContent, { relates_to: { rel: 'edit', target: 'obj-p' }, agent_task: { task_id: 'task-hijack' } }, now + 60_000)
const next = msg('n', ME, 'Thanks', {}, now + 90_000)

Deno.test('an edit replaces the complete content while the anchor keeps its ObjId, place, time and agent_task', () => {
  const folded = foldMessageRelations([question, placeholder, final, next])
  equal(folded.map(item => item.ui_message_id), ['q', 'p', 'n'])
  const bubble = folded[1]
  equal(effectiveContent(bubble), finalContent)
  equal([displayedContent(bubble), messageSummaryText(bubble)], [finalContent.content, finalContent.content])
  equal([messageObjId(bubble), bubble.created_at_ms], ['obj-p', now + 1])
  equal(bubble.content, placeholder.content)
  equal(messageRelations(bubble)?.edited?.edits, [{ id: 'obj-f', at: now + 60_000, content: finalContent }])
  // The task of a bubble is the anchor's; whatever an edit carries is ignored.
  equal([messageAgentTaskId(bubble), messageAgentTaskId(final), messageAgentTaskId(question)], ['task-1', undefined, undefined])
  equal(messageAgentTaskId({ ...question, agent_task: { task_id: 7 } as never }), undefined)
  equal(messageAgentTaskId({ ...question, agent_task: 'task-1' as never }), undefined)
  // An attachment-only edit is summarised by its attachment names.
  const attachmentOnly = foldMessageRelations([placeholder, msg('f2', AGENT, { refs: finalContent.refs }, { relates_to: { rel: 'edit', target: 'obj-p' } }, now + 5)])[0]
  equal([displayedContent(attachmentOnly), messageSummaryText(attachmentOnly), effectiveContent(attachmentOnly).format], ['', 'report.pdf', undefined])
})

Deno.test('edit folding tolerates an edit that arrives first, replays, and an original that is not loaded', () => {
  const expected = foldMessageRelations([question, placeholder, final])
  equal(foldMessageRelations([final, question, placeholder]).map(item => [item.ui_message_id, displayedContent(item)]), [['q', 'Summarize the build'], ['p', finalContent.content]])
  equal(messageRelations(foldMessageRelations([final, question, placeholder])[1]), messageRelations(expected[1]))
  equal(messageRelations(foldMessageRelations([question, placeholder, final, final, { ...final }])[1]), messageRelations(expected[1]))
  // Only the edit is loaded: it has no row of its own and nothing to fold into.
  equal(foldMessageRelations([final, next]).map(item => item.ui_message_id), ['n'])
  // Two edits: the newest wins whatever the arrival order; both stay in the record.
  const early = msg('e', AGENT, 'draft answer', { relates_to: { rel: 'edit', target: 'obj-p' } }, now + 30_000)
  for (const order of [[placeholder, early, final], [final, placeholder, early]]) {
    const [bubble] = foldMessageRelations(order)
    equal([displayedContent(bubble), messageRelations(bubble)?.edited?.edits.map(edit => edit.id)], [finalContent.content, ['obj-e', 'obj-f']])
  }
})

Deno.test('edits by another author, of a redacted original, or of another edit are ignored', () => {
  const foreign = msg('x', BOB, 'hijacked', { relates_to: { rel: 'edit', target: 'obj-p' } }, now + 70_000)
  const [hijacked] = foldMessageRelations([placeholder, foreign])
  equal([displayedContent(hijacked), messageRelations(hijacked)], ['收到，开始处理…', undefined])
  const [mixed] = foldMessageRelations([placeholder, final, foreign])
  equal(displayedContent(mixed), finalContent.content)
  const redact = msg('r', AGENT, '', { relates_to: { rel: 'redact', target: 'obj-p' } }, now + 2)
  for (const order of [[placeholder, redact, final], [final, redact, placeholder]]) {
    const [redacted] = foldMessageRelations(order)
    equal(messageRelations(redacted), { redacted: { by: AGENT, at: now + 2 } })
    equal(effectiveContent(redacted), placeholder.content)
  }
  const editOfEdit = msg('ee', AGENT, 'edit of the edit', { relates_to: { rel: 'edit', target: 'obj-f' } }, now + 80_000)
  equal(displayedContent(foldMessageRelations([placeholder, final, editOfEdit])[0]), finalContent.content)
})

Deno.test('quotes show the effective content and the edits of a bubble are found for reading together', () => {
  const reply = msg('re', ME, 'about that', { relates_to: { rel: 'thread', target: 'obj-p' } }, now + 95_000)
  const folded = foldMessageRelations([placeholder, final, reply])
  equal(messageRelations(folded[1])?.replyTo?.content, finalContent.content)
  const foreign = msg('x', BOB, 'hijacked', { relates_to: { rel: 'edit', target: 'obj-p' } }, now + 70_000)
  equal(editsOf([question, placeholder, final, foreign, next], [placeholder]).map(item => item.ui_message_id), ['f'])
  equal(editsOf([question, placeholder, final], [question]), [])
})

/* ── Agent questions (`agent_ask`) ── */

const textAsk: AgentAsk = { anchor: 'obj-p', call_id: 'call-1', turn: 3, input: { kind: 'text', options: [], allow_text: false } }
const choiceAsk: AgentAsk = { anchor: 'obj-p', call_id: 'call-2', turn: 3, input: { kind: 'choice', options: [{ id: 'a', label: 'docs/ only' }, { id: 'b', label: 'The whole repository' }], allow_text: false } }
const onP = { relates_to: { rel: 'thread', target: 'obj-p' } }
const q1 = msg('q1', AGENT, 'Who are the docs for?', { ...onP, agent_ask: textAsk }, now + 10)
const a1 = msg('a1', ME, 'New contributors.', onP, now + 20)
const q2 = msg('q2', AGENT, 'Which docs?\n\na. docs/ only\nb. The whole repository', { ...onP, agent_ask: choiceAsk }, now + 30)
const a2 = msg('a2', ME, { format: 'text/plain', content: 'docs/ only', machine: { intent: 'agent_ask.answer', data: { call_id: 'call-2', option: 'a' } } }, onP, now + 40)

Deno.test('agent_ask is read tolerantly and an answer is built from it', () => {
  equal(messageAgentAsk(q2), choiceAsk)
  equal(messageAgentAsk(msg('x', AGENT, 'no ask')), undefined)
  // No call_id, or not an object: no question at all.
  equal(messageAgentAsk({ ...q1, agent_ask: { anchor: 'obj-p', input: { kind: 'text' } } as never }), undefined)
  equal(messageAgentAsk({ ...q1, agent_ask: 'ask' as never }), undefined)
  // A choice without a usable option, or an unknown kind, is a text question; duplicate and empty options are dropped.
  equal(messageAgentAsk({ ...q1, agent_ask: { call_id: 'c', input: { kind: 'choice', options: [{ id: 'a' }, 'b'] } } as never })?.input.kind, 'text')
  equal(messageAgentAsk({ ...q1, agent_ask: { call_id: 'c', anchor: '', turn: 'x', input: { kind: 'choice', options: [{ id: 'a', label: 'A' }, { id: 'a', label: 'again' }, { id: 'b', label: ' ' }], allow_text: true } } as never }), { call_id: 'c', input: { kind: 'choice', options: [{ id: 'a', label: 'A' }], allow_text: true } })
  equal(messageAgentAsk({ ...q1, agent_ask: { call_id: 'c', input: { kind: 'poll' } } as never })?.input.kind, 'text')
  // An edit never asks.
  equal(messageAgentAsk({ ...q2, relates_to: { rel: 'edit', target: 'obj-q1' } }), undefined)
  equal(messageAgentAskAnswer(a2), { call_id: 'call-2', option: 'a' })
  equal(messageAgentAskAnswer(a1), undefined)

  equal(buildAgentAskAnswer(q2, choiceAsk, { option: 'b' }), { text: 'The whole repository', relates_to: { rel: 'thread', target: 'obj-p' }, machine: { intent: 'agent_ask.answer', data: { call_id: 'call-2', option: 'b' } } })
  equal(buildAgentAskAnswer(q2, choiceAsk, { option: 'z' }), undefined)
  equal(buildAgentAskAnswer(q1, textAsk, { text: 'Me' }), { text: 'Me', relates_to: { rel: 'thread', target: 'obj-p' } })
  // Without an anchor (a channel that cannot edit) the answer replies to the question itself.
  equal(buildAgentAskAnswer(q1, { ...textAsk, anchor: undefined }, { text: 'Me' })?.relates_to, { rel: 'thread', target: 'obj-q1' })

  // The option list the body repeats for other clients is not shown twice.
  equal(agentAskQuestionText(q2.content.content ?? '', choiceAsk), 'Which docs?')
  equal(agentAskQuestionText('Which docs?\n- 1) docs/ only\n- 2) The whole repository\n\n', choiceAsk), 'Which docs?')
  equal(agentAskQuestionText('Which docs: docs/ only or The whole repository?', choiceAsk), 'Which docs: docs/ only or The whole repository?')
  equal(agentAskQuestionText('Which docs?\nb. The whole repository\na. docs/ only', choiceAsk), 'Which docs?\nb. The whole repository\na. docs/ only')
  equal(agentAskQuestionText('  Who?  ', textAsk), 'Who?')
})

Deno.test('the questions and answers of a Turn fold under its anchor, also after the final edit', () => {
  const rows = foldMessageRelations([question, placeholder, q1, a1, q2, a2, final])
  equal(rows.map(item => item.ui_message_id), ['q', 'p'])
  const view = messageRelations(rows[1])
  equal(displayedContent(rows[1]), finalContent.content)
  equal(view?.thread?.map(entry => [entry.id, entry.from, entry.content.content, entry.at, entry.agent_ask?.call_id ?? null, entry.option ?? null]), [
    ['obj-q1', AGENT, 'Who are the docs for?', now + 10, 'call-1', null],
    ['obj-a1', ME, 'New contributors.', now + 20, null, null],
    ['obj-q2', AGENT, q2.content.content, now + 30, 'call-2', null],
    ['obj-a2', ME, 'docs/ only', now + 40, null, 'a'],
  ])
  // While waiting: no edit yet, the list is there already.
  equal(messageRelations(foldMessageRelations([question, placeholder, q1])[1])?.thread?.length, 1)
  // The Q/A records are read together with the bubble that shows them.
  equal(threadMessagesOf([question, placeholder, q1, a1, q2, a2, final, next], [placeholder]).map(item => item.ui_message_id), ['q1', 'a1', 'q2', 'a2'])
  equal(threadMessagesOf([question, placeholder, q1, a1], [question]), [])
  // An edited answer folds with its latest content; a recalled one is marked.
  const fix = msg('a1e', ME, 'New contributors and reviewers.', { relates_to: { rel: 'edit', target: 'obj-a1' } }, now + 25)
  const recall = msg('q1r', AGENT, '', { relates_to: { rel: 'redact', target: 'obj-q1' } }, now + 26)
  const edited = messageRelations(foldMessageRelations([placeholder, q1, a1, fix, recall])[0])?.thread
  equal(edited?.map(entry => [entry.id, entry.content.content, entry.redacted ?? false]), [['obj-q1', 'Who are the docs for?', true], ['obj-a1', 'New contributors and reviewers.', false]])
})

Deno.test('other replies to an Agent message keep their rows', () => {
  // No question is open: a reply to the Agent's reply is an ordinary reply.
  const reply = msg('re', ME, 'about that', onP, now + 50)
  let rows = foldMessageRelations([question, placeholder, final, reply])
  equal(rows.map(item => item.ui_message_id), ['q', 'p', 're'])
  equal([messageRelations(rows[1])?.thread, messageRelations(rows[2])?.replyTo?.content], [undefined, finalContent.content])
  // The answer closed the question; the next reply is an ordinary one again.
  rows = foldMessageRelations([placeholder, q1, a1, reply])
  equal([rows.map(item => item.ui_message_id), messageRelations(rows[0])?.thread?.length], [['p', 're'], 2])
  // The asker's own follow-up is not an answer; a "question" from someone else than the anchor's author is no question.
  rows = foldMessageRelations([placeholder, q1, msg('more', AGENT, 'one more thing', onP, now + 15)])
  equal(rows.map(item => item.ui_message_id), ['p', 'more'])
  equal(foldMessageRelations([placeholder, msg('fake', BOB, 'Q?', { ...onP, agent_ask: textAsk }, now + 16)]).map(item => item.ui_message_id), ['p', 'fake'])
  // A reply to a message without agent_task, or to an anchor that is not loaded or comes later, keeps its row.
  equal(foldMessageRelations([question, msg('r2', AGENT, 'x', { relates_to: { rel: 'thread', target: 'obj-q' }, agent_ask: textAsk }, now + 5)]).length, 2)
  equal(foldMessageRelations([q1, a1]).map(item => item.ui_message_id), ['q1', 'a1'])
  equal(foldMessageRelations([q1, placeholder]).map(item => item.ui_message_id), ['q1', 'p'])
})

Deno.test('the input state follows the newest message of the timeline only', () => {
  const state = (timeline: MessageObject[], group = false) => conversationInputState(timeline, ME, { group })
  const summary = (timeline: MessageObject[], group = false) => {
    const value = state(timeline, group)
    return value.mode === 'answer' ? [value.mode, value.question.ui_message_id, value.ask.input.kind, value.relatesTo, value.mentions ?? null] : [value.mode]
  }
  equal(summary([question, placeholder, q1]), ['answer', 'q1', 'text', { rel: 'thread', target: 'obj-p' }, null])
  equal(summary([question, placeholder, q1, a1, q2]), ['answer', 'q2', 'choice', { rel: 'thread', target: 'obj-p' }, null])
  // Answered, from this or another device: the newest message is the viewer's.
  equal(summary([question, placeholder, q1, a1]), ['normal'])
  equal(summary([question, placeholder, q1, msg('top', ME, 'never mind', {}, now + 15)]), ['normal'])
  // No question, or one the viewer asked.
  equal(summary([question, placeholder]), ['normal'])
  equal(summary([placeholder, msg('mine', ME, 'Q?', { agent_ask: textAsk }, now + 5)]), ['normal'])
  equal(summary([]), ['normal'])
  // Without an anchor the answer replies to the question.
  const bare = msg('bare', AGENT, 'Which branch?', { agent_ask: { call_id: 'call-9', input: { kind: 'text', options: [], allow_text: false } } }, now + 5)
  equal(summary([question, bare]), ['answer', 'bare', 'text', { rel: 'thread', target: 'obj-bare' }, null])
  // Reactions, redactions of other messages, edits of other messages, events, status pills and failed sends do not end it.
  const reaction = msg('re1', ME, '👍', { relates_to: { rel: 'reaction', target: 'obj-q1', key: '👍' } }, now + 11)
  const status = { ...msg('st', AGENT, 'typing'), ui_item_kind: 'status', created_at_ms: now + 12 }
  const event = { ...msg('ev', ME, 'session.title_changed'), kind: 'event' as const, content: { content: 'session.title_changed', machine: { intent: 'buckyos.action_log', data: {} } } }
  const failed = { ...msg('fail', ME, 'my answer'), ui_delivery_status: 'failed' }
  const otherEdit = msg('oe', ME, 'fixed', { relates_to: { rel: 'edit', target: 'obj-q' } }, now + 13)
  equal(summary([question, placeholder, q1, reaction, status, event, failed, otherEdit])[0], 'answer')
  // The Turn's final reply (an edit of the anchor by the asker) ends it; a recalled question is no question.
  equal(summary([question, placeholder, q1, final]), ['normal'])
  equal(summary([question, placeholder, q1, msg('fx', BOB, 'hijack', { relates_to: { rel: 'edit', target: 'obj-p' } })])[0], 'answer')
  equal(summary([question, placeholder, q1, msg('rq', AGENT, '', { relates_to: { rel: 'redact', target: 'obj-q1' } }, now + 14)]), ['normal'])
  // A malformed agent_ask is an ordinary message.
  equal(summary([question, placeholder, { ...q1, agent_ask: { anchor: 'obj-p' } as never }]), ['normal'])
  // In a group only the viewer the question mentions answers, and the answer mentions the asker.
  equal(summary([question, placeholder, q1], true), ['normal'])
  equal(summary([question, placeholder, { ...q1, mentions: { dids: [ME] } }], true), ['answer', 'q1', 'text', { rel: 'thread', target: 'obj-p' }, { dids: [AGENT] }])
  // `/stop` is a command, never an answer.
  equal([isStopCommand('/stop'), isStopCommand(' /stop too slow '), isStopCommand('/stopped'), isStopCommand('please /stop')], [true, true, false, false])
  equal([mergeMentions(undefined, undefined), mergeMentions({ dids: [BOB] }, { dids: [AGENT, BOB] }), mergeMentions({ all: true }, undefined)], [undefined, { dids: [BOB, AGENT] }, { all: true }])
})

function node(taskId: string, patch: Partial<TaskWatchDetail> = {}): TaskWatchDetail {
  return { taskId, name: taskId, rootId: 'root', phase: 'Running', revision: 1, createdAt: now, updatedAt: now + 1_000, schemaId: 'opendan.turn/v1', ...patch }
}
const ready = (task: TaskWatchDetail, stale = false): TaskSnapshot => ({ taskId: task.taskId, status: 'ready', task, stale })
const page = (items: TaskWatchNode[], nextCursor?: string): TaskChildren => ({ status: 'ready', items, nextCursor })
const anchor = { edited: false, createdAt: now }

Deno.test('the default task area: state and activity, then sub-task counts, wait reason and last update', () => {
  const running = taskAreaView(ready(node('root', { message: 'Checking the projection' })), page([node('a'), node('b'), node('c', { phase: 'Terminal', outcome: 'Succeeded' })]), anchor, now + 5_000)
  equal(running, { tone: 'active', label: { key: 'messagehub.task.state.running' }, activity: 'Checking the projection', secondary: [{ key: 'messagehub.task.children.running', vars: { count: '2' } }], updatedAt: now + 1_000 })
  const waiting = taskAreaView(ready(node('root', { phase: 'Waiting', waitReason: { kind: 'External', message: 'Waiting for the doc lock' } })), page([]), anchor, now)
  equal([waiting?.label.key, waiting?.secondary], ['messagehub.task.state.waitingExternal', ['Waiting for the doc lock']])
  equal(taskNodeState(node('root', { phase: 'Waiting', waitReason: { kind: 'ChildTask' } })).label.key, 'messagehub.task.state.waitingChild')
  equal(taskNodeState(node('root', { phase: 'Paused' })).label.key, 'messagehub.task.state.paused')
  // Waiting for the viewer's answer: the label says it, the runner's restated text is not repeated.
  const input = taskAreaView(ready(node('root', { phase: 'Waiting', message: 'waiting for input', waitReason: { kind: 'Other', code: 'input', message: 'waiting for input' } })), page([]), anchor, now)
  equal([input?.label.key, input?.activity, input?.secondary], ['messagehub.task.state.waitingInput', undefined, []])
  equal(taskAreaView(ready(node('root', { phase: 'Waiting', message: 'Asking about the docs', waitReason: { kind: 'Other', code: 'input', message: 'waiting for input' } })), page([]), anchor, now)?.activity, 'Asking about the docs')
  equal(taskNodeState(node('root', { phase: 'Waiting', waitReason: { kind: 'Other', code: 'tool' } })).label.key, 'messagehub.task.state.waiting')
  // More children than the loaded page: the count says so instead of guessing a total.
  equal(taskAreaView(ready(node('root')), page([node('a')], '20'), anchor, now)?.secondary, [{ key: 'messagehub.task.children.running', vars: { count: '1+' } }])
  // A stop request is "stopping" until the runner reports the terminal state.
  equal(taskNodeState(node('root', { pendingControl: 'Cancel' })).label.key, 'messagehub.task.state.stopping')
  equal(taskNodeState(node('root', { phase: 'Terminal', outcome: 'Canceled', pendingControl: 'Cancel' })).label.key, 'messagehub.task.state.canceled')
  const failed = taskAreaView(ready(node('root', { phase: 'Terminal', outcome: 'Failed', message: 'Deploy step', error: { code: 'quota', message: 'Bucket over quota' } })), page([]), { edited: true, createdAt: now }, now)
  equal([failed?.tone, failed?.activity, failed?.updatedAt], ['failed', 'Bucket over quota', undefined])
})

Deno.test('no task area for quick replies and unreadable tasks; a finished task whose edit is pending says so', () => {
  const done = node('root', { phase: 'Terminal', outcome: 'Succeeded', completedAt: now + 60_000, updatedAt: now + 60_000 })
  // An ordinary quick reply: created as its Turn ended, no sub-tasks.
  equal(taskAreaView(ready(done), page([]), { edited: false, createdAt: now + 59_500 }, now + 61_000), null)
  equal(taskAreaView(ready(done), page([]), { edited: true, createdAt: now }, now + 61_000), null)
  // The placeholder is still there although the task finished: the final edit has not arrived.
  equal(taskAreaView(ready(done), page([]), anchor, now + 61_000)?.label.key, 'messagehub.task.state.replySyncing')
  equal(taskAreaView(ready(done), page([]), anchor, now + 60_000 + 11 * 60_000), null)
  // After the edit, a task with sub-tasks keeps a collapsed summary.
  const summary = taskAreaView(ready(done), page([node('a', { phase: 'Terminal', outcome: 'Succeeded' }), node('b', { phase: 'Terminal', outcome: 'Failed' })]), { edited: true, createdAt: now }, now + 61_000)
  equal([summary?.label.key, summary?.secondary], ['messagehub.task.state.succeeded', [{ key: 'messagehub.task.children.finished', vars: { count: '2' } }, { key: 'messagehub.task.children.failed', vars: { count: '1' } }]])
  for (const status of ['loading', 'denied', 'missing', 'error'] as const) equal(taskAreaView({ taskId: 'root', status, stale: false }, undefined, anchor, now), null)
  equal(taskAreaView(undefined, undefined, anchor, now), null)
  // A read that failed after a good one never keeps saying "running".
  equal(taskAreaView(ready(node('root'), true), page([]), anchor, now)?.label.key, 'messagehub.task.state.unavailable')
})

class FakeSource implements TaskWatchSource {
  tasks = new Map<string, TaskWatchDetail>()
  denied = new Set<string>()
  offline = false
  calls = { getTask: 0, getSubtasks: 0, listEvents: 0 }
  hints = new Map<string, Set<() => void>>()
  /** Answers queued for the next `getTask` calls, to replay an out-of-date response. */
  scripted: TaskWatchDetail[] = []
  set(task: TaskWatchDetail) { this.tasks.set(task.taskId, task) }
  bump(taskId: string, patch: Partial<TaskWatchDetail>) { const task = this.tasks.get(taskId)!; this.tasks.set(taskId, { ...task, ...patch, revision: task.revision + 1 }) }
  hint(path: string) { this.hints.get(path)?.forEach(listener => listener()) }
  async getTask(taskId: string) {
    this.calls.getTask++
    if (this.offline) throw Error('offline')
    if (this.denied.has(taskId)) throw Object.assign(Error('denied'), { code: 'permission_denied' })
    const scripted = this.scripted.shift()
    if (scripted) return scripted
    const task = this.tasks.get(taskId)
    if (!task) throw Object.assign(Error('missing'), { code: 'task_not_found' })
    return { ...task }
  }
  async getSubtasks(taskId: string, cursor: string | undefined, limit: number) {
    this.calls.getSubtasks++
    if (this.offline) throw Error('offline')
    const children = [...this.tasks.values()].filter(task => task.parentId === taskId)
    const start = cursor ? Number(cursor) : 0
    return { tasks: children.slice(start, start + limit), nextCursor: start + limit < children.length ? String(start + limit) : undefined }
  }
  async listEvents(rootId: string) {
    this.calls.listEvents++
    return [{ eventId: `${rootId}:1`, taskId: rootId, type: 'TaskCreated', revision: 1, at: now }]
  }
  subscribe(scope: 'task' | 'tree', id: string, onHint: () => void) {
    const path = `${scope}:${id}`
    const listeners = this.hints.get(path) ?? new Set()
    this.hints.set(path, listeners)
    listeners.add(onHint)
    return () => { listeners.delete(onHint); if (listeners.size === 0) this.hints.delete(path) }
  }
  failure(error: unknown) {
    const code = (error as { code?: string }).code
    return code === 'permission_denied' ? 'denied' as const : code === 'task_not_found' ? 'missing' as const : 'error' as const
  }
}

const slow = { pollDelaysMs: [60_000] }

Deno.test('visible bubbles share one cached read and one subscription per task, released with the last of them', async () => {
  const source = new FakeSource()
  source.set(node('root', { message: 'step 1' }))
  source.set(node('child', { parentId: 'root' }))
  const watch = new TaskWatch(source, slow)
  let notified = 0
  watch.subscribe(() => { notified++ })
  const first = watch.watch('root'), second = watch.watch('root')
  await until(() => watch.children('root')?.status === 'ready', 'the summary')
  equal([source.calls.getTask, source.calls.getSubtasks, source.calls.listEvents, [...source.hints.keys()]], [1, 1, 0, ['task:root']])
  equal([watch.snapshot('root')?.task?.message, watch.children('root')?.items.map(item => item.taskId), notified > 0], ['step 1', ['child'], true])
  // An event is only a hint to read again; the revision decides what is kept.
  source.bump('root', { message: 'step 2' })
  source.hint('task:root')
  await until(() => watch.snapshot('root')?.task?.message === 'step 2', 'the new revision')
  // A response older than the cached revision is dropped.
  source.scripted.push(node('root', { message: 'step 1', revision: 1 }))
  source.hint('task:root')
  await until(() => source.calls.getTask === 3, 'the stale read')
  await sleep(10)
  equal([watch.snapshot('root')?.task?.message, watch.snapshot('root')?.task?.revision], ['step 2', 2])
  first()
  equal([...source.hints.keys()], ['task:root'])
  second(); second()
  equal([[...source.hints.keys()], watch.isLive('root'), watch.snapshot('root')?.task?.revision], [[], false, 2])
  watch.dispose()
})

Deno.test('polling is the fallback for lost events, is bounded, and stops once the terminal snapshot is cached', async () => {
  const source = new FakeSource()
  source.set(node('root'))
  const watch = new TaskWatch(source, { pollDelaysMs: [5], maxUnchangedPolls: 4 })
  const release = watch.watch('root')
  await until(() => watch.snapshot('root')?.status === 'ready', 'the first read')
  source.bump('root', { message: 'no event was delivered' })
  await until(() => watch.snapshot('root')?.task?.message === 'no event was delivered', 'the poll')
  // Nothing changes any more: polling gives up, the subscription stays.
  await until(() => !watch.isLive('root') || source.calls.getTask >= 6, 'the unchanged polls')
  await sleep(40)
  const afterBound = source.calls.getTask
  await sleep(40)
  equal([source.calls.getTask, [...source.hints.keys()]], [afterBound, ['task:root']])
  // An event still gets through, and a terminal task ends the subscription too.
  source.bump('root', { phase: 'Terminal', outcome: 'Succeeded' })
  source.hint('task:root')
  await until(() => watch.snapshot('root')?.task?.phase === 'Terminal' && !watch.isLive('root'), 'the terminal snapshot')
  equal([...source.hints.keys()], [])
  const settledReads = source.calls.getTask
  await sleep(30)
  equal(source.calls.getTask, settledReads)
  release()
  // A bubble scrolled back into view reuses the terminal snapshot without reading or subscribing.
  const again = watch.watch('root')
  await sleep(10)
  equal([source.calls.getTask, [...source.hints.keys()], watch.snapshot('root')?.task?.outcome], [settledReads, [], 'Succeeded'])
  again()
  watch.dispose()
})

Deno.test('a lost connection marks the snapshot stale, resync catches up; unreadable and cleaned-up tasks are not followed', async () => {
  const source = new FakeSource()
  source.set(node('root', { message: 'working' }))
  const watch = new TaskWatch(source, { pollDelaysMs: [5], maxFailures: 2 })
  const release = watch.watch('root')
  await until(() => watch.snapshot('root')?.status === 'ready', 'the first read')
  source.offline = true
  await until(() => watch.snapshot('root')?.stale === true, 'the stale mark')
  equal(watch.snapshot('root')?.task?.message, 'working')
  await sleep(40)
  const failedReads = source.calls.getTask
  await sleep(40)
  equal(source.calls.getTask, failedReads)
  source.offline = false
  source.bump('root', { message: 'moved on while offline' })
  watch.resync()
  await until(() => watch.snapshot('root')?.stale === false && watch.snapshot('root')?.task?.message === 'moved on while offline', 'the catch-up')
  // The task is cleaned up: it is reported as missing and no longer followed.
  source.tasks.delete('root')
  source.hint('task:root')
  await until(() => watch.snapshot('root')?.status === 'missing', 'the missing state')
  await until(() => !watch.isLive('root'), 'the end of the watch')
  equal([watch.snapshot('root')?.task, [...source.hints.keys()]], [undefined, []])
  release()
  source.set(node('private'))
  source.denied.add('private')
  const releasePrivate = watch.watch('private')
  await until(() => watch.snapshot('private')?.status === 'denied', 'the denied state')
  await until(() => !watch.isLive('private'), 'the end of the denied watch')
  const deniedReads = source.calls.getTask
  await sleep(30)
  equal([source.calls.getTask, [...source.hints.keys()]], [deniedReads, []])
  releasePrivate()
  // A never-readable task without a snapshot is an error, not a state.
  source.offline = true
  const releaseError = watch.watch('unknown')
  await until(() => watch.snapshot('unknown')?.status === 'error', 'the error state')
  releaseError()
  watch.dispose()
})

Deno.test('the tree and its events are read only while watched, level by level and page by page, and released afterwards', async () => {
  const source = new FakeSource()
  source.set(node('root'))
  for (let index = 0; index < 5; index++) source.set(node(`child-${index}`, { parentId: 'root' }))
  source.set(node('grandchild', { parentId: 'child-0' }))
  const watch = new TaskWatch(source, { ...slow, pageSize: 2 })
  const release = watch.watch('root')
  await until(() => watch.children('root')?.status === 'ready', 'the summary')
  equal([watch.children('root')?.items.length, watch.children('root')?.nextCursor, watch.children('child-0'), source.calls.listEvents, [...source.hints.keys()]], [2, '2', undefined, 0, ['task:root']])
  // Without a watched tree nothing below the first level can be loaded.
  await watch.expandNode('root', 'child-0')
  equal(watch.children('child-0'), undefined)
  const releaseTree = watch.watchTree('root')
  equal([...source.hints.keys()].sort(), ['task:root', 'tree:root'])
  await watch.loadChildren('root', true)
  equal([watch.children('root')?.items.map(item => item.taskId), watch.children('root')?.nextCursor], [['child-0', 'child-1', 'child-2', 'child-3'], '4'])
  await watch.expandNode('root', 'child-0')
  await watch.loadEvents('root')
  equal([watch.children('child-0')?.items.map(item => item.taskId), watch.events('root')?.items.length], [['grandchild'], 1])
  // A tree event re-reads what is loaded and keeps the loaded page count.
  source.bump('grandchild', { message: 'deep progress' })
  source.hint('tree:root')
  await until(() => watch.children('child-0')?.items[0].message === 'deep progress', 'the tree refresh')
  equal(watch.children('root')?.items.length, 4)
  watch.collapseNode('root', 'child-0')
  equal(watch.children('child-0'), undefined)
  await watch.expandNode('root', 'child-0')
  releaseTree()
  equal([watch.children('child-0'), watch.events('root'), [...source.hints.keys()], watch.children('root')?.items.length], [undefined, undefined, ['task:root'], 4])
  release()
  watch.dispose()
})
