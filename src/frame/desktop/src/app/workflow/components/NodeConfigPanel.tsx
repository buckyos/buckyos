/* ── Workflow node config panel (read-only) ── */

import { Copy, X } from 'lucide-react'
import type {
  AnalysisIssue,
  WorkflowGraphNode,
} from '../mock/types'

interface Props {
  node: WorkflowGraphNode
  issues: AnalysisIssue[]
  onClose: () => void
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="grid grid-cols-[110px_1fr] gap-2 py-1">
      <div className="text-[11px]" style={{ color: 'var(--cp-muted)' }}>
        {label}
      </div>
      <div className="text-xs" style={{ color: 'var(--cp-text)' }}>
        {children}
      </div>
    </div>
  )
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section
      className="mb-3 rounded-lg p-3"
      style={{
        background: 'var(--cp-surface)',
        border: '1px solid var(--cp-border)',
      }}
    >
      <div
        className="mb-1.5 text-[10px] font-semibold uppercase tracking-wider"
        style={{ color: 'var(--cp-muted)' }}
      >
        {title}
      </div>
      {children}
    </section>
  )
}

function Code({ value }: { value: string }) {
  return (
    <code
      className="rounded px-1 py-0.5 text-[11px]"
      style={{
        background: 'var(--cp-surface-2)',
        border: '1px solid var(--cp-border)',
        color: 'var(--cp-text)',
        fontFamily: 'ui-monospace, monospace',
      }}
    >
      {value}
    </code>
  )
}

function CopyButton({ text }: { text: string }) {
  return (
    <button
      type="button"
      onClick={() => navigator.clipboard.writeText(text).catch(() => {})}
      className="ml-1 inline-flex h-5 w-5 items-center justify-center rounded"
      style={{ color: 'var(--cp-muted)' }}
      title="Copy"
    >
      <Copy size={11} />
    </button>
  )
}

export function NodeConfigPanel({ node, issues, onClose }: Props) {
  return (
    <div
      className="flex h-full w-[340px] shrink-0 flex-col overflow-hidden"
      style={{
        background: 'var(--cp-surface-2)',
        borderLeft: '1px solid var(--cp-border)',
      }}
    >
      <div
        className="flex items-center gap-2 px-3 py-2.5"
        style={{ borderBottom: '1px solid var(--cp-border)' }}
      >
        <div className="flex-1 truncate text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
          {node.name}
        </div>
        <button
          type="button"
          onClick={onClose}
          className="flex h-6 w-6 items-center justify-center rounded"
          style={{ color: 'var(--cp-muted)' }}
        >
          <X size={14} />
        </button>
      </div>
      <div className="desktop-scrollbar flex-1 overflow-y-auto p-3">
        <Section title="Basics">
          <Field label="id">
            <Code value={node.id} />
            <CopyButton text={node.id} />
          </Field>
          <Field label="name">{node.name}</Field>
          {node.kind === 'task' ? (
            <Field label="step type">{node.stepType}</Field>
          ) : (
            <Field label="control">{node.controlType}</Field>
          )}
          {'description' in node && node.description && (
            <Field label="description">{node.description}</Field>
          )}
        </Section>

        {node.kind === 'task' && (
          <>
            <Section title="Executor">
              {node.executor ? (
                <>
                  <Field label="raw">
                    <Code value={node.executor.raw} />
                  </Field>
                  {node.executor.resolvedNamespace && (
                    <Field label="namespace">
                      {node.executor.resolvedNamespace}
                    </Field>
                  )}
                  {node.executor.resolvedTarget && (
                    <Field label="resolved">
                      <Code value={node.executor.resolvedTarget} />
                    </Field>
                  )}
                </>
              ) : (
                <div className="text-xs" style={{ color: 'var(--cp-muted)' }}>
                  No executor configured.
                </div>
              )}
            </Section>

            <Section title="Input">
              {node.inputBindings.length === 0 ? (
                <div className="text-xs" style={{ color: 'var(--cp-muted)' }}>
                  No input bindings.
                </div>
              ) : (
                node.inputBindings.map((b, i) => (
                  <Field key={i} label={b.field}>
                    {b.kind === 'literal' ? (
                      <Code value={JSON.stringify(b.value)} />
                    ) : (
                      <Code
                        value={`\${${b.nodeId}.output${
                          b.fieldPath.length ? '.' + b.fieldPath.join('.') : ''
                        }}`}
                      />
                    )}
                  </Field>
                ))
              )}
            </Section>

            <Section title="Output">
              <Field label="output_mode">{node.outputMode}</Field>
              {node.subjectRef && (
                <Field label="subject_ref">
                  <Code
                    value={`\${${node.subjectRef.nodeId}.output${
                      node.subjectRef.fieldPath.length
                        ? '.' + node.subjectRef.fieldPath.join('.')
                        : ''
                    }}`}
                  />
                </Field>
              )}
            </Section>

            <Section title="Behavior">
              <Field label="idempotent">{String(node.idempotent)}</Field>
              <Field label="skippable">{String(node.skippable)}</Field>
              {node.prompt && <Field label="prompt">{node.prompt}</Field>}
            </Section>

            {node.guards && (
              <Section title="Guards">
                {node.guards.budget && (
                  <>
                    {node.guards.budget.maxTokens != null && (
                      <Field label="max tokens">
                        {node.guards.budget.maxTokens.toLocaleString()}
                      </Field>
                    )}
                    {node.guards.budget.maxCostUsdb != null && (
                      <Field label="max cost (UsDB)">
                        {node.guards.budget.maxCostUsdb}
                      </Field>
                    )}
                    {node.guards.budget.maxDuration && (
                      <Field label="max duration">
                        {node.guards.budget.maxDuration}
                      </Field>
                    )}
                  </>
                )}
                {node.guards.retry && (
                  <Field label="retry">
                    {`${node.guards.retry.maxAttempts}× ${node.guards.retry.backoff} → ${node.guards.retry.fallback}`}
                  </Field>
                )}
                {node.guards.permissions && node.guards.permissions.length > 0 && (
                  <Field label="permissions">
                    {node.guards.permissions.join(', ')}
                  </Field>
                )}
              </Section>
            )}
          </>
        )}

        {node.kind === 'control' && node.controlType === 'branch' && (
          <Section title="Branch">
            <Field label="on">
              <Code
                value={`\${${node.on.nodeId}.output${
                  node.on.fieldPath.length ? '.' + node.on.fieldPath.join('.') : ''
                }}`}
              />
            </Field>
            <div className="mt-1 space-y-0.5">
              {Object.entries(node.paths).map(([k, v]) => (
                <Field key={k} label={k}>
                  → <Code value={v} />
                </Field>
              ))}
            </div>
            {node.maxIterations != null && (
              <Field label="max iterations">{node.maxIterations}</Field>
            )}
          </Section>
        )}

        {node.kind === 'control' && node.controlType === 'parallel' && (
          <Section title="Parallel">
            <Field label="branches">
              {node.branches.map((b) => (
                <Code key={b} value={b} />
              ))}
            </Field>
            <Field label="join">
              {`${node.join.strategy}${node.join.n ? ` (${node.join.n})` : ''}`}
            </Field>
          </Section>
        )}

        {node.kind === 'control' && node.controlType === 'for_each' && (
          <Section title="For each">
            <Field label="items">
              <Code
                value={`\${${node.items.nodeId}.output${
                  node.items.fieldPath.length
                    ? '.' + node.items.fieldPath.join('.')
                    : ''
                }}`}
              />
            </Field>
            <Field label="steps">
              {node.steps.map((s) => (
                <Code key={s} value={s} />
              ))}
            </Field>
            <Field label="max items">{node.maxItems.toLocaleString()}</Field>
            <Field label="concurrency">
              {node.effectiveConcurrency} / {node.concurrency}
              {node.degradedReason && (
                <span
                  className="ml-1 text-[10px]"
                  style={{ color: 'var(--cp-warning)' }}
                >
                  (downgraded)
                </span>
              )}
            </Field>
            {node.degradedReason && (
              <Field label="reason">
                <span style={{ color: 'var(--cp-warning)' }}>
                  {node.degradedReason}
                </span>
              </Field>
            )}
          </Section>
        )}

        {issues.length > 0 && (
          <Section title="Analysis">
            <div className="space-y-1.5">
              {issues.map((iss, i) => (
                <div
                  key={i}
                  className="rounded p-2 text-[11px]"
                  style={{
                    background: 'var(--cp-surface-2)',
                    border:
                      '1px solid ' +
                      (iss.severity === 'error'
                        ? 'var(--cp-danger)'
                        : iss.severity === 'warn'
                          ? 'var(--cp-warning)'
                          : 'var(--cp-border)'),
                    color: 'var(--cp-text)',
                  }}
                >
                  <div
                    className="text-[10px] font-semibold uppercase"
                    style={{
                      color:
                        iss.severity === 'error'
                          ? 'var(--cp-danger)'
                          : iss.severity === 'warn'
                            ? 'var(--cp-warning)'
                            : 'var(--cp-muted)',
                    }}
                  >
                    {iss.severity} · {iss.code}
                  </div>
                  <div className="mt-0.5">{iss.message}</div>
                </div>
              ))}
            </div>
          </Section>
        )}
      </div>
    </div>
  )
}
