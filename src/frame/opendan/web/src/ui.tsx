import { useState, type ReactNode } from 'react'
import clsx from 'clsx'
import { ChevronLeft } from 'lucide-react'
import { fmtAgo, fmtTime, sessionHref, toText } from './lib'
import { OpenDanError } from './model'
import { toneOf, type Tone } from './tone'

const DOT: Record<Tone, string> = { accent: 'bg-accent', ok: 'bg-ok', warn: 'bg-warn', bad: 'bg-bad', mute: 'bg-bar' }

/** The 6px status dot; without a tone it keeps its place, unseen. */
export function Dot({ tone, className }: { tone?: Tone; className?: string }) {
  return <span aria-hidden className={clsx('inline-block h-1.5 w-1.5 shrink-0 rounded-full', tone ? DOT[tone] : 'bg-transparent', className)} />
}

/** A state with its dot; a value with no known state is a quiet label. */
export function Badge({ value, tone }: { value: ReactNode; tone?: string }) {
  const resolved = toneOf(tone ?? (typeof value === 'string' ? value : ''))
  if (!resolved) return <span className="whitespace-nowrap text-xs text-mute">{value}</span>
  return (
    <span className={clsx('inline-flex items-baseline gap-1.5 whitespace-nowrap text-xs', resolved === 'bad' ? 'text-bad' : resolved === 'mute' ? 'text-mute' : 'text-fg')}>
      <Dot tone={resolved} className="self-center" />
      {value}
    </span>
  )
}

/** A kind or a reason, not a state: quiet text without a dot. */
export function Tag({ children }: { children: ReactNode }) {
  return <span className="whitespace-nowrap text-xs text-mute">{children}</span>
}

export function Panel({ title, count, extra, children, testId }: { title: ReactNode; count?: ReactNode; extra?: ReactNode; children: ReactNode; testId?: string }) {
  return (
    <section className="mb-4 min-w-0 rounded-xl border border-line bg-panel" data-testid={testId}>
      <header className="flex min-h-12 flex-wrap items-center justify-between gap-x-4 gap-y-2 px-4 py-2">
        <h2 className="flex items-baseline gap-2 text-[13px] font-medium">
          {title}
          {count != null && <span className="font-mono text-xs font-normal tabular-nums text-mute">{count}</span>}
        </h2>
        {extra && <div className="flex flex-wrap items-center gap-x-4 gap-y-2 text-xs text-mute">{extra}</div>}
      </header>
      <div className="overflow-x-auto px-4 pb-4">{children}</div>
    </section>
  )
}

/** The head of a detail page: a way back, the subject, its states. */
export function PageHead({ back, title, children, meta, testId }: { back: { href: string; label: ReactNode }; title: ReactNode; children?: ReactNode; meta?: ReactNode; testId?: string }) {
  return (
    <div className="mb-4">
      <a href={back.href} className="-ml-1 inline-flex h-6 items-center gap-0.5 rounded-md pr-1.5 text-xs text-mute transition-colors hover:text-fg">
        <ChevronLeft size={14} />
        {back.label}
      </a>
      <div className="mt-1 flex flex-wrap items-center gap-x-4 gap-y-1" data-testid={testId}>
        <h1 className="min-w-0 break-words text-lg font-medium leading-7">{title}</h1>
        {children}
      </div>
      {meta}
    </div>
  )
}

export function Mono({ children, className }: { children: ReactNode; className?: string }) {
  return <span className={clsx('break-words font-mono text-xs', className)}>{children}</span>
}

export function Sid({ sid }: { sid: string }) {
  return (
    <a href={sessionHref(sid)} className="whitespace-nowrap font-mono text-xs text-accent hover:underline">
      {sid}
    </a>
  )
}

export function Time({ ms }: { ms?: number | null }) {
  return (
    <span className="whitespace-nowrap text-xs tabular-nums text-mute" title={ms ? new Date(ms).toISOString() : undefined}>
      {fmtTime(ms)}
      {ms ? ` · ${fmtAgo(ms)}` : ''}
    </span>
  )
}

export function Empty({ children = 'None' }: { children?: ReactNode }) {
  return <div className="py-1 text-xs text-mute">{children}</div>
}

export function Pre({ value, className }: { value: unknown; className?: string }) {
  return (
    <pre className={clsx('max-h-80 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-bg px-3 py-2 font-mono text-xs leading-5', className)}>
      {toText(value)}
    </pre>
  )
}

/** A one-line note with its dot: a result, a warning, a pause. */
export function Note({ tone, children, testId, className }: { tone: Tone; children: ReactNode; testId?: string; className?: string }) {
  return (
    <div className={clsx('flex items-start gap-2 text-xs', className)} data-testid={testId}>
      <Dot tone={tone} className="mt-[7px]" />
      <div className="min-w-0 break-words">{children}</div>
    </div>
  )
}

export function ErrorBox({ error }: { error: unknown }) {
  if (!error) return null
  const kind = error instanceof OpenDanError ? error.kind : 'error'
  const message = error instanceof Error ? error.message : String(error)
  return (
    <div className="mb-3 flex items-start gap-2 rounded-lg border border-line bg-panel px-3 py-2 text-xs" role="alert">
      <Dot tone="bad" className="mt-[7px]" />
      <div className="min-w-0 break-words">
        <span className="font-mono text-bad">{kind}</span>: {message}
      </div>
    </div>
  )
}

export function Fields({ rows }: { rows: [string, ReactNode][] }) {
  return (
    <dl className="grid grid-cols-[max-content_minmax(0,1fr)] gap-x-6 gap-y-2">
      {rows.map(([label, value]) => (
        <div key={label} className="contents">
          <dt className="text-xs leading-5 text-mute">{label}</dt>
          <dd className="m-0 min-w-0 break-words leading-5">{value ?? <span className="text-mute">–</span>}</dd>
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
    <div className="scrim">
      <div role="dialog" aria-modal="true" aria-label={request.title} className="dialog">
        <h3 className="mb-2 text-[15px] font-medium leading-6">{request.title}</h3>
        {request.detail && <div className="mb-4 text-xs leading-5 text-mute">{request.detail}</div>}
        {request.inputLabel && (
          <label className="mb-4 block text-xs text-mute">
            {request.inputLabel}
            <input className="input mt-1 w-full" value={input} onChange={(e) => setInput(e.target.value)} />
          </label>
        )}
        <ErrorBox error={error} />
        <div className="flex justify-end gap-2">
          <button className="btn" onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button className={clsx('btn', request.danger ? 'btn-danger' : 'btn-primary')} onClick={confirm} disabled={busy}>
            {request.confirmLabel}
          </button>
        </div>
      </div>
    </div>
  )
}
