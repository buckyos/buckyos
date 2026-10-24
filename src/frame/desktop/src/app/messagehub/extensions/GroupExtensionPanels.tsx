import { Component, useEffect, useMemo, useState, type ErrorInfo, type ReactNode } from 'react'
import { useMediaQuery } from '@mui/material'
import { ChevronDown, ChevronRight, LayoutPanelLeft, ShieldAlert, X } from 'lucide-react'
import { useI18n } from '../../../i18n/provider'
import { hubButtonClass } from '../SessionDialogs'
import { panelOriginTrusted, panelSrc, type GroupPanelRef, type PanelContext } from './model'

/** A panel that has not fired `load` after this long is shown as unavailable. */
export const PANEL_LOAD_TIMEOUT_MS = 10_000
const SANDBOX = 'allow-scripts allow-same-origin allow-forms'

export interface GroupExtensionPanelsProps {
  /** The panels in effect; where they come from (group config, Agent settings) is the caller's business. */
  panels: readonly GroupPanelRef[]
  context: PanelContext
  /** The Zone that must serve every panel (D-2); the group Doc's `host`. */
  hostDid: string
  /** Extra origins accepted as trusted; the mock store passes its own origin for the sample page. */
  allowOrigins?: readonly string[]
  /** Open the panel list at once instead of collapsed. */
  defaultOpen?: boolean
  /** A failed load is reported as unavailable after this long (tests shorten it). */
  loadTimeoutMs?: number
}

/**
 * Host of the embedded application panels of a Session (EXT-02): a collapsed
 * list of tabs; the open tab is an iframe of the panel's `entry_url` with the
 * Session context in its query. On a phone the open panel is a full-screen
 * page over the details. Nothing of the viewer (token, DID, cookies of this
 * origin) reaches the frame; a crash or a failed load stays inside this
 * component.
 */
export function GroupExtensionPanels({ panels, context, hostDid, allowOrigins, defaultOpen = false, loadTimeoutMs = PANEL_LOAD_TIMEOUT_MS }: GroupExtensionPanelsProps) {
  const { t } = useI18n()
  const phone = useMediaQuery('(max-width: 768px)')
  const [open, setOpen] = useState(defaultOpen)
  const [activeId, setActiveId] = useState<string | null>(null)
  if (panels.length === 0) return null
  const active = panels.find(panel => panel.id === activeId) ?? (phone ? null : panels[0])
  const openPanel = (id: string) => { setActiveId(id); setOpen(true) }
  return <section className="space-y-2" data-testid="group-panels" aria-label={t('messagehub.panels.title')}>
    <button type="button" className="flex min-h-11 w-full items-center gap-2 text-left" aria-expanded={open} onClick={() => setOpen(value => !value)} data-testid="group-panels-toggle">
      {open ? <ChevronDown size={14} aria-hidden /> : <ChevronRight size={14} aria-hidden />}
      <LayoutPanelLeft size={14} aria-hidden className="text-[color:var(--cp-muted)]" />
      <span className="text-xs font-semibold uppercase tracking-wide text-[color:var(--cp-muted)]">{t('messagehub.panels.title')} · {panels.length}</span>
    </button>
    {open && <>
      <div role="tablist" aria-label={t('messagehub.panels.title')} className="flex flex-wrap gap-1.5" data-testid="group-panel-tabs">
        {panels.map(panel => {
          const selected = !phone && active?.id === panel.id
          return <button key={panel.id} type="button" role="tab" aria-selected={selected} data-panel-id={panel.id} onClick={() => openPanel(panel.id)}
            className="min-h-9 rounded-lg px-3 text-[13px] font-medium"
            style={selected ? { background: 'color-mix(in srgb, var(--cp-accent) 14%, transparent)', color: 'var(--cp-accent)' } : { background: 'color-mix(in srgb, var(--cp-text) 5%, transparent)', color: 'var(--cp-text)' }}>{panel.title}</button>
        })}
      </div>
      {active && !phone && <PanelFrameBoundary resetKey={`${active.id}\n${context.session}`} fallback={(_, reset) => <PanelUnavailable onRetry={reset} />}>
        <PanelFrame panel={active} context={context} hostDid={hostDid} allowOrigins={allowOrigins} loadTimeoutMs={loadTimeoutMs} className="h-[420px] w-full" />
      </PanelFrameBoundary>}
    </>}
    {active && phone && <div className="fixed inset-0 z-[70] flex flex-col bg-[color:var(--cp-surface)]" role="dialog" aria-label={active.title} data-testid="group-panel-page">
      <header className="flex shrink-0 items-center justify-between border-b border-[color:var(--cp-border)] py-1 pl-4 pr-1">
        <h2 className="truncate text-sm font-semibold">{active.title}</h2>
        <button type="button" className="flex min-h-11 min-w-11 items-center justify-center" aria-label={t('messagehub.close')} title={t('messagehub.close')} onClick={() => setActiveId(null)}><X size={18} /></button>
      </header>
      <PanelFrameBoundary resetKey={`${active.id}\n${context.session}`} fallback={(_, reset) => <PanelUnavailable onRetry={reset} />}>
        <PanelFrame panel={active} context={context} hostDid={hostDid} allowOrigins={allowOrigins} loadTimeoutMs={loadTimeoutMs} className="min-h-0 w-full flex-1" />
      </PanelFrameBoundary>
    </div>}
  </section>
}

type FrameState = 'loading' | 'ready' | 'failed'

function PanelFrame({ panel, context, hostDid, allowOrigins, loadTimeoutMs, className }: { panel: GroupPanelRef; context: PanelContext; hostDid: string; allowOrigins?: readonly string[]; loadTimeoutMs: number; className: string }) {
  const { t } = useI18n()
  const [attempt, setAttempt] = useState(0)
  const trusted = useMemo(() => panelOriginTrusted(panel.entryUrl, hostDid, allowOrigins), [panel.entryUrl, hostDid, allowOrigins])
  const src = useMemo(() => trusted ? panelSrc(panel.entryUrl, context) : '', [trusted, panel.entryUrl, context])
  if (!trusted) {
    return <div className={`${className} flex flex-col items-center justify-center gap-2 rounded-xl p-4 text-center text-[13px]`} style={{ background: 'color-mix(in srgb, var(--cp-danger) 8%, transparent)' }} role="alert" data-testid="group-panel-untrusted" data-panel-id={panel.id}>
      <ShieldAlert size={22} aria-hidden className="text-[color:var(--cp-danger)]" />
      <p className="font-medium">{t('messagehub.panels.untrusted')}</p>
      <p className="break-all text-xs text-[color:var(--cp-muted)]">{panel.entryUrl}</p>
    </div>
  }
  // A new `src` (session switch) or a retry remounts the frame, so its load state starts over.
  return <FrameLoader key={`${attempt}\n${src}`} panel={panel} src={src} loadTimeoutMs={loadTimeoutMs} className={className} onRetry={() => setAttempt(value => value + 1)} />
}

function FrameLoader({ panel, src, loadTimeoutMs, className, onRetry }: { panel: GroupPanelRef; src: string; loadTimeoutMs: number; className: string; onRetry: () => void }) {
  const { t } = useI18n()
  const [state, setState] = useState<FrameState>('loading')
  // Chromium fires `load` on a cross-origin frame even when the server is down (it shows its
  // own error page), so reachability is probed first: an opaque no-cors response means the
  // App answers; a network / DNS / TLS failure or the gateway dropping the connection fails it.
  const [reachable, setReachable] = useState(false)
  useEffect(() => {
    const controller = new AbortController()
    const timer = setTimeout(() => { controller.abort(); setState(current => current === 'loading' ? 'failed' : current) }, loadTimeoutMs)
    fetch(src, { mode: 'no-cors', cache: 'no-store', credentials: 'omit', referrerPolicy: 'origin', signal: controller.signal })
      .then(() => setReachable(true), () => { if (!controller.signal.aborted) setState('failed') })
    return () => { clearTimeout(timer); controller.abort() }
  }, [src, loadTimeoutMs])
  return <div className={`${className} relative overflow-hidden rounded-xl`} style={{ background: 'color-mix(in srgb, var(--cp-text) 4%, transparent)' }} data-testid="group-panel-frame" data-panel-id={panel.id} data-state={state}>
    {state === 'failed' ? <PanelUnavailable onRetry={onRetry} />
      : <>
        {state === 'loading' && <p role="status" className="pointer-events-none absolute inset-x-0 top-0 p-3 text-center text-xs text-[color:var(--cp-muted)]">{t('messagehub.panels.loading')}</p>}
        {reachable && <iframe title={panel.title} src={src} sandbox={SANDBOX} referrerPolicy="origin" className="h-full w-full border-0" onLoad={() => setState('ready')} onError={() => setState('failed')} />}
      </>}
  </div>
}

function PanelUnavailable({ onRetry }: { onRetry: () => void }) {
  const { t } = useI18n()
  return <div className="flex h-full min-h-[120px] flex-col items-center justify-center gap-2 p-4 text-center text-[13px]" role="alert" data-testid="group-panel-unavailable">
    <p>{t('messagehub.panels.unavailable')}</p>
    <button type="button" className={hubButtonClass} onClick={onRetry}>{t('messagehub.panels.retry')}</button>
  </div>
}

interface BoundaryProps { children: ReactNode; fallback: (error: Error, reset: () => void) => ReactNode; resetKey: string }
interface BoundaryState { error: Error | null; key: string }

/** A panel that throws while rendering never takes the details pane or the conversation with it. */
class PanelFrameBoundary extends Component<BoundaryProps, BoundaryState> {
  state: BoundaryState = { error: null, key: this.props.resetKey }
  static getDerivedStateFromProps(props: BoundaryProps, state: BoundaryState): Partial<BoundaryState> | null {
    return props.resetKey !== state.key ? { error: null, key: props.resetKey } : null
  }
  static getDerivedStateFromError(error: Error): Partial<BoundaryState> { return { error } }
  componentDidCatch(error: Error, info: ErrorInfo) { console.error('[messagehub] group panel failed', error, info.componentStack) }
  render() {
    if (this.state.error) return this.props.fallback(this.state.error, () => this.setState({ error: null }))
    return this.props.children
  }
}
