import { useState } from 'react'
import { ArrowLeft, ChevronDown, ChevronRight, Clock, MessageCircleQuestion, Plus } from 'lucide-react'
import { useI18n } from '../../../i18n/provider'
import { hubButtonClass } from '../../../components/conversation/DialogFocus'
import { GoalStatusLabel } from '../../../components/conversation/goals/GoalStatusLabel'
import { activationResult, ago, arrangeGoals, budgetHold, clockTime, periodUsage, unansweredQuestions, type Goal, type GoalState } from '../../../components/conversation/goals/model'
import { useGoalList } from '../../../components/conversation/goals/useGoals'
import { formatTokens } from '../../../components/conversation/worksessions/model'
import { useMessageHubClock } from '../store'
import { usePanelAction } from './hooks'

interface GoalListProps {
  agentDid: string
  /** false: something covers the list, so nothing is read. */
  active: boolean
  onBack: () => void
  onOpen: (goal: Goal) => void
  /** Opens the default conversation with the Agent, ready to ask for a goal. */
  onNew: () => Promise<void>
}

/** The Goals of an Agent: what it looks after for the viewer, the active ones first, the ended ones folded away. */
export function GoalList({ agentDid, active, onBack, onOpen, onNew }: GoalListProps) {
  const { t } = useI18n()
  const now = useMessageHubClock()
  const [showEnded, setShowEnded] = useState(false)
  const { status, goals, states, switchedOff, refresh } = useGoalList(agentDid, active, showEnded)
  const action = usePanelAction()
  const { live, ended } = arrangeGoals(goals, goal => switchedOff[goal.goal_id] === true)
  const row = (goal: Goal) => <GoalListRow key={goal.goal_id} goal={goal} state={states[goal.goal_id]} switchedOff={switchedOff[goal.goal_id] === true} now={now} onOpen={() => onOpen(goal)} />
  return (
    <section className="flex h-full flex-col bg-[color:var(--cp-surface)]" data-testid="goal-list" data-agent={agentDid}>
      <header className="flex shrink-0 items-center gap-1 border-b border-[color:var(--cp-border)] py-1 pl-1 pr-2">
        <button type="button" className="flex min-h-11 min-w-11 items-center justify-center rounded-lg text-[color:var(--cp-muted)]" aria-label={t('messagehub.agentPanel.backToPanel')} title={t('messagehub.agentPanel.backToPanel')} onClick={onBack} data-testid="goals-back"><ArrowLeft size={18} /></button>
        <h2 className="min-w-0 flex-1 truncate text-sm font-medium">{t('messagehub.goals.title')}</h2>
        <button type="button" className="flex min-h-11 shrink-0 items-center gap-1.5 rounded-lg px-3 text-sm font-medium text-[color:var(--cp-accent)] disabled:opacity-40" disabled={!!action.pending} onClick={() => void action.run('new', onNew)} data-testid="goal-new"><Plus size={16} aria-hidden />{t('messagehub.goals.new')}</button>
      </header>
      <div className="shell-scrollbar min-h-0 flex-1 overflow-y-auto overflow-x-hidden p-2" data-testid="goal-list-body">
        {action.error ? <p role="alert" className="px-2 py-1 text-xs text-[color:var(--cp-danger)]">{t('messagehub.goals.newFailed')}</p> : null}
        {status === 'loading' ? <p className="px-2 py-2 text-[13px] text-[color:var(--cp-muted)]" role="status">{t('messagehub.agentPanel.loading')}</p>
          : status === 'unavailable' ? <div className="space-y-2 px-2 py-2" role="note" data-testid="goals-unavailable"><p className="text-[13px] text-[color:var(--cp-muted)]">{t('messagehub.agentPanel.unavailable')}</p><button type="button" className={hubButtonClass} onClick={refresh}>{t('messagehub.retry')}</button></div>
          : <>
            {live.length > 0 ? <ul data-testid="goals-live">{live.map(row)}</ul> : <div className="space-y-1 px-2 py-3" data-testid="goals-empty"><p className="text-sm">{t('messagehub.goals.empty')}</p><p className="text-[13px] text-[color:var(--cp-muted)]">{t('messagehub.goals.emptyHint')}</p></div>}
            {ended.length > 0 ? <>
              <button type="button" className="mt-2 flex min-h-11 w-full items-center gap-1.5 rounded-lg px-2 text-left text-[13px] font-medium text-[color:var(--cp-muted)] hover:bg-[color:color-mix(in_srgb,var(--cp-text)_5%,transparent)]" aria-expanded={showEnded} onClick={() => setShowEnded(!showEnded)} data-testid="goals-ended-toggle">{showEnded ? <ChevronDown size={15} aria-hidden /> : <ChevronRight size={15} aria-hidden />}{t('messagehub.goals.ended', undefined, { count: ended.length })}</button>
              {showEnded ? <ul data-testid="goals-ended">{ended.map(row)}</ul> : null}
            </> : null}
          </>}
      </div>
    </section>
  )
}

function GoalListRow({ goal, state, switchedOff, now, onOpen }: { goal: Goal; state: GoalState | null | undefined; switchedOff: boolean; now: number; onOpen: () => void }) {
  const { t, locale } = useI18n()
  const ended = goal.status === 'achieved' || goal.status === 'archived'
  const last = state?.last_activation
  const questions = ended ? 0 : unansweredQuestions(state)
  const usage = periodUsage(goal, state, now)
  const hold = ended || switchedOff ? null : budgetHold(state, now)
  const spent = usage.budget ? t('messagehub.goals.usageOf', undefined, { used: formatTokens(usage.used), budget: formatTokens(usage.budget) }) : t('messagehub.goals.usage', undefined, { used: formatTokens(usage.used) })
  return (
    <li>
      <button type="button" className="flex min-h-11 w-full items-start gap-2 rounded-lg px-2 py-2.5 text-left hover:bg-[color:color-mix(in_srgb,var(--cp-text)_5%,transparent)]" onClick={onOpen} data-testid="goal-row" data-goal={goal.goal_id} data-status={goal.status}>
        <span className="min-w-0 flex-1 space-y-1">
          <span className="flex min-w-0 items-center gap-2">
            <span className="min-w-0 truncate text-[15px] font-medium" data-testid="goal-row-title">{goal.title}</span>
            {goal.builtin ? <span className="shrink-0 rounded border border-[color:var(--cp-border)] px-1 text-[11px] leading-4 text-[color:var(--cp-muted)]" data-testid="goal-builtin">{t('messagehub.goals.builtin')}</span> : null}
          </span>
          <span className="flex flex-wrap items-center gap-x-1.5 gap-y-0.5 text-xs text-[color:var(--cp-muted)]">
            <GoalStatusLabel goal={goal} switchedOff={switchedOff} />
            {last ? <span className="min-w-0" data-testid="goal-row-last" data-result={activationResult(last)}>· <span style={last.outcome === 'failed' ? { color: 'var(--cp-danger)' } : undefined}>{t(`messagehub.goals.result.${activationResult(last)}`)}</span> · {ago(last.at_ms, now, locale)}</span> : state !== undefined && !ended ? <span>· {t('messagehub.goals.neverWoken')}</span> : null}
          </span>
          {last?.summary ? <span className="line-clamp-2 break-words text-[13px]" data-testid="goal-row-summary">{plainText(last.summary)}</span> : null}
          {!ended && state ? <span className="block text-xs tabular-nums text-[color:var(--cp-muted)]" data-testid="goal-row-usage">{spent}</span> : null}
          {hold ? <span className="flex items-start gap-1.5 text-xs" data-testid="goal-row-hold"><Clock size={14} className="mt-px shrink-0 text-[color:var(--cp-warning)]" aria-hidden />{hold.until_ms ? t(`messagehub.goals.hold.${hold.reason}Until`, undefined, { time: clockTime(hold.until_ms, now, locale) }) : t(`messagehub.goals.hold.${hold.reason}`)}</span> : null}
          {questions > 0 ? <span className="flex items-center gap-1.5 text-xs font-medium" data-testid="goal-row-questions"><MessageCircleQuestion size={14} className="shrink-0 text-[color:var(--cp-warning)]" aria-hidden />{t('messagehub.goals.questions', undefined, { count: questions })}</span> : null}
        </span>
        <ChevronRight size={18} className="mt-1 shrink-0 text-[color:var(--cp-muted)]" aria-hidden />
      </button>
    </li>
  )
}

/** The first lines of a Markdown report as one run of text. */
function plainText(markdown: string): string {
  return markdown.replace(/```[\s\S]*?```/g, ' ').replace(/!?\[([^\]]*)\]\([^)]*\)/g, '$1').replace(/^\s*(?:[#>]+|[-*+]|\d+\.)\s+/gm, '').replace(/[*`]/g, '').replace(/\s+/g, ' ').trim()
}
