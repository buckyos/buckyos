/* ── Workflow sidebar: org tree of Definitions + Apps + Script Apps ── */

import { useMemo, useState } from 'react'
import {
  AlertTriangle,
  AppWindow,
  ChevronDown,
  ChevronRight,
  FileCode2,
  PlugZap,
  Plus,
  RefreshCw,
  Search,
  Sparkles,
  Workflow as WorkflowIcon,
} from 'lucide-react'
import { useWorkflowStore } from '../hooks/use-workflow-store'
import type {
  AppWorkflowMountPoint,
  WorkflowApp,
  WorkflowDefinition,
  WorkflowSelection,
} from '../mock/types'

interface SidebarProps {
  selection: WorkflowSelection
  onSelect: (selection: WorkflowSelection) => void
  onImport: () => void
}

const sourceLabels: Record<WorkflowDefinition['source'], string> = {
  system: 'system',
  user_imported: 'imported',
  app_registered: 'app',
  agent_generated: 'agent',
}

function statusBadge(status: WorkflowDefinition['status']): {
  label: string
  bg: string
  fg: string
} {
  switch (status) {
    case 'draft':
      return {
        label: 'draft',
        bg: 'color-mix(in srgb, var(--cp-warning) 18%, transparent)',
        fg: 'var(--cp-warning)',
      }
    case 'active':
      return {
        label: 'active',
        bg: 'color-mix(in srgb, var(--cp-success) 18%, transparent)',
        fg: 'var(--cp-success)',
      }
    case 'archived':
      return {
        label: 'archived',
        bg: 'color-mix(in srgb, var(--cp-muted) 22%, transparent)',
        fg: 'var(--cp-muted)',
      }
  }
}

export function WorkflowSidebar({ selection, onSelect, onImport }: SidebarProps) {
  const store = useWorkflowStore()
  const [search, setSearch] = useState('')
  const [showArchived, setShowArchived] = useState(false)
  const [expanded, setExpanded] = useState<Record<string, boolean>>({
    defs: true,
    apps: true,
    scripts: true,
  })

  const definitions = useMemo(() => {
    const all = store.listDefinitions()
    return all.filter((d) => {
      if (!showArchived && d.status === 'archived') return false
      if (search) {
        const q = search.toLowerCase()
        if (
          !d.name.toLowerCase().includes(q) &&
          !d.id.toLowerCase().includes(q)
        )
          return false
      }
      return true
    })
  }, [store, search, showArchived])

  const apps = useMemo(() => store.listApps().filter((a) => a.kind === 'app'), [store])
  const scripts = useMemo(
    () => store.listApps().filter((a) => a.kind === 'script_app'),
    [store],
  )

  function isDefSelected(d: WorkflowDefinition) {
    return selection.kind === 'definition' && selection.definitionId === d.id
  }

  function isMountSelected(appId: string, mountPointId: string) {
    return (
      selection.kind === 'mount' &&
      selection.appId === appId &&
      selection.mountPointId === mountPointId
    )
  }

  return (
    <aside
      className="flex h-full w-72 shrink-0 flex-col overflow-hidden"
      style={{ borderRight: '1px solid var(--cp-border)' }}
    >
      {/* Header */}
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

      {/* Search */}
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
            placeholder="Search definitions / apps"
            className="w-full bg-transparent text-xs outline-none placeholder:text-[color:var(--cp-muted)]"
            style={{ color: 'var(--cp-text)' }}
          />
        </label>
      </div>

      {/* Tree body */}
      <div className="desktop-scrollbar flex-1 overflow-y-auto px-2 pb-3">
        {/* Definitions section */}
        <SectionHeader
          icon={<FileCode2 size={13} />}
          label="Definitions"
          expanded={expanded.defs}
          onToggle={() =>
            setExpanded((e) => ({ ...e, defs: !e.defs }))
          }
          count={definitions.length}
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
              <RefreshCw size={11} />
            </button>
          }
        />
        {expanded.defs && (
          <div className="mt-1 space-y-0.5">
            {definitions.map((d) => {
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
                  <FileCode2
                    size={13}
                    className="mt-0.5 shrink-0"
                    style={{ color: 'var(--cp-muted)' }}
                  />
                  <div className="min-w-0 flex-1">
                    <div
                      className="flex items-center gap-1.5 text-xs"
                      style={{ color: 'var(--cp-text)' }}
                    >
                      <span className="truncate">{d.name}</span>
                      <span
                        className="text-[10px]"
                        style={{ color: 'var(--cp-muted)' }}
                      >
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
                        <span
                          className="flex items-center gap-0.5 text-[10px]"
                          style={{ color: 'var(--cp-danger)' }}
                        >
                          <AlertTriangle size={10} />
                          {errors}
                        </span>
                      )}
                      {warns > 0 && (
                        <span
                          className="flex items-center gap-0.5 text-[10px]"
                          style={{ color: 'var(--cp-warning)' }}
                        >
                          <AlertTriangle size={10} />
                          {warns}
                        </span>
                      )}
                    </div>
                  </div>
                </button>
              )
            })}
            {definitions.length === 0 && (
              <div
                className="px-2 py-3 text-center text-xs"
                style={{ color: 'var(--cp-muted)' }}
              >
                No definitions match.
              </div>
            )}
          </div>
        )}

        {/* Apps section */}
        <div className="mt-3">
          <SectionHeader
            icon={<AppWindow size={13} />}
            label="Apps"
            expanded={expanded.apps}
            onToggle={() => setExpanded((e) => ({ ...e, apps: !e.apps }))}
            count={apps.length}
          />
          {expanded.apps && (
            <div className="mt-1">
              {apps.map((app) => (
                <AppGroup
                  key={app.id}
                  app={app}
                  store={store}
                  isMountSelected={isMountSelected}
                  onSelectMount={(mp) =>
                    onSelect({
                      kind: 'mount',
                      appId: app.id,
                      mountPointId: mp.id,
                    })
                  }
                />
              ))}
            </div>
          )}
        </div>

        {/* Script Apps section */}
        <div className="mt-3">
          <SectionHeader
            icon={<PlugZap size={13} />}
            label="Script Apps"
            expanded={expanded.scripts}
            onToggle={() => setExpanded((e) => ({ ...e, scripts: !e.scripts }))}
            count={scripts.length}
          />
          {expanded.scripts && (
            <div className="mt-1">
              {scripts.map((app) => (
                <AppGroup
                  key={app.id}
                  app={app}
                  store={store}
                  isMountSelected={isMountSelected}
                  onSelectMount={(mp) =>
                    onSelect({
                      kind: 'mount',
                      appId: app.id,
                      mountPointId: mp.id,
                    })
                  }
                />
              ))}
              {scripts.length === 0 && (
                <div
                  className="px-2 py-2 text-center text-xs"
                  style={{ color: 'var(--cp-muted)' }}
                >
                  None.
                </div>
              )}
            </div>
          )}
        </div>
      </div>
    </aside>
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
    <button
      type="button"
      onClick={onToggle}
      className="flex w-full items-center gap-1.5 px-1.5 py-1.5 text-left"
      style={{ color: 'var(--cp-muted)' }}
    >
      {expanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
      {icon}
      <span className="text-[11px] font-semibold uppercase tracking-wider">
        {label}
      </span>
      {count != null && (
        <span className="text-[10px]" style={{ color: 'var(--cp-muted)' }}>
          ({count})
        </span>
      )}
      {right && <span className="ml-auto">{right}</span>}
    </button>
  )
}

function AppGroup({
  app,
  store,
  isMountSelected,
  onSelectMount,
}: {
  app: WorkflowApp
  store: ReturnType<typeof useWorkflowStore>
  isMountSelected: (appId: string, mountPointId: string) => boolean
  onSelectMount: (mp: AppWorkflowMountPoint) => void
}) {
  const [open, setOpen] = useState(true)
  return (
    <div className="mb-1">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-1.5 rounded-md px-2 py-1 text-left"
        style={{ color: 'var(--cp-text)' }}
      >
        {open ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
        <span className="text-xs font-medium">{app.name}</span>
        <span
          className="ml-auto text-[10px]"
          style={{ color: 'var(--cp-muted)' }}
        >
          {app.mountPoints.length}
        </span>
      </button>
      {open && (
        <div className="ml-4 space-y-0.5">
          {app.mountPoints.map((mp) => {
            const selected = isMountSelected(app.id, mp.id)
            const def = mp.currentBinding
              ? store.getDefinition(mp.currentBinding.definitionId)
              : undefined
            const requiredMissing = mp.required && !mp.currentBinding
            return (
              <button
                key={mp.id}
                type="button"
                onClick={() => onSelectMount(mp)}
                className="flex w-full items-start gap-1.5 rounded-md px-2 py-1.5 text-left"
                style={{
                  background: selected
                    ? 'color-mix(in srgb, var(--cp-accent) 12%, var(--cp-surface))'
                    : 'transparent',
                  border: selected
                    ? '1px solid color-mix(in srgb, var(--cp-accent) 22%, var(--cp-border))'
                    : '1px solid transparent',
                }}
              >
                <div className="min-w-0 flex-1">
                  <div className="text-xs" style={{ color: 'var(--cp-text)' }}>
                    {mp.name}
                  </div>
                  <div
                    className="mt-0.5 truncate text-[11px]"
                    style={{
                      color: requiredMissing
                        ? 'var(--cp-danger)'
                        : 'var(--cp-muted)',
                    }}
                  >
                    {def
                      ? `↔ ${def.name} v${mp.currentBinding?.definitionVersion}`
                      : requiredMissing
                        ? '⚠ Required, not configured'
                        : mp.allowEmpty
                          ? 'Empty'
                          : 'Not bound'}
                  </div>
                </div>
              </button>
            )
          })}
        </div>
      )}
    </div>
  )
}
