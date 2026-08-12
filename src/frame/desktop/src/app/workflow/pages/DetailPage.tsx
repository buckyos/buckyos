/* ── Definition / MountPoint detail page ── */

import { useMemo, useState } from 'react'
import {
  ExternalLink,
  GitMerge,
  History,
  Link2,
  RefreshCw,
  Unlink,
} from 'lucide-react'
import { useWorkflowStore } from '../hooks/use-workflow-store'
import { GraphCanvas } from '../components/GraphCanvas'
import { NodeConfigPanel } from '../components/NodeConfigPanel'
import { AnalysisBar } from '../components/AnalysisBar'
import { RunsList } from '../components/RunsList'
import { BindDialog } from '../components/BindDialog'
import type {
  AnalysisIssue,
  WorkflowDefinition,
  WorkflowSelection,
} from '../mock/types'

interface DetailPageProps {
  selection:
    | { kind: 'definition'; definitionId: string }
    | { kind: 'mount'; appId: string; mountPointId: string }
  onSelect: (s: WorkflowSelection) => void
}

function issuesByNode(def: WorkflowDefinition): Record<string, AnalysisIssue[]> {
  const out: Record<string, AnalysisIssue[]> = {}
  for (const i of def.analysis.issues) {
    if (!i.nodeId) continue
    if (!out[i.nodeId]) out[i.nodeId] = []
    out[i.nodeId].push(i)
  }
  return out
}

export function DetailPage({ selection, onSelect }: DetailPageProps) {
  const store = useWorkflowStore()
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null)
  const [bindOpen, setBindOpen] = useState(false)
  const [showAmendments, setShowAmendments] = useState(false)
  const [tick, force] = useState(0)

  const ctx = useMemo(() => {
    if (selection.kind === 'definition') {
      const def = store.getDefinition(selection.definitionId)
      return def
        ? {
            kind: 'definition' as const,
            definition: def,
            mp: undefined,
            app: undefined,
          }
        : null
    }
    const found = store.findMountPoint(selection.appId, selection.mountPointId)
    if (!found) return null
    const def = found.mp.currentBinding
      ? store.getDefinition(found.mp.currentBinding.definitionId)
      : undefined
    return {
      kind: 'mount' as const,
      definition: def,
      mp: found.mp,
      app: found.app,
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [store, selection, tick])

  if (!ctx) {
    return (
      <div
        className="flex h-full items-center justify-center text-xs"
        style={{ color: 'var(--cp-muted)' }}
      >
        Selection no longer exists.
      </div>
    )
  }

  const def = ctx.definition
  const issuesMap = def ? issuesByNode(def) : {}
  const selectedNode = def?.graph.nodes.find((n) => n.id === selectedNodeId) ?? null
  const usedBy = def ? store.listMountPointsUsing(def.id) : []
  const runs =
    ctx.kind === 'mount' && ctx.mp
      ? store.listRunsForMountPoint(ctx.app!.id, ctx.mp.id)
      : []

  return (
    <div className="flex h-full min-h-0 flex-col">
      {/* Header */}
      <div
        className="flex items-center gap-3 px-5 py-3"
        style={{ borderBottom: '1px solid var(--cp-border)' }}
      >
        <div className="min-w-0 flex-1">
          <div
            className="text-[10px] uppercase tracking-wider"
            style={{ color: 'var(--cp-muted)' }}
          >
            {ctx.kind === 'mount'
              ? `App / ${ctx.app!.name} · mount point`
              : 'Definition'}
          </div>
          <div className="mt-0.5 flex items-center gap-2 text-base font-semibold" style={{ color: 'var(--cp-text)' }}>
            <span className="truncate">
              {ctx.kind === 'mount'
                ? `${ctx.mp!.name}`
                : def?.name ?? '—'}
            </span>
            {def && (
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
            )}
            {def && (
              <span className="text-[11px]" style={{ color: 'var(--cp-muted)' }}>
                v{def.version} · {def.source}
              </span>
            )}
          </div>
          {ctx.kind === 'mount' && (
            <div className="mt-0.5 text-xs" style={{ color: 'var(--cp-muted)' }}>
              {ctx.mp!.required ? '· required' : '· optional'}
              {ctx.mp!.allowEmpty ? ' · allowEmpty' : ' · must-bind'}
              {def ? ` · bound to ${def.name} v${ctx.mp!.currentBinding?.definitionVersion}` : ' · unbound'}
            </div>
          )}
        </div>

        <div className="flex items-center gap-1.5">
          <span
            className="rounded-md px-2 py-1 text-[10px] uppercase tracking-wider"
            style={{
              background: 'var(--cp-surface-2)',
              color: 'var(--cp-muted)',
              border: '1px solid var(--cp-border)',
            }}
          >
            Read-only
          </span>
          {ctx.kind === 'definition' && def && (
            <button
              type="button"
              onClick={() => setBindOpen(true)}
              disabled={def.analysis.errorCount > 0 || def.status === 'archived'}
              className="inline-flex items-center gap-1 rounded-md px-2.5 py-1 text-[11px]"
              style={{
                background: 'var(--cp-accent)',
                color: 'white',
                opacity: def.analysis.errorCount > 0 ? 0.5 : 1,
                border: '1px solid var(--cp-border)',
              }}
            >
              <Link2 size={12} /> Bind to mount
            </button>
          )}
          {ctx.kind === 'mount' && def && (
            <button
              type="button"
              onClick={() => setBindOpen(true)}
              className="inline-flex items-center gap-1 rounded-md px-2.5 py-1 text-[11px]"
              style={{
                background: 'var(--cp-surface-2)',
                color: 'var(--cp-text)',
                border: '1px solid var(--cp-border)',
              }}
            >
              <GitMerge size={12} /> Replace binding
            </button>
          )}
          {ctx.kind === 'mount' && ctx.mp!.defaultDefinitionId && (
            <button
              type="button"
              onClick={() => {
                store.restoreDefaultBinding(ctx.app!.id, ctx.mp!.id)
                force((x) => x + 1)
              }}
              className="inline-flex items-center gap-1 rounded-md px-2.5 py-1 text-[11px]"
              style={{
                background: 'var(--cp-surface-2)',
                color: 'var(--cp-text)',
                border: '1px solid var(--cp-border)',
              }}
            >
              <RefreshCw size={12} /> Restore default
            </button>
          )}
          {ctx.kind === 'mount' && ctx.mp!.allowEmpty && def && (
            <button
              type="button"
              onClick={() => {
                store.unbindMountPoint(ctx.app!.id, ctx.mp!.id)
                force((x) => x + 1)
              }}
              className="inline-flex items-center gap-1 rounded-md px-2.5 py-1 text-[11px]"
              style={{
                background: 'var(--cp-surface-2)',
                color: 'var(--cp-text)',
                border: '1px solid var(--cp-border)',
              }}
            >
              <Unlink size={12} /> Unbind
            </button>
          )}
        </div>
      </div>

      {!def && (
        <div className="flex flex-1 items-center justify-center px-5 py-6 text-sm" style={{ color: 'var(--cp-muted)' }}>
          {ctx.kind === 'mount'
            ? ctx.mp!.required
              ? '⚠ Required mount point is not bound. Pick a Definition from the sidebar to bind.'
              : 'This mount point is empty. Bind a Definition or leave it empty.'
            : 'Definition not found.'}
        </div>
      )}

      {def && (
        <div className="flex flex-1 min-h-0 flex-col">
          <div className="px-5 py-3">
            <AnalysisBar
              report={def.analysis}
              onLocateNode={(id) => setSelectedNodeId(id)}
            />
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

          {/* Footer area */}
          <div
            className="px-5 py-3"
            style={{ borderTop: '1px solid var(--cp-border)' }}
          >
            {ctx.kind === 'mount' ? (
              <>
                <div
                  className="mb-2 flex items-center gap-2 text-[10px] uppercase tracking-wider"
                  style={{ color: 'var(--cp-muted)' }}
                >
                  Recent runs
                  <span
                    className="ml-1 rounded px-1 text-[9px]"
                    style={{
                      background: 'var(--cp-surface-2)',
                      color: 'var(--cp-muted)',
                      border: '1px solid var(--cp-border)',
                    }}
                  >
                    {runs.length}
                  </span>
                  <span
                    className="ml-2 inline-flex items-center gap-1 normal-case"
                    style={{ color: 'var(--cp-muted)' }}
                  >
                    All actions (approve / retry / abort) live in TaskMgr UI.
                    <ExternalLink size={10} />
                  </span>
                  <button
                    type="button"
                    onClick={() => setShowAmendments((v) => !v)}
                    className="ml-auto inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px]"
                    style={{
                      background: 'var(--cp-surface-2)',
                      color: 'var(--cp-text)',
                      border: '1px solid var(--cp-border)',
                    }}
                  >
                    <History size={10} />
                    {showAmendments ? 'Hide' : 'Show'} amendment history
                  </button>
                </div>
                <RunsList runs={runs} />
                {showAmendments && (
                  <AmendmentList runs={runs} />
                )}
              </>
            ) : (
              <div>
                <div
                  className="mb-2 text-[10px] uppercase tracking-wider"
                  style={{ color: 'var(--cp-muted)' }}
                >
                  Used by mount points
                </div>
                {usedBy.length === 0 ? (
                  <div
                    className="rounded-lg p-3 text-xs"
                    style={{
                      background: 'var(--cp-surface)',
                      border: '1px dashed var(--cp-border)',
                      color: 'var(--cp-muted)',
                    }}
                  >
                    Not bound to any mount point yet.
                  </div>
                ) : (
                  <div className="flex flex-wrap gap-2">
                    {usedBy.map(({ app, mp }) => (
                      <button
                        key={`${app.id}/${mp.id}`}
                        type="button"
                        onClick={() =>
                          onSelect({
                            kind: 'mount',
                            appId: app.id,
                            mountPointId: mp.id,
                          })
                        }
                        className="rounded-lg px-2.5 py-1.5 text-[11px]"
                        style={{
                          background: 'var(--cp-surface)',
                          color: 'var(--cp-text)',
                          border: '1px solid var(--cp-border)',
                        }}
                      >
                        {app.name} / {mp.name}
                      </button>
                    ))}
                  </div>
                )}
              </div>
            )}
          </div>
        </div>
      )}

      {def && bindOpen && (
        <BindDialog
          definition={def}
          onClose={() => setBindOpen(false)}
          onDone={(appId, mountPointId) => {
            setBindOpen(false)
            force((x) => x + 1)
            onSelect({ kind: 'mount', appId, mountPointId })
          }}
        />
      )}
    </div>
  )
}

function AmendmentList({ runs }: { runs: { runId: string; planVersion: number }[] }) {
  const store = useWorkflowStore()
  const all = runs.flatMap((r) => store.listAmendments(r.runId))
  if (all.length === 0) {
    return (
      <div
        className="mt-2 rounded-lg p-3 text-xs"
        style={{
          background: 'var(--cp-surface)',
          border: '1px dashed var(--cp-border)',
          color: 'var(--cp-muted)',
        }}
      >
        No amendments recorded.
      </div>
    )
  }
  return (
    <div
      className="mt-2 overflow-hidden rounded-lg"
      style={{
        background: 'var(--cp-surface)',
        border: '1px solid var(--cp-border)',
      }}
    >
      {all.map((am, i) => (
        <div
          key={i}
          className="px-3 py-2 text-xs"
          style={{
            borderTop: i === 0 ? undefined : '1px solid var(--cp-border)',
            color: 'var(--cp-text)',
          }}
        >
          <div
            className="flex items-center gap-2 text-[11px] font-semibold"
            style={{ color: 'var(--cp-text)' }}
          >
            <span style={{ color: 'var(--cp-warning)' }}>pv{am.planVersion}</span>
            <span style={{ color: 'var(--cp-muted)' }}>· {am.runId}</span>
            <span
              className="rounded px-1 text-[9px] uppercase"
              style={{
                color:
                  am.approvalStatus === 'approved'
                    ? 'var(--cp-success)'
                    : am.approvalStatus === 'rejected'
                      ? 'var(--cp-danger)'
                      : 'var(--cp-warning)',
                background: 'var(--cp-surface-2)',
                border: '1px solid var(--cp-border)',
              }}
            >
              {am.approvalStatus}
            </span>
            <span className="ml-auto text-[10px]" style={{ color: 'var(--cp-muted)' }}>
              by {am.submittedBy} @ {am.submittedAtStep}
            </span>
          </div>
          {am.reason && (
            <div className="mt-1 text-[11px]" style={{ color: 'var(--cp-muted)' }}>
              {am.reason}
            </div>
          )}
          <ul className="mt-1 space-y-0.5">
            {am.operations.map((op, j) => (
              <li
                key={j}
                className="text-[11px]"
                style={{ color: 'var(--cp-muted)' }}
              >
                · {op.op}
                {op.target ? ` ${op.target}` : ''}
                {op.description ? ` — ${op.description}` : ''}
              </li>
            ))}
          </ul>
        </div>
      ))}
    </div>
  )
}
