import { useI18n } from '../../../i18n/provider'
import type { Goal, GoalStatus } from './model'

const TONES: Record<GoalStatus | 'off', string> = { active: 'var(--cp-success)', paused: 'var(--cp-warning)', achieved: 'var(--cp-accent)', archived: 'var(--cp-muted)', off: 'var(--cp-muted)' }

export function GoalStatusLabel({ goal, switchedOff }: { goal: Goal; switchedOff: boolean }) {
  const { t } = useI18n()
  const status = switchedOff && (goal.status === 'active' || goal.status === 'paused') ? 'off' : goal.status
  return (
    <span className="inline-flex shrink-0 items-center gap-1.5" data-testid="goal-status" data-status={status}>
      <span className="h-2 w-2 shrink-0 rounded-full" style={{ background: TONES[status] }} aria-hidden />
      {t(`messagehub.goals.status.${status}`)}
    </span>
  )
}
