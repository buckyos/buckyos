import type { AgentPanelSource } from '../agentpanel/source'
import { parseUiConversation, type AgentSummary, type Delivery, type UiGeneration, type UiSessionView } from '../agentpanel/model'
import { getTaskWatchSource } from '../conversation/tasks'
import { flattenWork, isWorkFinished, workChildren, type WorkCard, type WorkNode } from '../conversation/worksessions/model'
import { MOCK_AGENT_TASKS, MOCK_SELF_DID } from './data'
import { MockTaskSource } from './tasks'
import { MOCK_WORK_UI_SESSION, mockSessionCards } from './worksessions'

const AGENT = 'did:buckyos:agent:codeassistant'
const ARTIFACTS = 'obj://bucket/user/codeassistant/self/artifacts'

interface MockConversation { uiSession: string | null; generations: UiGeneration[]; paused: UiSessionView['paused']; openTask?: { turn: number; task_id: string }; prompt: UiSessionView['prompt'] }

function seedConversations(now: number): Record<string, MockConversation> {
  const minutes = (value: number) => now - value * 60_000
  return {
    'session-coder-work': {
      uiSession: MOCK_WORK_UI_SESSION,
      generations: [
        { session_id: 'ui-session-work-0', created_at_ms: minutes(600), finished_at_ms: minutes(75), outcome: 'stopped' },
        { session_id: MOCK_WORK_UI_SESSION, created_at_ms: minutes(71) },
      ],
      paused: null,
      openTask: { turn: 8, task_id: MOCK_AGENT_TASKS.workAnalyze },
      prompt: { frozen_at_ms: minutes(71), current: false, changed: ['role.md', 'cmd_skills/ui_core'] },
    },
    'session-coder-ask': {
      uiSession: 'ui-session-1',
      generations: [{ session_id: 'ui-session-1', created_at_ms: minutes(200) }],
      paused: null,
      openTask: { turn: 3, task_id: MOCK_AGENT_TASKS.asking },
      prompt: { frozen_at_ms: minutes(200), current: true, changed: [] },
    },
  }
}

function seedDeliveries(now: number): Delivery[] {
  const minutes = (value: number) => now - value * 60_000
  const entry = (at: number, name: string, id: string, session: string, source: Delivery['source'], mime: string, size: number, own = true): Delivery => ({ at_ms: minutes(at), name, obj_id: `cyfile:mock-${id}`, url: `${ARTIFACTS}/${id}/${name}`, mime, size, session_id: session, ui_session_id: own ? MOCK_WORK_UI_SESSION : 'ui-session-1', route_key: own ? 'mock-route:session-coder-work' : 'mock-route:session-coder-task', source })
  return [
    entry(5, 'llm-context-survey.md', 'survey', 'work-docs-t01', 'report', 'text/markdown', 4_200),
    entry(45, 'build-failures.md', 'failures', 'ui-session-1', 'reply', 'text/markdown', 2_100, false),
    entry(52, 'release-notes-audit.pdf', 'audit', 'plan-notes', 'report', 'application/pdf', 182_000),
    entry(300, 'deploy-diagram.png', 'diagram', 'work-deploy', 'report', 'image/png', 96_000, false),
    ...Array.from({ length: 24 }, (_, index) => entry(400 + index * 30, `nightly-report-${String(index + 1).padStart(2, '0')}.md`, `nightly-${index + 1}`, 'work-nightly', 'report', 'text/markdown', 1_000 + index * 10, false)),
  ]
}

let conversations: Record<string, MockConversation> | null = null
let deliveries: Delivery[] | null = null
const state = () => conversations ??= seedConversations(Date.now())
const delivered = () => deliveries ??= seedDeliveries(Date.now())
const tasks = () => { const source = getTaskWatchSource(); return source instanceof MockTaskSource ? source : null }

function conversationOf(sessionId: string): MockConversation {
  return state()[sessionId] ??= { uiSession: `ui-${sessionId}`, generations: [{ session_id: `ui-${sessionId}`, created_at_ms: Date.now() - 3_600_000 }], paused: null, prompt: { frozen_at_ms: Date.now() - 3_600_000, current: true, changed: [] } }
}
const conversationBySid = (sid: string) => Object.values(state()).find(item => item.uiSession === sid)

function unfinished(cards: readonly WorkCard[]): WorkNode[] {
  return cards.filter(card => !isWorkFinished(card.state)).map(card => ({
    ...card,
    todos: card.todos ? { ...card.todos, items: card.todos.items.map(todo => ({ ...todo, card: todo.card ? unfinished([todo.card])[0] ?? todo.card : null })) } : null,
    children: unfinished(card.children ?? []),
  }))
}

function findWork(sid: string): WorkCard | undefined {
  return flattenWork(Object.values(state()).flatMap(item => item.uiSession ? mockSessionCards(item.uiSession) : [])).find(card => card.session_id === sid)
}

const SUMMARY: AgentSummary = {
  sessions: { working: 3, waiting: 1, failed: 0, interrupted: 0, total: 42 },
  memory: { perceptions: 128, pending: 6, cognitions: 37, last_consolidation_at_ms: Date.now() - 5 * 3_600_000 },
  goals: { active: 2 },
  skills: { work: 13, plan: 3, goal: 0, disabled: 1 },
  runtimes: { enabled: 2, unavailable: 0 },
}

/** The open Turn ends as stopped: its task is canceled and its placeholder says so. */
function endTurn(sessionId: string, conversation: MockConversation, text: string) {
  const task = conversation.openTask
  if (!task) return
  tasks()?.update(task.task_id, { phase: 'Terminal', outcome: 'Canceled', message: 'stopped' })
  conversation.openTask = undefined
  conversation.paused = null
  mockAgentPanel.onTurnEnded?.(sessionId, task.task_id, text)
}

/** Scripted OpenDAN service of the seeded Agent; tests drive it through `window.__messageHubMockAgent`. */
export const mockAgentPanel = {
  /** Every call served so far, in order. */
  calls: [] as Array<{ method: string; params: Record<string, unknown> }>,
  /** The viewer is not the Agent's owner (or the Agent is offline): every call fails. */
  denied: (() => { try { return localStorage.getItem('buckyos.messagehub.mock.agentDenied') === '1' } catch { return false } })(),
  summary: structuredClone(SUMMARY),
  /** Set by the mock store: the Agent replaces the placeholder of a Turn it stopped (`text`), as it does when a Turn ends. */
  onTurnEnded: null as ((sessionId: string, taskId: string, text: string) => void) | null,
  conversation(sessionId: string) { return conversationOf(sessionId) },
  work(sid: string) { return findWork(sid) },
  deliveries() { return delivered() },
  /** The host restarted while the session worked: the Turn and every unfinished work session under it stop as interrupted. */
  interrupt(sessionId: string) {
    const conversation = conversationOf(sessionId)
    for (const card of flattenWork(conversation.uiSession ? mockSessionCards(conversation.uiSession) : [])) if (!isWorkFinished(card.state) && card.state !== 'paused') Object.assign(card, { state: 'paused', paused: 'interrupted' })
    if (!conversation.openTask) return
    conversation.paused = 'interrupted'
    tasks()?.update(conversation.openTask.task_id, { phase: 'Paused', message: 'interrupted: the host restarted' })
  },
  /** The owner retracted an input of the open Turn: the bridge stops the Turn and the Agent marks its placeholder. */
  retract(sessionId: string) {
    const conversation = state()[sessionId]
    if (!conversation?.openTask) return
    endTurn(sessionId, conversation, 'Stopped.')
  },
}
if (import.meta.env.DEV && typeof window !== 'undefined') Object.assign(window, { __messageHubMockAgent: mockAgentPanel })

async function served(method: string, params: Record<string, unknown>) {
  mockAgentPanel.calls.push({ method, params })
  await new Promise(resolve => setTimeout(resolve, 20))
  if (mockAgentPanel.denied) throw Object.assign(Error('permission denied'), { code: 'NoPermission' })
}

function cascade(card: WorkCard, change: (card: WorkCard, root: boolean) => void) {
  change(card, true)
  for (const child of flattenWork(workChildren(card))) change(child, false)
}

export const mockAgentPanelSource: AgentPanelSource = {
  async summary(agentDid) {
    await served('home.summary', {})
    if (agentDid !== AGENT) throw Error('Agent service unavailable')
    const interrupted = Object.values(state()).reduce((sum, item) => sum + (item.paused === 'interrupted' ? 1 : 0) + flattenWork(item.uiSession ? mockSessionCards(item.uiSession) : []).filter(card => card.paused === 'interrupted').length, 0)
    return structuredClone({ ...mockAgentPanel.summary, sessions: { ...mockAgentPanel.summary.sessions, interrupted } })
  },
  async conversation(agentDid, peerDid, sessionId) {
    await served('ui.conversation', { peer_did: peerDid, session_id: sessionId })
    if (agentDid !== AGENT || peerDid !== MOCK_SELF_DID) throw Error('Agent service unavailable')
    const conversation = conversationOf(sessionId)
    const works = conversation.uiSession ? flattenWork(mockSessionCards(conversation.uiSession)).filter(card => !isWorkFinished(card.state)).length : 0
    const current: (UiSessionView & { route_key: string }) | null = conversation.uiSession ? {
      session_id: conversation.uiSession,
      run_state: conversation.openTask ? 'running' : 'waiting',
      paused: conversation.paused,
      needs_input: !conversation.openTask,
      one_line_status: conversation.openTask ? 'working (analyze_intent)' : '',
      route_key: `mock-route:${sessionId}`,
      ...(conversation.openTask ? { open_turn: conversation.openTask } : {}),
      unfinished_works: works,
      prompt: conversation.prompt,
    } : null
    // Parsed as the service's answer is: `route_key` comes from `current`.
    const answer = parseUiConversation(structuredClone({ current, generations: conversation.generations }))
    if (!answer) throw Error('Agent conversation unavailable')
    return answer
  },
  async workTree(agentDid, sid) {
    await served('session.work_tree', { sid, unfinished_only: true })
    const sessions = unfinished(mockSessionCards(sid))
    return structuredClone({ agent_did: agentDid, session_id: sid, complete: sessions.length === 0, sessions })
  },
  async deliveries(_agentDid, { limit, before, routeKey }) {
    await served('deliveries.recent', { limit, ...(before !== undefined ? { before } : {}), ...(routeKey ? { route_key: routeKey } : {}) })
    const matching = delivered().filter(item => (!routeKey || item.route_key === routeKey) && (before === undefined || item.at_ms < before)).sort((a, b) => b.at_ms - a.at_ms)
    const entries = matching.slice(0, limit)
    return structuredClone({ entries, ...(matching.length > limit ? { next_before: entries[entries.length - 1].at_ms } : {}) })
  },
  async pause(_agentDid, sid) {
    await served('session.pause', { sid })
    const card = findWork(sid)
    if (!card || isWorkFinished(card.state)) throw Error('session is finished')
    cascade(card, (item, root) => { if (!isWorkFinished(item.state) && item.state !== 'paused') Object.assign(item, { state: 'paused', paused: root ? 'user' : 'parent' }) })
  },
  async resume(_agentDid, sid) {
    await served('session.resume', { sid })
    const conversation = conversationBySid(sid)
    if (conversation) {
      if (!conversation.paused) return
      const source = conversation.paused
      conversation.paused = null
      if (conversation.openTask) tasks()?.update(conversation.openTask.task_id, { phase: 'Running', message: 'working (analyze_intent)' })
      for (const card of flattenWork(mockSessionCards(sid))) if (card.state === 'paused' && card.paused === source) Object.assign(card, { state: 'running', paused: undefined })
      return
    }
    const card = findWork(sid)
    if (!card || card.state !== 'paused') return
    const source = card.paused
    cascade(card, (item, root) => { if (item.state === 'paused' && (root || item.paused === 'parent' || item.paused === source)) Object.assign(item, { state: 'running', paused: undefined }) })
  },
  async stop(_agentDid, sid, scope) {
    await served('session.stop', { sid, scope })
    const conversation = conversationBySid(sid)
    if (conversation) {
      if (scope === 'session') throw Error('a UI session cannot be stopped: start a new conversation to clear its context')
      if (!conversation.openTask) throw Error('no Turn in progress')
      const sessionId = Object.keys(state()).find(key => state()[key] === conversation)
      if (sessionId) endTurn(sessionId, conversation, 'Stopped.')
      return
    }
    const card = findWork(sid)
    if (!card) throw Error(`session ${sid} not found`)
    cascade(card, item => { if (!isWorkFinished(item.state)) Object.assign(item, { state: 'stopped', paused: undefined, finished_at_ms: Date.now() }) })
  },
  async rotate(_agentDid, sid) {
    await served('ui.rotate', { sid })
    const conversation = conversationBySid(sid)
    if (!conversation) throw Error(`session ${sid} is not a UI session of this agent`)
    const works = flattenWork(mockSessionCards(sid)).filter(card => !isWorkFinished(card.state)).length
    if (works > 0) throw Error(`unfinished_works: ${works}`)
    const generation = conversation.generations.find(item => item.session_id === sid)
    if (generation) Object.assign(generation, { finished_at_ms: Date.now(), outcome: 'stopped' })
    conversation.uiSession = null
    conversation.openTask = undefined
    conversation.paused = null
  },
}
