import { useState } from 'react'
import { Ban, Check, ChevronDown, ChevronRight, CircleAlert, CircleCheck, CirclePause, CircleX, Clock, Copy, LoaderCircle, RefreshCw } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import type { TaskWatchNode } from '../../../../api/task_mgr'
import { messageAgentTaskId, type MessageObject } from '../../protocol/msgobj'
import { relativeActivity } from '../../sessionModel'
import { useMessageHubClock } from '../../store'
import { messageObjId, messageRelations } from '../history/relations'
import { getTaskWatch, useTaskChildren, useTaskEvents, useTaskExpanded, useTaskSummary, useTaskTree } from './index'
import { taskAreaView, taskNodeState, type TaskText, type TaskTone } from './taskView'

const toneColor: Record<TaskTone, string> = {
  active: 'var(--cp-accent)',
  waiting: 'var(--cp-warning)',
  paused: 'var(--cp-muted)',
  success: 'var(--cp-success)',
  failed: 'var(--cp-danger)',
  canceled: 'var(--cp-muted)',
  unavailable: 'var(--cp-muted)',
}

function ToneIcon({ tone, size = 15 }: { tone: TaskTone; size?: number }) {
  const props = { size, 'aria-hidden': true, style: { color: toneColor[tone] }, className: 'mt-0.5 shrink-0' }
  switch (tone) {
    case 'active': return <LoaderCircle {...props} className={`${props.className} mh-task-spin`} />
    case 'waiting': return <Clock {...props} />
    case 'paused': return <CirclePause {...props} />
    case 'success': return <CircleCheck {...props} />
    case 'failed': return <CircleX {...props} />
    case 'canceled': return <Ban {...props} />
    default: return <CircleAlert {...props} />
  }
}

function useTaskText() {
  const { t } = useI18n()
  return (text: TaskText | string) => typeof text === 'string' ? text : t(text.key, undefined, text.vars)
}

/**
 * The task area of an Agent reply: by default two lines (state + current
 * activity, then sub-task counts / wait reason / last update); expanded, the
 * Turn's task tree. Nothing is rendered when the message carries no
 * `agent_task`, the task cannot be read, or it is an ordinary quick reply.
 */
export function MessageTaskArea({ message }: { message: MessageObject }) {
  const taskId = messageAgentTaskId(message)
  const { snapshot, children } = useTaskSummary(taskId)
  const now = useMessageHubClock()
  const { t } = useI18n()
  const text = useTaskText()
  const [expanded, setExpanded] = useTaskExpanded(messageObjId(message) ?? taskId ?? '')
  if (!taskId) return null
  const view = taskAreaView(snapshot, children, { edited: !!messageRelations(message)?.edited, createdAt: message.created_at_ms }, now)
  if (!view) return null
  const second = [...view.secondary.map(text), view.updatedAt ? t('messagehub.task.updated', undefined, { time: relativeActivity(view.updatedAt, now, t('messagehub.now')) }) : ''].filter(Boolean).join(' · ')
  const expandLabel = t(expanded ? 'messagehub.task.collapse' : 'messagehub.task.expand')
  return (
    <div className="mh-task mt-2 rounded-xl px-2.5 py-2" data-testid="message-task" data-tone={view.tone} data-state={view.label.key.split('.').at(-1)}>
      <div className="flex items-start gap-2">
        <ToneIcon tone={view.tone} />
        <div className="min-w-0 flex-1" role="status">
          <p className="truncate text-[13px] font-medium" data-testid="task-line-1">{text(view.label)}{view.activity ? ` · ${view.activity}` : ''}</p>
          {second ? <p className="truncate text-[12px] opacity-75" data-testid="task-line-2">{second}</p> : null}
        </div>
        <button type="button" className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full hover:bg-[color:color-mix(in_srgb,var(--cp-text)_8%,transparent)]" aria-expanded={expanded} aria-label={expandLabel} title={expandLabel} onClick={() => setExpanded(!expanded)} data-testid="task-expand">{expanded ? <ChevronDown size={16} /> : <ChevronRight size={16} />}</button>
      </div>
      {expanded ? <div className="mt-2 border-t pt-2" style={{ borderColor: 'color-mix(in srgb, currentColor 14%, transparent)' }}><TaskTree rootId={taskId} /></div> : null}
    </div>
  )
}

/** The task tree under `rootId`, one lazily loaded level at a time; released when unmounted. */
export function TaskTree({ rootId, events = false }: { rootId: string; events?: boolean }) {
  useTaskTree(rootId, events)
  return <div data-testid="task-tree"><TaskTreeLevel rootId={rootId} parentId={rootId} /></div>
}

function TaskTreeLevel({ rootId, parentId }: { rootId: string; parentId: string }) {
  const { t } = useI18n()
  const page = useTaskChildren(parentId)
  const watch = getTaskWatch()
  if (!page || (page.status === 'loading' && page.items.length === 0)) return <p className="text-[12px] opacity-70" role="status">{t('messagehub.task.loading')}</p>
  if (page.status === 'error') return <p className="flex items-center gap-2 text-[12px]" role="alert">{t('messagehub.task.treeFailed')}<button type="button" className="underline" onClick={() => { void watch.loadChildren(parentId) }}>{t('messagehub.retry')}</button></p>
  if (page.items.length === 0) return <p className="text-[12px] opacity-70">{t('messagehub.task.noChildren')}</p>
  return (
    <ul className="space-y-1.5">
      {page.items.map(node => <TaskTreeNode key={node.taskId} rootId={rootId} node={node} />)}
      {page.nextCursor ? <li><button type="button" className="text-[12px] underline disabled:opacity-50" disabled={page.status === 'loading'} onClick={() => { void watch.loadChildren(parentId, true) }} data-testid="task-tree-more">{t('messagehub.loadMore')}</button></li> : null}
    </ul>
  )
}

function TaskTreeNode({ rootId, node }: { rootId: string; node: TaskWatchNode }) {
  const { t } = useI18n()
  const text = useTaskText()
  const [open, setOpen] = useState(false)
  const watch = getTaskWatch()
  const state = taskNodeState(node)
  const toggle = () => {
    if (open) watch.collapseNode(rootId, node.taskId); else void watch.expandNode(rootId, node.taskId)
    setOpen(!open)
  }
  const label = t(open ? 'messagehub.task.collapse' : 'messagehub.task.expand')
  const waiting = node.phase === 'Waiting' ? node.waitReason?.message : undefined
  return (
    <li data-testid="task-node" data-task-id={node.taskId} data-state={state.label.key.split('.').at(-1)}>
      <div className="flex items-start gap-1.5">
        <button type="button" className="mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded" aria-expanded={open} aria-label={`${label}: ${node.name}`} title={label} onClick={toggle} data-testid="task-node-expand">{open ? <ChevronDown size={14} /> : <ChevronRight size={14} />}</button>
        <ToneIcon tone={state.tone} size={14} />
        <div className="min-w-0 flex-1">
          <p className="break-words text-[13px]">{node.name}</p>
          <p className="break-words text-[12px] opacity-75">{[text(state.label), node.message, waiting && waiting !== node.message ? waiting : ''].filter(Boolean).join(' · ')}</p>
        </div>
      </div>
      {open ? <div className="ml-6 mt-1.5 border-l pl-2" style={{ borderColor: 'color-mix(in srgb, currentColor 14%, transparent)' }}><TaskTreeLevel rootId={rootId} parentId={node.taskId} /></div> : null}
    </li>
  )
}

function formatJson(value: unknown): string {
  if (typeof value === 'string') return value
  try { return JSON.stringify(value, null, 2) } catch { return String(value) }
}

/** `result.artifacts` style lists: strings, or objects with a name / path / obj_id. */
function artifactRefs(result: unknown): string[] {
  if (!result || typeof result !== 'object') return []
  const record = result as Record<string, unknown>
  const list = [record.artifacts, record.outputs, record.files].find(Array.isArray) as unknown[] | undefined
  return (list ?? []).map(item => {
    if (typeof item === 'string') return item
    if (!item || typeof item !== 'object') return ''
    const entry = item as Record<string, unknown>
    return [entry.name, entry.path, entry.uri, entry.obj_id].find(value => typeof value === 'string') as string | undefined ?? ''
  }).filter(Boolean)
}

export function CopyValue({ label, value, testId }: { label: string; value: string; testId?: string }) {
  const { t } = useI18n()
  const [copied, setCopied] = useState(false)
  const copy = () => { void navigator.clipboard.writeText(value).then(() => { setCopied(true); setTimeout(() => setCopied(false), 1500) }, () => undefined) }
  const title = t(copied ? 'messagehub.message.copied' : 'messagehub.message.copy')
  return (
    <div className="flex min-w-0 items-center gap-1 text-xs">
      <span className="shrink-0 text-[color:var(--cp-muted)]">{label}</span>
      <code className="min-w-0 flex-1 truncate" title={value} data-testid={testId}>{value}</code>
      <button type="button" className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-[color:var(--cp-muted)] hover:bg-[color:color-mix(in_srgb,var(--cp-text)_6%,transparent)]" aria-label={`${title}: ${label}`} title={title} onClick={copy}>{copied ? <Check size={14} /> : <Copy size={14} />}</button>
    </div>
  )
}

const sectionTitleClass = 'text-xs font-semibold uppercase tracking-wide text-[color:var(--cp-muted)]'

/**
 * The live task section of the message details: summary, tree, the events
 * the viewer may read, result, artifact references and the copyable task id.
 * An unreadable or cleaned-up task says so instead of showing a state.
 */
export function TaskDetailsSection({ message }: { message: MessageObject }) {
  const taskId = messageAgentTaskId(message)
  if (!taskId) return null
  return <TaskDetailsBody key={taskId} taskId={taskId} />
}

export function TaskDetailsBody({ taskId }: { taskId: string }) {
  const { t } = useI18n()
  const text = useTaskText()
  const { snapshot } = useTaskSummary(taskId)
  const watch = getTaskWatch()
  const task = snapshot?.status === 'ready' ? snapshot.task : undefined
  const state = task ? taskNodeState(task) : null
  const unavailable = snapshot?.status === 'denied' ? 'messagehub.task.denied' : snapshot?.status === 'missing' ? 'messagehub.task.missing' : snapshot?.status === 'error' ? 'messagehub.task.readFailed' : ''
  const artifacts = artifactRefs(task?.result)
  return (
    <section className="space-y-3" data-testid="detail-task" data-status={snapshot?.status ?? 'loading'}>
      <div className="flex items-center justify-between gap-2">
        <h3 className={sectionTitleClass}>{t('messagehub.task.title')}</h3>
        <button type="button" className="flex h-8 w-8 items-center justify-center rounded-lg text-[color:var(--cp-muted)] hover:bg-[color:color-mix(in_srgb,var(--cp-text)_6%,transparent)]" aria-label={t('messagehub.task.refresh')} title={t('messagehub.task.refresh')} onClick={() => { void watch.refresh(taskId) }} data-testid="detail-task-refresh"><RefreshCw size={14} /></button>
      </div>
      <CopyValue label={t('messagehub.task.id')} value={taskId} testId="detail-task-id" />
      {!snapshot || snapshot.status === 'loading' ? <p className="text-[13px] text-[color:var(--cp-muted)]" role="status">{t('messagehub.task.loading')}</p> : null}
      {unavailable ? <p className="text-[13px] text-[color:var(--cp-muted)]" role="note" data-testid="detail-task-unavailable">{t(unavailable)}</p> : null}
      {task && state ? <>
        <div className="flex items-start gap-2" data-testid="detail-task-summary" data-state={state.label.key.split('.').at(-1)}>
          <ToneIcon tone={state.tone} />
          <div className="min-w-0 flex-1">
            <p className="break-words text-[13px] font-medium">{task.name}</p>
            <p className="break-words text-[13px]">{[text(state.label), task.message, task.phase === 'Waiting' ? task.waitReason?.message : ''].filter(Boolean).join(' · ')}</p>
            {snapshot?.stale ? <p className="text-xs text-[color:var(--cp-warning)]" role="note">{t('messagehub.task.staleHint')}</p> : null}
            <p className="text-xs text-[color:var(--cp-muted)]">{t('messagehub.task.updatedAt', undefined, { time: new Date(task.updatedAt).toLocaleString() })}</p>
            {task.originRef ? <p className="break-all text-xs text-[color:var(--cp-muted)]">{task.originRef.kind} · {task.originRef.id}</p> : null}
          </div>
        </div>
        <div>
          <h4 className={`${sectionTitleClass} mb-1.5`}>{t('messagehub.task.tree')}</h4>
          <TaskTree rootId={taskId} events />
        </div>
        <TaskEventList rootId={taskId} />
        {task.error ? <div data-testid="detail-task-error"><h4 className={`${sectionTitleClass} mb-1.5`}>{t('messagehub.task.error')}</h4><p className="break-words text-[13px] text-[color:var(--cp-danger)]">{task.error.code} · {task.error.message}</p></div> : null}
        {task.result !== undefined && task.result !== null ? <div data-testid="detail-task-result"><h4 className={`${sectionTitleClass} mb-1.5`}>{t('messagehub.task.result')}</h4><pre className="max-h-48 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-[color:color-mix(in_srgb,var(--cp-text)_5%,transparent)] p-2 text-xs">{formatJson(task.result)}</pre></div> : null}
        {artifacts.length > 0 ? <div data-testid="detail-task-artifacts"><h4 className={`${sectionTitleClass} mb-1.5`}>{t('messagehub.task.artifacts')}</h4><ul className="space-y-1 text-[13px]">{artifacts.map(item => <li key={item} className="break-all">{item}</li>)}</ul></div> : null}
      </> : null}
    </section>
  )
}

function TaskEventList({ rootId }: { rootId: string }) {
  const { t } = useI18n()
  const events = useTaskEvents(rootId)
  if (!events || events.status === 'loading') return null
  return (
    <div data-testid="detail-task-events">
      <h4 className={`${sectionTitleClass} mb-1.5`}>{t('messagehub.task.events')}</h4>
      {events.status === 'error' ? <p className="text-[13px] text-[color:var(--cp-muted)]" role="note">{t('messagehub.task.eventsUnavailable')}</p>
        : events.items.length === 0 ? <p className="text-[13px] text-[color:var(--cp-muted)]">{t('messagehub.task.noEvents')}</p>
        : <ol className="space-y-1 text-xs">{events.items.map(event => <li key={event.eventId} className="flex gap-2"><time className="shrink-0 tabular-nums text-[color:var(--cp-muted)]">{new Date(event.at).toLocaleTimeString()}</time><span className="min-w-0 break-words">{event.type}{event.taskId !== rootId ? ` · ${event.taskId}` : ''}</span></li>)}</ol>}
    </div>
  )
}
