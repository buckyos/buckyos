/* ── Definition detail page ── */

import { useState } from 'react'
import { Archive, Lock } from 'lucide-react'
import { useWorkflowStore } from '../hooks/use-workflow-store'
import { GraphCanvas } from '../components/GraphCanvas'
import { NodeConfigPanel } from '../components/NodeConfigPanel'
import { AnalysisBar } from '../components/AnalysisBar'
import { RunsList } from '../components/RunsList'
import type { AnalysisIssue, WorkflowDefinition, WorkflowSelection } from '../types'

interface DetailPageProps {
  definitionId: string
  onSelect: (s: WorkflowSelection) => void
}

const EMPTY_ISSUES: Record<string, AnalysisIssue[]> = {}
// Cached per definition object so the graph canvas receives a stable map and
// only re-runs layout when the definition itself changes, not on every click.
const issuesCache = new WeakMap<WorkflowDefinition, Record<string, AnalysisIssue[]>>()

function issuesByNode(def: WorkflowDefinition): Record<string, AnalysisIssue[]> {
  const cached = issuesCache.get(def)
  if (cached) return cached
  const out: Record<string, AnalysisIssue[]> = {}
  for (const i of def.analysis.issues) {
    if (!i.nodeId) continue
    if (!out[i.nodeId]) out[i.nodeId] = []
    out[i.nodeId].push(i)
  }
  issuesCache.set(def, out)
  return out
}

export function DetailPage({ definitionId, onSelect }: DetailPageProps) {
  const store = useWorkflowStore()
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)
  const def = store.getDefinition(definitionId)

  if (!def) {
    return (
      <div className="flex h-full items-center justify-center text-xs" style={{ color: 'var(--cp-muted)' }}>
        {store.loading ? 'Loading…' : 'Definition not found.'}
      </div>
    )
  }

  const issuesMap = def ? issuesByNode(def) : EMPTY_ISSUES
  const selectedNode = def.graph.nodes.find((n) => n.id === selectedNodeId) ?? null
  const runs = store.listRunsForDefinition(def.id)

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex items-center gap-3 px-5 py-3" style={{ borderBottom: '1px solid var(--cp-border)' }}>
        <div className="min-w-0 flex-1">
          <div className="text-[10px] uppercase tracking-wider" style={{ color: 'var(--cp-muted)' }}>
            {def.readOnly ? 'System definition' : 'Definition'}
          </div>
          <div className="mt-0.5 flex items-center gap-2 text-base font-semibold" style={{ color: 'var(--cp-text)' }}>
            <span className="truncate">{def.name}</span>
            <span
              className="rounded-full px-2 py-0.5 text-[10px] uppercase"
              style={{
                background: 'var(--cp-surface-2)',
                border: '1px solid var(--cp-border)',
                color:
                  def.status === 'active'
                    ? 'var(--cp-success)'
                    : def.status === 'draft'
                      ? 'var(--cp-warning)'
                      : 'var(--cp-muted)',
              }}
            >
              {def.status}
            </span>
            <span className="text-[11px]" style={{ color: 'var(--cp-muted)' }}>
              v{def.version} · {def.source} · revision{' '}
              <span className="font-mono" title={def.revision}>{def.revision.slice(0, 12)}</span>
            </span>
          </div>
          {def.description && (
            <div className="mt-0.5 truncate text-xs" style={{ color: 'var(--cp-muted)' }}>{def.description}</div>
          )}
        </div>

        <div className="flex items-center gap-1.5">
          <span
            className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-[10px] uppercase tracking-wider"
            style={{ background: 'var(--cp-surface-2)', color: 'var(--cp-muted)', border: '1px solid var(--cp-border)' }}
            title={def.readOnly ? 'Shipped with the system; only a service release changes it.' : 'The WebUI never edits definitions in place.'}
          >
            {def.readOnly && <Lock size={11} />} Read-only
          </span>
          {!def.readOnly && def.status !== 'archived' && (
            <button
              type="button"
              onClick={() => {
                void store.archiveDefinition(def.id).then(setActionError)
              }}
              className="inline-flex items-center gap-1 rounded-md px-2.5 py-1 text-[11px]"
              style={{ background: 'var(--cp-surface-2)', color: 'var(--cp-text)', border: '1px solid var(--cp-border)' }}
            >
              <Archive size={12} /> Archive
            </button>
          )}
        </div>
      </div>

      {actionError && (
        <div className="px-5 pt-3 text-xs" style={{ color: 'var(--cp-danger)' }}>{actionError}</div>
      )}

      <div className="flex flex-1 min-h-0 flex-col">
        <div className="px-5 py-3">
          <AnalysisBar report={def.analysis} onLocateNode={(id) => setSelectedNodeId(id)} />
        </div>

        <div className="flex flex-1 min-h-0">
          <div className="flex-1 min-w-0">
            <GraphCanvas
              graph={def.graph}
              issuesByNode={issuesMap}
              selectedNodeId={selectedNodeId}
              onSelectNode={setSelectedNodeId}
            />
          </div>
          {selectedNode && (
            <NodeConfigPanel
              node={selectedNode}
              issues={issuesMap[selectedNode.id] ?? []}
              onClose={() => setSelectedNodeId(null)}
            />
          )}
        </div>

        <div className="px-5 py-3" style={{ borderTop: '1px solid var(--cp-border)' }}>
          <div className="mb-2 flex items-center gap-2 text-[10px] uppercase tracking-wider" style={{ color: 'var(--cp-muted)' }}>
            Runs
            <span
              className="ml-1 rounded px-1 text-[9px]"
              style={{ background: 'var(--cp-surface-2)', color: 'var(--cp-muted)', border: '1px solid var(--cp-border)' }}
            >
              {runs.length}
            </span>
            <span className="ml-2 normal-case" style={{ color: 'var(--cp-muted)' }}>
              Approve / retry / abort live in TaskMgr; this view observes.
            </span>
            <button
              type="button"
              onClick={() => void store.refreshRuns()}
              className="ml-auto rounded px-1.5 py-0.5 text-[10px]"
              style={{ background: 'var(--cp-surface-2)', color: 'var(--cp-text)', border: '1px solid var(--cp-border)' }}
            >
              Refresh
            </button>
          </div>
          <RunsList runs={runs} onOpen={(runId) => onSelect({ kind: 'run', runId })} />
        </div>
      </div>
    </div>
  )
}
