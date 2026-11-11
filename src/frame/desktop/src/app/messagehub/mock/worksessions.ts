import { parseWorkCards, type WorkCard, type WorkCards, type WorkTodo, type WorkUsage } from '../conversation/worksessions/model'
import type { WorkCardsSource } from '../conversation/worksessions/source'
import type { WorkSessionInfo } from '../conversation/worklog/model'
import { MOCK_AGENT_TASKS } from './data'

/** The UI session the seeded work Turns belong to (the `session_id` of their tasks, `mock/tasks.ts`). */
export const MOCK_WORK_UI_SESSION = 'ui-session-work'
const AGENT = 'did:buckyos:agent:codeassistant'

const usage = (input: number, output: number, rounds: number, model = 'gpt-6.1-sol@cliproxy'): WorkUsage => ({ input_tokens: input, output_tokens: output, rounds, models: rounds ? [{ model, input_tokens: input, output_tokens: output }] : [] })
const sum = (...parts: WorkUsage[]): WorkUsage => {
  const models = new Map<string, { model: string; input_tokens: number; output_tokens: number }>()
  for (const part of parts) for (const item of part.models) {
    const known = models.get(item.model) ?? { model: item.model, input_tokens: 0, output_tokens: 0 }
    models.set(item.model, { model: item.model, input_tokens: known.input_tokens + item.input_tokens, output_tokens: known.output_tokens + item.output_tokens })
  }
  return { input_tokens: parts.reduce((total, part) => total + part.input_tokens, 0), output_tokens: parts.reduce((total, part) => total + part.output_tokens, 0), rounds: parts.reduce((total, part) => total + part.rounds, 0), models: [...models.values()] }
}

function card(seed: Partial<WorkCard> & Pick<WorkCard, 'session_id' | 'title' | 'state' | 'created_at_ms'>): WorkCard {
  return { shape: 'simple', finished_at_ms: null, line: '', usage: usage(0, 0, 0), model: null, subtree_usage: null, todos: null, ...seed }
}

function todo(seed: Partial<WorkTodo> & Pick<WorkTodo, 'todo_id' | 'seq' | 'title' | 'status'>): WorkTodo {
  return { mode: 'serial', attempt: seed.card ? 1 : 0, blocked_by: [], block_reason: null, card: null, ...seed }
}

function withTodos(plan: WorkCard, items: WorkTodo[]): WorkCard {
  return {
    ...plan,
    shape: 'plan',
    subtree_usage: sum(plan.usage, ...items.map(item => item.card?.usage).filter((value): value is WorkUsage => !!value)),
    todos: { rev: items.length + 2, total: items.filter(item => item.status !== 'cancelled').length, done: items.filter(item => item.status === 'completed').length, items },
  }
}

/**
 * Three seeded Turns of `session-coder-work`: a simple work session that is
 * running; a plan that is running (two Todos bound to sessions, two not,
 * one cancelled); a plan that finished with many Todos.
 */
function seedCards(now: number): Map<string, WorkCards> {
  const minutes = (value: number) => now - value * 60_000
  const page = (taskId: string, sessions: WorkCard[], complete = false): WorkCards => ({ agent_did: AGENT, session_id: MOCK_WORK_UI_SESSION, task_id: taskId, complete, sessions })
  const builds = card({ session_id: 'work-builds', title: 'Run both slow builds', state: 'running', created_at_ms: minutes(4.2), line: 'Watching the two cargo builds', usage: usage(86_400, 9_100, 12), model: 'gpt-6.1-sol@cliproxy' })
  const t01 = card({ session_id: 'work-docs-t01', title: 'Survey the current state', state: 'succeeded', created_at_ms: minutes(6), finished_at_ms: minutes(4.95), line: 'Found 3 places that disagree with the source', usage: usage(21_000, 2_300, 5), model: 'gpt-6.1-sol@cliproxy' })
  const t02 = card({ session_id: 'work-docs-t02', title: 'Rewrite §3', state: 'running', created_at_ms: minutes(4.6), line: 'Editing doc/llm_context/LLM Context Design.md', usage: usage(40_000, 5_000, 9), model: 'gpt-6.1-sol@cliproxy', children: [
    card({ session_id: 'work-docs-t02-survey', shape: 'explore', title: 'Find every caller of the context API', state: 'running', created_at_ms: minutes(3.1), line: 'running grep', usage: usage(6_200, 480, 3), model: 'gpt-6.1-mini@cliproxy' }),
  ] })
  const plan = withTodos(card({ session_id: 'plan-docs', title: 'Rewrite the llm_context design doc', state: 'running', created_at_ms: minutes(6.3), line: 'waiting for 1 sub session(s)', usage: usage(25_000, 1_800, 4), model: 'gpt-6.1-sol@cliproxy' }), [
    todo({ todo_id: 'T01', seq: 1, title: 'Survey the current state', status: 'completed', card: t01 }),
    todo({ todo_id: 'T02', seq: 2, title: 'Rewrite §3', status: 'running', card: t02 }),
    todo({ todo_id: 'T03', seq: 3, title: 'Add the tests', status: 'pending' }),
    todo({ todo_id: 'T04', seq: 4, title: 'Acceptance', status: 'blocked', blocked_by: ['T03'], block_reason: 'runs after T03' }),
    todo({ todo_id: 'T05', seq: 5, title: 'Translate the doc', status: 'cancelled' }),
  ])
  const done = withTodos(card({ session_id: 'plan-notes', title: 'Audit the 2.3 release notes', state: 'succeeded', created_at_ms: minutes(70), finished_at_ms: minutes(52), line: 'All 14 sections checked; 2 fixes committed', usage: usage(30_000, 3_200, 6), model: 'llm.plan' }),
    Array.from({ length: 14 }, (_, index) => {
      const seq = index + 1
      const id = `T${String(seq).padStart(2, '0')}`
      return todo({ todo_id: id, seq, title: `Check section ${seq}`, status: 'completed', attempt: seq === 3 ? 2 : 1, card: card({ session_id: `work-notes-${id.toLowerCase()}`, title: `Check section ${seq}`, state: 'succeeded', created_at_ms: minutes(69 - seq), finished_at_ms: minutes(68 - seq), line: `Section ${seq} is consistent`, usage: usage(4_000 + seq * 100, 400 + seq * 10, 2), model: seq === 7 ? 'gpt-6.1-mini@cliproxy' : 'gpt-6.1-sol@cliproxy' }) })
    }))
  return new Map([
    [MOCK_AGENT_TASKS.workSimple, page(MOCK_AGENT_TASKS.workSimple, [builds])],
    [MOCK_AGENT_TASKS.workPlan, page(MOCK_AGENT_TASKS.workPlan, [plan])],
    [MOCK_AGENT_TASKS.workDone, page(MOCK_AGENT_TASKS.workDone, [done], true)],
  ])
}

let cards: Map<string, WorkCards> | null = null
const store = () => cards ??= seedCards(Date.now())

/** Scripted `session.work_cards` of mock mode; tests change it through `window.__messageHubMockWorkCards`. */
export const mockWorkCards = {
  /** Reads served so far, by UI Turn task id. */
  reads: {} as Record<string, number>,
  /** The viewer may not read the UI session (an observer, a group member); a test sets it before the page loads with the `buckyos.messagehub.mock.workCardsDenied` localStorage flag. */
  denied: (() => { try { return localStorage.getItem('buckyos.messagehub.mock.workCardsDenied') === '1' } catch { return false } })(),
  offline: false,
  get(taskId: string): WorkCards | undefined { return store().get(taskId) },
  set(taskId: string, page: WorkCards) { store().set(taskId, page) },
  /** Changes one card (a Todo card when `todoId` is given) of a Turn. */
  update(taskId: string, sessionId: string, patch: Partial<WorkCard>, todoId?: string) {
    const page = store().get(taskId)
    if (!page) return
    page.sessions = page.sessions.map(item => {
      if (item.session_id !== sessionId) return item
      if (!todoId) return { ...item, ...patch }
      return { ...item, todos: item.todos && { ...item.todos, rev: item.todos.rev + 1, items: item.todos.items.map(entry => entry.todo_id === todoId && entry.card ? { ...entry, card: { ...entry.card, ...patch } } : entry) } }
    })
  },
  complete(taskId: string, complete = true) { const page = store().get(taskId); if (page) page.complete = complete },
}

/** The live cards of every Turn of a UI session, oldest Turn first (the mock Agent changes them in place). */
export function mockSessionCards(uiSessionId: string): WorkCard[] {
  return [...store().values()].filter(page => page.session_id === uiSessionId).flatMap(page => page.sessions).sort((a, b) => a.created_at_ms - b.created_at_ms)
}

export const mockWorkCardsSource: WorkCardsSource = async ({ agentDid, sessionId, taskId }) => {
  mockWorkCards.reads[taskId] = (mockWorkCards.reads[taskId] ?? 0) + 1
  await new Promise(resolve => setTimeout(resolve, 20))
  if (mockWorkCards.offline) throw Error('Mock work cards offline')
  if (mockWorkCards.denied) throw Object.assign(Error('permission denied'), { code: 'NoPermission' })
  const page = store().get(taskId)
  if (!page || page.agent_did !== agentDid || page.session_id !== sessionId) return { agent_did: agentDid, session_id: sessionId, task_id: taskId, complete: false, sessions: [] }
  // As the service answers it: a Todo's card carries nothing under it (the tree is `session.work_tree`).
  return parseWorkCards(structuredClone(page)) ?? { agent_did: agentDid, session_id: sessionId, task_id: taskId, complete: false, sessions: [] }
}

function findCard(sessionId: string): WorkCard | undefined {
  const search = (items: readonly WorkCard[]): WorkCard | undefined => {
    for (const item of items) {
      if (item.session_id === sessionId) return item
      const below = search([...(item.todos?.items ?? []).flatMap(entry => entry.card ? [entry.card] : []), ...(item.children ?? [])])
      if (below) return below
    }
    return undefined
  }
  for (const page of store().values()) { const found = search(page.sessions); if (found) return found }
  return undefined
}

/** Whether a seeded session ended (an unknown one counts as ended: its log is complete). */
export function mockWorkSessionFinished(sessionId: string): boolean {
  const found = findCard(sessionId)
  return !found || found.state === 'succeeded' || found.state === 'failed' || found.state === 'stopped'
}

/** `session.read` of a seeded work session (the head of its work log). */
export async function mockReadWorkSession(_agentDid: string, sessionId: string): Promise<WorkSessionInfo> {
  await new Promise(resolve => setTimeout(resolve, 20))
  const found = findCard(sessionId) ?? card({ session_id: sessionId, title: sessionId, state: 'succeeded', created_at_ms: Date.now() - 600_000, finished_at_ms: Date.now() - 300_000, line: 'Done' })
  const finished = found.state === 'succeeded' || found.state === 'failed' || found.state === 'stopped'
  const objective = `${found.title}.\n\nWork in the shared workspace. Keep the existing structure, verify every claim against the source, and report what changed with file paths. Ask before deleting anything.`
  return {
    entry: { session_id: sessionId, objective, title: found.title, status: { run_state: finished ? 'finished' : 'running', outcome: finished ? found.state : null } },
    state: { finished_at_ms: found.finished_at_ms },
    report: finished ? `## Done\n\n${found.line}.` : null,
    config: {
      session: { objective, title: found.title, created_at_ms: found.created_at_ms, class: found.shape === 'plan' ? 'plan_simple' : 'work' },
      runtime: { frozen: { runtime_id: 'private', connector: { type: 'native' } } },
      workspace: { workspace_id: 'ws-buckyos', access: 'read_write' },
      behavior: found.shape === 'plan' ? 'plan_simple' : 'work_simple',
    },
  }
}
