/* ── Workflow home / welcome page ── */

import { FileCode2, Plus, Sparkles, Workflow as WorkflowIcon } from 'lucide-react'
import { useWorkflowStore } from '../hooks/use-workflow-store'
import type { WorkflowSelection } from '../mock/types'

interface Props {
  onSelect: (s: WorkflowSelection) => void
  onImport: () => void
}

export function HomePage({ onSelect, onImport }: Props) {
  const store = useWorkflowStore()
  const defs = store.listDefinitions()
  const apps = store.listApps()
  const errorCount = defs.reduce((a, d) => a + d.analysis.errorCount, 0)
  const warnCount = defs.reduce((a, d) => a + d.analysis.warnCount, 0)
  const requiredMissing = apps.flatMap((a) =>
    a.mountPoints.filter((m) => m.required && !m.currentBinding).map((m) => ({ app: a, mp: m })),
  )

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
            style={{
              background: 'color-mix(in srgb, var(--cp-accent) 20%, transparent)',
              color: 'var(--cp-accent)',
            }}
          >
            <WorkflowIcon size={20} />
          </div>
          <div className="flex-1">
            <div className="text-base font-semibold" style={{ color: 'var(--cp-text)' }}>
              Workflow Definitions
            </div>
            <p className="mt-1 text-xs" style={{ color: 'var(--cp-muted)' }}>
              Browse Definitions, attach them to application mount points, and use TaskMgr to
              monitor Run progress. WebUI is read-only for runtime — all approval / retry / abort
              actions live in TaskMgr UI.
            </p>
          </div>
        </div>
        <div className="mt-3 flex flex-wrap gap-2">
          <button
            type="button"
            onClick={onImport}
            className="inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs"
            style={{
              background: 'var(--cp-accent)',
              color: 'white',
              border: '1px solid var(--cp-border)',
            }}
          >
            <Plus size={13} /> Import Definition
          </button>
          <button
            type="button"
            onClick={() => onSelect({ kind: 'ai_prompt' })}
            className="inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs"
            style={{
              background: 'var(--cp-surface-2)',
              color: 'var(--cp-text)',
              border: '1px solid var(--cp-border)',
            }}
          >
            <Sparkles size={13} /> Use AI prompt
          </button>
        </div>
      </div>

      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        <Stat label="Definitions" value={defs.length.toString()} />
        <Stat label="Mount points" value={apps.reduce((a, x) => a + x.mountPoints.length, 0).toString()} />
        <Stat
          label="Issues (warn / error)"
          value={`${warnCount} / ${errorCount}`}
          tone={errorCount > 0 ? 'danger' : warnCount > 0 ? 'warning' : 'normal'}
        />
        <Stat
          label="Required-missing"
          value={requiredMissing.length.toString()}
          tone={requiredMissing.length > 0 ? 'danger' : 'normal'}
        />
      </div>

      <section>
        <div
          className="mb-2 text-[10px] font-semibold uppercase tracking-wider"
          style={{ color: 'var(--cp-muted)' }}
        >
          Definitions
        </div>
        <div className="grid gap-2 sm:grid-cols-2">
          {defs.slice(0, 6).map((d) => (
            <button
              key={d.id}
              type="button"
              onClick={() => onSelect({ kind: 'definition', definitionId: d.id })}
              className="flex items-start gap-2 rounded-lg p-3 text-left"
              style={{
                background: 'var(--cp-surface)',
                border: '1px solid var(--cp-border)',
              }}
            >
              <FileCode2 size={14} style={{ color: 'var(--cp-muted)' }} />
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
                    {d.status}
                  </span>
                  <span style={{ color: 'var(--cp-muted)' }}>
                    {d.graph.nodes.length} nodes
                  </span>
                </div>
              </div>
            </button>
          ))}
        </div>
      </section>

      {requiredMissing.length > 0 && (
        <section>
          <div
            className="mb-2 text-[10px] font-semibold uppercase tracking-wider"
            style={{ color: 'var(--cp-danger)' }}
          >
            Required mount points without binding
          </div>
          <div className="space-y-1.5">
            {requiredMissing.map(({ app, mp }) => (
              <button
                key={`${app.id}/${mp.id}`}
                type="button"
                onClick={() =>
                  onSelect({ kind: 'mount', appId: app.id, mountPointId: mp.id })
                }
                className="flex w-full items-center gap-2 rounded-lg p-2.5 text-left"
                style={{
                  background: 'color-mix(in srgb, var(--cp-danger) 10%, var(--cp-surface))',
                  border: '1px solid color-mix(in srgb, var(--cp-danger) 35%, var(--cp-border))',
                }}
              >
                <div className="flex-1 min-w-0">
                  <div className="text-xs" style={{ color: 'var(--cp-text)' }}>
                    {app.name} / {mp.name}
                  </div>
                  <div className="text-[11px]" style={{ color: 'var(--cp-danger)' }}>
                    Required mount point not configured.
                  </div>
                </div>
              </button>
            ))}
          </div>
        </section>
      )}
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
    tone === 'danger'
      ? 'var(--cp-danger)'
      : tone === 'warning'
        ? 'var(--cp-warning)'
        : 'var(--cp-text)'
  return (
    <div
      className="rounded-lg p-3"
      style={{
        background: 'var(--cp-surface)',
        border: '1px solid var(--cp-border)',
      }}
    >
      <div className="text-[10px] uppercase" style={{ color: 'var(--cp-muted)' }}>
        {label}
      </div>
      <div className="mt-1 text-lg font-semibold" style={{ color: accent }}>
        {value}
      </div>
    </div>
  )
}
