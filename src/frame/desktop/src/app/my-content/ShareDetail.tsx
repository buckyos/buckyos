/**
 * Share details (design §3.12): link and policy controls, snapshot,
 * retention, delivery, consumption and history; stop / hide.
 */

import { Check, Copy, ExternalLink, X } from 'lucide-react'
import { useEffect, useState, type ReactNode } from 'react'
import { useI18n } from '../../i18n/provider'
import { toShareError, type ShareDetail as Detail, type ShareError, type ShareService, type ShareStatsResult } from '../../api/shares'
import { desktopUIStore } from '../../models/DesktopUIDataModel'
import {
  EXPIRY_CHOICES,
  accessLabel,
  appName,
  displaySourcePath,
  expiryFor,
  expiryLabel,
  formatBytes,
  formatTime,
  localDateInput,
  shareErrorMessage,
  statusLabel,
  type ExpiryChoice,
} from '../../sysdlg/shareFormat'
import { ConsumptionChart } from './ConsumptionChart'
import { StatusChip } from './StatusChip'

function Row({ label, children, testId }: { label: string; children: ReactNode; testId?: string }) {
  return (
    <div className="grid gap-1 border-b border-[var(--cp-border)] py-2.5 text-sm sm:grid-cols-[132px_minmax(0,1fr)]" data-testid={testId}>
      <dt className="text-[var(--cp-muted)]">{label}</dt>
      <dd className="min-w-0 break-words">{children}</dd>
    </div>
  )
}

function SmallBtn({ children, onClick, testId, danger, disabled, primary }: { children: ReactNode; onClick: () => void; testId?: string; danger?: boolean; disabled?: boolean; primary?: boolean }) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      data-testid={testId}
      className={`inline-flex min-h-9 items-center gap-1.5 rounded-lg border px-3 text-sm disabled:opacity-40 ${primary ? 'border-[var(--cp-accent)] bg-[var(--cp-accent)] font-semibold text-[var(--cp-surface-opaque)]' : danger ? 'border-[var(--cp-danger)] text-[var(--cp-danger)]' : 'border-[var(--cp-border)]'}`}
    >
      {children}
    </button>
  )
}

const copy = (text: string) => navigator.clipboard?.writeText(text).catch(() => {})

export function ShareDetail({
  service,
  shareId,
  onClose,
  onChanged,
  onSelect,
}: {
  service: ShareService
  shareId: string
  onClose: () => void
  onChanged: () => void
  onSelect: (shareId: string) => void
}) {
  const { t, locale } = useI18n()
  const [detail, setDetail] = useState<Detail | null>(null)
  const [stats, setStats] = useState<ShareStatsResult | null>(null)
  const [error, setError] = useState<ShareError | null>(null)
  const [busy, setBusy] = useState(false)
  const [reload, setReload] = useState(0)
  const [editing, setEditing] = useState<'expiry' | 'password' | 'stop' | null>(null)
  const [expiry, setExpiry] = useState<ExpiryChoice>('7d')
  const [customDate, setCustomDate] = useState(() => localDateInput(7))
  const [password, setPassword] = useState('')
  const [copied, setCopied] = useState(false)

  useEffect(() => {
    let alive = true
    service
      .get(shareId)
      .then((d) => {
        if (!alive) return
        setDetail(d)
        if (d.kind === 'link') {
          service.stats({ shareId, granularity: 'day' }).then((s) => alive && setStats(s)).catch(() => {})
        } else {
          setStats(null)
        }
      })
      .catch((err: unknown) => alive && setError(toShareError(err)))
    return () => {
      alive = false
    }
  }, [service, shareId, reload])

  const act = async (op: (d: Detail) => Promise<unknown>) => {
    if (!detail) return
    setBusy(true)
    setError(null)
    try {
      await op(detail)
      setEditing(null)
      setPassword('')
      onChanged()
    } catch (err) {
      setError(toShareError(err))
    } finally {
      setBusy(false)
      setReload((n) => n + 1)
    }
  }

  if (!detail) {
    return (
      <aside className="mc-detail" data-testid="mc-detail">
        <div className="flex justify-end"><button type="button" aria-label={t('myContent.detail.close')} onClick={onClose}><X size={18} /></button></div>
        {error ? <p className="text-sm text-[var(--cp-danger)]">{shareErrorMessage(t, error)}</p> : <p className="text-sm text-[var(--cp-muted)]">…</p>}
      </aside>
    )
  }

  const active = detail.status === 'active'
  const isLink = detail.kind === 'link'
  const related = detail.related
  const openInApp = () => {
    if (detail.target_app === 'messagehub') desktopUIStore.openAppWindow('messagehub')
    else desktopUIStore.openAppWindow('homestation')
  }
  const stopText = !isLink
    ? t('myContent.detail.stopDeliveryConfirm')
    : related?.kind === 'app_delivery'
      ? `${t('myContent.detail.stopConfirm')} ${t('myContent.detail.stopLinkedConfirm')}`
      : t('myContent.detail.stopConfirm')

  return (
    <aside className="mc-detail" data-testid="mc-detail" aria-label={detail.title}>
      <header className="flex items-start gap-2">
        <div className="min-w-0 flex-1">
          <h2 className="break-words text-base font-semibold" data-testid="mc-detail-title">{detail.title}</h2>
          <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-[var(--cp-muted)]">
            <StatusChip status={detail.status} />
            <span>{isLink ? t('myContent.kind.link') : t('myContent.kind.app_delivery', '', { app: appName(detail.target_app) })}</span>
          </div>
        </div>
        <button type="button" className="rounded-lg p-1" aria-label={t('myContent.detail.close')} data-testid="mc-detail-close" onClick={onClose}><X size={18} /></button>
      </header>
      {error && <p role="alert" className="mt-2 rounded-lg border border-[var(--cp-danger)] p-2 text-sm" data-testid="mc-detail-error">{shareErrorMessage(t, error)}</p>}

      {isLink && detail.url && active && (
        <div className="mt-3 flex items-center gap-2">
          <input readOnly value={detail.url} aria-label={t('myContent.detail.link')} data-testid="mc-detail-url" className="min-w-0 flex-1 rounded-lg border border-[var(--cp-border)] bg-[var(--cp-surface-2)] px-2 py-1.5 font-mono text-xs" onFocus={(e) => e.currentTarget.select()} />
          <SmallBtn primary testId="mc-detail-copy" onClick={() => { void copy(detail.url!); setCopied(true) }}>{copied ? <Check size={14} /> : <Copy size={14} />}{copied ? t('share.dialog.copied') : t('share.dialog.copy')}</SmallBtn>
        </div>
      )}

      <dl className="mt-3">
        {isLink && (
          <>
            <Row label={t('share.dialog.access')}>{accessLabel(t, detail)}</Row>
            <Row label={t('myContent.detail.expires')} testId="mc-detail-expiry">
              {editing === 'expiry' ? (
                <span className="flex flex-wrap items-center gap-2">
                  <select value={expiry} onChange={(e) => setExpiry(e.target.value as ExpiryChoice)} aria-label={t('myContent.detail.expires')} data-testid="mc-expiry-select" className="rounded-lg border border-[var(--cp-border)] bg-[var(--cp-surface)] px-2 py-1">
                    {EXPIRY_CHOICES.map((c) => <option key={c} value={c}>{t(`share.dialog.expiry.${c}`)}</option>)}
                  </select>
                  {expiry === 'custom' && <input type="date" value={customDate} min={localDateInput(1)} onChange={(e) => setCustomDate(e.target.value)} aria-label={t('share.dialog.expiry.custom')} className="rounded-lg border border-[var(--cp-border)] bg-[var(--cp-surface)] px-2 py-1" />}
                  <SmallBtn primary disabled={busy} testId="mc-expiry-save" onClick={() => void act((d) => service.update(d.share_id, d.revision, { expires_at: expiryFor(expiry, customDate) }))}>{t('myContent.detail.save')}</SmallBtn>
                  <SmallBtn onClick={() => setEditing(null)}>{t('myContent.detail.cancel')}</SmallBtn>
                </span>
              ) : (
                <span className="flex flex-wrap items-center gap-2">
                  {expiryLabel(t, detail.expires_at, locale)}
                  {active && <button type="button" className="text-[var(--cp-accent)]" data-testid="mc-expiry-edit" onClick={() => setEditing('expiry')}>{t('myContent.detail.changeExpiry')}</button>}
                </span>
              )}
            </Row>
            <Row label={t('myContent.detail.password')} testId="mc-detail-password">
              {editing === 'password' ? (
                <span className="flex flex-wrap items-center gap-2">
                  <input type="password" autoComplete="new-password" value={password} maxLength={128} onChange={(e) => setPassword(e.target.value)} placeholder={t('myContent.detail.newPassword')} aria-label={t('myContent.detail.newPassword')} data-testid="mc-password-input" className="rounded-lg border border-[var(--cp-border)] bg-[var(--cp-surface)] px-2 py-1" />
                  <SmallBtn primary disabled={busy || !password} testId="mc-password-save" onClick={() => void act((d) => service.update(d.share_id, d.revision, { password }))}>{t('myContent.detail.save')}</SmallBtn>
                  <SmallBtn onClick={() => setEditing(null)}>{t('myContent.detail.cancel')}</SmallBtn>
                </span>
              ) : (
                <span className="flex flex-wrap items-center gap-2">
                  {detail.has_password ? t('myContent.detail.passwordOn') : t('myContent.detail.passwordOff')}
                  {active && <button type="button" className="text-[var(--cp-accent)]" data-testid="mc-password-edit" onClick={() => setEditing('password')}>{detail.has_password ? t('myContent.detail.changePassword') : t('myContent.detail.setPassword')}</button>}
                  {active && detail.has_password && <button type="button" className="text-[var(--cp-accent)]" data-testid="mc-password-remove" onClick={() => void act((d) => service.update(d.share_id, d.revision, { password: null }))}>{t('myContent.detail.removePassword')}</button>}
                </span>
              )}
            </Row>
            <Row label={t('share.dialog.allowDownload')}>
              <input type="checkbox" checked={!!detail.allow_download} disabled={!active || busy} aria-label={t('share.dialog.allowDownload')} data-testid="mc-allow-download" onChange={(e) => void act((d) => service.update(d.share_id, d.revision, { allow_download: e.target.checked }))} />
            </Row>
          </>
        )}
        {!isLink && (
          <Row label={t('myContent.detail.delivery')} testId="mc-detail-delivery">
            <span className="flex flex-wrap items-center gap-2">
              {t(`myContent.delivery.${detail.delivery_state ?? 'unknown'}`, '', { time: formatTime(detail.handoff_at, locale) })}
              <button type="button" className="inline-flex items-center gap-1 text-[var(--cp-accent)]" onClick={openInApp}><ExternalLink size={14} />{t('myContent.detail.openInApp', '', { app: appName(detail.target_app) })}</button>
            </span>
          </Row>
        )}
        {related && (
          <Row label={t('myContent.detail.related')}>
            <button type="button" className="text-left text-[var(--cp-accent)]" data-testid="mc-detail-related" onClick={() => onSelect(related.share_id)}>
              {related.kind === 'link' ? t('myContent.kind.link') : t('myContent.kind.app_delivery', '', { app: appName(related.target_app) })} · {statusLabel(t, related.status)}
            </button>
          </Row>
        )}
        <Row label={t('myContent.detail.size')}>{formatBytes(detail.total_size)}{detail.content_kind === 'folder' && detail.item_count != null ? ` · ${t('share.dialog.items', '', { count: detail.item_count })}` : ''}</Row>
        <Row label={t('myContent.detail.created')}>{formatTime(detail.created_at, locale)}</Row>
        <Row label={t('myContent.detail.source')}><span className="font-mono text-xs">{displaySourcePath(detail.source_path)}</span></Row>
        <Row label={t('myContent.detail.snapshot')}>
          <details>
            <summary className="cursor-pointer font-mono text-xs">{detail.obj_id.slice(0, 18)}…</summary>
            <p className="mt-1 break-all font-mono text-xs" data-testid="mc-detail-objid">{detail.obj_id}</p>
            <button type="button" className="text-xs text-[var(--cp-accent)]" onClick={() => void copy(detail.obj_id)}>{t('myContent.detail.copyId')}</button>
          </details>
        </Row>
        <Row label={t('myContent.detail.retention')}>
          {detail.retained ? t('myContent.detail.retained') : t('myContent.detail.released', '', { time: formatTime(detail.stopped_at, locale) })}
        </Row>
      </dl>

      <section className="mt-4">
        <h3 className="mb-2 text-sm font-semibold">{t('myContent.detail.consumption')}</h3>
        {isLink && stats ? (
          <>
            <p className="mb-2 text-xs text-[var(--cp-muted)]">
              {t('myContent.views')} {detail.stats.views} · {t('myContent.downloads')} {detail.stats.downloads} · {t('myContent.visitors')} {detail.stats.visitors} · {t('myContent.traffic')} {formatBytes(detail.stats.bytes)}
            </p>
            <ConsumptionChart series={stats.series} from={stats.from} to={stats.to} width={400} testId="mc-detail-chart" />
          </>
        ) : (
          <p className="text-xs text-[var(--cp-muted)]">{isLink ? '…' : t('myContent.detail.notCounted')}</p>
        )}
      </section>

      <section className="mt-4">
        <h3 className="mb-2 text-sm font-semibold">{t('myContent.detail.audit')}</h3>
        <ol className="space-y-1 text-xs" data-testid="mc-detail-audit">
          {detail.audit.map((a, i) => (
            <li key={`${a.at}-${i}`} className="flex justify-between gap-2">
              <span>{t(`myContent.audit.${a.action}`, a.action, { reason: a.reason ?? '' })}{a.actor_app_id ? ` · ${a.actor_app_id}` : ''}{a.source !== 'gui' ? ` · ${a.source}` : ''}</span>
              <span className="shrink-0 text-[var(--cp-muted)]">{formatTime(a.at, locale)}</span>
            </li>
          ))}
        </ol>
      </section>

      {editing === 'stop' && (
        <div className="mt-4 rounded-xl border border-[var(--cp-danger)] p-3 text-sm" role="alertdialog" aria-label={t('myContent.detail.stop')} data-testid="mc-stop-confirm">
          <p>{stopText}</p>
          <div className="mt-2 flex justify-end gap-2">
            <SmallBtn onClick={() => setEditing(null)}>{t('myContent.detail.cancel')}</SmallBtn>
            <SmallBtn danger disabled={busy} testId="mc-stop-confirm-button" onClick={() => void act((d) => service.stop(d.share_id, d.revision))}>{t('myContent.detail.stop')}</SmallBtn>
          </div>
        </div>
      )}
      <footer className="mt-4 flex flex-wrap justify-between gap-2 border-t border-[var(--cp-border)] pt-3">
        <SmallBtn testId="mc-hide" disabled={busy} onClick={() => void act((d) => service.hide(d.share_id, !d.hidden))}>{detail.hidden ? t('myContent.detail.unhide') : t('myContent.detail.hide')}</SmallBtn>
        {active && editing !== 'stop' && <SmallBtn danger testId="mc-stop" onClick={() => setEditing('stop')}>{t('myContent.detail.stop')}</SmallBtn>}
      </footer>
    </aside>
  )
}
