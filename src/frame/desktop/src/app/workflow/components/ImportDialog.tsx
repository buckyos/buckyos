/* ── Workflow Import dialog (file / URL / text): dry_run then submit_definition ── */

import { useState } from 'react'
import { CheckCircle2, FileUp, Link2, Sparkles, X, XCircle } from 'lucide-react'
import type { WorkflowDefinition as DslDefinition } from 'buckyos'
import { usePickFiles } from '../../../sysdlg/select/pickFiles'
import { useWorkflowStore } from '../hooks/use-workflow-store'
import type { AnalysisIssue } from '../types'

type Mode = 'text' | 'file' | 'url'

interface DryRunResult {
  ok: boolean
  issues: AnalysisIssue[]
  message?: string
  revision?: string
  definition?: DslDefinition
}

interface ImportDialogProps {
  onClose: () => void
  onShowAiPrompt: () => void
  onImported: (definitionId: string) => void
}

function parseDefinition(text: string): { definition?: DslDefinition; issue?: AnalysisIssue } {
  if (!text.trim()) {
    return { issue: { severity: 'error', code: 'empty_input', message: 'No definition supplied.' } }
  }
  let parsed: unknown
  try {
    parsed = JSON.parse(text)
  } catch {
    return { issue: { severity: 'error', code: 'invalid_json', message: 'Definition is not valid JSON.' } }
  }
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
    return { issue: { severity: 'error', code: 'invalid_root', message: 'Definition root must be a JSON object.' } }
  }
  return { definition: parsed as DslDefinition }
}

export function ImportDialog({ onClose, onShowAiPrompt, onImported }: ImportDialogProps) {
  const store = useWorkflowStore()
  const [mode, setMode] = useState<Mode>('text')
  const [text, setText] = useState('')
  const [url, setUrl] = useState('')
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<DryRunResult | null>(null)
  const pickFiles = usePickFiles('workflow.import')
  const [fileName, setFileName] = useState('')
  const [fileError, setFileError] = useState('')

  async function chooseFile() {
    setFileError('')
    try {
      const file = (await pickFiles({ accept: '.json,.dsl,.txt' }))?.[0]
      if (!file) return
      setText(await file.text())
      setFileName(file.name)
      setResult(null)
    } catch (error) {
      setFileError(error instanceof Error ? error.message : String(error))
    }
  }

  async function sourceText(): Promise<string> {
    if (mode !== 'url') return text
    const resp = await fetch(url)
    if (!resp.ok) throw new Error(`fetch ${url}: HTTP ${resp.status}`)
    return await resp.text()
  }

  async function runDryRun() {
    setBusy(true)
    try {
      const payload = await sourceText()
      const parsed = parseDefinition(payload)
      if (!parsed.definition) {
        setResult({ ok: false, issues: parsed.issue ? [parsed.issue] : [] })
        return
      }
      const outcome = await store.dryRun(parsed.definition)
      setResult({
        ok: outcome.ok,
        issues: outcome.analysis.issues,
        message: outcome.message,
        revision: outcome.revision,
        definition: parsed.definition,
      })
    } catch (err) {
      setResult({
        ok: false,
        issues: [{ severity: 'error', code: 'load_failed', message: err instanceof Error ? err.message : String(err) }],
      })
    } finally {
      setBusy(false)
    }
  }

  async function submit() {
    if (!result?.ok || !result.definition) return
    setBusy(true)
    const outcome = await store.submitDefinition(result.definition)
    setBusy(false)
    if ('error' in outcome) {
      setResult({ ...result, ok: false, message: outcome.error })
      return
    }
    onImported(outcome.definitionId)
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
              placeholder='{"schema_version": "0.2.0", "id": "my_pipeline", "name": "my_pipeline", "trigger": {"type": "manual"}, "steps": [...], "edges": [...]}'
              className="block h-44 w-full resize-none rounded-lg p-3 font-mono text-[11px]"
              style={{
                background: 'var(--cp-surface-2)',
                color: 'var(--cp-text)',
                border: '1px solid var(--cp-border)',
              }}
            />
          )}

          {mode === 'file' && (
            <button
              type="button"
              onClick={() => void chooseFile()}
              data-testid="workflow-import-choose-file"
              className="flex w-full flex-col items-center justify-center gap-2 rounded-lg py-10 text-xs"
              style={{
                background: 'var(--cp-surface-2)',
                border: '1px dashed var(--cp-border)',
                color: fileError ? 'var(--cp-danger)' : 'var(--cp-muted)',
              }}
            >
              <FileUp size={20} />
              {fileError
                ? `Could not read the file: ${fileError}`
                : text && fileName
                  ? `Loaded ${fileName} (${text.length} chars). Click to choose another.`
                  : 'Click to choose a JSON file.'}
            </button>
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
                {result.revision && (
                  <span className="ml-2 font-mono normal-case" style={{ color: 'var(--cp-muted)' }}>
                    revision {result.revision.slice(0, 12)}
                  </span>
                )}
              </div>
              {result.message && (
                <div className="mb-1" style={{ color: 'var(--cp-danger)' }}>{result.message}</div>
              )}
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
              onClick={() => void runDryRun()}
              disabled={busy}
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
              onClick={() => void submit()}
              disabled={!result?.ok || busy}
              className="rounded-lg px-3 py-1.5 text-xs"
              style={{
                background: result?.ok ? 'var(--cp-accent)' : 'var(--cp-surface-2)',
                color: result?.ok ? 'white' : 'var(--cp-muted)',
                border: '1px solid var(--cp-border)',
                opacity: result?.ok ? 1 : 0.6,
              }}
            >
              Submit definition
            </button>
          </div>
        </div>
      </div>
    </div>
  )
}
