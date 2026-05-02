/* ── Workflow Import dialog (file / URL / text) ─ purely simulated dry_run ── */

import { useState } from 'react'
import { CheckCircle2, FileUp, Link2, Sparkles, X, XCircle } from 'lucide-react'
import { useWorkflowStore } from '../hooks/use-workflow-store'
import type { AnalysisIssue } from '../mock/types'

type Mode = 'text' | 'file' | 'url'

interface DryRunResult {
  ok: boolean
  issues: AnalysisIssue[]
  imported?: { id: string; name: string }
}

interface ImportDialogProps {
  onClose: () => void
  onShowAiPrompt: () => void
  onImported: (definitionId: string) => void
}

function fakeDryRun(text: string): DryRunResult {
  const issues: AnalysisIssue[] = []
  if (!text.trim()) {
    return {
      ok: false,
      issues: [
        {
          severity: 'error',
          code: 'empty_input',
          message: 'No definition supplied.',
        },
      ],
    }
  }
  let parsed: unknown
  try {
    parsed = JSON.parse(text)
  } catch {
    return {
      ok: false,
      issues: [
        {
          severity: 'error',
          code: 'invalid_json',
          message: 'Definition is not valid JSON.',
        },
      ],
    }
  }
  if (!parsed || typeof parsed !== 'object') {
    return {
      ok: false,
      issues: [
        {
          severity: 'error',
          code: 'invalid_root',
          message: 'Definition root must be a JSON object.',
        },
      ],
    }
  }
  const obj = parsed as Record<string, unknown>
  if (!obj.name || typeof obj.name !== 'string') {
    issues.push({
      severity: 'error',
      code: 'missing_name',
      message: 'Top-level "name" is required.',
    })
  }
  if (!obj.schema_version) {
    issues.push({
      severity: 'warn',
      code: 'missing_schema_version',
      message: 'schema_version not specified; defaulting to current.',
    })
  }
  if (!obj.nodes || !Array.isArray(obj.nodes) || obj.nodes.length === 0) {
    issues.push({
      severity: 'error',
      code: 'no_nodes',
      message: '"nodes" must be a non-empty array.',
    })
  }

  const ok = !issues.some((i) => i.severity === 'error')
  return {
    ok,
    issues,
    imported: ok ? { id: `wf-imported-${Date.now()}`, name: String(obj.name) } : undefined,
  }
}

export function ImportDialog({ onClose, onShowAiPrompt, onImported }: ImportDialogProps) {
  const store = useWorkflowStore()
  const [mode, setMode] = useState<Mode>('text')
  const [text, setText] = useState('')
  const [url, setUrl] = useState('')
  const [result, setResult] = useState<DryRunResult | null>(null)

  function loadFile(file: File) {
    file.text().then(setText).catch(() => undefined)
  }

  function runDryRun() {
    let payload = text
    if (mode === 'url') {
      payload = `{"name":"fetched_from_url","schema_version":"${store.schemaVersion}","nodes":[{"id":"a","type":"autonomous"}]}`
    }
    setResult(fakeDryRun(payload))
  }

  function submit() {
    if (!result?.ok || !result.imported) return
    // simulate addition by inserting a draft definition with the same shape as user-imported example
    const def = store.getDefinition('wf-imported-001')
    if (def) {
      const cloned = {
        ...def,
        id: result.imported.id,
        name: result.imported.name,
        version: 1,
        status: 'draft' as const,
        source: 'user_imported' as const,
        createdAt: new Date().toISOString(),
        updatedAt: new Date().toISOString(),
        graph: {
          ...def.graph,
          definitionId: result.imported.id,
        },
      }
      store.addDefinition(cloned)
      onImported(cloned.id)
    }
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center"
      style={{ background: 'rgba(0,0,0,0.4)' }}
      onClick={onClose}
    >
      <div
        className="w-[640px] max-w-[94vw] rounded-2xl"
        style={{
          background: 'var(--cp-surface)',
          border: '1px solid var(--cp-border)',
          boxShadow: 'var(--cp-window-shadow)',
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <div
          className="flex items-center gap-2 px-4 py-3"
          style={{ borderBottom: '1px solid var(--cp-border)' }}
        >
          <div className="flex-1 text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
            Import Workflow Definition
          </div>
          <button
            type="button"
            onClick={onShowAiPrompt}
            className="inline-flex items-center gap-1 rounded-lg px-2 py-1 text-xs"
            style={{
              background: 'var(--cp-surface-2)',
              color: 'var(--cp-accent)',
              border: '1px solid var(--cp-border)',
            }}
          >
            <Sparkles size={12} /> AI prompt
          </button>
          <button
            type="button"
            onClick={onClose}
            className="flex h-6 w-6 items-center justify-center"
            style={{ color: 'var(--cp-muted)' }}
          >
            <X size={14} />
          </button>
        </div>

        <div className="p-4">
          <div className="mb-3 flex gap-1">
            {(
              [
                { k: 'text', icon: <FileUp size={12} />, label: 'Paste text' },
                { k: 'file', icon: <FileUp size={12} />, label: 'File' },
                { k: 'url', icon: <Link2 size={12} />, label: 'URL' },
              ] as { k: Mode; icon: React.ReactNode; label: string }[]
            ).map((opt) => (
              <button
                key={opt.k}
                type="button"
                onClick={() => {
                  setMode(opt.k)
                  setResult(null)
                }}
                className="inline-flex items-center gap-1 rounded-lg px-2.5 py-1.5 text-xs"
                style={{
                  background:
                    mode === opt.k
                      ? 'color-mix(in srgb, var(--cp-accent) 14%, var(--cp-surface-2))'
                      : 'var(--cp-surface-2)',
                  color: mode === opt.k ? 'var(--cp-accent)' : 'var(--cp-text)',
                  border: '1px solid var(--cp-border)',
                }}
              >
                {opt.icon} {opt.label}
              </button>
            ))}
          </div>

          {mode === 'text' && (
            <textarea
              value={text}
              onChange={(e) => setText(e.target.value)}
              placeholder='{"name": "my_pipeline", "schema_version": "0.4", "nodes": [...]}'
              className="block h-44 w-full resize-none rounded-lg p-3 font-mono text-[11px]"
              style={{
                background: 'var(--cp-surface-2)',
                color: 'var(--cp-text)',
                border: '1px solid var(--cp-border)',
              }}
            />
          )}

          {mode === 'file' && (
            <label
              className="flex cursor-pointer flex-col items-center justify-center gap-2 rounded-lg py-10 text-xs"
              style={{
                background: 'var(--cp-surface-2)',
                border: '1px dashed var(--cp-border)',
                color: 'var(--cp-muted)',
              }}
            >
              <FileUp size={20} />
              {text
                ? `Loaded ${text.length} chars. Click again to choose another.`
                : 'Click to choose a JSON file.'}
              <input
                type="file"
                accept=".json,.dsl,.txt"
                className="hidden"
                onChange={(e) => {
                  const f = e.target.files?.[0]
                  if (f) loadFile(f)
                }}
              />
            </label>
          )}

          {mode === 'url' && (
            <input
              type="url"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              placeholder="https://example.com/my-pipeline.json"
              className="block w-full rounded-lg px-3 py-2 text-xs"
              style={{
                background: 'var(--cp-surface-2)',
                color: 'var(--cp-text)',
                border: '1px solid var(--cp-border)',
              }}
            />
          )}

          {result && (
            <div
              className="mt-3 rounded-lg p-3 text-xs"
              style={{
                background: 'var(--cp-surface-2)',
                border:
                  '1px solid ' +
                  (result.ok ? 'var(--cp-success)' : 'var(--cp-danger)'),
                color: 'var(--cp-text)',
              }}
            >
              <div
                className="mb-2 inline-flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider"
                style={{
                  color: result.ok ? 'var(--cp-success)' : 'var(--cp-danger)',
                }}
              >
                {result.ok ? <CheckCircle2 size={13} /> : <XCircle size={13} />}
                {result.ok ? 'dry_run passed' : 'dry_run failed'}
              </div>
              {result.issues.length === 0 ? (
                <div style={{ color: 'var(--cp-muted)' }}>No issues reported.</div>
              ) : (
                <ul className="space-y-1">
                  {result.issues.map((iss, i) => (
                    <li key={i} className="flex items-start gap-2">
                      <span
                        className="rounded px-1 text-[9px] font-semibold uppercase"
                        style={{
                          color:
                            iss.severity === 'error'
                              ? 'var(--cp-danger)'
                              : iss.severity === 'warn'
                                ? 'var(--cp-warning)'
                                : 'var(--cp-muted)',
                        }}
                      >
                        {iss.severity}
                      </span>
                      <span className="font-mono text-[10px]" style={{ color: 'var(--cp-muted)' }}>
                        {iss.code}
                      </span>
                      <span>{iss.message}</span>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          )}

          <div className="mt-3 flex justify-end gap-2">
            <button
              type="button"
              onClick={onClose}
              className="rounded-lg px-3 py-1.5 text-xs"
              style={{
                background: 'var(--cp-surface-2)',
                color: 'var(--cp-text)',
                border: '1px solid var(--cp-border)',
              }}
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={runDryRun}
              className="rounded-lg px-3 py-1.5 text-xs"
              style={{
                background: 'var(--cp-surface-2)',
                color: 'var(--cp-text)',
                border: '1px solid var(--cp-border)',
              }}
            >
              Run dry_run
            </button>
            <button
              type="button"
              onClick={submit}
              disabled={!result?.ok}
              className="rounded-lg px-3 py-1.5 text-xs"
              style={{
                background: result?.ok ? 'var(--cp-accent)' : 'var(--cp-surface-2)',
                color: result?.ok ? 'white' : 'var(--cp-muted)',
                border: '1px solid var(--cp-border)',
                opacity: result?.ok ? 1 : 0.6,
              }}
            >
              Submit as draft
            </button>
          </div>
        </div>
      </div>
    </div>
  )
}
