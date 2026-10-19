/**
 * System share dialog (`sysdlg/share`, design §3.12): one place to share a
 * file or folder — a link (holder or password, with an expiry), or a hand-off
 * to MessageHub / HomeStation. Opening and closing it has no side effects:
 * only `share.plan` and `share.capabilities` run until the user acts.
 */

import { AlertTriangle, Check, Copy, ExternalLink, FileText, Folder, Globe, Home, Link2, Loader2, MessageSquare, Rocket } from 'lucide-react'
import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { useI18n } from '../i18n/provider'
import {
  ShareError,
  shareService,
  toShareError,
  waitShareOperation,
  type ShareCapabilities,
  type ShareOperation,
  type SharePlan,
  type ShareService,
  type ShareTargetApp,
} from '../api/shares'
import {
  EXPIRY_CHOICES,
  accessLabel,
  expiryFor,
  expiryLabel,
  formatBytes,
  localDateInput,
  shareErrorMessage,
  timeZone,
  type ExpiryChoice,
} from './shareFormat'
import { openShareTarget } from './shareTargets'
import { desktopUIStore } from '../models/DesktopUIDataModel'

export const SHARE_DIALOG_PATH = 'sysdlg/share' as const

export interface ShareDialogParams {
  /** DFS path of the file or folder, e.g. `/home/alice/report.pdf`. */
  source: string
  title?: string
  preferred?: 'link' | ShareTargetApp
}

export type ShareDialogResult =
  | { action: 'created'; shareId: string; url: string }
  | { action: 'handed_off'; shareId: string; app: ShareTargetApp }
  | { action: 'background'; operationId: string }
  | { action: 'close' }

type Step =
  | { kind: 'home' }
  | { kind: 'target'; app: ShareTargetApp; plan: SharePlan | null }
  | { kind: 'progress'; op: ShareOperation }
  | { kind: 'created'; op: ShareOperation }
  | { kind: 'site' }
  | { kind: 'pikg' }

const newKey = () => (typeof crypto !== 'undefined' && 'randomUUID' in crypto ? crypto.randomUUID() : `${Date.now()}-${Math.random()}`)

function copyToClipboard(text: string): Promise<void> {
  if (navigator.clipboard?.writeText) return navigator.clipboard.writeText(text)
  const area = document.createElement('textarea')
  area.value = text
  document.body.appendChild(area)
  area.select()
  document.execCommand('copy')
  area.remove()
  return Promise.resolve()
}

function Btn({ children, onClick, primary, disabled, testId, type = 'button' }: { children: ReactNode; onClick?: () => void; primary?: boolean; disabled?: boolean; testId?: string; type?: 'button' | 'submit' }) {
  return (
    <button
      type={type}
      onClick={onClick}
      disabled={disabled}
      data-testid={testId}
      className={`inline-flex min-h-10 items-center justify-center gap-2 rounded-xl border px-4 py-2 text-sm font-semibold disabled:cursor-not-allowed disabled:opacity-40 ${primary ? 'border-[var(--cp-accent)] bg-[var(--cp-accent)] text-[var(--cp-surface-opaque)]' : 'border-[var(--cp-border)] bg-[var(--cp-surface)] text-[var(--cp-text)] hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_14%,var(--cp-surface))]'}`}
    >
      {children}
    </button>
  )
}

function Notice({ tone = 'info', children, testId }: { tone?: 'info' | 'warn' | 'error'; children: ReactNode; testId?: string }) {
  const border = tone === 'error' ? 'var(--cp-danger)' : tone === 'warn' ? 'var(--cp-warning)' : 'var(--cp-border)'
  return (
    <div role={tone === 'error' ? 'alert' : undefined} data-testid={testId} className="flex items-start gap-2 rounded-xl border p-3 text-sm" style={{ borderColor: border }}>
      {tone !== 'info' && <AlertTriangle size={16} className="mt-0.5 shrink-0" style={{ color: border }} aria-hidden />}
      <div className="min-w-0">{children}</div>
    </div>
  )
}

function CopyField({ url, testId }: { url: string; testId: string }) {
  const { t } = useI18n()
  const [copied, setCopied] = useState(false)
  return (
    <div className="flex items-center gap-2">
      <input readOnly value={url} aria-label={t('share.dialog.copy')} data-testid={`${testId}-url`} onFocus={(e) => e.currentTarget.select()} className="min-w-0 flex-1 rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-2)] px-3 py-2 font-mono text-xs" />
      <Btn primary testId={testId} onClick={() => void copyToClipboard(url).then(() => setCopied(true))}>
        {copied ? <Check size={16} aria-hidden /> : <Copy size={16} aria-hidden />}
        {copied ? t('share.dialog.copied') : t('share.dialog.copy')}
      </Btn>
    </div>
  )
}

export function ShareDialog({ params, onDone }: { params: ShareDialogParams; onDone: (result: ShareDialogResult) => void }) {
  const { t, locale } = useI18n()
  const [service, setService] = useState<ShareService | null>(null)
  const [caps, setCaps] = useState<ShareCapabilities | null>(null)
  const [plan, setPlan] = useState<SharePlan | null>(null)
  const [error, setError] = useState<ShareError | null>(null)
  const [step, setStep] = useState<Step>({ kind: 'home' })
  const [busy, setBusy] = useState(false)
  const [access, setAccess] = useState<'holder' | 'password'>('holder')
  const [password, setPassword] = useState('')
  const [expiry, setExpiry] = useState<ExpiryChoice>('7d')
  const [customDate, setCustomDate] = useState(() => localDateInput(7))
  const [allowDownload, setAllowDownload] = useState(true)
  const [reload, setReload] = useState(0)
  const following = useRef<AbortController | null>(null)

  useEffect(() => () => following.current?.abort(), [])

  useEffect(() => {
    let alive = true
    shareService()
      .then(async (svc) => {
        if (!alive) return
        setService(svc)
        const [c, p] = await Promise.all([svc.capabilities(), svc.plan(params.source)])
        if (!alive) return
        setCaps(c)
        setPlan(p)
        setError(null)
      })
      .catch((err: unknown) => alive && setError(toShareError(err)))
    return () => {
      alive = false
    }
  }, [params.source, reload])

  const blocked = useMemo(() => new Map((caps?.blocked ?? []).map((b) => [b.action, b.reason])), [caps])
  const name = params.title || plan?.source.name || params.source.split('/').pop() || params.source
  const isDir = plan?.source.kind === 'dir'
  const isPikg = /\.pikg$/i.test(name)
  const expiresAt = expiryFor(expiry, customDate)
  const passwordMissing = access === 'password' && !password

  const finish = (op: ShareOperation) => {
    if (op.state === 'completed') {
      if (op.intent === 'app_handoff' && op.handoff) {
        void openShareTarget(op.handoff)
        onDone({ action: 'handed_off', shareId: op.handoff.share_id, app: op.handoff.launch.appId })
        return
      }
      setStep({ kind: 'created', op })
      return
    }
    const reason = typeof op.error?.reason === 'string' ? op.error.reason : op.state === 'canceled' ? 'canceled' : undefined
    setError(new ShareError(op.error?.message ?? op.state, op.error?.code ?? 'INTERNAL', reason))
    setStep({ kind: 'home' })
  }

  const follow = async (svc: ShareService, op: ShareOperation) => {
    if (op.state === 'running' || op.state === 'preparing') {
      setStep({ kind: 'progress', op })
      following.current?.abort()
      const ctl = new AbortController()
      following.current = ctl
      const done = await waitShareOperation(svc, op, { signal: ctl.signal, onProgress: (cur) => !ctl.signal.aborted && setStep({ kind: 'progress', op: cur }) })
      if (ctl.signal.aborted) return
      finish(done)
      return
    }
    finish(op)
  }

  const createLink = async (redoFrom?: string) => {
    if (!service || !plan) return
    setBusy(true)
    setError(null)
    try {
      if (redoFrom) {
        const old = await service.get(redoFrom)
        await service.stop(redoFrom, old.revision, 'redo')
      }
      const op = await service.create(
        params.source,
        { access, password: access === 'password' ? password : undefined, expires_at: expiresAt, allow_download: allowDownload },
        { title: params.title, idempotencyKey: newKey(), planFingerprint: redoFrom ? undefined : plan.plan_fingerprint },
      )
      await follow(service, op)
    } catch (err) {
      const e = toShareError(err)
      setError(e)
      if (e.reason === 'plan_stale') setReload((n) => n + 1)
    } finally {
      setBusy(false)
    }
  }

  const chooseTarget = async (app: ShareTargetApp) => {
    if (!service) return
    setStep({ kind: 'target', app, plan: null })
    setError(null)
    try {
      const p = await service.plan(params.source, { intent: 'app_handoff', target: app })
      setStep({ kind: 'target', app, plan: p })
    } catch (err) {
      setError(toShareError(err))
      setStep({ kind: 'home' })
    }
  }

  const handoff = async (app: ShareTargetApp) => {
    if (!service) return
    setBusy(true)
    setError(null)
    try {
      const op = await service.handoff(params.source, app, { title: params.title, idempotencyKey: newKey() })
      await follow(service, op)
    } catch (err) {
      setError(toShareError(err))
    } finally {
      setBusy(false)
    }
  }

  const openMyContent = (shareId?: string) => {
    desktopUIStore.openAppWindow('my-content', { launch: { requestId: newKey(), payload: shareId ? { shareId } : {} } })
  }

  const header = (
    <div className="flex items-start gap-3">
      <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-[var(--cp-surface-2)]">
        {isDir ? <Folder size={20} aria-hidden /> : <FileText size={20} aria-hidden />}
      </div>
      <div className="min-w-0">
        <h2 className="truncate text-base font-semibold" data-testid="share-dialog-name">{name}</h2>
        <p className="text-xs text-[var(--cp-muted)]" data-testid="share-dialog-summary">
          {plan ? (
            <>
              {formatBytes(plan.estimate.total_size)}
              {isDir && <> · {t('share.dialog.items', '{{count}} items', { count: plan.estimate.item_count })}</>}
              {plan.estimate.excluded.length > 0 && <> · {t('share.dialog.excluded', '{{count}} left out', { count: plan.estimate.excluded.length })}</>}
            </>
          ) : (
            params.source
          )}
        </p>
      </div>
    </div>
  )

  const body = (() => {
    if (!plan && !error) {
      return (
        <p className="flex items-center gap-2 text-sm text-[var(--cp-muted)]" data-testid="share-dialog-loading">
          <Loader2 size={16} className="animate-spin" aria-hidden />
          {t('share.dialog.loading')}
        </p>
      )
    }
    if (!plan) return null
    if (step.kind === 'progress') {
      const done = step.op.progress.done_bytes ?? 0
      const total = step.op.progress.total_bytes ?? plan.estimate.total_size
      const pct = total ? Math.min(100, Math.round((done / total) * 100)) : 0
      return (
        <section className="space-y-3" data-testid="share-dialog-progress">
          <p className="text-sm font-semibold">{t('share.dialog.preparing')}</p>
          <div className="h-2 overflow-hidden rounded-full bg-[var(--cp-surface-2)]" role="progressbar" aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100}>
            <div className="h-full rounded-full bg-[var(--cp-accent)] transition-[width]" style={{ width: `${pct}%` }} />
          </div>
          <p className="text-xs text-[var(--cp-muted)]">{t('share.dialog.progress', '{{done}} of {{total}}', { done: formatBytes(done), total: formatBytes(total) })}</p>
          <div className="flex flex-wrap justify-end gap-2">
            <Btn testId="share-dialog-cancel-prepare" onClick={() => service && void service.operation(step.op.operation_id, { cancel: true })}>{t('share.dialog.cancelPrepare')}</Btn>
            <Btn primary testId="share-dialog-background" onClick={() => onDone({ action: 'background', operationId: step.op.operation_id })}>{t('share.dialog.background')}</Btn>
          </div>
        </section>
      )
    }
    if (step.kind === 'created') {
      const url = step.op.url ?? ''
      const share = step.op.share
      return (
        <section className="space-y-3" data-testid="share-dialog-created">
          <p className="flex items-center gap-2 text-sm font-semibold"><Check size={16} className="text-[var(--cp-success)]" aria-hidden />{t('share.dialog.created')}</p>
          <CopyField url={url} testId="share-dialog-copy" />
          {share && (
            <p className="text-xs text-[var(--cp-muted)]">
              {accessLabel(t, share)} · {expiryLabel(t, share.expires_at, locale)}
            </p>
          )}
          {step.op.source_changed && (
            <Notice tone="warn" testId="share-dialog-source-changed">
              <p>{t('share.dialog.sourceChanged')}</p>
              <button type="button" className="mt-1 text-sm font-semibold text-[var(--cp-accent)]" disabled={busy} onClick={() => void createLink(step.op.share_id ?? undefined)}>
                {t('share.dialog.redo')}
              </button>
            </Notice>
          )}
          <div className="flex flex-wrap justify-between gap-2">
            <Btn testId="share-dialog-manage" onClick={() => { openMyContent(step.op.share_id ?? undefined); onDone({ action: 'created', shareId: step.op.share_id ?? '', url }) }}>
              <ExternalLink size={16} aria-hidden />{t('share.dialog.manage')}
            </Btn>
            <Btn primary testId="share-dialog-done" onClick={() => onDone({ action: 'created', shareId: step.op.share_id ?? '', url })}>{t('share.dialog.done')}</Btn>
          </div>
        </section>
      )
    }
    if (step.kind === 'target') {
      const fit = step.plan?.target_fit
      return (
        <section className="space-y-3" data-testid={`share-dialog-target-${step.app}`}>
          {!fit ? (
            <p className="flex items-center gap-2 text-sm text-[var(--cp-muted)]"><Loader2 size={16} className="animate-spin" aria-hidden />{t('share.dialog.loading')}</p>
          ) : fit.mode === 'attachment' ? (
            <p className="text-sm">{t(`share.dialog.target.attachment.${step.app}`, '', { name })}</p>
          ) : (
            <Notice tone="warn" testId="share-dialog-target-link">
              {t('share.dialog.target.link', '', { reason: t(`share.dialog.target.reason.${fit.reason ?? 'too_large'}`) })}
            </Notice>
          )}
          <div className="flex flex-wrap justify-end gap-2">
            <Btn testId="share-dialog-target-back" onClick={() => setStep({ kind: 'home' })}>{t('share.dialog.target.back')}</Btn>
            <Btn primary disabled={!fit || busy} testId="share-dialog-target-continue" onClick={() => void handoff(step.app)}>
              {busy && <Loader2 size={16} className="animate-spin" aria-hidden />}
              {t('share.dialog.target.continue')}
            </Btn>
          </div>
        </section>
      )
    }
    if (step.kind === 'site' || step.kind === 'pikg') {
      return (
        <section className="space-y-3" data-testid={`share-dialog-${step.kind}`}>
          {step.kind === 'site' && <h3 className="text-sm font-semibold">{t('share.dialog.site.title')}</h3>}
          <p className="text-sm">{step.kind === 'site' ? t('share.dialog.site.body') : t('share.dialog.pikg.body')}</p>
          <div className="flex flex-wrap justify-end gap-2">
            <Btn onClick={() => setStep({ kind: 'home' })}>{t('share.dialog.target.back')}</Btn>
            {step.kind === 'pikg' && (
              <Btn primary onClick={() => { desktopUIStore.openAppWindow('app-service'); onDone({ action: 'close' }) }}>{t('share.dialog.pikg.open')}</Btn>
            )}
          </div>
        </section>
      )
    }
    const linkBlocked = blocked.get('create_link')
    const appBlocked = blocked.get('app_handoff')
    return (
      <div className="space-y-4">
        {(plan.source_public.in_publish_dir || plan.source_public.public_acl) && (
          <Notice tone="warn" testId="share-dialog-public">
            <p>{t('share.dialog.alreadyPublic')}</p>
            {plan.source_public.public_url && (
              <a className="mt-1 inline-flex items-center gap-1 break-all text-[var(--cp-accent)]" href={plan.source_public.public_url} target="_blank" rel="noreferrer">
                <Globe size={14} aria-hidden />{plan.source_public.public_url}
              </a>
            )}
          </Notice>
        )}
        <form
          className="space-y-3 rounded-2xl border border-[var(--cp-border)] p-4"
          data-testid="share-dialog-link-form"
          onSubmit={(e) => { e.preventDefault(); if (!passwordMissing) void createLink() }}
        >
          <h3 className="flex items-center gap-2 text-sm font-semibold"><Link2 size={16} aria-hidden />{t('share.dialog.linkTitle')}</h3>
          <fieldset className="space-y-1">
            <legend className="mb-1 text-xs text-[var(--cp-muted)]">{t('share.dialog.access')}</legend>
            {(['holder', 'password'] as const).map((value) => (
              <label key={value} className="flex min-h-9 cursor-pointer items-center gap-2 text-sm">
                <input type="radio" name="share-access" value={value} checked={access === value} onChange={() => setAccess(value)} data-testid={`share-access-${value}`} />
                {t(`share.dialog.access.${value}`)}
              </label>
            ))}
            {access === 'password' && (
              <input
                type="password"
                autoComplete="new-password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                placeholder={t('share.dialog.password')}
                aria-label={t('share.dialog.password')}
                maxLength={128}
                data-testid="share-password"
                className="mt-1 w-full rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface)] px-3 py-2 text-sm"
              />
            )}
          </fieldset>
          <div className="space-y-1">
            <label className="block text-xs text-[var(--cp-muted)]" htmlFor="share-expiry">{t('share.dialog.expiry')}</label>
            <div className="flex flex-wrap items-center gap-2">
              <select id="share-expiry" value={expiry} onChange={(e) => setExpiry(e.target.value as ExpiryChoice)} data-testid="share-expiry" className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface)] px-3 py-2 text-sm">
                {EXPIRY_CHOICES.map((c) => <option key={c} value={c}>{t(`share.dialog.expiry.${c}`)}</option>)}
              </select>
              {expiry === 'custom' && (
                <input type="date" value={customDate} min={localDateInput(1)} onChange={(e) => setCustomDate(e.target.value)} aria-label={t('share.dialog.expiry.custom')} data-testid="share-expiry-date" className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface)] px-3 py-2 text-sm" />
              )}
            </div>
            <p className="text-xs text-[var(--cp-muted)]" data-testid="share-expiry-preview">
              {expiresAt == null ? t('share.dialog.expiry.never') : t('share.dialog.expiryAt', '', { date: new Date(expiresAt * 1000).toLocaleString(locale, { dateStyle: 'medium', timeStyle: 'short' }), zone: timeZone() })}
            </p>
          </div>
          <label className="flex cursor-pointer items-center gap-2 text-sm">
            <input type="checkbox" checked={allowDownload} onChange={(e) => setAllowDownload(e.target.checked)} data-testid="share-allow-download" />
            {t('share.dialog.allowDownload')}
          </label>
          <p className="text-xs text-[var(--cp-muted)]">{t('share.dialog.snapshotNote')}</p>
          {plan.estimate.excluded.length > 0 && (
            <details className="text-xs" data-testid="share-dialog-excluded">
              <summary className="cursor-pointer text-[var(--cp-muted)]">{t('share.dialog.excludedTitle')} ({plan.estimate.excluded.length})</summary>
              <ul className="mt-1 space-y-0.5">
                {plan.estimate.excluded.slice(0, 50).map((x) => (
                  <li key={x.path} className="flex justify-between gap-2"><span className="truncate font-mono">{x.path}</span><span className="shrink-0 text-[var(--cp-muted)]">{t(`share.dialog.excludedReason.${x.reason}`, x.reason)}</span></li>
                ))}
              </ul>
            </details>
          )}
          {linkBlocked ? (
            <Notice tone="warn">{t(`share.dialog.blocked.${linkBlocked}`, linkBlocked)}</Notice>
          ) : (
            <div className="flex justify-end">
              <Btn primary type="submit" disabled={busy || passwordMissing} testId="share-dialog-create">
                {busy && <Loader2 size={16} className="animate-spin" aria-hidden />}
                {busy ? t('share.dialog.creating') : t('share.dialog.create')}
              </Btn>
            </div>
          )}
        </form>
        <section className="space-y-2">
          <h3 className="text-xs text-[var(--cp-muted)]">{t('share.dialog.withApp')}</h3>
          <div className="flex flex-wrap gap-2">
            <Btn disabled={!!appBlocked} testId="share-dialog-messagehub" onClick={() => void chooseTarget('messagehub')}><MessageSquare size={16} aria-hidden />{t('share.dialog.messagehub')}</Btn>
            <Btn disabled={!!appBlocked} testId="share-dialog-homestation" onClick={() => void chooseTarget('homestation')}><Home size={16} aria-hidden />{t('share.dialog.homestation')}</Btn>
            {(plan.site.has_index_html || isPikg) && (
              <Btn testId="share-dialog-golive" onClick={() => setStep({ kind: isPikg ? 'pikg' : 'site' })}><Rocket size={16} aria-hidden />{t('share.dialog.goLive')}</Btn>
            )}
          </div>
        </section>
        {plan.existing.length > 0 && (
          <section className="space-y-2" data-testid="share-dialog-existing">
            <h3 className="text-xs text-[var(--cp-muted)]">{t('share.dialog.existing')}</h3>
            {plan.existing.map((s) => (
              <div key={s.share_id} className="space-y-1 rounded-xl border border-[var(--cp-border)] p-3">
                <p className="text-xs text-[var(--cp-muted)]">{accessLabel(t, s)} · {expiryLabel(t, s.expires_at, locale)}</p>
                {s.url && <CopyField url={s.url} testId={`share-existing-copy-${s.share_id}`} />}
              </div>
            ))}
          </section>
        )}
      </div>
    )
  })()

  return (
    <div className="space-y-4 p-1" data-testid="share-dialog">
      {header}
      {error && <Notice tone="error" testId="share-dialog-error">{shareErrorMessage(t, error)}</Notice>}
      {body}
      {step.kind === 'home' && (
        <div className="flex justify-end border-t border-[var(--cp-border)] pt-3">
          <Btn testId="share-dialog-close" onClick={() => onDone({ action: 'close' })}>{t('share.dialog.close')}</Btn>
        </div>
      )}
    </div>
  )
}
