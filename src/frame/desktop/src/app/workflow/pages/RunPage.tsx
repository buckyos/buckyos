/* ── Run detail page: per-step state / source / structured error, run output, TaskMgr link ── */

import { useEffect, useState } from 'react'
import { ArrowLeft, ExternalLink, OctagonX, RefreshCw } from 'lucide-react'
import { useWorkflowStore } from '../hooks/use-workflow-store'
import type { NodeRunState, RunStepView, StepError, WorkflowSelection } from '../types'
import { formatTime, runStatusStyle } from '../components/run-status'

interface Props {
  runId: string
  onSelect: (s: WorkflowSelection) => void
}

const stateColor: Record<NodeRunState, string> = {
  pending: 'var(--cp-muted)',
  ready: 'var(--cp-muted)',
  running: 'var(--cp-accent)',
  completed: 'var(--cp-success)',
  failed: 'var(--cp-danger)',
  retrying: 'var(--cp-warning)',
  waiting_human: 'var(--cp-warning)',
  skipped: 'var(--cp-muted)',
  aborted: 'var(--cp-muted)',
  cancelled: 'var(--cp-muted)',
}

const sourceLabel: Record<string, string> = {
  cache: 'cache hit',
  executed: 'executed',
  direct: 'direct adapter',
  submitted: 'agent submitted',
}

function taskCenterHref(runId: string): string {
  return `/taskcenter?taskid=${encodeURIComponent(runId)}`
}

export function RunPage({ runId, onSelect }: Props) {
  const store = useWorkflowStore()
  const [cancelError, setCancelError] = useState<string | null>(null)
  useEffect(() => {
    void store.watchRun(runId)
    return () => store.unwatchRun(runId)
  }, [store, runId])

  const run = store.getRunDetail(runId)
  if (!run) {
    return (
      <div className="flex h-full items-center justify-center text-xs" style={{ color: 'var(--cp-muted)' }}>
        {store.error ?? 'Loading run…'}
      </div>
    )
  }
  const sty = runStatusStyle[run.status]
  const definition = store.getDefinition(run.definitionId)
  const interrupted = run.steps.some((s) => s.error?.code === 'interrupted')

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex items-center gap-3 px-5 py-3" style={{ borderBottom: '1px solid var(--cp-border)' }}>
        <button
          type="button"
          onClick={() => onSelect({ kind: 'definition', definitionId: run.definitionId })}
          className="flex h-7 w-7 items-center justify-center rounded-lg"
          style={{ background: 'var(--cp-surface-2)', color: 'var(--cp-text)', border: '1px solid var(--cp-border)' }}
          title="Back to definition"
        >
          <ArrowLeft size={14} />
        </button>
        <div className="min-w-0 flex-1">
          <div className="text-[10px] uppercase tracking-wider" style={{ color: 'var(--cp-muted)' }}>
            Run · {definition?.name ?? run.definitionName}
          </div>
          <div className="mt-0.5 flex items-center gap-2 text-base font-semibold" style={{ color: 'var(--cp-text)' }}>
            <span className="truncate font-mono text-sm">{run.runId}</span>
            <span
              className="inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] uppercase"
              style={{ background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)', color: sty.color }}
            >
              <span className="h-1.5 w-1.5 rounded-full" style={{ background: sty.color }} />
              {sty.label}
            </span>
            {interrupted && (
              <span className="rounded-full px-2 py-0.5 text-[10px] uppercase" style={{ background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)', color: 'var(--cp-warning)' }}>
                interrupted
              </span>
            )}
          </div>
          <div className="mt-0.5 text-[11px]" style={{ color: 'var(--cp-muted)' }}>
            requested by {run.requestedBy ?? '—'} · executor {run.executor} · revision{' '}
            <span className="font-mono" title={run.revision}>{run.revision.slice(0, 12)}</span> · epoch {run.executionEpoch}
            {' · '}created {formatTime(run.createdAt)} · updated {formatTime(run.updatedAt)}
          </div>
        </div>
        <button
          type="button"
          onClick={() => void store.loadRunDetail(runId)}
          className="inline-flex items-center gap-1 rounded-md px-2.5 py-1 text-[11px]"
          style={{ background: 'var(--cp-surface-2)', color: 'var(--cp-text)', border: '1px solid var(--cp-border)' }}
        >
          <RefreshCw size={12} /> Refresh
        </button>
        <a
          href={taskCenterHref(run.runId)}
          title={`TaskMgr task ${run.runId}`}
          target="_blank"
          rel="noreferrer"
          className="inline-flex items-center gap-1 rounded-md px-2.5 py-1 text-[11px]"
          style={{ background: 'var(--cp-surface-2)', color: 'var(--cp-accent)', border: '1px solid var(--cp-border)' }}
        >
          TaskMgr tree <ExternalLink size={12} />
        </a>
        {!run.terminal && (
          <button
            type="button"
            onClick={() => {
              void store.cancelRun(run.runId, 'cancelled from Workflow WebUI').then(setCancelError)
            }}
            className="inline-flex items-center gap-1 rounded-md px-2.5 py-1 text-[11px]"
            style={{ background: 'var(--cp-surface-2)', color: 'var(--cp-danger)', border: '1px solid var(--cp-border)' }}
          >
            <OctagonX size={12} /> Cancel run
          </button>
        )}
      </div>

      <div className="desktop-scrollbar flex-1 overflow-y-auto px-5 py-4">
        {cancelError && <Notice tone="danger" text={cancelError} />}
        {run.runError && (
          <Notice tone={run.runError.code === 'cancelled' ? 'muted' : 'danger'} text={`${run.runError.code}: ${run.runError.message}`} />
        )}
        {run.humanWaitingNodes.length > 0 && (
          <Notice tone="warning" text={`Waiting for a human decision on: ${run.humanWaitingNodes.join(', ')} (act in TaskMgr).`} />
        )}

        <SectionLabel text="Steps" />
        <div className="overflow-hidden rounded-lg" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
          {run.steps.map((step, i) => (
            <StepRow key={step.nodeId} step={step} first={i === 0} />
          ))}
        </div>

        <SectionLabel text="Run output (terminal nodes)" />
        {Object.keys(run.output).length === 0 ? (
          <div className="rounded-lg p-3 text-xs" style={{ background: 'var(--cp-surface)', border: '1px dashed var(--cp-border)', color: 'var(--cp-muted)' }}>
            {run.terminal ? 'No output recorded.' : 'Run has not completed; output is not final.'}
          </div>
        ) : (
          Object.entries(run.output).map(([node, value]) => (
            <OutputBlock key={node} node={node} value={value} />
          ))
        )}
      </div>
    </div>
  )
}

function StepRow({ step, first }: { step: RunStepView; first: boolean }) {
  const [open, setOpen] = useState(false)
  const color = stateColor[step.state]
  const summary = step.output ? summarizeOutput(step.output) : null
  return (
    <div className="px-3 py-2 text-xs" style={{ borderTop: first ? undefined : '1px solid var(--cp-border)', color: 'var(--cp-text)' }}>
      <div className="flex items-center gap-2">
        <span className="h-2 w-2 shrink-0 rounded-full" style={{ background: color }} />
        <span className="font-semibold">{step.name}</span>
        <span className="font-mono text-[10px]" style={{ color: 'var(--cp-muted)' }}>{step.nodeId}</span>
        <span className="rounded px-1 text-[9px] uppercase" style={{ color, background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}>
          {step.state}
        </span>
        {step.source && (
          <span
            className="rounded px-1 text-[9px]"
            style={{
              color: step.source === 'cache' ? 'var(--cp-success)' : 'var(--cp-muted)',
              background: 'var(--cp-surface-2)',
              border: '1px solid var(--cp-border)',
            }}
          >
            {sourceLabel[step.source] ?? step.source}
          </span>
        )}
        {step.attempt > 1 && (
          <span className="text-[10px]" style={{ color: 'var(--cp-warning)' }}>attempt {step.attempt}</span>
        )}
        {step.pendingExecutionIds.length > 0 && (
          <span className="truncate font-mono text-[10px]" style={{ color: 'var(--cp-muted)' }} title={step.pendingExecutionIds.join(', ')}>
            exec {step.pendingExecutionIds[0]}
          </span>
        )}
        {(summary || step.error) && (
          <button
            type="button"
            onClick={() => setOpen((v) => !v)}
            className="ml-auto text-[10px]"
            style={{ color: 'var(--cp-accent)' }}
          >
            {open ? 'hide' : 'details'}
          </button>
        )}
      </div>
      {step.error && <ErrorBlock error={step.error} />}
      {summary && !open && (
        <div className="mt-1 truncate text-[11px]" style={{ color: 'var(--cp-muted)' }}>{summary}</div>
      )}
      {open && step.output !== undefined && (
        <pre className="mt-1 max-h-64 overflow-auto rounded p-2 font-mono text-[10px]" style={{ background: 'var(--cp-surface-2)', color: 'var(--cp-text)' }}>
          {JSON.stringify(step.output, null, 2)}
        </pre>
      )}
    </div>
  )
}

function ErrorBlock({ error }: { error: StepError }) {
  const details = error.details
  return (
    <div
      className="mt-1 rounded p-2 text-[11px]"
      style={{
        background: 'color-mix(in srgb, var(--cp-danger) 8%, var(--cp-surface))',
        border: '1px solid color-mix(in srgb, var(--cp-danger) 30%, var(--cp-border))',
        color: 'var(--cp-text)',
      }}
    >
      <span className="font-mono font-semibold" style={{ color: 'var(--cp-danger)' }}>{error.code}</span>
      {error.retryable && <span className="ml-2 text-[10px]" style={{ color: 'var(--cp-warning)' }}>retryable</span>}
      <span className="ml-2">{error.message}</span>
      {details !== undefined && details !== null && (
        <pre className="mt-1 max-h-40 overflow-auto font-mono text-[10px]" style={{ color: 'var(--cp-muted)' }}>
          {JSON.stringify(details, null, 2)}
        </pre>
      )}
    </div>
  )
}

function OutputBlock({ node, value }: { node: string; value: unknown }) {
  const manifest = value && typeof value === 'object' ? (value as { summary?: Record<string, unknown>; complete?: boolean; manifest_obj_id?: string }) : null
  return (
    <div className="mb-2 rounded-lg p-3" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
      <div className="mb-1 flex items-center gap-2 text-[11px] font-semibold" style={{ color: 'var(--cp-text)' }}>
        <span className="font-mono">{node}</span>
        {manifest?.complete === false && (
          <span className="rounded px-1 text-[9px] uppercase" style={{ color: 'var(--cp-warning)', background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}>
            partial
          </span>
        )}
      </div>
      {manifest?.summary && (
        <div className="mb-1 flex flex-wrap gap-2 text-[10px]" style={{ color: 'var(--cp-muted)' }}>
          {Object.entries(manifest.summary).map(([k, v]) => (
            <span key={k} className="rounded px-1" style={{ background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}>
              {k}: {String(v)}
            </span>
          ))}
        </div>
      )}
      <pre className="max-h-64 overflow-auto rounded p-2 font-mono text-[10px]" style={{ background: 'var(--cp-surface-2)', color: 'var(--cp-text)' }}>
        {JSON.stringify(value, null, 2)}
      </pre>
    </div>
  )
}

function summarizeOutput(value: unknown): string {
  const text = JSON.stringify(value)
  return text.length > 160 ? `${text.slice(0, 157)}…` : text
}

function SectionLabel({ text }: { text: string }) {
  return (
    <div className="mb-2 mt-4 text-[10px] font-semibold uppercase tracking-wider" style={{ color: 'var(--cp-muted)' }}>
      {text}
    </div>
  )
}

function Notice({ tone, text }: { tone: 'danger' | 'warning' | 'muted'; text: string }) {
  const color = tone === 'danger' ? 'var(--cp-danger)' : tone === 'warning' ? 'var(--cp-warning)' : 'var(--cp-muted)'
  return (
    <div
      className="mb-2 rounded-lg px-3 py-2 text-xs"
      style={{
        background: `color-mix(in srgb, ${color} 10%, var(--cp-surface))`,
        border: `1px solid color-mix(in srgb, ${color} 35%, var(--cp-border))`,
        color,
      }}
    >
      {text}
    </div>
  )
}
