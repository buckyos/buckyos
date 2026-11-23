/* ── Workflow node config panel (read-only) ── */

import { useI18n } from '../../../i18n/provider'
import { Copy, X } from 'lucide-react'
import type {
  AnalysisIssue,
  WorkflowGraphNode,
} from '../types'

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
  const { t } = useI18n()
  return (
    <button
      type="button"
      onClick={() => navigator.clipboard.writeText(text).catch(() => {})}
      className="ml-1 inline-flex h-5 w-5 items-center justify-center rounded"
      style={{ color: 'var(--cp-muted)' }}
      title={t('common.copy')}
    >
      <Copy size={11} />
    </button>
  )
}

export function NodeConfigPanel({ node, issues, onClose }: Props) {
  const { t, locale } = useI18n()
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
          aria-label={t('common.close')}
          onClick={onClose}
          className="flex h-6 w-6 items-center justify-center rounded"
          style={{ color: 'var(--cp-muted)' }}
        >
          <X size={14} />
        </button>
      </div>
      <div className="desktop-scrollbar flex-1 overflow-y-auto p-3">
        <Section title={t('workflow.basics')}>
          <Field label={t('workflow.field.id')}>
            <Code value={node.id} />
            <CopyButton text={node.id} />
          </Field>
          <Field label={t('workflow.field.name')}>{node.name}</Field>
          {node.kind === 'task' ? (
            <Field label={t('workflow.field.stepType')}>{t(`workflow.node.${node.stepType}`, node.stepType)}</Field>
          ) : (
            <Field label={t('workflow.field.control')}>{t(`workflow.node.${node.controlType}`, node.controlType)}</Field>
          )}
          {'description' in node && node.description && (
            <Field label={t('workflow.field.description')}>{node.description}</Field>
          )}
        </Section>

        {node.kind === 'task' && (
          <>
            <Section title={t('workflow.executor')}>
              {node.executor ? (
                <>
                  <Field label={t('workflow.field.raw')}>
                    <Code value={node.executor.raw} />
                  </Field>
                  {node.executor.resolvedNamespace && (
                    <Field label={t('workflow.field.namespace')}>
                      {node.executor.resolvedNamespace}
                    </Field>
                  )}
                  {node.executor.resolvedTarget && (
                    <Field label={t('workflow.field.resolved')}>
                      <Code value={node.executor.resolvedTarget} />
                    </Field>
                  )}
                </>
              ) : (
                <div className="text-xs" style={{ color: 'var(--cp-muted)' }}>
                  {t('workflow.noExecutor')}
                </div>
              )}
            </Section>

            <Section title={t('workflow.input')}>
              {node.inputBindings.length === 0 ? (
                <div className="text-xs" style={{ color: 'var(--cp-muted)' }}>
                  {t('workflow.noBindings')}
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

            <Section title={t('workflow.output')}>
              <Field label={t('workflow.field.outputMode')}>{t(`workflow.node.${node.outputMode}`, node.outputMode)}</Field>
              {node.subjectRef && (
                <Field label={t('workflow.field.subjectRef')}>
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

            <Section title={t('workflow.behavior')}>
              <Field label={t('workflow.field.idempotent')}>{node.idempotent ? t('workflow.yes') : t('workflow.no')}</Field>
              <Field label={t('workflow.field.skippable')}>{node.skippable ? t('workflow.yes') : t('workflow.no')}</Field>
              {node.prompt && <Field label={t('workflow.field.prompt')}>{node.prompt}</Field>}
            </Section>

            {node.guards && (
              <Section title={t('workflow.guards')}>
                {node.guards.budget && (
                  <>
                    {node.guards.budget.maxTokens != null && (
                      <Field label={t('workflow.field.maxTokens')}>
                        {node.guards.budget.maxTokens.toLocaleString(locale)}
                      </Field>
                    )}
                    {node.guards.budget.maxCostUsdb != null && (
                      <Field label={t('workflow.field.maxCost')}>
                        {node.guards.budget.maxCostUsdb}
                      </Field>
                    )}
                    {node.guards.budget.maxDuration && (
                      <Field label={t('workflow.field.maxDuration')}>
                        {node.guards.budget.maxDuration}
                      </Field>
                    )}
                  </>
                )}
                {node.guards.retry && (
                  <Field label={t('workflow.field.retry')}>
                    {`${node.guards.retry.maxAttempts}× ${node.guards.retry.backoff} → ${node.guards.retry.fallback}`}
                  </Field>
                )}
                {node.guards.permissions && node.guards.permissions.length > 0 && (
                  <Field label={t('workflow.field.permissions')}>
                    {node.guards.permissions.join(', ')}
                  </Field>
                )}
              </Section>
            )}
          </>
        )}

        {node.kind === 'control' && node.controlType === 'branch' && (
          <Section title={t('workflow.node.branch')}>
            <Field label={t('workflow.field.on')}>
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
              <Field label={t('workflow.field.maxIterations')}>{node.maxIterations}</Field>
            )}
          </Section>
        )}

        {node.kind === 'control' && node.controlType === 'parallel' && (
          <Section title={t('workflow.node.parallel')}>
            <Field label={t('workflow.field.branches')}>
              {node.branches.map((b) => (
                <Code key={b} value={b} />
              ))}
            </Field>
            <Field label={t('workflow.field.join')}>
              {`${node.join.strategy}${node.join.n ? ` (${node.join.n})` : ''}`}
            </Field>
          </Section>
        )}

        {node.kind === 'control' && node.controlType === 'for_each' && (
          <Section title={t('workflow.node.for_each')}>
            <Field label={t('workflow.field.items')}>
              <Code
                value={`\${${node.items.nodeId}.output${
                  node.items.fieldPath.length
                    ? '.' + node.items.fieldPath.join('.')
                    : ''
                }}`}
              />
            </Field>
            <Field label={t('workflow.steps')}>
              {node.steps.map((s) => (
                <Code key={s} value={s} />
              ))}
            </Field>
            <Field label={t('workflow.field.maxItems')}>{node.maxItems.toLocaleString(locale)}</Field>
            <Field label={t('workflow.field.concurrency')}>
              {node.effectiveConcurrency} / {node.concurrency}
              {node.effectiveConcurrency < node.concurrency && (
                <span
                  className="ml-1 text-[10px]"
                  style={{ color: 'var(--cp-warning)' }}
                >
                  {t('workflow.downgraded')}
                </span>
              )}
            </Field>
            {node.effectiveConcurrency < node.concurrency && (
              <Field label={t('workflow.field.reason')}>
                <span style={{ color: 'var(--cp-warning)' }}>
                  {t('workflow.degradedReason', undefined, { count: node.effectiveConcurrency })}
                </span>
              </Field>
            )}
          </Section>
        )}

        {issues.length > 0 && (
          <Section title={t('workflow.analysis')}>
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
                    {t(`workflow.severity.${iss.severity}`, iss.severity)} · {iss.code}
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
