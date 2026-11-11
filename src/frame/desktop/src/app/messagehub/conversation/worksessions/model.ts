import type { TaskText, TaskTone } from '../tasks/taskView'

export type WorkState = 'queued' | 'running' | 'needs_input' | 'paused' | 'succeeded' | 'failed' | 'stopped'
export type WorkShape = 'simple' | 'plan' | 'explore'
/** Why a session is paused: the owner paused it, its parent was paused, the host restarted while it worked, or its Goal is dormant. */
export type PauseSource = 'user' | 'parent' | 'interrupted' | 'goal'
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
  /** Only while `state` is `paused`. */
  paused?: PauseSource
  /** `session.work_tree` only: the work sessions it created that are bound to no Todo. */
  children?: WorkCard[]
}
/** A card of `session.work_tree`: its sub sessions are listed under it. */
export type WorkNode = WorkCard & { children: WorkCard[] }
/** The unfinished (or all) work under one session, as `session.work_tree` projects it. */
export interface WorkTree { agent_did: string; session_id: string; complete: boolean; sessions: WorkNode[] }
export interface WorkCards { agent_did: string; session_id: string; task_id: string; complete: boolean; sessions: WorkCard[] }

const STATES: readonly WorkState[] = ['queued', 'running', 'needs_input', 'paused', 'succeeded', 'failed', 'stopped']
const SHAPES: readonly WorkShape[] = ['simple', 'plan', 'explore']
const PAUSE_SOURCES: readonly PauseSource[] = ['user', 'parent', 'interrupted', 'goal']
/** Levels of sessions a work tree lists under the session it is read for. */
export const WORK_TREE_DEPTH = 4
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

/** `tree`: levels of sub sessions still read under this card (a `session.work_tree` card); undefined for a `session.work_cards` card, whose Todo cards carry nothing below them. */
function parseCard(value: unknown, nested: boolean, tree?: number): WorkCard | null {
  const card = record(value)
  if (!card || typeof card.session_id !== 'string' || !card.session_id || !STATES.includes(card.state as WorkState)) return null
  const below = tree === undefined ? undefined : tree - 1
  const todos = tree === undefined ? (!nested ? record(card.todos) : null) : below! > 0 ? record(card.todos) : null
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
        card: parseCard(item!.card, true, below),
      })).sort((a, b) => a.seq - b.seq),
    } : null,
    ...(card.state === 'paused' && PAUSE_SOURCES.includes(card.paused as PauseSource) ? { paused: card.paused as PauseSource } : {}),
    ...(tree === undefined ? {} : { children: below! > 0 && Array.isArray(card.children) ? card.children.map(child => parseCard(child, true, below)).filter((child): child is WorkCard => !!child).sort((a, b) => a.created_at_ms - b.created_at_ms) : [] }),
  }
}

/** A `session.work_tree` answer, or null when it is not one. Cards below `WORK_TREE_DEPTH` levels are dropped. */
export function parseWorkTree(value: unknown): WorkTree | null {
  const page = record(value)
  if (!page || typeof page.agent_did !== 'string' || typeof page.session_id !== 'string' || !Array.isArray(page.sessions)) return null
  const sessions = page.sessions.map(card => parseCard(card, false, WORK_TREE_DEPTH)).filter((card): card is WorkNode => !!card).sort((a, b) => a.created_at_ms - b.created_at_ms)
  return { agent_did: page.agent_did, session_id: page.session_id, complete: page.complete === true, sessions }
}

/** The sessions directly under a card: those bound to its Todos, then the others. */
export function workChildren(card: WorkCard): WorkCard[] {
  return [...(card.todos?.items ?? []).flatMap(todo => todo.card ? [todo.card] : []), ...(card.children ?? [])]
}

/** Every card of a tree, parents before their sub sessions. */
export function flattenWork(cards: readonly WorkCard[]): WorkCard[] {
  return cards.flatMap(card => [card, ...flattenWork(workChildren(card))])
}

/**
 * The interrupted sessions no interrupted session is above: resuming each of
 * them resumes the rest, since a resume cascades to the sub sessions the same
 * interruption stopped.
 */
export function interruptedRoots(cards: readonly WorkCard[]): WorkCard[] {
  return cards.flatMap(card => card.state === 'paused' && card.paused === 'interrupted' ? [card] : interruptedRoots(workChildren(card)))
}

/** What the viewer may do to a work session from its card. */
export function workControls(card: Pick<WorkCard, 'state' | 'paused'>): { pause: boolean; resume: boolean; stop: boolean } {
  if (isWorkFinished(card.state)) return { pause: false, resume: false, stop: false }
  const paused = card.state === 'paused'
  // A session paused with its Goal resumes when the Goal is woken.
  return { pause: !paused, resume: paused && card.paused !== 'goal', stop: true }
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

/** Whether a card or anything under it still changes. */
export function cardRunning(card: WorkCard): boolean {
  return flattenWork([card]).some(item => !isWorkFinished(item.state))
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

const PAUSED_LINES: Record<PauseSource, string> = {
  user: 'messagehub.work.line.pausedUser',
  parent: 'messagehub.work.line.pausedParent',
  interrupted: 'messagehub.work.line.interrupted',
  goal: 'messagehub.work.line.pausedGoal',
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
export function localizeLine(line: string, state: WorkState, paused?: PauseSource): TaskText | string {
  if (state === 'paused' && paused) return { key: PAUSED_LINES[paused] }
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
