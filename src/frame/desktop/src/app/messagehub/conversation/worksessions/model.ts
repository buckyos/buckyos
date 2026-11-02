import type { TaskText, TaskTone } from '../tasks/taskView'

export type WorkState = 'queued' | 'running' | 'needs_input' | 'paused' | 'succeeded' | 'failed' | 'stopped'
export type WorkShape = 'simple' | 'plan' | 'explore'
export type TodoStatus = 'pending' | 'running' | 'blocked' | 'completed' | 'failed' | 'timeout' | 'stopped' | 'cancelled'

export interface WorkModelUsage { model: string; input_tokens: number; output_tokens: number }
export interface WorkUsage { input_tokens: number; output_tokens: number; rounds: number; models: WorkModelUsage[] }
export interface WorkTodo {
  todo_id: string
  seq: number
  title: string
  status: TodoStatus | string
  mode: string
  attempt: number
  blocked_by: string[]
  block_reason: string | null
  card: WorkCard | null
}
export interface WorkTodos { rev: number; total: number; done: number; items: WorkTodo[] }
/** One work session as `session.work_cards` projects it (TODO §3.5). */
export interface WorkCard {
  session_id: string
  shape: WorkShape
  title: string
  created_at_ms: number
  finished_at_ms: number | null
  state: WorkState
  line: string
  usage: WorkUsage
  model: string | null
  subtree_usage: WorkUsage | null
  todos: WorkTodos | null
}
export interface WorkCards { agent_did: string; session_id: string; task_id: string; complete: boolean; sessions: WorkCard[] }

const STATES: readonly WorkState[] = ['queued', 'running', 'needs_input', 'paused', 'succeeded', 'failed', 'stopped']
const SHAPES: readonly WorkShape[] = ['simple', 'plan', 'explore']
const ENDED_TODOS: readonly string[] = ['completed', 'failed', 'timeout', 'stopped', 'cancelled']

function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : null
}
const count = (value: unknown) => typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : 0
const text = (value: unknown) => typeof value === 'string' ? value : ''

function parseUsage(value: unknown): WorkUsage | null {
  const usage = record(value)
  if (!usage) return null
  const models = Array.isArray(usage.models) ? usage.models.map(record).filter(item => !!item && typeof item.model === 'string').map(item => ({ model: item!.model as string, input_tokens: count(item!.input_tokens), output_tokens: count(item!.output_tokens) })) : []
  return { input_tokens: count(usage.input_tokens), output_tokens: count(usage.output_tokens), rounds: count(usage.rounds), models }
}

function parseCard(value: unknown, nested: boolean): WorkCard | null {
  const card = record(value)
  if (!card || typeof card.session_id !== 'string' || !card.session_id || !STATES.includes(card.state as WorkState)) return null
  const todos = !nested ? record(card.todos) : null
  return {
    session_id: card.session_id,
    shape: SHAPES.includes(card.shape as WorkShape) ? card.shape as WorkShape : 'simple',
    title: text(card.title),
    created_at_ms: count(card.created_at_ms),
    finished_at_ms: typeof card.finished_at_ms === 'number' ? card.finished_at_ms : null,
    state: card.state as WorkState,
    line: text(card.line),
    usage: parseUsage(card.usage) ?? { input_tokens: 0, output_tokens: 0, rounds: 0, models: [] },
    model: typeof card.model === 'string' ? card.model : null,
    subtree_usage: parseUsage(card.subtree_usage),
    todos: todos && Array.isArray(todos.items) ? {
      rev: count(todos.rev), total: count(todos.total), done: count(todos.done),
      items: todos.items.map(record).filter(item => !!item && typeof item.todo_id === 'string').map(item => ({
        todo_id: item!.todo_id as string,
        seq: count(item!.seq),
        title: text(item!.title),
        status: text(item!.status),
        mode: text(item!.mode),
        attempt: count(item!.attempt),
        blocked_by: Array.isArray(item!.blocked_by) ? item!.blocked_by.filter(id => typeof id === 'string') : [],
        block_reason: typeof item!.block_reason === 'string' ? item!.block_reason : null,
        card: parseCard(item!.card, true),
      })).sort((a, b) => a.seq - b.seq),
    } : null,
  }
}

/** A `session.work_cards` answer, or null when it is not one (cards that do not parse are dropped). */
export function parseWorkCards(value: unknown): WorkCards | null {
  const page = record(value)
  if (!page || typeof page.agent_did !== 'string' || typeof page.session_id !== 'string' || typeof page.task_id !== 'string' || !Array.isArray(page.sessions)) return null
  const sessions = page.sessions.map(card => parseCard(card, false)).filter((card): card is WorkCard => !!card).sort((a, b) => a.created_at_ms - b.created_at_ms)
  return { agent_did: page.agent_did, session_id: page.session_id, task_id: page.task_id, complete: page.complete === true, sessions }
}

/** The model a provider served (`gpt-6.1-sol@cliproxy` → `gpt-6.1-sol`); request aliases (`llm.*`) and empty names are not shown. */
export function displayModel(model: string | null | undefined): string | undefined {
  const name = (model ?? '').trim().replace(/@[^@]*$/, '').replace(/:[^:]*$/, '').trim()
  return name && !name.startsWith('llm.') ? name : undefined
}

/** `45s` / `3m12s` / `1h05m` / `2d3h`. */
export function formatDuration(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1000))
  if (seconds < 60) return `${seconds}s`
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes}m${String(seconds % 60).padStart(2, '0')}s`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours}h${String(minutes % 60).padStart(2, '0')}m`
  return `${Math.floor(hours / 24)}d${hours % 24}h`
}

/** Below 1000 as is, otherwise `k` / `M` with one decimal (`86k`, `9.1k`, `1.2M`). */
export function formatTokens(value: number): string {
  if (value < 1000) return String(Math.max(0, Math.round(value)))
  const [unit, divisor] = value < 999_950 ? ['k', 1000] : ['M', 1_000_000]
  return `${(value / divisor).toFixed(1).replace(/\.0$/, '')}${unit}`
}

/** Wall-clock time from creation, to the end once the session finished. */
export function workDuration(card: Pick<WorkCard, 'created_at_ms' | 'finished_at_ms'>, now: number): number {
  return (card.finished_at_ms ?? now) - card.created_at_ms
}

export function isWorkFinished(state: WorkState): boolean {
  return state === 'succeeded' || state === 'failed' || state === 'stopped'
}

/** The tokens a card shows: a plan counts itself and all of its Todo sessions. */
export function cardUsage(card: WorkCard): WorkUsage {
  return card.shape === 'plan' && card.subtree_usage ? card.subtree_usage : card.usage
}

export function workTone(state: WorkState): TaskTone {
  switch (state) {
    case 'running': return 'active'
    case 'needs_input':
    case 'queued': return 'waiting'
    case 'paused': return 'paused'
    case 'succeeded': return 'success'
    case 'failed': return 'failed'
    default: return 'canceled'
  }
}

const DEFAULT_LINES: Partial<Record<WorkState, string>> = {
  queued: 'messagehub.work.line.queued',
  needs_input: 'messagehub.work.line.needsInput',
  paused: 'messagehub.work.line.paused',
  stopped: 'messagehub.work.line.stopped',
}

/**
 * Line 2 of a card. A line the model wrote (an intent, a reply) is shown as
 * is; the runtime's known English status lines are localized; an empty line
 * takes the state's default.
 */
export function localizeLine(line: string, state: WorkState): TaskText | string {
  const value = line.trim()
  if (!value) return DEFAULT_LINES[state] ? { key: DEFAULT_LINES[state]! } : ''
  let match = /^waiting for (\d+) task\(s\)$/.exec(value)
  if (match) return { key: 'messagehub.work.line.waitingTasks', vars: { count: Number(match[1]) } }
  match = /^waiting for (\d+) sub session\(s\)$/.exec(value)
  if (match) return { key: 'messagehub.work.line.waitingChildren', vars: { count: Number(match[1]) } }
  if (value === 'waiting for input') return { key: 'messagehub.work.line.needsInput' }
  if (value === 'working' || /^working \([^()]*\)$/.test(value)) return { key: 'messagehub.work.line.working' }
  match = /^running (\S+)$/.exec(value)
  if (match) return { key: 'messagehub.work.line.running', vars: { tool: match[1] } }
  return value
}

export function isTodoEnded(todo: Pick<WorkTodo, 'status'>): boolean {
  return ENDED_TODOS.includes(todo.status)
}

/** The Todo card title: `T02 <title>`, `#2` after it for a retried Todo. */
export function todoTitle(todo: Pick<WorkTodo, 'todo_id' | 'title' | 'attempt'>): string {
  return `${todo.todo_id} ${todo.title}${todo.attempt > 1 ? ` #${todo.attempt}` : ''}`.trim()
}

/** Above this many Todos only the open ones and the last few that ended are listed. */
export const TODO_FOLD_ABOVE = 12
const TODO_RECENT_ENDED = 3

/** The Todos listed by default, in `seq` order, and how many are folded into "N more". */
export function visibleTodos(items: readonly WorkTodo[], showAll: boolean): { items: WorkTodo[]; hidden: number } {
  if (showAll || items.length <= TODO_FOLD_ABOVE) return { items: [...items], hidden: 0 }
  const endedAt = (todo: WorkTodo) => todo.card?.finished_at_ms ?? todo.seq
  const recent = new Set(items.filter(isTodoEnded).sort((a, b) => endedAt(b) - endedAt(a) || b.seq - a.seq).slice(0, TODO_RECENT_ENDED))
  const shown = items.filter(todo => !isTodoEnded(todo) || recent.has(todo))
  return { items: shown, hidden: items.length - shown.length }
}
