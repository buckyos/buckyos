import { foregroundState, latestTurnAnchor, turnInputMessage, turnMode } from '../../src/app/messagehub/conversation/input/foregroundState.ts'
import { flattenWork, interruptedRoots, localizeLine, parseWorkCards, parseWorkTree, workControls, type WorkCard } from '../../src/app/messagehub/conversation/worksessions/model.ts'
import { taskAreaView, taskNodeState } from '../../src/app/messagehub/conversation/tasks/taskView.ts'
import { agentConversationState, formatSize, generationBreaks, generationNumber, parseAgentSummary, parseDeliveries, parseUiConversation, type UiSessionView } from '../../src/app/messagehub/agentpanel/model.ts'
import { rowsWithBreaks } from '../../src/app/messagehub/agentpanel/breaks.ts'
import type { MessageObject } from '../../src/app/messagehub/protocol/msgobj.ts'

function equal(actual: unknown, expected: unknown) { if (JSON.stringify(actual) !== JSON.stringify(expected)) throw Error(`Expected ${JSON.stringify(expected)}, received ${JSON.stringify(actual)}`) }

const SELF = 'did:test:me'
const AGENT = 'did:test:agent'
let clock = 1_000
const message = (id: string, from: string, extra: Partial<MessageObject> = {}): MessageObject => ({ from, to: [from === SELF ? AGENT : SELF], kind: 'chat', created_at_ms: clock += 1_000, content: { format: 'text/plain', content: id }, ui_message_id: id, ui_msg_obj_id: id, ...extra } as MessageObject)
const placeholder = (id: string, task: string) => message(id, AGENT, { agent_task: { task_id: task } })
const relation = (id: string, from: string, rel: string, target: string) => message(id, from, { relates_to: { rel, target } } as Partial<MessageObject>)
const task = (phase: string, text?: string) => ({ phase, message: text } as Parameters<typeof turnMode>[0])

Deno.test('a Turn is running while its task runs, interrupted or paused while the task is paused, idle otherwise', () => {
  equal([undefined, task('Running'), task('Accepted'), task('Paused', 'interrupted: the host restarted'), task('Paused', 'paused: by owner'), task('Paused'), task('Waiting', 'waiting for 1 sub session(s)'), task('Terminal'), task('Promised')].map(turnMode), ['idle', 'running', 'running', 'interrupted', 'paused', 'paused', 'idle', 'idle', 'idle'])
})

Deno.test('the placeholder in front is the newest Agent message carrying a task; relations, own messages and redacted placeholders are skipped', () => {
  equal(latestTurnAnchor([], SELF), null)
  equal(latestTurnAnchor([message('m1', SELF), message('m2', AGENT)], SELF), null)
  const first = placeholder('p1', 'task-1'), second = placeholder('p2', 'task-2')
  equal(latestTurnAnchor([message('m1', SELF), first, message('m3', SELF), second, message('m5', SELF)], SELF)?.taskId, 'task-2')
  // The final edit of a Turn carries no task of its own; a reaction is not a placeholder.
  equal(latestTurnAnchor([first, relation('e1', AGENT, 'edit', 'p1'), relation('r1', SELF, 'reaction', 'p1')], SELF)?.taskId, 'task-1')
  // A redacted placeholder is passed over for the one before it.
  equal(latestTurnAnchor([first, second, relation('x1', AGENT, 'redact', 'p2')], SELF)?.taskId, 'task-1')
  // The viewer's own message never is one, whatever it carries.
  equal(latestTurnAnchor([message('own', SELF, { agent_task: { task_id: 'task-9' } })], SELF), null)
})

Deno.test('the foreground state blocks sending only for a placeholder whose Turn runs, is interrupted or paused', () => {
  const anchor = { message: placeholder('p1', 'task-1'), taskId: 'task-1' }
  equal(foregroundState(null, task('Running')), { mode: 'idle' })
  equal(foregroundState(anchor, undefined), { mode: 'idle' })
  equal(foregroundState(anchor, task('Waiting')), { mode: 'idle' })
  equal(foregroundState(anchor, task('Terminal')), { mode: 'idle' })
  equal(foregroundState(anchor, task('Running')).mode, 'running')
  equal(foregroundState(anchor, task('Paused', 'interrupted')).mode, 'interrupted')
  equal(foregroundState(anchor, task('Paused', 'paused: by owner')).mode, 'paused')
  equal(foregroundState(anchor, task('Running')).anchor?.taskId, 'task-1')
})

Deno.test('the input of a Turn is the message its task names, else the viewer\'s newest message before the placeholder', () => {
  const first = message('m1', SELF), second = message('m2', SELF)
  const anchor = placeholder('p1', 'task-1')
  const later = message('m3', SELF)
  const timeline = [first, second, anchor, later]
  equal(turnInputMessage(timeline, anchor, SELF)?.ui_message_id, 'm2')
  equal(turnInputMessage(timeline, anchor, SELF, ['m1'])?.ui_message_id, 'm1')
  equal(turnInputMessage(timeline, anchor, SELF, ['unknown'])?.ui_message_id, 'm2')
  // Retracted already, or nothing of the viewer before the placeholder: nothing to retract.
  equal(turnInputMessage([first, relation('x', SELF, 'redact', 'm1'), anchor], anchor, SELF), null)
  equal(turnInputMessage([anchor, later], anchor, SELF), null)
  // Relation messages and failed sends are not inputs.
  equal(turnInputMessage([first, relation('r', SELF, 'reaction', 'p0'), message('failed', SELF, { ui_delivery_status: 'failed' } as Partial<MessageObject>), anchor], anchor, SELF)?.ui_message_id, 'm1')
})

const usage = { input_tokens: 0, output_tokens: 0, rounds: 0, models: [] }
const card = (id: string, extra: Record<string, unknown> = {}) => ({ session_id: id, shape: 'simple', title: id, created_at_ms: 1, finished_at_ms: null, state: 'running', line: '', usage, model: null, subtree_usage: null, todos: null, ...extra })
const todo = (seq: number, bound?: unknown) => ({ todo_id: `T0${seq}`, seq, title: `Step ${seq}`, status: bound ? 'running' : 'pending', mode: 'serial', attempt: bound ? 1 : 0, blocked_by: [], block_reason: null, card: bound ?? null })

Deno.test('a work tree keeps Todo cards and other sub sessions recursively, four levels deep, and the pause source of paused cards', () => {
  const deep = card('l4', { children: [card('l5')] })
  const tree = parseWorkTree({ agent_did: AGENT, session_id: 'ui', complete: false, sessions: [
    card('plan', { shape: 'plan', todos: { rev: 1, total: 2, done: 0, items: [todo(1, card('t1', { state: 'paused', paused: 'parent', children: [card('l3', { children: [deep] })] })), todo(2)] }, children: [card('side', { state: 'paused', paused: 'interrupted' })] }),
    card('solo', { state: 'paused', paused: 'nonsense' }),
  ] })!
  equal(tree.sessions.map(item => item.session_id), ['plan', 'solo'])
  equal(flattenWork(tree.sessions).map(item => item.session_id), ['plan', 't1', 'l3', 'l4', 'side', 'solo'])
  const t1 = tree.sessions[0].todos!.items[0].card!
  equal([t1.paused, tree.sessions[0].children![0].paused, tree.sessions[1].paused], ['parent', 'interrupted', undefined])
  equal(parseWorkTree({ agent_did: AGENT, sessions: [] }), null)
  equal(parseWorkTree(null), null)
  // `session.work_cards` is unchanged: a Todo's card carries nothing under it.
  const cards = parseWorkCards({ agent_did: AGENT, session_id: 'ui', task_id: 't', complete: false, sessions: [card('plan', { shape: 'plan', todos: { rev: 1, total: 1, done: 0, items: [todo(1, card('t1', { children: [card('below')], todos: { rev: 1, total: 0, done: 0, items: [] } }))] } })] })!
  equal([cards.sessions[0].children, cards.sessions[0].todos!.items[0].card!.children, cards.sessions[0].todos!.items[0].card!.todos], [undefined, undefined, null])
})

Deno.test('only the topmost interrupted sessions are resumed; a finished card offers nothing; a card paused with its Goal waits for the Goal', () => {
  const work = (id: string, extra: Partial<WorkCard> = {}): WorkCard => ({ ...card(id), ...extra } as WorkCard)
  const interrupted = { state: 'paused', paused: 'interrupted' } as const
  const tree = [
    work('plan', { ...interrupted, children: [work('under', interrupted)] }),
    work('running', { children: [work('deep', interrupted)], todos: { rev: 1, total: 1, done: 0, items: [{ ...todo(1), card: work('todo', interrupted) }] } }),
    work('paused-by-user', { state: 'paused', paused: 'user', children: [work('ignored', { state: 'paused', paused: 'parent' })] }),
  ]
  equal(interruptedRoots(tree).map(item => item.session_id), ['plan', 'todo', 'deep'])
  equal(workControls(work('a')), { pause: true, resume: false, stop: true })
  equal(workControls(work('a', { state: 'needs_input' })), { pause: true, resume: false, stop: true })
  equal(workControls(work('a', { state: 'paused', paused: 'user' })), { pause: false, resume: true, stop: true })
  equal(workControls(work('a', interrupted)), { pause: false, resume: true, stop: true })
  equal(workControls(work('a', { state: 'paused', paused: 'parent' })), { pause: false, resume: true, stop: true })
  equal(workControls(work('a', { state: 'paused', paused: 'goal' })), { pause: false, resume: false, stop: true })
  equal(workControls(work('a', { state: 'paused' })), { pause: false, resume: true, stop: true })
  for (const state of ['succeeded', 'failed', 'stopped'] as const) equal(workControls(work('a', { state })), { pause: false, resume: false, stop: false })
})

Deno.test('a paused card says why it is paused; the task area names an interruption and localizes the runtime\'s status lines', () => {
  equal((['user', 'parent', 'interrupted', 'goal'] as const).map(source => localizeLine('whatever', 'paused', source)), [{ key: 'messagehub.work.line.pausedUser' }, { key: 'messagehub.work.line.pausedParent' }, { key: 'messagehub.work.line.interrupted' }, { key: 'messagehub.work.line.pausedGoal' }])
  equal(localizeLine('', 'paused'), { key: 'messagehub.work.line.paused' })
  const node = (extra: Record<string, unknown>) => ({ taskId: 't', name: 'Turn', rootId: 't', phase: 'Running', revision: 1, createdAt: 1, updatedAt: 2, ...extra }) as Parameters<typeof taskNodeState>[0]
  equal(taskNodeState(node({ phase: 'Paused', message: 'interrupted: the host restarted' })).label.key, 'messagehub.task.state.interrupted')
  equal(taskNodeState(node({ phase: 'Paused', message: 'paused: by owner' })).label.key, 'messagehub.task.state.paused')
  const view = (extra: Record<string, unknown>) => taskAreaView({ status: 'ready', stale: false, task: { ...node(extra), schemaId: 'opendan.agent_turn/v1' } } as Parameters<typeof taskAreaView>[0], undefined, { edited: false, createdAt: 1 }, 10)
  equal(view({ message: 'working (analyze_intent)' })?.activity, { key: 'messagehub.work.line.working' })
  equal(view({ message: 'running shell' })?.activity, { key: 'messagehub.work.line.running', vars: { tool: 'shell' } })
  equal(view({ message: 'Checking the message projection' })?.activity, 'Checking the message projection')
  // The runner's own words for a state the label already says are not repeated.
  equal(view({ phase: 'Paused', message: 'interrupted: the host restarted' })?.activity, undefined)
  equal(view({ phase: 'Terminal', outcome: 'Canceled', message: 'stopped' })?.activity, undefined)
  equal(view({ phase: 'Terminal', outcome: 'Failed', message: 'Deploy step failed' })?.activity, 'Deploy step failed')
})

Deno.test('ui.conversation: state in the panel\'s words, the generation number and where the timeline breaks', () => {
  const view = (extra: Partial<UiSessionView> = {}): UiSessionView => ({ session_id: 'ui-2', run_state: 'waiting', paused: null, needs_input: true, one_line_status: '', unfinished_works: 0, prompt: { frozen_at_ms: 1, current: true, changed: [] }, ...extra })
  equal(agentConversationState(null, false), 'fresh')
  equal(agentConversationState(view({ paused: 'interrupted' }), true), 'interrupted')
  equal(agentConversationState(view({ paused: 'user' }), false), 'paused')
  equal(agentConversationState(view(), true), 'answering')
  equal(agentConversationState(view({ open_turn: { turn: 3, task_id: 't' }, needs_input: false }), false), 'working')
  equal(agentConversationState(view({ run_state: 'running', needs_input: false }), false), 'working')
  equal(agentConversationState(view({ open_turn: { turn: 3, task_id: 't' } }), false), 'idle')
  equal(agentConversationState(view(), false), 'idle')

  const parsed = parseUiConversation({ current: { session_id: 'ui-2', run_state: 'running', paused: 'goal', needs_input: false, one_line_status: 'working', open_turn: { turn: 4, task_id: 'task-4' }, unfinished_works: 2, route_key: 'inbox-1', prompt: { frozen_at_ms: 9, current: false, changed: ['role.md', 7] } }, generations: [{ session_id: 'ui-2', created_at_ms: 200 }, { session_id: 'ui-1', created_at_ms: 100, finished_at_ms: 180, outcome: 'stopped' }] })!
  equal(parsed.current, { session_id: 'ui-2', run_state: 'running', paused: 'goal', needs_input: false, one_line_status: 'working', open_turn: { turn: 4, task_id: 'task-4' }, unfinished_works: 2, prompt: { frozen_at_ms: 9, current: false, changed: ['role.md'] } })
  equal(parsed.generations.map(item => item.session_id), ['ui-1', 'ui-2'])
  equal([generationNumber(parsed), generationBreaks(parsed), parsed.route_key], [2, [180], 'inbox-1'])
  const archived = parseUiConversation({ current: null, generations: [{ session_id: 'ui-1', created_at_ms: 100, finished_at_ms: 180 }] })!
  equal([archived.current, generationNumber(archived), generationBreaks(archived), archived.route_key], [null, 2, [180], null])
  equal(parseUiConversation({ current: { paused: true } , generations: [] })?.current, null)
  equal(parseUiConversation({ current: { session_id: 'ui-3', open_turn: { turn: 2, task_id: null } }, generations: [] })?.current?.open_turn, { turn: 2, task_id: null })
  equal(parseUiConversation({ current: null }), null)
})

Deno.test('summary, deliveries and sizes parse defensively', () => {
  equal(parseAgentSummary({ sessions: { working: 2, waiting: 1, failed: 0, interrupted: 3, total: 9 }, memory: { perceptions: 5, pending: 2, cognitions: 4, last_consolidation_at_ms: 77 }, goals: { active: 1 }, skills: { work: 13, plan: 3, goal: 0, disabled: 1 }, runtimes: { enabled: 2, unavailable: 1 } }), { sessions: { working: 2, waiting: 1, failed: 0, interrupted: 3, total: 9 }, memory: { perceptions: 5, pending: 2, cognitions: 4, last_consolidation_at_ms: 77 }, goals: { active: 1 }, skills: { work: 13, plan: 3, goal: 0, disabled: 1 }, runtimes: { enabled: 2, unavailable: 1 } })
  equal(parseAgentSummary({ sessions: {} })?.memory, { perceptions: 0, pending: 0, cognitions: 0, last_consolidation_at_ms: null })
  equal(parseAgentSummary({}), null)
  const page = parseDeliveries({ entries: [{ at_ms: 5, name: 'a.md', obj_id: 'cyfile:1', url: 'obj://bucket/b/k/a.md', mime: 'text/markdown', size: 10, session_id: 'w1', ui_session_id: 'ui', route_key: 'r', source: 'reply' }, { at_ms: 4, obj_id: 'cyfile:2', url: 'obj://bucket/b/k/b.bin', session_id: 'w2' }, { obj_id: 'cyfile:3', session_id: 'w3' }, { obj_id: 'cyfile:4' }], next_before: 4 })!
  equal(page, { entries: [{ at_ms: 5, name: 'a.md', obj_id: 'cyfile:1', url: 'obj://bucket/b/k/a.md', mime: 'text/markdown', size: 10, session_id: 'w1', ui_session_id: 'ui', route_key: 'r', source: 'reply' }, { at_ms: 4, name: 'b.bin', obj_id: 'cyfile:2', url: 'obj://bucket/b/k/b.bin', session_id: 'w2', source: 'report' }, { at_ms: 0, name: 'cyfile:3', obj_id: 'cyfile:3', session_id: 'w3', source: 'report' }], next_before: 4 })
  equal(parseDeliveries({}), null)
  equal([0, 812, 1024, 4_300, 1_363_149, 5 * 1024 ** 3].map(formatSize), ['0 B', '812 B', '1 KB', '4.2 KB', '1.3 MB', '5 GB'])
})

Deno.test('generation breaks are placed in the rows by time', () => {
  const rows = [message('a', SELF, { created_at_ms: 100 }), message('b', AGENT, { created_at_ms: 200 }), message('c', SELF, { created_at_ms: 300 })]
  equal(rowsWithBreaks(rows, [], 'New', 's').map(row => row.ui_message_id), ['a', 'b', 'c'])
  equal(rowsWithBreaks(rows, [150, 400], 'New', 's').map(row => row.ui_message_id), ['a', 'generation-break:150', 'b', 'c', 'generation-break:400'])
  equal(rowsWithBreaks([], [150], 'New', 's'), [])
  const pill = rowsWithBreaks(rows, [250], 'New', 's')[2]
  equal([pill.ui_item_kind, pill.ui_status_type, pill.content.content, pill.created_at_ms], ['status', 'info', 'New', 250])
})
