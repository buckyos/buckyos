import type { PauseSource } from '../conversation/worksessions/model'

export type { PauseSource }

const PAUSE_SOURCES: readonly PauseSource[] = ['user', 'parent', 'interrupted', 'goal']

/** `home.summary`: the Agent's overall state, shared with the Agent's home page. */
export interface AgentSummary {
  sessions: { working: number; waiting: number; failed: number; interrupted: number; total: number }
  memory: { perceptions: number; pending: number; cognitions: number; last_consolidation_at_ms: number | null }
  goals: { active: number }
  skills: { work: number; plan: number; goal: number; disabled: number }
  runtimes: { enabled: number; unavailable: number }
}

export interface PromptVersion {
  frozen_at_ms: number
  /** The session's frozen prompt equals what a new session would freeze now. */
  current: boolean
  /** Names of what differs (identity texts, behaviors, cmd_skills). */
  changed: string[]
}

/** The current generation of the UI Session behind a conversation. */
export interface UiSessionView {
  session_id: string
  run_state: string
  paused: PauseSource | null
  needs_input: boolean
  one_line_status: string
  /** `task_id` is null while the Turn has no TaskMgr task. */
  open_turn?: { turn: number; task_id: string | null }
  /** Work sessions under it that have not ended. */
  unfinished_works: number
  prompt: PromptVersion
}

export interface UiGeneration { session_id: string; created_at_ms: number; finished_at_ms?: number; outcome?: string }

/** `ui.conversation`: the Agent side of one conversation. `current` is null between an archive and the next message. */
export interface UiConversation {
  current: UiSessionView | null
  /** Every generation of the conversation's UI Session, oldest first. */
  generations: UiGeneration[]
  /** The inbox address of the current generation (`current.route_key`); `deliveries.recent` filters by it. Null while no generation is open. */
  route_key: string | null
}

export type DeliverySource = 'report' | 'reply'
/** One file the Agent delivered (`deliveries.recent`). */
export interface Delivery {
  at_ms: number
  name: string
  obj_id: string
  /** `obj://bucket/...` binding in the Agent's `artifacts` bucket; absent when the delivery recorded none. */
  url?: string
  mime?: string
  size?: number
  session_id: string
  ui_session_id?: string
  route_key?: string
  source: DeliverySource
}
export interface DeliveryPage { entries: Delivery[]; next_before?: number }

function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : null
}
const count = (value: unknown) => typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : 0
const text = (value: unknown) => typeof value === 'string' ? value : ''
const counts = <K extends string>(value: unknown, keys: readonly K[]) => Object.fromEntries(keys.map(key => [key, count(record(value)?.[key])])) as Record<K, number>

export function parseAgentSummary(value: unknown): AgentSummary | null {
  const summary = record(value)
  if (!summary || !record(summary.sessions)) return null
  const memory = record(summary.memory)
  return {
    sessions: counts(summary.sessions, ['working', 'waiting', 'failed', 'interrupted', 'total']),
    memory: { ...counts(memory, ['perceptions', 'pending', 'cognitions']), last_consolidation_at_ms: typeof memory?.last_consolidation_at_ms === 'number' ? memory.last_consolidation_at_ms : null },
    goals: counts(summary.goals, ['active']),
    skills: counts(summary.skills, ['work', 'plan', 'goal', 'disabled']),
    runtimes: counts(summary.runtimes, ['enabled', 'unavailable']),
  }
}

function parseSessionView(value: unknown): UiSessionView | null {
  const view = record(value)
  if (!view || typeof view.session_id !== 'string' || !view.session_id) return null
  const turn = record(view.open_turn)
  const prompt = record(view.prompt)
  return {
    session_id: view.session_id,
    run_state: text(view.run_state),
    paused: PAUSE_SOURCES.includes(view.paused as PauseSource) ? view.paused as PauseSource : null,
    needs_input: view.needs_input === true,
    one_line_status: text(view.one_line_status),
    ...(turn ? { open_turn: { turn: count(turn.turn), task_id: typeof turn.task_id === 'string' && turn.task_id ? turn.task_id : null } } : {}),
    unfinished_works: count(view.unfinished_works),
    prompt: { frozen_at_ms: count(prompt?.frozen_at_ms), current: prompt?.current !== false, changed: Array.isArray(prompt?.changed) ? prompt.changed.filter((item): item is string => typeof item === 'string') : [] },
  }
}

export function parseUiConversation(value: unknown): UiConversation | null {
  const page = record(value)
  if (!page || !Array.isArray(page.generations)) return null
  const generations = page.generations.map(record).filter(item => !!item && typeof item.session_id === 'string').map(item => ({
    session_id: item!.session_id as string,
    created_at_ms: count(item!.created_at_ms),
    ...(typeof item!.finished_at_ms === 'number' ? { finished_at_ms: item!.finished_at_ms } : {}),
    ...(typeof item!.outcome === 'string' ? { outcome: item!.outcome } : {}),
  })).sort((a, b) => a.created_at_ms - b.created_at_ms)
  const routeKey = record(page.current)?.route_key
  return { current: parseSessionView(page.current), generations, route_key: typeof routeKey === 'string' && routeKey ? routeKey : null }
}

export function parseDeliveries(value: unknown): DeliveryPage | null {
  const page = record(value)
  if (!page || !Array.isArray(page.entries)) return null
  const entries = page.entries.map(record).filter(item => !!item && typeof item.obj_id === 'string' && typeof item.session_id === 'string').map(item => ({
    at_ms: count(item!.at_ms),
    name: text(item!.name) || text(item!.url).split('/').pop() || (item!.obj_id as string),
    obj_id: item!.obj_id as string,
    ...(typeof item!.url === 'string' && item!.url ? { url: item!.url } : {}),
    ...(typeof item!.mime === 'string' && item!.mime ? { mime: item!.mime } : {}),
    ...(typeof item!.size === 'number' ? { size: item!.size } : {}),
    session_id: item!.session_id as string,
    ...(typeof item!.ui_session_id === 'string' ? { ui_session_id: item!.ui_session_id } : {}),
    ...(typeof item!.route_key === 'string' ? { route_key: item!.route_key } : {}),
    source: item!.source === 'reply' ? 'reply' as const : 'report' as const,
  }))
  return { entries, ...(typeof page.next_before === 'number' ? { next_before: page.next_before } : {}) }
}

/** What the Agent does in this conversation, in the panel's words. */
export type AgentConversationState = 'interrupted' | 'paused' | 'answering' | 'working' | 'idle' | 'fresh'

/**
 * `answering`: the composer is locked to a question of the Agent (the
 * timeline says so). `fresh`: no UI Session yet — the next message starts one.
 */
export function agentConversationState(view: UiSessionView | null, answering: boolean): AgentConversationState {
  if (!view) return 'fresh'
  if (view.paused === 'interrupted') return 'interrupted'
  if (view.paused) return 'paused'
  if (answering) return 'answering'
  if (view.open_turn && !view.needs_input) return 'working'
  return view.run_state === 'running' || view.run_state === 'ready' ? 'working' : 'idle'
}

/** Which generation of the conversation is current (1-based); the next one while none is open. */
export function generationNumber(conversation: UiConversation): number {
  const index = conversation.current ? conversation.generations.findIndex(item => item.session_id === conversation.current!.session_id) : -1
  return index >= 0 ? index + 1 : conversation.generations.length + 1
}

/** When each archived generation ended: where the timeline shows "a new conversation started here". */
export function generationBreaks(conversation: UiConversation): number[] {
  return conversation.generations.filter(item => item.finished_at_ms !== undefined && item.session_id !== conversation.current?.session_id).map(item => item.finished_at_ms!).sort((a, b) => a - b)
}

/** Bytes as `812 B` / `4.2 KB` / `1.3 MB`. */
export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let value = bytes / 1024, unit = 0
  while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit++ }
  return `${value.toFixed(1).replace(/\.0$/, '')} ${units[unit]}`
}
