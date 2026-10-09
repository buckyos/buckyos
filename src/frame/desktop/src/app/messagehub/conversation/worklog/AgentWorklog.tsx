import { useLayoutEffect, useRef, useState } from 'react'
import { ArrowDown, ArrowLeft, ArrowUpRight, LoaderCircle } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { MessageMarkdown } from '../history/MessageMarkdown'
import { useTaskSummary } from '../tasks'
import { TaskDetailsBody } from '../tasks/MessageTask'
import { taskNodeState } from '../tasks/taskView'
import { taskWorklogTarget, worklogRows, type WorklogRow, type WorklogTarget } from './model'
import { useWorklog } from './useWorklog'

type Destination = { taskId: string; details?: boolean } | { target: WorklogTarget; title: string }

export function AgentWorklog({ taskId }: { taskId: string }) {
  const { t } = useI18n()
  const [stack, setStack] = useState<Destination[]>([])
  const destination = stack.at(-1) ?? { taskId }
  const navigate = (next: Destination) => setStack(current => [...current, next])
  return <div className="flex min-h-0 flex-1 flex-col" data-testid="agent-worklog">
    {stack.length > 0 ? <button className="flex shrink-0 items-center gap-2 border-b border-[color:var(--cp-border)] px-4 py-3 text-xs" onClick={() => setStack(current => current.slice(0, -1))} data-testid="worklog-back"><ArrowLeft size={14} />{t('messagehub.worklog.back')}</button> : null}
    {'target' in destination
      ? <SessionWorklog key={`${destination.target.sessionId}/${destination.target.turn}`} target={destination.target} title={destination.title} navigate={navigate} />
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
    {row.child ? <button className="mh-worklog-link" onClick={() => navigate({ target: { ...target, sessionId: row.child!.session_id, turn: 1 }, title: row.child!.name })} data-testid="worklog-child"><ArrowUpRight size={14} /><span>{row.child.name || t('messagehub.worklog.openTask')}</span></button>
      : row.taskId ? <button className="mh-worklog-link" onClick={() => navigate({ taskId: row.taskId! })} data-testid="worklog-task"><ArrowUpRight size={14} />{t('messagehub.worklog.openTask')}</button> : null}
  </li>
}

function SessionWorklog({ target, title, status, taskId, navigate }: { target: WorklogTarget; title: string; status?: string; taskId?: string; navigate: (next: Destination) => void }) {
  const { t } = useI18n()
  const log = useWorklog(target)
  const scroll = useRef<HTMLDivElement>(null)
  const follow = useRef(true)
  const previous = useRef({ height: 0, first: '', session: '' })
  const [atBottom, setAtBottom] = useState(true)
  const page = log.page
  const rows = worklogRows(page?.entries ?? [], page?.children ?? [], target.turn)
  const first = rows[0]?.id ?? ''
  const session = `${target.sessionId}/${target.turn}`
  useLayoutEffect(() => {
    const element = scroll.current
    if (!element) return
    if (previous.current.session !== session) follow.current = true
    if (follow.current) element.scrollTop = element.scrollHeight
    else if (first !== previous.current.first) element.scrollTop += element.scrollHeight - previous.current.height
    previous.current = { height: element.scrollHeight, first, session }
  }, [page, first, session, log.error, log.loading])
  const currentTaskId = taskId ?? page?.task_id
  return <div className="flex min-h-0 flex-1 flex-col" data-testid="turn-worklog" data-session-id={target.sessionId} data-turn={target.turn}>
    <div className="shrink-0 border-b border-[color:var(--cp-border)] px-4 py-3">
      <h3 className="break-words text-sm font-semibold">{title}</h3>
      <div className="mt-1 flex items-center justify-between gap-2 text-xs text-[color:var(--cp-muted)]">
        <span>{t('messagehub.worklog.turn', undefined, { turn: target.turn })}{status ? ` · ${status}` : ''}</span>
        {currentTaskId ? <button className="underline" onClick={() => navigate({ taskId: currentTaskId, details: true })} data-testid="worklog-task-details">{t('messagehub.worklog.taskDetails')}</button> : null}
      </div>
    </div>
    <div ref={scroll} className="shell-scrollbar min-h-0 flex-1 overflow-y-auto px-5 py-4 [overflow-anchor:none]" data-testid="worklog-scroll" onScroll={() => {
      const element = scroll.current!
      follow.current = element.scrollHeight - element.scrollTop - element.clientHeight < 48
      setAtBottom(follow.current)
      if (element.scrollTop < 24 && page?.next_before != null && !log.older) log.loadOlder?.()
    }}>
      {page?.next_before != null ? <button className="mb-4 w-full text-xs text-[color:var(--cp-muted)] disabled:opacity-50" disabled={log.older} onClick={log.loadOlder} data-testid="worklog-older">{t(log.older ? 'messagehub.worklog.loading' : 'messagehub.worklog.older')}</button> : null}
      {log.loading ? <p role="status" className="text-sm text-[color:var(--cp-muted)]">{t('messagehub.worklog.loading')}</p> : null}
      {log.error ? <p role="alert" className="mb-4 text-sm text-[color:var(--cp-muted)]">{t('messagehub.worklog.failed')} <button onClick={log.retry} className="underline">{t('messagehub.retry')}</button></p> : null}
      {page && rows.length === 0 && !log.error ? <p className="text-sm text-[color:var(--cp-muted)]">{t('messagehub.worklog.empty')}</p> : null}
      <ol className="mh-worklog-timeline">{rows.map(row => <TimelineRow key={row.id} row={row} target={target} navigate={navigate} complete={page?.complete ?? false} />)}</ol>
      {page && !page.complete && !log.error ? <div className="flex items-center gap-2 pl-5 text-sm text-[color:var(--cp-muted)]" role="status"><LoaderCircle size={14} className="mh-task-spin" />{t('messagehub.worklog.working')}</div> : null}
    </div>
    {!atBottom ? <button className="flex shrink-0 items-center justify-center gap-2 border-t border-[color:var(--cp-border)] py-2 text-xs" onClick={() => { follow.current = true; if (scroll.current) scroll.current.scrollTop = scroll.current.scrollHeight; setAtBottom(true) }}><ArrowDown size={14} />{t('messagehub.worklog.latest')}</button> : null}
  </div>
}
