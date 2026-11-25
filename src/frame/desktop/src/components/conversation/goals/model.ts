import type { MessageObject } from '../model/msgobj'
import type { WorklogEntry } from '../worklog/model'

/** `paused` is shown as "dormant". */
export type GoalStatus = 'active' | 'paused' | 'achieved' | 'archived'
export type ActivationReasonKind = 'created' | 'scheduled' | 'child' | 'feedback' | 'manual' | 'reopened' | 'restarted' | 'event' | 'answer' | 'guarantee'
export type ActivationOutcome = 'completed' | 'failed' | 'budget_exhausted' | 'stopped' | 'needs_input'
export type ActivationDecision = 'no_action' | 'acted' | 'created_work' | 'needs_info' | 'unknown'
export type GoalQuestionStatus = 'open' | 'answered' | 'dismissed' | 'expired' | 'canceled'
export type HoldReason = 'min_interval' | 'period_budget' | 'agent_budget'

/** The part of a Goal definition (`goals.list`, `goals.get`) the conversation side shows. */
export interface Goal {
  goal_id: string
  title: string
  objective: string
  status: GoalStatus
  status_changed_at_ms: number
  achieved_when: string
  healthy_state: string
  /** The Goal Session of the current epoch. */
  session: { epoch: number; sid: string }
  period: { window_secs: number; max_total_tokens?: number }
  builtin: boolean
  updated_at_ms: number
}

export interface ActivationReason { kind: ActivationReasonKind; key: string; ref?: string; missed?: number; note?: string }

/** One line of `goals.activations`: one Turn (`seq`) of the epoch's Goal Session. */
export interface GoalActivation {
  epoch: number
  seq: number
  session_id: string
  started_at_ms: number
  ended_at_ms: number
  reasons: ActivationReason[]
  outcome: ActivationOutcome
  decision: ActivationDecision
  summary: string
  works_created: string[]
  tokens: number
  end_reason?: string
}

/** The Activation in progress (`session.read` of the Goal Session: `state.goal.activation`). */
export interface OpenActivation { epoch: number; seq: number; session_id: string; started_at_ms: number; reasons: ActivationReason[]; line: string }

export interface GoalQuestion {
  text: string
  at_ms: number
  epoch: number
  activation: number
  notify_id?: string
  /** Absent: it was never sent as a notification, so it is open. */
  status?: GoalQuestionStatus
  answer?: string
  options: Array<{ id: string; label: string }>
}

export interface LastActivation { epoch: number; seq: number; at_ms: number; outcome: ActivationOutcome; decision: ActivationDecision; summary: string }

/** `goals.state`. */
export interface GoalState {
  last_activation?: LastActivation
  distance: string
  pending: string[]
  questions: GoalQuestion[]
  usage: { period_start_ms: number; tokens: number; work_tokens: number }
  holding?: { reason: HoldReason; since_ms: number; until_ms?: number }
  /** Ids of its Work that has not ended. */
  active_works: string[]
}

/** The part of `goals.schedule` the conversation side shows. */
export interface GoalSchedule {
  /** A built-in Goal the owner switched off in the Agent's settings. */
  switched_off: boolean
  check_errors: number
  last_check_error?: string
  last_check?: { at_ms: number; notified?: string }
}

const GOAL_STATUSES: readonly GoalStatus[] = ['active', 'paused', 'achieved', 'archived']
const REASON_KINDS: readonly ActivationReasonKind[] = ['created', 'scheduled', 'child', 'feedback', 'manual', 'reopened', 'restarted', 'event', 'answer', 'guarantee']
const OUTCOMES: readonly ActivationOutcome[] = ['completed', 'failed', 'budget_exhausted', 'stopped', 'needs_input']
const DECISIONS: readonly ActivationDecision[] = ['no_action', 'acted', 'created_work', 'needs_info', 'unknown']
const QUESTION_STATUSES: readonly GoalQuestionStatus[] = ['open', 'answered', 'dismissed', 'expired', 'canceled']
const HOLD_REASONS: readonly HoldReason[] = ['min_interval', 'period_budget', 'agent_budget']
/** `agent.toml [session.goal]`: the statistics period of a Goal that sets none. */
const DEFAULT_WINDOW_SECS = 86_400

function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : null
}
const list = (value: unknown) => Array.isArray(value) ? value : []
const records = (value: unknown) => list(value).map(record).filter((item): item is Record<string, unknown> => !!item)
const strings = (value: unknown) => list(value).filter((item): item is string => typeof item === 'string' && !!item.trim())
const count = (value: unknown) => typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : 0
const text = (value: unknown) => typeof value === 'string' ? value : ''
const optional = <T>(key: string, value: T | undefined) => value === undefined || value === '' ? {} : { [key]: value }
function oneOf<T extends string>(values: readonly T[], value: unknown): T | undefined {
  return values.includes(value as T) ? value as T : undefined
}

export function parseGoal(value: unknown): Goal | null {
  const goal = record(value)
  const session = record(goal?.session)
  const status = oneOf(GOAL_STATUSES, goal?.status)
  if (!goal || !session || !status || typeof goal.goal_id !== 'string' || !goal.goal_id || typeof session.sid !== 'string') return null
  const period = record(record(goal.budget)?.period)
  return {
    goal_id: goal.goal_id,
    title: text(goal.title).trim() || goal.goal_id,
    objective: text(goal.objective),
    status,
    status_changed_at_ms: count(goal.status_changed_at_ms),
    achieved_when: text(goal.achieved_when),
    healthy_state: text(goal.healthy_state),
    session: { epoch: count(session.epoch), sid: session.sid },
    period: { window_secs: count(period?.window_secs) || DEFAULT_WINDOW_SECS, ...optional('max_total_tokens', count(period?.max_total_tokens) || undefined) },
    builtin: !!record(record(goal.extensions)?.opendan)?.builtin,
    updated_at_ms: count(goal.updated_at_ms) || count(goal.created_at_ms),
  }
}

export function parseGoals(value: unknown): Goal[] | null {
  return Array.isArray(value) ? value.map(parseGoal).filter((goal): goal is Goal => !!goal) : null
}

function parseReasons(value: unknown): ActivationReason[] {
  return records(value).flatMap(item => {
    const kind = oneOf(REASON_KINDS, item.kind)
    return kind ? [{ kind, key: text(item.key), ...optional('ref', text(item.ref)), ...optional('missed', count(item.missed) || undefined), ...optional('note', text(item.note)) }] : []
  })
}

export function parseActivations(value: unknown): GoalActivation[] | null {
  if (!Array.isArray(value)) return null
  return records(value).filter(item => count(item.seq) > 0 && typeof item.session_id === 'string').map(item => ({
    epoch: count(item.epoch),
    seq: count(item.seq),
    session_id: item.session_id as string,
    started_at_ms: count(item.started_at_ms),
    ended_at_ms: count(item.ended_at_ms),
    reasons: parseReasons(item.reasons),
    outcome: oneOf(OUTCOMES, item.outcome) ?? 'completed',
    decision: oneOf(DECISIONS, item.decision) ?? 'unknown',
    summary: text(item.summary).trim(),
    works_created: strings(item.works_created),
    tokens: count(record(item.cost)?.total_tokens),
    ...optional('end_reason', text(item.end_reason)),
  }))
}

/** The open Activation of a Goal Session's `session.read`, if there is one. */
export function parseOpenActivation(value: unknown): OpenActivation | null {
  const view = record(value)
  const entry = record(view?.entry)
  const goal = record(record(view?.state)?.goal)
  const open = record(goal?.activation)
  if (!open || typeof entry?.session_id !== 'string' || !count(open.seq)) return null
  return { epoch: count(goal?.epoch), seq: count(open.seq), session_id: entry.session_id, started_at_ms: count(open.started_at_ms), reasons: parseReasons(open.reasons), line: text(record(entry.status)?.one_line_status) }
}

export function parseGoalState(value: unknown): GoalState | null {
  const state = record(value)
  if (!state) return null
  const last = record(state.last_activation)
  const usage = record(state.usage)
  const holding = record(state.holding)
  const reason = oneOf(HOLD_REASONS, holding?.reason)
  return {
    ...(last ? { last_activation: { epoch: count(last.epoch), seq: count(last.seq), at_ms: count(last.at_ms), outcome: oneOf(OUTCOMES, last.outcome) ?? 'completed', decision: oneOf(DECISIONS, last.decision) ?? 'unknown', summary: text(last.summary).trim() } } : {}),
    distance: text(record(state.distance)?.text).trim(),
    pending: strings(state.pending),
    questions: records(state.questions).filter(item => !!text(item.text).trim()).map(item => ({
      text: text(item.text).trim(), at_ms: count(item.at_ms), epoch: count(item.epoch), activation: count(item.activation),
      ...optional('notify_id', text(item.notify_id)), ...optional('status', oneOf(QUESTION_STATUSES, item.status)), ...optional('answer', text(item.answer).trim()),
      options: records(item.options).filter(option => !!text(option.label).trim()).map(option => ({ id: text(option.id), label: text(option.label) })),
    })),
    usage: { period_start_ms: count(usage?.period_start_ms), tokens: count(usage?.tokens), work_tokens: count(usage?.work_tokens) },
    ...(holding && reason ? { holding: { reason, since_ms: count(holding.since_ms), ...optional('until_ms', count(holding.until_ms) || undefined) } } : {}),
    active_works: records(state.active_works).map(work => text(work.sid)).filter(Boolean),
  }
}

export function parseGoalSchedule(value: unknown): GoalSchedule | null {
  const schedule = record(value)
  if (!schedule) return null
  const check = record(schedule.last_check)
  return {
    switched_off: schedule.switched_off === true,
    check_errors: count(schedule.check_errors),
    ...optional('last_check_error', text(schedule.last_check_error)),
    ...(check ? { last_check: { at_ms: count(check.at_ms), ...optional('notified', text(check.notified)) } } : {}),
  }
}

export const isLiveGoal = (goal: Goal) => goal.status === 'active' || goal.status === 'paused'

/** Active ones first, dormant (or switched off) ones next, each with the last revised on top; the ended ones apart, last ended first. */
export function arrangeGoals(goals: readonly Goal[], switchedOff: (goal: Goal) => boolean): { live: Goal[]; ended: Goal[] } {
  const rank = (goal: Goal) => goal.status === 'active' && !switchedOff(goal) ? 0 : 1
  return {
    live: goals.filter(isLiveGoal).sort((a, b) => rank(a) - rank(b) || b.updated_at_ms - a.updated_at_ms),
    ended: goals.filter(goal => !isLiveGoal(goal)).sort((a, b) => b.status_changed_at_ms - a.status_changed_at_ms),
  }
}

/** Tokens of the Goal and its Work in the current period; the Goal State keeps those of the period of its last change. */
export function periodUsage(goal: Goal, state: GoalState | null | undefined, now: number): { used: number; budget?: number } {
  const window = goal.period.window_secs * 1000
  const current = Math.floor(now / window) * window
  const used = state && state.usage.period_start_ms >= current ? state.usage.tokens + state.usage.work_tokens : 0
  return { used, ...optional('budget', goal.period.max_total_tokens) }
}

export const unansweredQuestions = (state: GoalState | null | undefined) => state?.questions.filter(question => !question.status || question.status === 'open').length ?? 0

/** What is said to the Goal waits for the budget (its own, or the Agent's background total). */
export function budgetHold(state: GoalState | null | undefined, now: number): { reason: 'period_budget' | 'agent_budget'; until_ms?: number } | null {
  const hold = state?.holding
  if (!hold || hold.reason === 'min_interval' || (hold.until_ms !== undefined && hold.until_ms <= now)) return null
  return { reason: hold.reason, ...optional('until_ms', hold.until_ms) }
}

export type ActivationResult = ActivationDecision | 'failed' | 'budget_exhausted' | 'stopped'
/** How an Activation ended, in one word: its decision, or why it made none. */
export function activationResult({ outcome, decision }: Pick<GoalActivation, 'outcome' | 'decision'>): ActivationResult {
  return outcome === 'completed' ? decision : outcome === 'needs_input' ? 'needs_info' : outcome
}

/** What the host's English note adds to a reason in words of the Goal or of the one who asked for it. */
export function reasonDetail(reason: ActivationReason): string {
  const note = reason.note?.trim() ?? ''
  switch (reason.kind) {
    case 'scheduled': {
      const detail = /^scheduled check \(.*?\)(?:; \d+ slots passed since the last one)?: ([\s\S]+)$/.exec(note)?.[1] ?? ''
      return detail === 'the check passed' ? '' : detail
    }
    case 'created': return /; check: ([\s\S]+)$/.exec(note)?.[1] ?? ''
    case 'reopened':
    case 'restarted': return (/^epoch \d+ started \(\w+\): ([\s\S]+)$/.exec(note)?.[1] ?? '').replace(/; (?:the check passed|the hard guarantee is due)$/, '').replace(/; check: /, ' · ')
    case 'manual': return note.startsWith('activated by ') ? '' : note
    case 'event': return note
    default: return ''
  }
}

/** A `feedback` the Agent passed on from one of its conversations (`goal post` of a UI Session), not one written here. */
export const isPassedOn = (reason: ActivationReason) => !!reason.note?.startsWith('feedback from session:')

const unescapeXml = (value: string) => value.replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&apos;/g, "'").replace(/&amp;/g, '&')

/** The message with input key `key` in the opening of a Turn (`<msg key="…" …>` blocks of its `user_message` records). */
export function messageInTurn(entries: readonly WorklogEntry[], key: string): string | undefined {
  for (const entry of entries) {
    if (entry.t !== 'user_message' || entry.brief) continue
    for (const [, attributes, body] of (entry.content ?? '').matchAll(/<msg( [^>]*)>\n?([\s\S]*?)\n?<\/msg>/g)) {
      const found = / key="([^"]*)"/.exec(attributes)?.[1]
      if (found !== undefined && unescapeXml(found) === key) return unescapeXml(body.replace(/\n?<attachment [^>]*\/>/g, '')).trim()
    }
  }
  return undefined
}

/** What the viewer wrote to a Goal from this browser. */
export interface GoalPost {
  /** Input key: the reason of the Activation that reads it names the same one. */
  key: string
  /** The Goal Session it was posted to. */
  sid: string
  text: string
  at_ms: number
}

export type GoalRow =
  | { type: 'activation'; activation: GoalActivation }
  | { type: 'running'; activation: OpenActivation }
  | { type: 'post'; post: GoalPost; pending: boolean }
  /** A message to the Goal that was not written in this browser: its text is in the Turn's work log. */
  | { type: 'feedback'; sid: string; turn: number; reason: ActivationReason }
  | { type: 'question'; question: GoalQuestion }
  | { type: 'notified'; title: string }

export interface GoalTimeline {
  goal: Goal
  state: GoalState | null
  schedule: GoalSchedule | null
  activations: GoalActivation[]
  /** Activations older than these are not loaded. */
  hasOlder: boolean
  open: OpenActivation | null
  posts: GoalPost[]
  /** Its notifications as messages, oldest first; null when they cannot be read. */
  notices: MessageObject[] | null
}

const noticeId = (message: MessageObject): string | undefined => {
  const id = (message.content?.machine?.data as { notify_id?: unknown } | undefined)?.notify_id
  return typeof id === 'string' ? id : undefined
}

/** A Goal's conversation, oldest first: its Activations, what was said to it, what it asked and what it sent. */
export function goalMessages({ goal, state, schedule, activations, hasOlder, open, posts, notices }: GoalTimeline, dids: { agent: string; self: string }): MessageObject[] {
  const from = hasOlder ? activations[0]?.started_at_ms ?? 0 : 0
  const rows: Array<{ at: number; message: MessageObject }> = []
  // A Goal's rows and its notification cards are no run of one sender's bubbles.
  const add = (id: string, at: number, own: boolean, text: string, row: GoalRow) => rows.push({
    at,
    message: { from: own ? dids.self : goal.goal_id, to: [own ? dids.agent : dids.self], kind: 'chat', created_at_ms: at, content: { format: 'text/plain', content: text }, ui_message_id: `goal:${goal.goal_id}:${id}`, ui_goal_row: row },
  })
  const all = [...activations.map(item => ({ item, row: { type: 'activation', activation: item } as GoalRow, text: item.summary })), ...(open ? [{ item: open, row: { type: 'running', activation: open } as GoalRow, text: '' }] : [])]
  const read = new Set(all.flatMap(({ item }) => item.reasons.map(reason => reason.key)))
  const known = new Set(posts.map(post => post.key))
  for (const post of posts) {
    if (post.at_ms < from) continue
    add(`post:${post.key}`, post.at_ms, true, post.text, { type: 'post', post, pending: !read.has(post.key) && isLiveGoal(goal) && post.sid === goal.session.sid })
  }
  for (const { item, row, text } of all) {
    for (const reason of item.reasons) {
      if (reason.kind === 'feedback' && !known.has(reason.key)) add(`feedback:${reason.key}`, item.started_at_ms, !isPassedOn(reason), '', { type: 'feedback', sid: item.session_id, turn: item.seq, reason })
    }
    // The running one keeps its place in the list when it ends.
    add(`activation:${item.epoch}.${item.seq}`, item.started_at_ms, false, text, row)
  }
  const listed = new Set((notices ?? []).map(noticeId))
  for (const message of notices ?? []) if (message.created_at_ms >= from) rows.push({ at: message.created_at_ms, message })
  for (const question of state?.questions ?? []) {
    if (question.at_ms < from || (question.notify_id && listed.has(question.notify_id))) continue
    add(`question:${question.epoch}.${question.activation}:${question.notify_id ?? question.text}`, question.at_ms, false, question.text, { type: 'question', question })
  }
  const check = schedule?.last_check
  if (!notices && check?.notified && check.at_ms >= from) add(`notified:${check.at_ms}`, check.at_ms, false, check.notified, { type: 'notified', title: check.notified })
  return rows.sort((a, b) => a.at - b.at).map(({ message }) => message)
}

export function goalRowOf(message: MessageObject): GoalRow | undefined {
  const row = message.ui_goal_row
  return row && typeof row === 'object' && 'type' in row ? row as GoalRow : undefined
}

/** `5 minutes ago` in the viewer's language. */
export function ago(at: number, now: number, locale: string): string {
  const format = new Intl.RelativeTimeFormat(locale, { numeric: 'auto' })
  const minutes = Math.max(0, Math.round((now - at) / 60_000))
  if (minutes < 1) return format.format(0, 'second')
  if (minutes < 60) return format.format(-minutes, 'minute')
  return minutes < 1440 ? format.format(-Math.round(minutes / 60), 'hour') : format.format(-Math.round(minutes / 1440), 'day')
}

/** The time alone on the day of `now`, with its day otherwise. */
export function clockTime(at: number, now: number, locale: string): string {
  const date = new Date(at)
  const sameDay = date.toDateString() === new Date(now).toDateString()
  return date.toLocaleString(locale, { ...(sameDay ? {} : { month: 'short', day: 'numeric' }), hour: '2-digit', minute: '2-digit' })
}
