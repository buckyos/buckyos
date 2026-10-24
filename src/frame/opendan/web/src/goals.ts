import type { TextKey } from './i18n'
import type { ActivationDecision, ActivationOutcome, GoalLifecycle, GoalSchedule, GoalState, GoalStatus } from './model'

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

/** The built-in Goal's gate says why it last skipped; an ordinary one keeps a list of skipped fires. */
export const lastSkipReason = (schedule: GoalSchedule, builtin: boolean): string | undefined =>
  (builtin ? schedule.gate?.last_skip_reason : undefined) ?? schedule.skipped?.at(-1)?.reason

export const candidatesWaiting = (state: GoalState | null | undefined): number =>
  state?.maintenance?.candidates?.filter((c) => c.status === 'needs_confirmation').length ?? 0

export const errorText = (error: unknown): string => {
  if (error == null) return ''
  if (typeof error === 'string') return error
  const e = error as { message?: unknown; kind?: unknown }
  if (typeof e.message === 'string') return typeof e.kind === 'string' ? `${e.kind}: ${e.message}` : e.message
  return JSON.stringify(error)
}
