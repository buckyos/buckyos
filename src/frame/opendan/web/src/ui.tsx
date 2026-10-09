import { useState, type ReactNode } from 'react'
import clsx from 'clsx'
import { fmtAgo, fmtTime, sessionHref, toText } from './lib'
import { OpenDanError } from './model'

const TONES: Record<string, string> = {
  running: 'text-accent border-accent',
  waiting: 'text-warn border-warn',
  pending: 'text-warn border-warn',
  ready: 'text-accent border-accent',
  finished: 'text-mute border-line',
  created: 'text-mute border-line',
  succeeded: 'text-ok border-ok',
  accepted: 'text-ok border-ok',
  sent: 'text-ok border-ok',
  ok: 'text-ok border-ok',
  failed: 'text-bad border-bad',
  discarded: 'text-bad border-bad',
  stopped: 'text-bad border-bad',
  bad: 'text-bad border-bad',
}

export function Badge({ value, tone }: { value: ReactNode; tone?: string }) {
  const key = tone ?? (typeof value === 'string' ? value : '')
  return (
    <span className={clsx('inline-block rounded border px-1 text-[11px] leading-4 whitespace-nowrap', TONES[key] ?? 'text-mute border-line')}>
      {value}
    </span>
  )
}

export function Panel({ title, extra, children, testId }: { title: ReactNode; extra?: ReactNode; children: ReactNode; testId?: string }) {
  return (
    <section className="mb-3 rounded border border-line bg-panel" data-testid={testId}>
      <header className="flex items-center justify-between gap-2 border-b border-line px-3 py-1.5">
        <h2 className="text-xs font-semibold uppercase tracking-wide text-mute">{title}</h2>
        {extra}
      </header>
      <div className="overflow-x-auto p-2">{children}</div>
    </section>
  )
}

export function Mono({ children, className }: { children: ReactNode; className?: string }) {
  return <span className={clsx('font-mono text-xs break-all', className)}>{children}</span>
}

export function Sid({ sid }: { sid: string }) {
  return (
    <a href={sessionHref(sid)} className="font-mono text-xs text-accent hover:underline">
      {sid}
    </a>
  )
}

export function Time({ ms }: { ms?: number | null }) {
  return (
    <span className="whitespace-nowrap text-xs text-mute" title={ms ? new Date(ms).toISOString() : undefined}>
      {fmtTime(ms)}
      {ms ? ` · ${fmtAgo(ms)}` : ''}
    </span>
  )
}

export function Empty({ children = 'None' }: { children?: ReactNode }) {
  return <div className="px-1 py-1 text-xs text-mute">{children}</div>
}

export function Pre({ value, className }: { value: unknown; className?: string }) {
  return (
    <pre className={clsx('max-h-80 overflow-auto whitespace-pre-wrap break-words rounded bg-bg p-2 font-mono text-xs', className)}>
      {toText(value)}
    </pre>
  )
}

export function ErrorBox({ error }: { error: unknown }) {
  if (!error) return null
  const kind = error instanceof OpenDanError ? error.kind : 'error'
  const message = error instanceof Error ? error.message : String(error)
  return (
    <div className="mb-3 rounded border border-bad px-3 py-2 text-xs text-bad" role="alert">
      <span className="font-mono">{kind}</span>: {message}
    </div>
  )
}

export function Fields({ rows }: { rows: [string, ReactNode][] }) {
  return (
    <dl className="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-1">
      {rows.map(([label, value]) => (
        <div key={label} className="contents">
          <dt className="text-xs text-mute">{label}</dt>
          <dd className="m-0 min-w-0">{value ?? <span className="text-mute">–</span>}</dd>
        </div>
      ))}
    </dl>
  )
}

export interface ConfirmRequest {
  title: string
  detail?: ReactNode
  inputLabel?: string
  confirmLabel: string
  danger?: boolean
  run: (input: string) => Promise<unknown>
}

export function ConfirmDialog({ request, onClose }: { request: ConfirmRequest; onClose: () => void }) {
  const [input, setInput] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<unknown>(null)

  const confirm = async () => {
    setBusy(true)
    setError(null)
    try {
      await request.run(input.trim())
      onClose()
    } catch (e) {
      setError(e)
      setBusy(false)
    }
  }

  return (
    <div className="fixed inset-0 z-10 flex items-center justify-center bg-black/40 p-4">
      <div role="dialog" aria-modal="true" aria-label={request.title} className="w-full max-w-md rounded border border-line bg-panel p-4">
        <h3 className="mb-2 text-sm font-semibold">{request.title}</h3>
        {request.detail && <div className="mb-3 text-xs text-mute">{request.detail}</div>}
        {request.inputLabel && (
          <label className="mb-3 block text-xs text-mute">
            {request.inputLabel}
            <input className="input mt-1 w-full" value={input} onChange={(e) => setInput(e.target.value)} />
          </label>
        )}
        <ErrorBox error={error} />
        <div className="flex justify-end gap-2">
          <button className="btn" onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button className={clsx('btn', request.danger ? 'border-bad text-bad' : 'border-accent text-accent')} onClick={confirm} disabled={busy}>
            {request.confirmLabel}
          </button>
        </div>
      </div>
    </div>
  )
}
