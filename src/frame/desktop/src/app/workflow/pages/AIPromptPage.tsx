/* ── AI prompt page – copy-and-paste a structured prompt to ChatGPT/etc. ── */

import { useMemo, useState } from 'react'
import { Check, ClipboardCopy, Sparkles } from 'lucide-react'
import { useWorkflowStore } from '../hooks/use-workflow-store'

export function AIPromptPage({ onImport }: { onImport: () => void }) {
  const store = useWorkflowStore()
  const prompt = useMemo(() => store.buildAiPrompt(), [store])
  const [copied, setCopied] = useState(false)

  function copy() {
    navigator.clipboard.writeText(prompt).then(() => {
      setCopied(true)
      setTimeout(() => setCopied(false), 1500)
    })
  }

  return (
    <div className="space-y-4">
      <div
        className="rounded-2xl px-5 py-4"
        style={{
          background:
            'linear-gradient(135deg, color-mix(in srgb, var(--cp-accent) 16%, var(--cp-surface)), var(--cp-surface))',
          border: '1px solid var(--cp-border)',
        }}
      >
        <div className="flex items-center gap-3">
          <div
            className="flex h-10 w-10 items-center justify-center rounded-xl"
            style={{
              background: 'color-mix(in srgb, var(--cp-accent) 20%, transparent)',
              color: 'var(--cp-accent)',
            }}
          >
            <Sparkles size={20} />
          </div>
          <div className="flex-1">
            <div className="text-base font-semibold" style={{ color: 'var(--cp-text)' }}>
              AI generate prompt
            </div>
            <p className="mt-1 text-xs" style={{ color: 'var(--cp-muted)' }}>
              Copy the prompt below into ChatGPT or another AI tool, describe the workflow you
              want, then paste the JSON it returns into the import dialog.
            </p>
          </div>
        </div>
        <div className="mt-3 flex gap-2">
          <button
            type="button"
            onClick={copy}
            className="inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs"
            style={{
              background: 'var(--cp-accent)',
              color: 'white',
              border: '1px solid var(--cp-border)',
            }}
          >
            {copied ? <Check size={13} /> : <ClipboardCopy size={13} />}
            {copied ? 'Copied' : 'Copy prompt'}
          </button>
          <button
            type="button"
            onClick={onImport}
            className="inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs"
            style={{
              background: 'var(--cp-surface-2)',
              color: 'var(--cp-text)',
              border: '1px solid var(--cp-border)',
            }}
          >
            Open import dialog
          </button>
        </div>
      </div>

      <div
        className="rounded-2xl"
        style={{
          background: 'var(--cp-surface)',
          border: '1px solid var(--cp-border)',
        }}
      >
        <div
          className="flex items-center gap-2 px-3 py-2"
          style={{ borderBottom: '1px solid var(--cp-border)' }}
        >
          <span
            className="text-[10px] uppercase tracking-wider"
            style={{ color: 'var(--cp-muted)' }}
          >
            schema_version: {store.schemaVersion} · executors: {store.executors.length}
          </span>
        </div>
        <pre
          className="desktop-scrollbar max-h-[60vh] overflow-auto whitespace-pre-wrap p-4 font-mono text-[11px] leading-relaxed"
          style={{ color: 'var(--cp-text)' }}
        >
          {prompt}
        </pre>
      </div>
    </div>
  )
}
