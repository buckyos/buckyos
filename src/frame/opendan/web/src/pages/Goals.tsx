import { useState } from 'react'
import clsx from 'clsx'
import { ChevronRight, Target } from 'lucide-react'
import { DECISION_LABEL, LIFECYCLE_LABEL, MANY_GOALS, STATUS_LABEL, STATUS_TEXT, candidatesWaiting, errorText, lastSkipReason } from '../goals'
import { ago, t } from '../i18n'
import { fmtTime, goalExtension, goalHref, nextOpportunityMs, parseTimetable, useQuery } from '../lib'
import { dataModel, type GoalDefinition, type GoalSchedule, type GoalStatus } from '../model'
import { ErrorBox } from '../ui'

const VISIBLE = 8
// A built-in goal switched off in the agent's configuration ranks after the dormant ones.
const STATUS_ORDER: Record<GoalStatus, number> = { active: 0, paused: 1, achieved: 3, archived: 4 }
const OFF_ORDER = 2

export function NextOpportunity({ schedule }: { schedule: GoalSchedule }) {
  if (schedule.switched_off) return <>{t('goalSwitchedOff')}</>
  const next = nextOpportunityMs(schedule)
  const table = parseTimetable(schedule.timetable)
  const check = schedule.last_check?.outcome
  return (
    <>
      {next ? fmtTime(next) : table && 'cron' in table ? <span className="font-mono">{`cron ${table.cron} · ${table.zone}`}</span> : t('goalManualOnly')}
      {check && <span className="font-mono"> ({t('goalCheckOutcome', { outcome: check })})</span>}
    </>
  )
}

/** Which built-in goals are switched off (only they can be). */
const useSwitchedOff = (goals: GoalDefinition[]): Set<string> => {
  const ids = goals.filter((g) => goalExtension(g)?.builtin).map((g) => g.goal_id)
  const off = useQuery(ids.length ? ['goals.switched_off', ...ids] : null, async () => {
    const schedules = await Promise.all(ids.map((id) => dataModel.goalSchedule(id)))
    return ids.filter((_, i) => schedules[i]?.switched_off)
  })
  return new Set(off.data ?? [])
}

function GoalRow({ goal, off }: { goal: GoalDefinition; off: boolean }) {
  const state = useQuery(['goals.state', goal.goal_id], () => dataModel.goalState(goal.goal_id))
  const schedule = useQuery(['goals.schedule', goal.goal_id], () => dataModel.goalSchedule(goal.goal_id))
  const builtin = Boolean(goalExtension(goal)?.builtin)
  const last = state.data?.last_activation
  const skip = schedule.data && !off ? lastSkipReason(schedule.data) : undefined
  const candidates = candidatesWaiting(state.data)
  const lastError = state.data?.last_error
  return (
    <li data-testid={`goal-${goal.goal_id}`}>
      <a href={goalHref(goal.goal_id)} className="flex min-h-14 items-center gap-3 rounded-xl px-2 py-2 active:bg-soft">
        <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-soft text-accent">
          <Target size={18} />
        </span>
        <span className="min-w-0 flex-1 text-xs text-mute">
          <span className="flex items-center gap-2 text-sm font-medium text-fg">
            <span className="truncate">{goal.title}</span>
            {builtin && (
              <span className="shrink-0 rounded border border-line px-1 text-[11px] font-normal leading-4 text-mute" data-testid="goal-builtin">
                {t('goalBuiltin')}
              </span>
            )}
          </span>
          <span className="block">
            {off ? (
              <span className="font-medium text-mute" data-testid="goal-off">{t('goalSwitchedOff')}</span>
            ) : (
              <span className={clsx('font-medium', STATUS_TEXT[goal.status])}>{t(STATUS_LABEL[goal.status])}</span>
            )}
            {' · '}
            {t(LIFECYCLE_LABEL[goal.lifecycle ?? 'maintain'])}
          </span>
          <span className="block truncate" data-testid="goal-last-activation">
            {t('goalLastActivation')}: {last ? `${ago(last.at_ms)} · ${t(DECISION_LABEL[last.decision])}` : t('goalNoActivation')}
          </span>
          {schedule.data && (
            <span className="block truncate" data-testid="goal-next">
              {t('goalNextOpportunity')}: <NextOpportunity schedule={schedule.data} />
            </span>
          )}
          {skip && (
            <span className="block truncate" data-testid="goal-skip">
              {t('goalLastSkip')}: <span className="font-mono">{skip}</span>
            </span>
          )}
          {candidates > 0 && (
            <span className="block text-warn" data-testid="goal-candidates">
              {t('goalCandidatesWaiting', { n: candidates })}
            </span>
          )}
          {lastError != null && (
            <span className="block truncate text-bad" data-testid="goal-error">
              {t('goalLastError')}: {errorText(lastError)}
            </span>
          )}
        </span>
        <ChevronRight size={18} className="shrink-0 text-mute" />
      </a>
    </li>
  )
}

export function GoalList() {
  const goals = useQuery(['goals.list'], () => dataModel.goals())
  const [all, setAll] = useState(false)
  const off = useSwitchedOff(goals.data ?? [])
  const rank = (g: GoalDefinition) => (off.has(g.goal_id) && (g.status === 'active' || g.status === 'paused') ? OFF_ORDER : STATUS_ORDER[g.status])
  const sorted = [...(goals.data ?? [])].sort((a, b) => rank(a) - rank(b) || (b.updated_at_ms ?? 0) - (a.updated_at_ms ?? 0))
  const active = sorted.filter((g) => g.status === 'active' && !off.has(g.goal_id)).length
  const visible = all ? sorted : sorted.slice(0, VISIBLE)
  return (
    <section className="card min-w-0 p-4" data-testid="goals">
      <h2 className="mb-3 text-base font-semibold">
        {t('goals')}
        {goals.data && <span className="ml-1.5 text-sm font-normal text-mute">{sorted.length}</span>}
      </h2>
      <ErrorBox error={goals.error} />
      {active > MANY_GOALS && (
        <p className="mb-2 rounded-lg border border-warn px-3 py-2 text-xs text-warn" data-testid="goals-many">
          {t('goalsMany', { n: active })}
        </p>
      )}
      {sorted.length ? (
        <ul className="-mx-2 divide-y divide-line">
          {visible.map((goal) => (
            <GoalRow key={goal.goal_id} goal={goal} off={off.has(goal.goal_id)} />
          ))}
        </ul>
      ) : (
        !goals.error && <p className="py-2 text-sm text-mute">{t(goals.isLoading ? 'loading' : 'noGoals')}</p>
      )}
      {sorted.length > VISIBLE && (
        <button className="tap mt-2 w-full text-accent" onClick={() => setAll(!all)} data-testid="goals-toggle">
          {all ? t('showLess') : t('showAll', { n: sorted.length })}
        </button>
      )}
    </section>
  )
}
