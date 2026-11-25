import type { TextKey } from './i18n'
import type { ActivationDecision, ActivationOutcome, GoalContext, GoalLifecycle, GoalSchedule, GoalState, GoalStatus, GoalUsage, SkillsState } from './model'

export const STATUS_LABEL: Record<GoalStatus, TextKey> = { active: 'gsActive', paused: 'gsPaused', achieved: 'gsAchieved', archived: 'gsArchived' }
export const STATUS_TEXT: Record<GoalStatus, string> = { active: 'text-ok', paused: 'text-warn', achieved: 'text-accent', archived: 'text-mute' }
export const STATUS_TONE: Record<GoalStatus, string> = { active: 'ok', paused: 'waiting', achieved: 'succeeded', archived: 'finished' }
export const LIFECYCLE_LABEL: Record<GoalLifecycle, TextKey> = { achieve: 'lifecycleAchieve', maintain: 'lifecycleMaintain' }
export const DECISION_LABEL: Record<ActivationDecision, TextKey> = {
  no_action: 'decNoAction',
  acted: 'decActed',
  created_work: 'decCreatedWork',
  needs_info: 'decNeedsInfo',
  unknown: 'decUnknown',
}
export const DECISION_TONE: Record<ActivationDecision, string | undefined> = {
  no_action: undefined,
  acted: 'ok',
  created_work: 'running',
  needs_info: 'waiting',
  unknown: 'bad',
}
export const OUTCOME_TONE: Record<ActivationOutcome, string> = {
  completed: 'ok',
  failed: 'failed',
  budget_exhausted: 'bad',
  stopped: 'stopped',
  needs_input: 'waiting',
}

export const CONTEXT_LABEL: Record<GoalContext, TextKey> = { fresh: 'gateContextFresh', carry: 'gateContextCarry' }
export const CONTEXT_HINT: Record<GoalContext, TextKey> = { fresh: 'gateContextFreshHint', carry: 'gateContextCarryHint' }

export const lastSkipReason = (schedule: GoalSchedule): string | undefined => schedule.skipped?.at(-1)?.reason

/** Defaults of `agent.toml [session.goal]` in the agent package (the page cannot read the agent's own). */
export const GOAL_DEFAULTS = {
  min_interval_secs: 900,
  guarantee_secs: 259_200,
  check_every_secs: 300,
  window_secs: 86_400,
  period_max_total_tokens: 2_000_000,
  context: 'fresh' as GoalContext,
}

/** Tokens of a statistics period: the Goal Sessions, their Work and the jobs. */
export const usageTokens = (usage?: GoalUsage) => {
  const goal = usage?.tokens ?? 0
  const works = usage?.work_tokens ?? 0
  const jobs = usage?.job_tokens ?? 0
  return { goal, works, jobs, total: goal + works + jobs }
}

export const BUILTIN_SELF_IMPROVE = 'self_improve'
/** Its period budget as the host seeds it. */
export const SELF_IMPROVE_DAILY_TOKENS = 1_800_000

export interface Duty {
  id: string
  label?: TextKey
  hint?: TextKey
}

const DUTIES: Duty[] = [
  { id: 'memory', label: 'dutyMemory', hint: 'dutyMemoryHint' },
  { id: 'manuals', label: 'dutyManuals', hint: 'dutyManualsHint' },
  { id: 'tools', label: 'dutyTools', hint: 'dutyToolsHint' },
]

/** The duties of the self improvement Goal: the known ones and any other the owner's switches name. */
export const dutiesOf = (state?: SkillsState): Duty[] => [
  ...DUTIES,
  ...Object.keys(state?.self_improve ?? {}).filter((id) => !DUTIES.some((d) => d.id === id)).map((id) => ({ id })),
]

/** A duty without a switch is on. */
export const dutyOn = (state: SkillsState | undefined, id: string): boolean => state?.self_improve?.[id] !== false

/** The check names the duties an Activation is for in its reason (`tools: …`). */
export const concernsDuty = (reasons: { note?: string }[] | undefined, id: string): boolean =>
  (reasons ?? []).some((r) => r.note?.includes(`${id}:`))

export const CHECK_MIN_EVERY_SECS = 60
/** Shortest hard guarantee (it overrides the budget); 0 turns it off. */
export const GUARANTEE_MIN_SECS = 3600

/** The active goals past which the list says goals are scarce. */
export const MANY_GOALS = 16

export const candidatesWaiting = (state: GoalState | null | undefined): number =>
  state?.maintenance?.candidates?.filter((c) => c.status === 'needs_confirmation').length ?? 0

export const errorText = (error: unknown): string => {
  if (error == null) return ''
  if (typeof error === 'string') return error
  const e = error as { message?: unknown; kind?: unknown }
  if (typeof e.message === 'string') return typeof e.kind === 'string' ? `${e.kind}: ${e.message}` : e.message
  return JSON.stringify(error)
}
