/* ── Workflow home / welcome page ── */

import { useI18n } from '../../../i18n/provider'
import { FileCode2, Lock, Plus, Sparkles, Workflow as WorkflowIcon } from 'lucide-react'
import { useWorkflowStore } from '../hooks/use-workflow-store'
import type { WorkflowSelection } from '../types'
import { RunsList } from '../components/RunsList'

interface Props {
  onSelect: (s: WorkflowSelection) => void
  onImport: () => void
}

export function HomePage({ onSelect, onImport }: Props) {
  const { t } = useI18n()
  const store = useWorkflowStore()
  const defs = store.listDefinitions()
  const runs = store.listRuns()
  const errorCount = defs.reduce((a, d) => a + d.analysis.errorCount, 0)
  const warnCount = defs.reduce((a, d) => a + d.analysis.warnCount, 0)
  const failed = runs.filter((r) => r.status === 'failed' || r.status === 'budget_exhausted').length

  return (
    <div className="space-y-5">
      <div
        className="rounded-2xl px-5 py-4"
        style={{
          background:
            'linear-gradient(135deg, color-mix(in srgb, var(--cp-accent) 16%, var(--cp-surface)), var(--cp-surface))',
          border: '1px solid var(--cp-border)',
        }}
      >
        <div className="flex items-start gap-3">
          <div
            className="flex h-10 w-10 items-center justify-center rounded-xl"
            style={{ background: 'color-mix(in srgb, var(--cp-accent) 20%, transparent)', color: 'var(--cp-accent)' }}
          >
            <WorkflowIcon size={20} />
          </div>
          <div className="flex-1">
            <div className="text-base font-semibold" style={{ color: 'var(--cp-text)' }}>
              {t('workflow.definitionsTitle')}
            </div>
            <p className="mt-1 text-xs" style={{ color: 'var(--cp-muted)' }}>
              {t('workflow.intro')}
            </p>
          </div>
        </div>
        <div className="mt-3 flex flex-wrap gap-2">
          <button
            type="button"
            onClick={onImport}
            className="inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs"
            style={{ background: 'var(--cp-accent)', color: 'white', border: '1px solid var(--cp-border)' }}
          >
            <Plus size={13} /> {t('workflow.importDefinition')}
          </button>
          <button
            type="button"
            onClick={() => onSelect({ kind: 'ai_prompt' })}
            className="inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs"
            style={{ background: 'var(--cp-surface-2)', color: 'var(--cp-text)', border: '1px solid var(--cp-border)' }}
          >
            <Sparkles size={13} /> {t('workflow.usePrompt')}
          </button>
        </div>
      </div>

      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        <Stat label={t('workflow.definitions')} value={t('workflow.systemCount', undefined, { total: defs.length, count: defs.filter((d) => d.source === 'system').length })} />
        <Stat label={t('workflow.activeRuns')} value={store.activeRunCount().toString()} />
        <Stat
          label={t('workflow.issues')}
          value={`${warnCount} / ${errorCount}`}
          tone={errorCount > 0 ? 'danger' : warnCount > 0 ? 'warning' : 'normal'}
        />
        <Stat label={t('workflow.failedRuns')} value={failed.toString()} tone={failed > 0 ? 'danger' : 'normal'} />
      </div>

      <section>
        <div className="mb-2 text-[10px] font-semibold uppercase tracking-wider" style={{ color: 'var(--cp-muted)' }}>
          {t('workflow.definitions')}
        </div>
        <div className="grid gap-2 sm:grid-cols-2">
          {defs.slice(0, 8).map((d) => (
            <button
              key={d.id}
              type="button"
              onClick={() => onSelect({ kind: 'definition', definitionId: d.id })}
              className="flex items-start gap-2 rounded-lg p-3 text-left"
              style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
            >
              {d.readOnly ? (
                <Lock size={14} style={{ color: 'var(--cp-muted)' }} />
              ) : (
                <FileCode2 size={14} style={{ color: 'var(--cp-muted)' }} />
              )}
              <div className="flex-1 min-w-0">
                <div className="text-xs font-semibold" style={{ color: 'var(--cp-text)' }}>
                  {d.name}{' '}
                  <span className="text-[10px]" style={{ color: 'var(--cp-muted)' }}>
                    v{d.version}
                  </span>
                </div>
                <div className="mt-0.5 truncate text-[11px]" style={{ color: 'var(--cp-muted)' }}>
                  {d.description ?? '—'}
                </div>
                <div className="mt-1 flex items-center gap-1 text-[10px]">
                  <span
                    className="rounded px-1 uppercase"
                    style={{
                      background: 'var(--cp-surface-2)',
                      color:
                        d.status === 'active'
                          ? 'var(--cp-success)'
                          : d.status === 'draft'
                            ? 'var(--cp-warning)'
                            : 'var(--cp-muted)',
                    }}
                  >
                    {t(`workflow.status.${d.status}`, d.status)}
                  </span>
                  <span className="rounded px-1" style={{ background: 'var(--cp-surface-2)', color: 'var(--cp-muted)' }}>
                    {t(`workflow.source.${d.source}`, d.source)}
                  </span>
                  <span style={{ color: 'var(--cp-muted)' }}>{t('workflow.nodeCount', undefined, { count: d.graph.nodes.length })}</span>
                </div>
              </div>
            </button>
          ))}
          {defs.length === 0 && (
            <div className="text-xs" style={{ color: 'var(--cp-muted)' }}>
              {store.loading ? t('states.loading') : store.error ?? t('workflow.noDefinitions')}
            </div>
          )}
        </div>
      </section>

      <section>
        <div className="mb-2 text-[10px] font-semibold uppercase tracking-wider" style={{ color: 'var(--cp-muted)' }}>
          {t('workflow.recentRuns')}
        </div>
        <RunsList runs={runs.slice(0, 10)} onOpen={(runId) => onSelect({ kind: 'run', runId })} />
      </section>
    </div>
  )
}

function Stat({
  label,
  value,
  tone = 'normal',
}: {
  label: string
  value: string
  tone?: 'normal' | 'warning' | 'danger'
}) {
  const accent =
    tone === 'danger' ? 'var(--cp-danger)' : tone === 'warning' ? 'var(--cp-warning)' : 'var(--cp-text)'
  return (
    <div className="rounded-lg p-3" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
      <div className="text-[10px] uppercase" style={{ color: 'var(--cp-muted)' }}>{label}</div>
      <div className="mt-1 text-lg font-semibold" style={{ color: accent }}>{value}</div>
    </div>
  )
}
