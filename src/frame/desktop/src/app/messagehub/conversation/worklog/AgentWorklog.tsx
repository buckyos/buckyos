import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { ArrowDown, ArrowLeft, ArrowUpRight, ChevronDown, ChevronRight, LoaderCircle } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { mockReadWorkSession } from '../../mock/worksessions'
import { isMessageHubMock } from '../../store'
import { MessageMarkdown } from '../history/MessageMarkdown'
import { useTaskSummary } from '../tasks'
import { TaskDetailsBody } from '../tasks/MessageTask'
import { taskNodeState } from '../tasks/taskView'
import { displayModel, formatTokens, type WorkCard } from '../worksessions/model'
import { taskWorklogTarget, worklogRows, type WorklogDestination, type WorklogRow, type WorklogTarget, type WorkSessionInfo } from './model'
import { readWorkSession } from './source'
import { useWorklog } from './useWorklog'

type Destination = { taskId: string; details?: boolean } | WorklogDestination

/**
 * The work log of a message's Turn task; `initial` opens a session's log on
 * top of it (back returns to the Turn). Without a Turn task (a session
 * opened from the Agent panel's work tree) `initial` is the bottom of the
 * stack and going back from it leaves the log (`onExit`).
 */
export function AgentWorklog({ taskId, initial, onExit }: { taskId?: string; initial?: WorklogDestination; onExit?: () => void }) {
  const { t } = useI18n()
  const [stack, setStack] = useState<Destination[]>(() => initial ? [{ target: initial.target, title: initial.title, card: initial.card }] : [])
  const destination = stack.at(-1) ?? { taskId: taskId ?? '' }
  const navigate = (next: Destination) => setStack(current => [...current, next])
  const back = () => { if (!taskId && stack.length <= 1) onExit?.(); else setStack(current => current.slice(0, -1)) }
  return <div className="flex min-h-0 flex-1 flex-col" data-testid="agent-worklog">
    {stack.length > (taskId ? 0 : 1) || (!taskId && onExit) ? <button className="flex shrink-0 items-center gap-2 border-b border-[color:var(--cp-border)] px-4 py-3 text-xs" onClick={back} data-testid="worklog-back"><ArrowLeft size={14} />{t(!taskId && stack.length <= 1 ? 'messagehub.agentPanel.backToPanel' : 'messagehub.worklog.back')}</button> : null}
    {'target' in destination
      ? <SessionWorklog key={`${destination.target.sessionId}/${destination.target.turn ?? ''}`} target={destination.target} title={destination.title} card={destination.card} navigate={navigate} />
      : destination.details ? <div className="shell-scrollbar min-h-0 overflow-auto p-4"><TaskDetailsBody key={destination.taskId} taskId={destination.taskId} /></div>
      : <TaskWorklog key={destination.taskId} taskId={destination.taskId} navigate={navigate} />}
  </div>
}

function TaskWorklog({ taskId, navigate }: { taskId: string; navigate: (next: Destination) => void }) {
  const { t } = useI18n()
  const { snapshot } = useTaskSummary(taskId)
  const task = snapshot?.status === 'ready' ? snapshot.task : undefined
  const target = task && taskWorklogTarget(task)
  if (!task || !target) return <div className="shell-scrollbar min-h-0 overflow-auto p-4">
    {task ? <p className="mb-4 text-sm text-[color:var(--cp-muted)]">{t('messagehub.worklog.noBinding')}</p> : null}
    <TaskDetailsBody taskId={taskId} />
  </div>
  const state = taskNodeState(task)
  return <SessionWorklog target={target} title={task.name} status={t(state.label.key)} taskId={taskId} navigate={navigate} />
}

function LogValue({ label, value }: { label: string; value: string }) {
  const { t } = useI18n()
  const [expanded, setExpanded] = useState(false)
  const long = value.length > 500 || value.split('\n').length > 5
  return <div className="mh-worklog-value">
    <span className="select-none text-[color:var(--cp-muted)]">{label}</span>
    <div className="min-w-0">
      <pre className={long && !expanded ? 'max-h-28 overflow-hidden' : 'overflow-x-auto'}>{value}</pre>
      {long ? <button className="mt-2 text-xs text-[color:var(--cp-muted)] hover:text-[color:var(--cp-text)]" aria-expanded={expanded} onClick={() => setExpanded(!expanded)}>{t(expanded ? 'messagehub.worklog.less' : 'messagehub.worklog.more')}</button> : null}
    </div>
  </div>
}

function TimelineRow({ row, target, navigate, complete }: { row: WorklogRow; target: WorklogTarget; navigate: (next: Destination) => void; complete: boolean }) {
  const { t } = useI18n()
  const label = row.kind === 'tool' && row.label !== 'result' ? row.label : t(`messagehub.worklog.${row.label}`, row.label)
  const status = row.status === 'pending' && complete ? 'unresolved' : row.status
  return <li className="mh-worklog-row" data-testid="worklog-row" data-kind={row.kind} data-status={status}>
    <span className="mh-worklog-dot" />
    <div className="mb-2 flex flex-wrap items-baseline gap-x-2 gap-y-1 text-sm">
      <span className="font-semibold">{label}</span>
      {row.kind === 'tool' && row.text ? <span className="min-w-0 break-words text-[color:var(--cp-muted)]">{row.text}</span> : null}
      {status ? <span className="text-xs text-[color:var(--cp-muted)]">{t(`messagehub.worklog.status.${status}`, status)}</span> : null}
      {row.at ? <time className="text-xs text-[color:var(--cp-muted)]">{new Date(row.at).toLocaleTimeString()}</time> : null}
    </div>
    {row.kind === 'tool' ? <div className="mh-worklog-io">
      {row.input !== undefined ? <LogValue label="IN" value={row.input} /> : <p className="p-3 text-xs text-[color:var(--cp-muted)]">{t('messagehub.worklog.earlierInput')}</p>}
      {row.output !== undefined ? <LogValue label="OUT" value={row.output} /> : <p className="p-3 text-xs text-[color:var(--cp-muted)]">{t(complete ? 'messagehub.worklog.noOutput' : 'messagehub.worklog.waitingOutput')}</p>}
    </div> : row.text ? <div className="break-words text-[13px] leading-relaxed"><MessageMarkdown text={row.text} /></div> : null}
    {row.child ? <button className="mh-worklog-link" onClick={() => navigate({ target: { agentDid: target.agentDid, sessionId: row.child!.session_id }, title: row.child!.name })} data-testid="worklog-child"><ArrowUpRight size={14} /><span>{row.child.name || t('messagehub.worklog.openTask')}</span></button>
      : row.taskId ? <button className="mh-worklog-link" onClick={() => navigate({ taskId: row.taskId! })} data-testid="worklog-task"><ArrowUpRight size={14} />{t('messagehub.worklog.openTask')}</button> : null}
  </li>
}

/** One Turn of a session, or (`target.turn` absent) the whole session from its first record, with what the work is about at the top. */
function SessionWorklog({ target, title, status, taskId, card, navigate }: { target: WorklogTarget; title: string; status?: string; taskId?: string; card?: WorkCard; navigate: (next: Destination) => void }) {
  const { t } = useI18n()
  const log = useWorklog(target)
  const scroll = useRef<HTMLDivElement>(null)
  const follow = useRef(true)
  const previous = useRef({ height: 0, first: '', session: '' })
  const [atBottom, setAtBottom] = useState(true)
  const page = log.page
  const rows = worklogRows(page?.entries ?? [], page?.children ?? [], target.turn)
  const first = rows[0]?.id ?? ''
  const session = `${target.sessionId}/${target.turn ?? ''}`
  useLayoutEffect(() => {
    const element = scroll.current
    if (!element) return
    if (previous.current.session !== session) follow.current = true
    if (follow.current) element.scrollTop = element.scrollHeight
    else if (first !== previous.current.first) element.scrollTop += element.scrollHeight - previous.current.height
    previous.current = { height: element.scrollHeight, first, session }
  }, [page, first, session, log.error, log.loading])
  const currentTaskId = taskId ?? page?.task_id
  return <div className="flex min-h-0 flex-1 flex-col" data-testid="turn-worklog" data-session-id={target.sessionId} data-turn={target.turn} data-scope={target.turn ? 'turn' : 'session'}>
    <div className="shrink-0 border-b border-[color:var(--cp-border)] px-4 py-3">
      <h3 className="break-words text-sm font-semibold">{title}</h3>
      <div className="mt-1 flex items-center justify-between gap-2 text-xs text-[color:var(--cp-muted)]">
        <span>{target.turn ? t('messagehub.worklog.turn', undefined, { turn: target.turn }) : t('messagehub.worklog.session')}{status ? ` · ${status}` : ''}</span>
        {currentTaskId ? <button className="underline" onClick={() => navigate({ taskId: currentTaskId, details: true })} data-testid="worklog-task-details">{t('messagehub.worklog.taskDetails')}</button> : null}
      </div>
    </div>
    {target.turn ? null : <WorkInfo target={target} card={card} />}
    <div ref={scroll} className="shell-scrollbar min-h-0 flex-1 overflow-y-auto px-5 py-4 [overflow-anchor:none]" data-testid="worklog-scroll" onScroll={() => {
      const element = scroll.current!
      follow.current = element.scrollHeight - element.scrollTop - element.clientHeight < 48
      setAtBottom(follow.current)
      if (element.scrollTop < 24 && page?.next_before != null && !log.older) log.loadOlder?.()
    }}>
      {page?.next_before != null ? <button className="mb-4 w-full text-xs text-[color:var(--cp-muted)] disabled:opacity-50" disabled={log.older} onClick={log.loadOlder} data-testid="worklog-older">{t(log.older ? 'messagehub.worklog.loading' : 'messagehub.worklog.older')}</button> : null}
      {log.loading ? <p role="status" className="text-sm text-[color:var(--cp-muted)]">{t('messagehub.worklog.loading')}</p> : null}
      {log.error ? <p role="alert" className="mb-4 text-sm text-[color:var(--cp-muted)]">{t('messagehub.worklog.failed')} <button onClick={log.retry} className="underline">{t('messagehub.retry')}</button></p> : null}
      {page && rows.length === 0 && !log.error ? <p className="text-sm text-[color:var(--cp-muted)]">{t(target.turn ? 'messagehub.worklog.empty' : 'messagehub.worklog.emptySession')}</p> : null}
      <ol className="mh-worklog-timeline">{rows.map(row => <TimelineRow key={row.id} row={row} target={target} navigate={navigate} complete={page?.complete ?? false} />)}</ol>
      {page && !page.complete && !log.error ? <div className="flex items-center gap-2 pl-5 text-sm text-[color:var(--cp-muted)]" role="status"><LoaderCircle size={14} className="mh-task-spin" />{t('messagehub.worklog.working')}</div> : null}
    </div>
    {!atBottom ? <button className="flex shrink-0 items-center justify-center gap-2 border-t border-[color:var(--cp-border)] py-2 text-xs" onClick={() => { follow.current = true; if (scroll.current) scroll.current.scrollTop = scroll.current.scrollHeight; setAtBottom(true) }}><ArrowDown size={14} />{t('messagehub.worklog.latest')}</button> : null}
  </div>
}

/**
 * 「工作内容」: what the user asked the session to do (its objective, the
 * key of its system prompt), when and where it runs, its tokens by model and,
 * once it ended, its final report. Read once with `session.read`.
 */
function WorkInfo({ target, card }: { target: WorklogTarget; card?: WorkCard }) {
  const { t } = useI18n()
  const [open, setOpen] = useState(true)
  const [info, setInfo] = useState<{ value?: WorkSessionInfo; failed?: boolean }>({})
  const [full, setFull] = useState(false)
  useEffect(() => {
    let cancelled = false
    void (isMessageHubMock() ? mockReadWorkSession : readWorkSession)(target.agentDid, target.sessionId).then(value => { if (!cancelled) setInfo({ value }) }, () => { if (!cancelled) setInfo({ failed: true }) })
    return () => { cancelled = true }
  }, [target.agentDid, target.sessionId])
  const view = info.value
  const objective = view?.config?.session?.objective || view?.entry.objective || ''
  const created = view?.config?.session?.created_at_ms ?? card?.created_at_ms
  const finished = view?.state?.finished_at_ms ?? card?.finished_at_ms
  const runtime = view?.config?.runtime?.frozen
  const workspace = view?.config?.workspace
  const models = card?.usage.models ?? []
  const long = objective.length > 400 || objective.split('\n').length > 6
  const field = (label: string, value: string, testId: string) => <div className="flex gap-3"><dt className="w-20 shrink-0 text-[color:var(--cp-muted)]">{label}</dt><dd className="min-w-0 flex-1 whitespace-pre-line break-words" data-testid={testId}>{value}</dd></div>
  return <section className="shrink-0 border-b border-[color:var(--cp-border)] px-4 py-2" data-testid="work-info" data-expanded={open}>
    <button type="button" className="mh-work-toggle -mx-1 flex min-h-8 items-center gap-1 rounded-lg px-1 text-xs font-semibold" aria-expanded={open} onClick={() => setOpen(!open)} data-testid="work-info-toggle">
      {open ? <ChevronDown size={14} aria-hidden /> : <ChevronRight size={14} aria-hidden />}{t('messagehub.work.info.title')}
    </button>
    {open ? <div className="shell-scrollbar max-h-[35vh] space-y-3 overflow-y-auto pb-2 text-[13px]">
      {!view && !info.failed ? <p role="status" className="text-[color:var(--cp-muted)]">{t('messagehub.work.info.loading')}</p> : null}
      {info.failed ? <p role="note" className="text-[color:var(--cp-muted)]">{t('messagehub.work.info.failed')}</p> : null}
      {objective ? <div data-testid="work-info-objective">
        <p className={`whitespace-pre-wrap break-words leading-relaxed ${long && !full ? 'line-clamp-6' : ''}`}>{objective}</p>
        {long ? <button type="button" className="mt-1 text-xs text-[color:var(--cp-muted)] hover:text-[color:var(--cp-text)]" aria-expanded={full} onClick={() => setFull(!full)}>{t(full ? 'messagehub.worklog.less' : 'messagehub.worklog.more')}</button> : null}
      </div> : null}
      <dl className="space-y-1 text-xs">
        {created ? field(t('messagehub.work.info.created'), new Date(created).toLocaleString(), 'work-info-created') : null}
        {finished ? field(t('messagehub.work.info.finished'), new Date(finished).toLocaleString(), 'work-info-finished') : null}
        {runtime?.runtime_id ? field(t('messagehub.work.info.runtime'), [runtime.runtime_id, runtime.connector?.type && runtime.connector.type !== 'native' ? runtime.connector.type : ''].filter(Boolean).join(' · '), 'work-info-runtime') : null}
        {workspace?.workspace_id ? field(t('messagehub.work.info.workspace'), [workspace.workspace_id, workspace.access].filter(Boolean).join(' · '), 'work-info-workspace') : null}
        {models.length > 0 ? field(t('messagehub.work.info.tokens'), models.map(item => `${displayModel(item.model) ?? item.model} · ${t('messagehub.work.tokensIn', undefined, { value: formatTokens(item.input_tokens) })} · ${t('messagehub.work.tokensOut', undefined, { value: formatTokens(item.output_tokens) })}`).join('\n'), 'work-info-tokens') : null}
      </dl>
      {view?.report?.trim() ? <div data-testid="work-info-report"><h4 className="mb-1 text-xs font-semibold text-[color:var(--cp-muted)]">{t('messagehub.work.info.report')}</h4><div className="break-words leading-relaxed"><MessageMarkdown text={view.report} /></div></div> : null}
    </div> : null}
  </section>
}
