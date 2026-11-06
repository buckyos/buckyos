/* ── Workflow sidebar: Definitions (system / mine) + recent Runs ── */

import { useState } from 'react'
import {
  AlertTriangle,
  ChevronDown,
  ChevronRight,
  FileCode2,
  ListChecks,
  Lock,
  Plus,
  RefreshCw,
  Search,
  Sparkles,
  Workflow as WorkflowIcon,
} from 'lucide-react'
import { useWorkflowStore } from '../hooks/use-workflow-store'
import type { WorkflowDefinition, WorkflowRunSummary, WorkflowSelection } from '../types'
import { runStatusStyle, sourceLabels, statusBadge } from './run-status'

interface SidebarProps {
  selection: WorkflowSelection
  onSelect: (selection: WorkflowSelection) => void
  onImport: () => void
}

export function WorkflowSidebar({ selection, onSelect, onImport }: SidebarProps) {
  const store = useWorkflowStore()
  const [search, setSearch] = useState('')
  const [showArchived, setShowArchived] = useState(false)
  const [expanded, setExpanded] = useState<Record<string, boolean>>({
    system: true,
    mine: true,
    runs: true,
  })

  const definitions = store.listDefinitions().filter((d) => {
    if (!showArchived && d.status === 'archived') return false
    if (search) {
      const q = search.toLowerCase()
      if (!d.name.toLowerCase().includes(q) && !d.id.toLowerCase().includes(q)) return false
    }
    return true
  })
  const systemDefs = definitions.filter((d) => d.source === 'system')
  const ownDefs = definitions.filter((d) => d.source !== 'system')
  const runs = store.listRuns().slice(0, 30)

  function isDefSelected(d: WorkflowDefinition) {
    return selection.kind === 'definition' && selection.definitionId === d.id
  }

  function renderDefinition(d: WorkflowDefinition) {
    const sb = statusBadge(d.status)
    const errors = d.analysis.errorCount
    const warns = d.analysis.warnCount
    const selected = isDefSelected(d)
    return (
      <button
        key={d.id}
        type="button"
        onClick={() => onSelect({ kind: 'definition', definitionId: d.id })}
        className="flex w-full items-start gap-2 rounded-lg px-2 py-1.5 text-left transition-colors"
        style={{
          background: selected
            ? 'color-mix(in srgb, var(--cp-accent) 12%, var(--cp-surface))'
            : 'transparent',
          border: selected
            ? '1px solid color-mix(in srgb, var(--cp-accent) 22%, var(--cp-border))'
            : '1px solid transparent',
        }}
      >
        {d.readOnly ? (
          <Lock size={13} className="mt-0.5 shrink-0" style={{ color: 'var(--cp-muted)' }} />
        ) : (
          <FileCode2 size={13} className="mt-0.5 shrink-0" style={{ color: 'var(--cp-muted)' }} />
        )}
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5 text-xs" style={{ color: 'var(--cp-text)' }}>
            <span className="truncate">{d.name}</span>
            <span className="text-[10px]" style={{ color: 'var(--cp-muted)' }}>
              v{d.version}
            </span>
          </div>
          <div className="mt-0.5 flex items-center gap-1">
            <span
              className="rounded px-1 py-px text-[9px] uppercase tracking-wide"
              style={{ background: sb.bg, color: sb.fg }}
            >
              {sb.label}
            </span>
            <span
              className="rounded px-1 py-px text-[9px]"
              style={{
                background: 'var(--cp-surface-2)',
                color: 'var(--cp-muted)',
                border: '1px solid var(--cp-border)',
              }}
            >
              {sourceLabels[d.source]}
            </span>
            {errors > 0 && (
              <span className="flex items-center gap-0.5 text-[10px]" style={{ color: 'var(--cp-danger)' }}>
                <AlertTriangle size={10} />
                {errors}
              </span>
            )}
            {warns > 0 && (
              <span className="flex items-center gap-0.5 text-[10px]" style={{ color: 'var(--cp-warning)' }}>
                <AlertTriangle size={10} />
                {warns}
              </span>
            )}
          </div>
        </div>
      </button>
    )
  }

  function renderRun(r: WorkflowRunSummary) {
    const sty = runStatusStyle[r.status]
    const selected = selection.kind === 'run' && selection.runId === r.runId
    return (
      <button
        key={r.runId}
        type="button"
        onClick={() => onSelect({ kind: 'run', runId: r.runId })}
        className="flex w-full items-start gap-2 rounded-lg px-2 py-1.5 text-left"
        style={{
          background: selected
            ? 'color-mix(in srgb, var(--cp-accent) 12%, var(--cp-surface))'
            : 'transparent',
          border: selected
            ? '1px solid color-mix(in srgb, var(--cp-accent) 22%, var(--cp-border))'
            : '1px solid transparent',
        }}
      >
        <span className="mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full" style={{ background: sty.color }} />
        <div className="min-w-0 flex-1">
          <div className="truncate text-xs" style={{ color: 'var(--cp-text)' }}>
            {r.definitionName}
          </div>
          <div className="mt-0.5 flex items-center gap-1 text-[10px]" style={{ color: 'var(--cp-muted)' }}>
            <span style={{ color: sty.color }}>{sty.label}</span>
            <span className="truncate font-mono">{r.runId.slice(0, 16)}</span>
          </div>
        </div>
      </button>
    )
  }

  return (
    <aside
      className="flex h-full w-72 shrink-0 flex-col overflow-hidden"
      style={{ borderRight: '1px solid var(--cp-border)' }}
    >
      <div
        className="flex items-center gap-2 px-3 py-3"
        style={{ borderBottom: '1px solid var(--cp-border)' }}
      >
        <div
          className="flex h-7 w-7 items-center justify-center rounded-lg"
          style={{
            background: 'color-mix(in srgb, var(--cp-accent) 16%, transparent)',
            color: 'var(--cp-accent)',
          }}
        >
          <WorkflowIcon size={15} />
        </div>
        <div className="flex-1 text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
          Workflow
        </div>
        <button
          type="button"
          onClick={() => void store.load()}
          title="Reload"
          className="flex h-7 w-7 items-center justify-center rounded-lg"
          style={{
            background: 'var(--cp-surface-2)',
            color: store.loading ? 'var(--cp-accent)' : 'var(--cp-text)',
            border: '1px solid var(--cp-border)',
          }}
        >
          <RefreshCw size={14} />
        </button>
        <button
          type="button"
          onClick={onImport}
          title="Import Workflow"
          className="flex h-7 w-7 items-center justify-center rounded-lg"
          style={{
            background: 'var(--cp-surface-2)',
            color: 'var(--cp-text)',
            border: '1px solid var(--cp-border)',
          }}
        >
          <Plus size={14} />
        </button>
        <button
          type="button"
          onClick={() => onSelect({ kind: 'ai_prompt' })}
          title="AI Generate Prompt"
          className="flex h-7 w-7 items-center justify-center rounded-lg"
          style={{
            background: 'var(--cp-surface-2)',
            color: 'var(--cp-text)',
            border: '1px solid var(--cp-border)',
          }}
        >
          <Sparkles size={14} />
        </button>
      </div>

      <div className="px-3 py-2.5">
        <label
          className="flex items-center gap-2 rounded-lg px-2.5 py-1.5"
          style={{
            background: 'var(--cp-surface)',
            border: '1px solid var(--cp-border)',
            color: 'var(--cp-muted)',
          }}
        >
          <Search size={13} />
          <input
            type="search"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search definitions"
            className="w-full bg-transparent text-xs outline-none placeholder:text-[color:var(--cp-muted)]"
            style={{ color: 'var(--cp-text)' }}
          />
        </label>
      </div>

      <div className="desktop-scrollbar flex-1 overflow-y-auto px-2 pb-3">
        {store.error && (
          <div
            className="mb-2 rounded-lg px-2 py-1.5 text-[11px]"
            style={{
              background: 'color-mix(in srgb, var(--cp-danger) 10%, var(--cp-surface))',
              border: '1px solid color-mix(in srgb, var(--cp-danger) 35%, var(--cp-border))',
              color: 'var(--cp-danger)',
            }}
          >
            {store.error}
          </div>
        )}
        <SectionHeader
          icon={<Lock size={13} />}
          label="System"
          expanded={expanded.system}
          onToggle={() => setExpanded((e) => ({ ...e, system: !e.system }))}
          count={systemDefs.length}
        />
        {expanded.system && (
          <div className="mt-1 space-y-0.5">
            {systemDefs.map(renderDefinition)}
            {systemDefs.length === 0 && <Empty text={store.loading ? 'Loading…' : 'No system definitions.'} />}
          </div>
        )}

        <div className="mt-3">
          <SectionHeader
            icon={<FileCode2 size={13} />}
            label="Definitions"
            expanded={expanded.mine}
            onToggle={() => setExpanded((e) => ({ ...e, mine: !e.mine }))}
            count={ownDefs.length}
            right={
              <button
                type="button"
                onClick={(e) => {
                  e.stopPropagation()
                  setShowArchived((v) => !v)
                }}
                className="text-[10px] uppercase tracking-wider"
                style={{ color: showArchived ? 'var(--cp-accent)' : 'var(--cp-muted)' }}
                title="Toggle archived"
              >
                archived
              </button>
            }
          />
          {expanded.mine && (
            <div className="mt-1 space-y-0.5">
              {ownDefs.map(renderDefinition)}
              {ownDefs.length === 0 && <Empty text="No definitions match." />}
            </div>
          )}
        </div>

        <div className="mt-3">
          <SectionHeader
            icon={<ListChecks size={13} />}
            label="Runs"
            expanded={expanded.runs}
            onToggle={() => setExpanded((e) => ({ ...e, runs: !e.runs }))}
            count={store.listRuns().length}
          />
          {expanded.runs && (
            <div className="mt-1 space-y-0.5">
              {runs.map(renderRun)}
              {runs.length === 0 && <Empty text="No runs visible to you." />}
            </div>
          )}
        </div>
      </div>
    </aside>
  )
}

function Empty({ text }: { text: string }) {
  return (
    <div className="px-2 py-3 text-center text-xs" style={{ color: 'var(--cp-muted)' }}>
      {text}
    </div>
  )
}

function SectionHeader({
  icon,
  label,
  expanded,
  onToggle,
  count,
  right,
}: {
  icon: React.ReactNode
  label: string
  expanded: boolean
  onToggle: () => void
  count?: number
  right?: React.ReactNode
}) {
  return (
    <div className="flex w-full items-center" style={{ color: 'var(--cp-muted)' }}>
      <button
        type="button"
        onClick={onToggle}
        className="flex min-w-0 flex-1 items-center gap-1.5 px-1.5 py-1.5 text-left"
      >
        {expanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
        {icon}
        <span className="text-[11px] font-semibold uppercase tracking-wider">{label}</span>
        {count != null && (
          <span className="text-[10px]" style={{ color: 'var(--cp-muted)' }}>
            ({count})
          </span>
        )}
      </button>
      {right && <span className="ml-auto flex items-center pr-1.5">{right}</span>}
    </div>
  )
}
