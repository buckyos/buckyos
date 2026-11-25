import { useContext, useEffect, useState, type CSSProperties } from 'react'
import { AlarmClock, ArrowUpRight, Bell, ChevronDown, ChevronRight, Clock, Flag, ListChecks, LoaderCircle, MessageCircleQuestion, MessageSquare, Play, Radio, RotateCcw, type LucideIcon } from 'lucide-react'
import { useI18n } from '../../../i18n/provider'
import { bodyTextClass, bubbleStyle, bubbleWidthClass, rowClass, type MessageRenderContext } from '../history/bubble'
import { LinkifiedText, MessageMarkdown } from '../history/MessageMarkdown'
import type { MessageObject } from '../model/msgobj'
import { TimelineRow } from '../worklog/AgentWorklog'
import { worklogRows } from '../worklog/model'
import { useWorklog } from '../worklog/useWorklog'
import { useSecondClock } from '../worksessions'
import { cardRunning, formatDuration, formatTokens, localizeLine, type WorkCard } from '../worksessions/model'
import { WorkCardList } from '../worksessions/WorkSessionCards'
import { GoalRowsContext, type GoalRowsState } from './context'
import { activationResult, clockTime, goalRowOf, isPassedOn, messageInTurn, reasonDetail, type ActivationReason, type ActivationReasonKind, type ActivationResult, type GoalActivation, type GoalPost, type GoalQuestion, type OpenActivation } from './model'
import { getGoalSource } from './source'

const REASON_ICONS: Record<ActivationReasonKind, LucideIcon> = {
  created: Flag, scheduled: Clock, child: ListChecks, feedback: MessageSquare, manual: Play, reopened: RotateCcw, restarted: RotateCcw, event: Radio, answer: MessageCircleQuestion, guarantee: AlarmClock,
}

const RESULT_TONES: Record<ActivationResult, string> = {
  no_action: 'var(--cp-muted)', acted: 'var(--cp-success)', created_work: 'var(--cp-accent)', needs_info: 'var(--cp-warning)', unknown: 'var(--cp-muted)',
  failed: 'var(--cp-danger)', budget_exhausted: 'var(--cp-warning)', stopped: 'var(--cp-muted)',
}

/** A report longer than this shows its beginning. */
const LONG_REPORT = 600
const FOLD_LINES = 10

function ResultLabel({ result }: { result: ActivationResult }) {
  const { t } = useI18n()
  return <span className="inline-flex min-w-0 items-center gap-1.5" data-testid="goal-result" data-result={result}>
    <span className="h-1.5 w-1.5 shrink-0 rounded-full" style={{ background: RESULT_TONES[result] }} aria-hidden />
    <span>{t(`messagehub.goals.result.${result}`)}</span>
  </span>
}

/** One row of a Goal's conversation (`goalMessages`); null for any other message. */
export function GoalRowView({ message, context }: { message: MessageObject; context: MessageRenderContext }) {
  const view = useContext(GoalRowsContext)
  const row = goalRowOf(message)
  if (!view || !row) return null
  switch (row.type) {
    case 'activation': return <ActivationView view={view} id={`${row.activation.epoch}.${row.activation.seq}`} activation={row.activation} />
    case 'running': return <ActivationView view={view} id={`${row.activation.epoch}.${row.activation.seq}`} open={row.activation} />
    case 'post': return <PostView view={view} post={row.post} pending={row.pending} context={context} />
    case 'feedback': return <FeedbackView view={view} sid={row.sid} turn={row.turn} reason={row.reason} context={context} />
    case 'question': return <QuestionView question={row.question} />
    case 'notified': return <NotifiedView title={row.title} />
  }
}

type ActivationViewProps = { view: GoalRowsState; id: string } & ({ activation: GoalActivation; open?: undefined } | { open: OpenActivation; activation?: undefined })

/** One Activation: why it woke, what it did (folded), what it concluded. */
function ActivationView({ view, id, activation, open }: ActivationViewProps) {
  const { t } = useI18n()
  const item = activation ?? open
  const unfolded = view.unfolded.has(id)
  const line = open ? localizeLine(open.line, 'running') : ''
  const status = typeof line === 'string' ? line : t(line.key, undefined, line.vars)
  const meta = activation ? [formatDuration(activation.ended_at_ms - activation.started_at_ms), activation.tokens > 0 ? t('messagehub.goals.tokens', undefined, { tokens: formatTokens(activation.tokens) }) : ''].filter(Boolean).join(' · ') : ''
  return (
    <section className="space-y-1.5 pt-4" data-testid="goal-activation" data-activation={id} data-running={open ? true : undefined}>
      <ul className="space-y-1" data-testid="goal-reasons">
        {item.reasons.map(reason => <ReasonLine key={reason.key || reason.kind} reason={reason} view={view} />)}
      </ul>
      <button type="button" className="mh-work-toggle -mx-1 flex min-h-11 max-w-full items-center gap-1.5 rounded-lg px-1 text-left text-[13px] text-[color:var(--cp-muted)] md:min-h-9" aria-expanded={unfolded} onClick={() => view.toggle(id)} data-testid="goal-process-toggle">
        {unfolded ? <ChevronDown size={15} className="shrink-0" aria-hidden /> : <ChevronRight size={15} className="shrink-0" aria-hidden />}
        <span className="shrink-0 font-medium">{t('messagehub.goals.process')}</span>
        {meta ? <span className="min-w-0 truncate tabular-nums">· {meta}</span> : null}
      </button>
      {unfolded ? <Process view={view} sid={item.session_id} turn={item.seq} report={activation?.summary ?? ''} /> : null}
      {open ? <p className="flex items-center gap-2 text-[14px] text-[color:var(--cp-muted)]" role="status" data-testid="goal-working"><LoaderCircle size={15} className="mh-task-spin shrink-0" aria-hidden /><span className="min-w-0 truncate">{status || t('messagehub.worklog.working')}</span></p> : null}
      {activation ? <Conclusion view={view} activation={activation} /> : null}
      {activation ? <Works view={view} id={id} activation={activation} /> : null}
    </section>
  )
}

function ReasonLine({ reason, view }: { reason: ActivationReason; view: GoalRowsState }) {
  const { t } = useI18n()
  const Icon = REASON_ICONS[reason.kind]
  const work = reason.kind === 'child' && reason.ref ? view.works[reason.ref] : undefined
  const ended = reason.kind !== 'child' || !reason.note || reason.note === 'finished'
  const label = reason.kind === 'child' ? (work ? t(`messagehub.goals.reason.${ended ? 'childNamed' : 'childWaitingNamed'}`, undefined, { title: work.title }) : t(`messagehub.goals.reason.${ended ? 'child' : 'childWaiting'}`))
    : t(`messagehub.goals.reason.${reason.kind === 'feedback' && isPassedOn(reason) ? 'feedbackPassed' : reason.kind === 'guarantee' && reason.note?.startsWith('the check script') ? 'checkFailing' : reason.kind}`)
  const detail = reasonDetail(reason)
  return (
    <li className="flex items-start gap-2 text-[13px] text-[color:var(--cp-muted)]" data-testid="goal-reason" data-kind={reason.kind}>
      <Icon size={14} className="mt-[3px] shrink-0" aria-hidden />
      <span className="min-w-0 flex-1 break-words">
        <span className="font-medium text-[color:var(--cp-text)]">{label}</span>
        {reason.missed && reason.missed > 1 ? <span> · {t('messagehub.goals.missed', undefined, { count: reason.missed })}</span> : null}
        {detail ? <span className="line-clamp-2 break-words" title={detail}>{detail}</span> : null}
      </span>
    </li>
  )
}

/** The tool calls of one Activation: the Turn's work log without its inputs (they are in the conversation) and its markers. */
function Process({ view, sid, turn, report }: { view: GoalRowsState; sid: string; turn: number; report: string }) {
  const { t } = useI18n()
  const target = { agentDid: view.agentDid, sessionId: sid, turn }
  const log = useWorklog(target)
  const page = log.page
  const rows = worklogRows(page?.entries ?? [], page?.children ?? [], turn)
    .filter(row => row.kind === 'tool' || row.kind === 'assistant' || (row.kind === 'event' && !!row.text && row.text.trim() !== report))
    .map(row => ({ ...row, taskId: undefined }))
  return (
    <div className="shell-scrollbar max-h-[45vh] overflow-y-auto overscroll-contain rounded-xl border border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-4 py-3" data-testid="goal-process">
      {page?.next_before != null ? <button type="button" className="mb-3 min-h-9 w-full text-xs text-[color:var(--cp-muted)] disabled:opacity-50" disabled={log.older} onClick={log.loadOlder}>{t(log.older ? 'messagehub.worklog.loading' : 'messagehub.worklog.older')}</button> : null}
      {log.loading ? <p role="status" className="text-[13px] text-[color:var(--cp-muted)]">{t('messagehub.worklog.loading')}</p> : null}
      {log.error ? <p role="alert" className="text-[13px] text-[color:var(--cp-muted)]">{t('messagehub.worklog.failed')} <button type="button" onClick={log.retry} className="underline">{t('messagehub.retry')}</button></p> : null}
      {page && rows.length === 0 && !log.error ? <p className="text-[13px] text-[color:var(--cp-muted)]">{t('messagehub.worklog.empty')}</p> : null}
      {rows.length > 0 ? <ol className="mh-worklog-timeline">{rows.map(row => <TimelineRow key={row.id} row={row} target={target} complete={page?.complete ?? false} navigate={next => { if ('target' in next) view.onOpenWorklog(next) }} />)}</ol> : null}
    </div>
  )
}

function Conclusion({ view, activation }: { view: GoalRowsState; activation: GoalActivation }) {
  const { t, locale } = useI18n()
  const [full, setFull] = useState(false)
  const result = activationResult(activation)
  const quiet = result === 'no_action'
  const text = activation.summary
  const long = text.length > LONG_REPORT || text.split('\n').length > FOLD_LINES
  return (
    <div className={`mh-bubble ${bubbleWidthClass} w-fit`} style={bubbleStyle(false, false)} data-testid="goal-conclusion">
      <p className="flex flex-wrap items-center gap-x-1.5 text-[12px] text-[color:var(--cp-muted)]"><span className="font-medium"><ResultLabel result={result} /></span><span className="tabular-nums">· {clockTime(activation.ended_at_ms, view.now, locale)}</span></p>
      {!text && activation.end_reason ? <p className="mt-0.5 break-words text-[13px] text-[color:var(--cp-muted)]">{activation.end_reason}</p> : null}
      {text ? <div className={`mt-0.5 ${bodyTextClass} ${quiet ? 'text-[color:var(--cp-muted)]' : ''} ${long && !full ? 'max-h-60 overflow-hidden' : ''}`}><MessageMarkdown text={text} /></div> : null}
      {long ? <button type="button" className="mt-1 min-h-9 text-[13px] text-[color:var(--cp-accent)]" aria-expanded={full} onClick={() => setFull(!full)} data-testid="goal-report-more">{t(full ? 'messagehub.worklog.less' : 'messagehub.worklog.more')}</button> : null}
    </div>
  )
}

/** The Work one Activation started. */
function Works({ view, id, activation }: { view: GoalRowsState; id: string; activation: GoalActivation }) {
  const { t } = useI18n()
  const cards = activation.works_created.map(sid => view.works[sid]).filter((card): card is WorkCard => !!card)
  const others = activation.works_created.filter(sid => !view.works[sid])
  const now = useSecondClock(cards.some(cardRunning))
  if (activation.works_created.length === 0) return null
  const open = (sessionId: string, title: string, card?: WorkCard) => view.onOpenWorklog({ target: { agentDid: view.agentDid, sessionId }, title, card })
  return (
    <div className="mh-work space-y-0.5 rounded-xl p-1" data-testid="goal-works">
      {cards.length > 0 ? <WorkCardList cards={cards} context={{ now, stale: false, viewerDid: view.viewerDid, anchorId: `goal:${view.goal.goal_id}:${id}`, onOpen: (card, title) => open(card.session_id, title, card) }} /> : null}
      {others.map(sid => <button key={sid} type="button" className="mh-work-card flex min-h-11 w-full items-center gap-2 rounded-lg px-2 text-left text-[13px]" onClick={() => open(sid, t('messagehub.goals.work'))} data-testid="goal-work-link"><ArrowUpRight size={15} className="shrink-0 text-[color:var(--cp-muted)]" aria-hidden />{t('messagehub.goals.work')}</button>)}
    </div>
  )
}

function OwnBubble({ text, note, context, pending }: { text: string; note?: string; context: Pick<MessageRenderContext, 'continued'>; pending?: boolean }) {
  return (
    <div className={rowClass(true, context.continued)} data-testid="goal-own-message" data-pending={pending || undefined}>
      <div className="flex w-full flex-col items-end">
        <div className={`mh-bubble ${bubbleWidthClass}`} style={bubbleStyle(true, context.continued)}><p className={`${bodyTextClass} whitespace-pre-wrap break-words`}><LinkifiedText text={text} /></p></div>
        {note ? <p className="mt-1 flex max-w-[85%] items-start gap-1.5 text-right text-[12px] text-[color:var(--cp-muted)]" role="status" data-testid="goal-post-note"><Clock size={12} className="mt-[3px] shrink-0" aria-hidden /><span className="min-w-0 break-words">{note}</span></p> : null}
      </div>
    </div>
  )
}

/** A message written here; until an Activation reads it, why it waits. */
function PostView({ view, post, pending, context }: { view: GoalRowsState; post: GoalPost; pending: boolean; context: MessageRenderContext }) {
  const { t, locale } = useI18n()
  const note = !pending ? undefined
    : view.goal.status === 'paused' ? t('messagehub.goals.post.dormant')
    : view.hold ? (view.hold.until_ms ? t('messagehub.goals.post.heldUntil', undefined, { time: clockTime(view.hold.until_ms, view.now, locale) }) : t('messagehub.goals.post.held'))
    : t('messagehub.goals.post.pending')
  return <OwnBubble text={post.text} note={note} context={context} pending={pending} />
}

const feedbackTexts = new Map<string, Promise<string | undefined>>()

/** A message written in another browser, or passed on from a conversation: its text is read from the opening of its Turn. */
function FeedbackView({ view, sid, turn, reason, context }: { view: GoalRowsState; sid: string; turn: number; reason: ActivationReason; context: MessageRenderContext }) {
  const { t } = useI18n()
  const [text, setText] = useState<string>()
  const { agentDid } = view
  const { key } = reason
  useEffect(() => {
    let cancelled = false
    const id = `${agentDid}\n${sid}\n${key}`
    let reading = feedbackTexts.get(id)
    if (!reading) {
      reading = getGoalSource().opening(agentDid, sid, turn).then(entries => messageInTurn(entries, key))
      feedbackTexts.set(id, reading)
      reading.catch(() => feedbackTexts.delete(id))
    }
    reading.then(value => { if (!cancelled) setText(value) }, () => undefined)
    return () => { cancelled = true }
  }, [agentDid, sid, turn, key])
  if (!text) return null
  if (!isPassedOn(reason)) return <OwnBubble text={text} context={context} />
  return (
    <div className={rowClass(false, context.continued)} data-testid="goal-passed-message">
      <div className={`mh-bubble ${bubbleWidthClass}`} style={bubbleStyle(false, context.continued)}>
        <p className="text-[12px] font-medium text-[color:var(--cp-muted)]">{t('messagehub.goals.reason.feedbackPassed')}</p>
        <p className={`mt-0.5 ${bodyTextClass} whitespace-pre-wrap break-words`}><LinkifiedText text={text} /></p>
      </div>
    </div>
  )
}

/** A question that has no notification card: it is answered like anything else that is said to the Goal. */
function QuestionView({ question }: { question: GoalQuestion }) {
  const { t } = useI18n()
  const open = !question.status || question.status === 'open'
  return (
    <div className={rowClass(false, false)} data-testid="goal-question" data-status={question.status ?? 'open'}>
      <div className={`mh-bubble ${bubbleWidthClass}`} style={bubbleStyle(false, false)}>
        <p className="flex items-center gap-1.5 text-[12px] font-medium text-[color:var(--cp-muted)]"><MessageCircleQuestion size={14} className="shrink-0" style={open ? { color: 'var(--cp-warning)' } as CSSProperties : undefined} aria-hidden />{t(`messagehub.notify.status.${question.status ?? 'open'}`)}</p>
        <p className={`mt-0.5 ${bodyTextClass} whitespace-pre-wrap break-words`}><LinkifiedText text={question.text} /></p>
        {question.options.length > 0 ? <ul className="mt-2 flex flex-wrap gap-1.5">{question.options.map(option => <li key={option.id} className="mh-qa-option rounded-lg px-2.5 py-1 text-[13px]" data-picked={question.answer === option.label || question.answer === option.id || undefined}>{option.label}</li>)}</ul> : null}
        {question.answer ? <p className="mt-2 text-[14px] font-medium">{t('messagehub.notify.result.answer', undefined, { answer: question.answer })}</p> : null}
        {open ? <p className="mt-1.5 text-[12px] text-[color:var(--cp-muted)]">{t('messagehub.goals.answerBelow')}</p> : null}
      </div>
    </div>
  )
}

function NotifiedView({ title }: { title: string }) {
  return (
    <div className={rowClass(false, false)} data-testid="goal-notified">
      <div className={`mh-bubble ${bubbleWidthClass} flex items-start gap-2`} style={bubbleStyle(false, false)}>
        <Bell size={15} className="mt-1 shrink-0 text-[color:var(--cp-muted)]" aria-hidden />
        <p className={`min-w-0 ${bodyTextClass} break-words`}>{title}</p>
      </div>
    </div>
  )
}
