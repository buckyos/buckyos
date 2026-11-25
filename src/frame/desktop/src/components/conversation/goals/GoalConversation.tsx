import { useMemo, useState } from 'react'
import { ArrowLeft, Target } from 'lucide-react'
import type { DID } from 'buckyos'
import { useI18n } from '../../../i18n/provider'
import { useConversationClock } from '../clock'
import { ConversationSurface } from '../ConversationSurface'
import { registerMessageRenderer } from '../history/messageRenderers'
import type { ConversationMessageReader } from '../history/types'
import type { MessageObject } from '../model/msgobj'
import type { SessionAccess, SessionPanelInfo } from '../model/types'
import { ConversationHosts, ConversationStatus } from '../SessionConversation'
import type { ConversationSource } from '../source'
import { formatTokens } from '../worksessions/model'
import type { WorklogDestination } from '../worklog/model'
import { GoalRowsContext, type GoalRowsState } from './context'
import { GoalRowView } from './GoalRows'
import { GoalStatusLabel } from './GoalStatusLabel'
import { ago, budgetHold, clockTime, goalMessages, goalRowOf, isLiveGoal, periodUsage, type Goal, type GoalSchedule, type GoalState } from './model'
import { useGoal } from './useGoals'

registerMessageRenderer('notice', (message, context) => goalRowOf(message) ? <GoalRowView message={message} context={context} /> : null)

const READ_ONLY: SessionAccess = { mode: 'read_only', canManage: false, canEnableWrite: false, canEditPresentation: false, canEditSharedState: false, canEditOwnMemberState: false }
const READ_WRITE: SessionAccess = { ...READ_ONLY, mode: 'read_write' }

/** Unsent texts of this page, by Agent and Goal. */
const drafts = new Map<string, string>()

function hash(text: string): number {
  let value = 5381
  for (let index = 0; index < text.length; index++) value = ((value << 5) + value + text.charCodeAt(index)) | 0
  return value
}

function readerOf(key: string, messages: readonly MessageObject[]): ConversationMessageReader {
  return {
    readerKey: key,
    // Rows change in place (a running Activation ends, a message is read): every change is a new revision, never an append.
    revision: hash(messages.map(message => message.ui_goal_row ? JSON.stringify([message.ui_message_id, message.ui_goal_row]) : String(message.ui_message_id)).join('\n')),
    totalCount: messages.length,
    timeline: messages,
    readRange: async (start: number, count: number) => messages.slice(Math.max(0, start), Math.max(0, start) + Math.max(0, count)),
  } as ConversationMessageReader
}

export interface GoalConversationProps {
  agentDid: DID
  agentName: string
  goalId: string
  /** Shown until the Goal is read. */
  title: string
  viewerDid: DID
  /** false: something covers it, so nothing is read. */
  active: boolean
  onBack: () => void
  onOpenWorklog: (destination: WorklogDestination) => void
}

/**
 * One Goal as a conversation: each Activation with why it woke, what it did and what it concluded, what it asked
 * and sent, and one box to say something to it (`goals.post`).
 */
export function GoalConversation(props: GoalConversationProps) {
  return <ConversationHosts><Bound {...props} /></ConversationHosts>
}

function Bound({ agentDid, agentName, goalId, title, viewerDid, active, onBack, onOpenWorklog }: GoalConversationProps) {
  const { t, locale } = useI18n()
  const now = useConversationClock()
  const view = useGoal(agentDid, goalId, active)
  const [unfolded, setUnfolded] = useState<ReadonlySet<string>>(() => new Set())
  const { goal, state, schedule, activations, hasOlder, open, posts, notices, works } = view
  const key = `goal:${agentDid}:${goalId}`
  const messages = useMemo(() => goal ? goalMessages({ goal, state, schedule, activations, hasOlder, open, posts: [...posts], notices }, { agent: agentDid, self: viewerDid }) : [], [goal, state, schedule, activations, hasOlder, open, posts, notices, agentDid, viewerDid])
  const reader = useMemo(() => readerOf(key, messages), [key, messages])
  const header = <GoalHeader title={goal?.title ?? title} goal={goal} state={state} schedule={schedule} now={now} onBack={onBack} />
  if (!goal) {
    return (
      <div className="flex h-full flex-col" style={{ background: 'var(--cp-message-canvas)' }}>
        {header}
        {view.status === 'loading' ? <ConversationStatus testId="goal-loading" loading text={t('messagehub.agentPanel.loading')} />
          : view.status === 'missing' ? <ConversationStatus testId="goal-missing" text={t('messagehub.goals.missing')} />
          : <ConversationStatus testId="goal-unavailable" text={t('messagehub.agentPanel.unavailable')} action={{ label: t('messagehub.retry'), onClick: view.retry }} />}
      </div>
    )
  }
  const live = isLiveGoal(goal)
  const hold = live ? budgetHold(state, now) : null
  const rows: GoalRowsState = {
    agentDid, viewerDid, goal, now, works, hold, unfolded, onOpenWorklog,
    toggle: id => setUnfolded(previous => { const next = new Set(previous); if (!next.delete(id)) next.add(id); return next }),
  }
  const usage = periodUsage(goal, state, now)
  const panel: SessionPanelInfo = {
    title: t('messagehub.goals.objective'),
    text: goal.objective,
    fields: !live ? [] : [{ label: t('messagehub.goals.periodUsage'), value: usage.budget ? t('messagehub.goals.ofBudget', undefined, { used: formatTokens(usage.used), budget: formatTokens(usage.budget) }) : formatTokens(usage.used), ...(usage.budget && usage.used >= usage.budget ? { tone: 'warning' as const } : {}) }],
  }
  const source: ConversationSource = {
    sessionId: key,
    selfDid: viewerDid,
    reader,
    historyStatus: 'ready',
    hasOlder,
    loadOlder: view.loadOlder,
    markRead: () => undefined,
    access: live ? READ_WRITE : READ_ONLY,
    canAct: false,
    attachments: false,
    send: payload => view.post(payload.content),
    draft: { text: drafts.get(key) ?? '', attachments: [], saveText: (value) => { drafts.set(key, value); return undefined }, saveAttachments: () => undefined },
    displayName: did => did === agentDid ? agentName : did,
    runtime: [],
    preferences: { showActions: true, pinned: null },
    panel,
    peerMarkdown: true,
    messageActions: { handleNotify: true },
  }
  return (
    <GoalRowsContext.Provider value={rows}>
      <div className="h-full" data-testid="goal-conversation" data-goal={goalId} data-status={goal.status}>
        <ConversationSurface
          source={source}
          features={{ search: false, drop: false }}
          header={() => header}
          banner={<GoalBanner goal={goal} schedule={schedule} hold={hold} now={now} locale={locale} />}
          panelIcon={<Target size={15} />}
          panelDetails={<GoalDetails goal={goal} state={state} />}
          emptyLabel={t('messagehub.goals.noActivations')}
          placeholder={t('messagehub.goals.placeholder')}
          readOnlyLabel={t(`messagehub.goals.endedNote.${goal.status === 'achieved' ? 'achieved' : 'archived'}`)}
        />
      </div>
    </GoalRowsContext.Provider>
  )
}

function GoalHeader({ title, goal, state, schedule, now, onBack }: { title: string; goal: Goal | null; state: GoalState | null; schedule: GoalSchedule | null; now: number; onBack: () => void }) {
  const { t, locale } = useI18n()
  const last = state?.last_activation
  return (
    <header className="flex shrink-0 items-center gap-1 border-b border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] py-1 pl-1 pr-3">
      <button type="button" className="flex min-h-11 min-w-11 items-center justify-center rounded-lg text-[color:var(--cp-muted)]" aria-label={t('messagehub.goals.backToList')} title={t('messagehub.goals.backToList')} onClick={onBack} data-testid="goal-back"><ArrowLeft size={18} /></button>
      <div className="min-w-0 flex-1">
        <h2 className="truncate text-[15px] font-medium" title={title} data-testid="goal-title">{title}</h2>
        {goal ? <p className="flex min-w-0 items-center gap-1.5 text-xs text-[color:var(--cp-muted)]">
          <GoalStatusLabel goal={goal} switchedOff={schedule?.switched_off === true} />
          {last ? <span className="min-w-0 truncate">· {t('messagehub.goals.lastLook', undefined, { time: ago(last.at_ms, now, locale) })}</span> : null}
        </p> : null}
      </div>
    </header>
  )
}

/** What keeps the Goal from acting right now. */
function GoalBanner({ goal, schedule, hold, now, locale }: { goal: Goal; schedule: GoalSchedule | null; hold: ReturnType<typeof budgetHold>; now: number; locale: string }) {
  const { t } = useI18n()
  if (!isLiveGoal(goal)) return null
  const note = schedule?.switched_off ? { id: 'off', text: t('messagehub.goals.offNote') }
    : goal.status === 'paused' ? { id: 'dormant', text: t('messagehub.goals.dormantNote') }
    : hold ? { id: hold.reason, warn: true, text: hold.until_ms ? t(`messagehub.goals.hold.${hold.reason}Until`, undefined, { time: clockTime(hold.until_ms, now, locale) }) : t(`messagehub.goals.hold.${hold.reason}`) }
    : schedule && schedule.check_errors > 0 ? { id: 'check', warn: true, text: t('messagehub.goals.checkFailing', undefined, { count: schedule.check_errors }), detail: schedule.last_check_error }
    : null
  if (!note) return null
  return <p className="shrink-0 border-b border-[color:var(--cp-border)] px-4 py-2 text-[13px] text-[color:var(--cp-text)]" style={{ background: 'warn' in note ? 'color-mix(in srgb, var(--cp-warning) 12%, var(--cp-surface))' : 'var(--cp-surface)' }} role="note" title={'detail' in note ? note.detail : undefined} data-testid="goal-banner" data-note={note.id}>{note.text}</p>
}

/** The rest of what is known of the Goal, in the unfolded panel over the conversation. */
function GoalDetails({ goal, state }: { goal: Goal; state: GoalState | null }) {
  const { t } = useI18n()
  const until = goal.achieved_when || goal.healthy_state
  if (!until && !state?.distance && !state?.pending.length) return null
  const section = (label: string, body: React.ReactNode) => <section><h4 className="text-[12px] font-medium text-[color:var(--cp-muted)]">{label}</h4>{body}</section>
  return (
    <div className="space-y-2 border-t border-[color:var(--cp-border)] pt-2" data-testid="goal-details">
      {until ? section(t(goal.achieved_when ? 'messagehub.goals.achievedWhen' : 'messagehub.goals.healthyState'), <p className="whitespace-pre-wrap break-words">{until}</p>) : null}
      {state?.distance ? section(t('messagehub.goals.distance'), <p className="whitespace-pre-wrap break-words">{state.distance}</p>) : null}
      {state?.pending.length ? section(t('messagehub.goals.pending'), <ul className="list-disc space-y-0.5 pl-4">{state.pending.map(item => <li key={item} className="break-words">{item}</li>)}</ul>) : null}
    </div>
  )
}
