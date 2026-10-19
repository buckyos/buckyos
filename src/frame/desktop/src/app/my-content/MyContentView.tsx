/**
 * My Content (`my-content`, D12): everything the user has put out — share
 * links, app deliveries, the Public folder, and running websites — with the
 * consumption nfs-server can observe. Income is reserved for a later phase.
 */

import { AlertTriangle, ExternalLink, FileText, Folder, Image as ImageIcon, Link2, Music, RefreshCw, Search, Send, Video } from 'lucide-react'
import { useCallback, useEffect, useState } from 'react'
import { useI18n } from '../../i18n/provider'
import {
  shareService,
  toShareError,
  type PublicListing,
  type ShareError,
  type ShareInfo,
  type ShareService,
  type ShareStatsResult,
} from '../../api/shares'
import { desktopUIStore } from '../../models/DesktopUIDataModel'
import { filesOpenLaunch } from '../filebrowser/launch'
import { accessLabel, appName, displaySourcePath, formatBytes, formatTime, shareErrorMessage } from '../../sysdlg/shareFormat'
import { ConsumptionChart } from './ConsumptionChart'
import { ShareDetail } from './ShareDetail'
import { StatusChip } from './StatusChip'
import type { MyContentLaunch } from './launch'
import './my-content.css'

type Tab = 'all' | 'links' | 'apps' | 'public' | 'sites'
const TABS: Tab[] = ['all', 'links', 'apps', 'public', 'sites']
const PAGE = 50

function KindIcon({ share }: { share: ShareInfo }) {
  const size = 16
  if (share.kind === 'app_delivery') return <Send size={size} aria-hidden />
  switch (share.content_kind) {
    case 'folder': return <Folder size={size} aria-hidden />
    case 'image': return <ImageIcon size={size} aria-hidden />
    case 'video': return <Video size={size} aria-hidden />
    case 'audio': return <Music size={size} aria-hidden />
    default: return <FileText size={size} aria-hidden />
  }
}

function Tile({ label, value, testId }: { label: string; value: string | number; testId: string }) {
  return (
    <div className="mc-tile" data-testid={testId}>
      <div className="mc-tile-value">{value}</div>
      <div className="mc-tile-label">{label}</div>
    </div>
  )
}

function PublicPane({ service }: { service: ShareService }) {
  const { t, locale } = useI18n()
  const [listing, setListing] = useState<PublicListing | null>(null)
  const [error, setError] = useState<ShareError | null>(null)
  const [refresh, setRefresh] = useState(0)
  useEffect(() => {
    let alive = true
    service.publicList({ refresh: refresh > 0 }).then((l) => alive && setListing(l)).catch((e: unknown) => alive && setError(toShareError(e)))
    return () => { alive = false }
  }, [service, refresh])
  return (
    <section className="space-y-3" data-testid="mc-public">
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div>
          <h2 className="text-sm font-semibold">{t('myContent.public.title')}</h2>
          <p className="text-xs text-[var(--cp-muted)]">{t('myContent.public.body')}</p>
        </div>
        <div className="flex gap-2">
          <button type="button" className="inline-flex min-h-9 items-center gap-1 rounded-lg border border-[var(--cp-border)] px-3 text-sm" onClick={() => setRefresh((n) => n + 1)}><RefreshCw size={14} />{t('myContent.refresh')}</button>
          <button type="button" className="inline-flex min-h-9 items-center gap-1 rounded-lg border border-[var(--cp-border)] px-3 text-sm" onClick={() => desktopUIStore.openAppWindow('files', { launch: filesOpenLaunch('/srv/publish') })}><ExternalLink size={14} />{t('myContent.public.openFiles')}</button>
        </div>
      </div>
      {error && <p role="alert" className="text-sm text-[var(--cp-danger)]">{shareErrorMessage(t, error)}</p>}
      {listing && (
        <>
          <p className="text-xs text-[var(--cp-muted)]">{t('myContent.public.lastScan', '', { time: formatTime(listing.last_scan_at, locale) })}</p>
          {listing.entries.length === 0 ? (
            <p className="py-6 text-center text-sm text-[var(--cp-muted)]">{t('myContent.public.empty')}</p>
          ) : (
            <ul className="divide-y divide-[var(--cp-border)] rounded-xl border border-[var(--cp-border)]">
              {listing.entries.map((e) => (
                <li key={e.rel_path} className="flex flex-wrap items-center gap-3 px-3 py-2 text-sm" data-testid="mc-public-entry">
                  {e.entry_kind === 'dir' ? <Folder size={16} aria-hidden /> : e.entry_kind === 'symlink' ? <Link2 size={16} aria-hidden /> : <FileText size={16} aria-hidden />}
                  <span className={`min-w-0 flex-1 break-all ${e.missing_since ? 'text-[var(--cp-muted)] line-through' : ''}`}>{e.rel_path}</span>
                  {e.link_ok === false && <span className="inline-flex items-center gap-1 text-xs text-[var(--cp-danger)]" data-testid="mc-public-broken"><AlertTriangle size={14} aria-hidden />{t('myContent.public.linkBroken')}</span>}
                  {e.missing_since && <span className="text-xs text-[var(--cp-muted)]">{t('myContent.public.missing', '', { time: formatTime(e.missing_since, locale) })}</span>}
                  {e.size != null && !e.missing_since && <span className="text-xs text-[var(--cp-muted)]">{formatBytes(e.size)}</span>}
                  {e.url && <a href={e.url} target="_blank" rel="noreferrer" className="text-xs text-[var(--cp-accent)]">{e.url}</a>}
                </li>
              ))}
            </ul>
          )}
        </>
      )}
    </section>
  )
}

export function MyContentView({ launch }: { launch?: MyContentLaunch }) {
  const { t, locale } = useI18n()
  const [service, setService] = useState<ShareService | null>(null)
  const [tab, setTab] = useState<Tab>(launch?.tab ?? 'all')
  const [query, setQuery] = useState('')
  const [q, setQ] = useState('')
  const [includeHidden, setIncludeHidden] = useState(false)
  const [items, setItems] = useState<ShareInfo[] | null>(null)
  const [cursor, setCursor] = useState<string | null>(null)
  const [stats, setStats] = useState<ShareStatsResult | null>(null)
  const [error, setError] = useState<ShareError | null>(null)
  const [selected, setSelected] = useState<string | null>(launch?.shareId ?? null)
  const [version, setVersion] = useState(0)

  useEffect(() => {
    let alive = true
    shareService().then((s) => alive && setService(s)).catch((e: unknown) => alive && setError(toShareError(e)))
    return () => { alive = false }
  }, [])

  useEffect(() => {
    const id = setTimeout(() => setQ(query.trim()), 250)
    return () => clearTimeout(id)
  }, [query])

  useEffect(() => {
    if (!service) return
    let alive = true
    service.stats({ granularity: 'day' }).then((s) => alive && setStats(s)).catch(() => {})
    return () => { alive = false }
  }, [service, version])

  const kind = tab === 'links' ? 'link' : tab === 'apps' ? 'app_delivery' : undefined
  useEffect(() => {
    if (!service || tab === 'public' || tab === 'sites') return
    let alive = true
    service
      .list({ kind, q: q || undefined, includeHidden, limit: PAGE })
      .then((page) => {
        if (!alive) return
        setItems(page.items)
        setCursor(page.next_cursor ?? null)
        setError(null)
      })
      .catch((e: unknown) => alive && setError(toShareError(e)))
    return () => { alive = false }
  }, [service, tab, kind, q, includeHidden, version])

  const loadMore = async () => {
    if (!service || !cursor) return
    try {
      const page = await service.list({ kind, q: q || undefined, includeHidden, limit: PAGE, cursor })
      setItems((cur) => [...(cur ?? []), ...page.items])
      setCursor(page.next_cursor ?? null)
    } catch (e) {
      setError(toShareError(e))
    }
  }

  const changed = useCallback(() => setVersion((n) => n + 1), [])

  return (
    <div className="mc-root relative flex h-full min-h-0 flex-col bg-[var(--cp-bg)] text-[var(--cp-text)]" data-testid="my-content">
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-5xl space-y-5 px-4 py-4 sm:px-6">
          <header className="flex flex-wrap items-center justify-between gap-2">
            <h1 className="text-lg font-semibold">{t('myContent.title')}</h1>
            <button type="button" className="inline-flex min-h-9 items-center gap-1 rounded-lg border border-[var(--cp-border)] px-3 text-sm" data-testid="mc-refresh" onClick={changed}><RefreshCw size={14} />{t('myContent.refresh')}</button>
          </header>

          <section aria-label={t('myContent.overview')} className="space-y-3" data-testid="mc-overview">
            <div className="flex items-baseline justify-between">
              <h2 className="text-sm font-semibold">{t('myContent.overview')}</h2>
              <span className="text-xs text-[var(--cp-muted)]">{t('myContent.period')}</span>
            </div>
            <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-6">
              <Tile label={t('myContent.views')} value={stats?.totals.views ?? '—'} testId="mc-tile-views" />
              <Tile label={t('myContent.downloads')} value={stats?.totals.downloads ?? '—'} testId="mc-tile-downloads" />
              <Tile label={t('myContent.traffic')} value={stats ? formatBytes(stats.totals.bytes) : '—'} testId="mc-tile-traffic" />
              <Tile label={t('myContent.visitors')} value={stats?.totals.visitors ?? '—'} testId="mc-tile-visitors" />
              <Tile label={t('myContent.activeShares')} value={stats?.summary.active ?? '—'} testId="mc-tile-active" />
              <Tile label={t('myContent.expiringSoon', '', { days: Math.round((stats?.summary.expiring_within ?? 259200) / 86400) })} value={stats?.summary.expiring_soon ?? '—'} testId="mc-tile-expiring" />
            </div>
            {stats && (
              <div className="mc-tile">
                <ConsumptionChart series={stats.series} from={stats.from} to={stats.to} testId="mc-overview-chart" />
                <p className="mt-2 text-xs text-[var(--cp-muted)]">{t('myContent.visitorsNote')} {t('myContent.unavailableNote')}</p>
              </div>
            )}
          </section>

          <div role="tablist" aria-label={t('myContent.title')} className="flex flex-wrap gap-1">
            {TABS.map((id) => (
              <button key={id} type="button" role="tab" aria-selected={tab === id} className="mc-tab" data-testid={`mc-tab-${id}`} onClick={() => setTab(id)}>
                {t(`myContent.tab.${id}`)}
              </button>
            ))}
          </div>

          {error && <p role="alert" className="rounded-lg border border-[var(--cp-danger)] p-2 text-sm" data-testid="mc-error">{shareErrorMessage(t, error)}</p>}

          {tab === 'public' && service && <PublicPane service={service} />}
          {tab === 'sites' && (
            <section className="space-y-2 text-sm" data-testid="mc-sites">
              <p>{t('myContent.sites.body')}</p>
              <button type="button" className="inline-flex min-h-9 items-center gap-1 rounded-lg border border-[var(--cp-border)] px-3" onClick={() => desktopUIStore.openAppWindow('app-service')}><ExternalLink size={14} />{t('myContent.sites.open')}</button>
            </section>
          )}
          {tab !== 'public' && tab !== 'sites' && (
            <section className="space-y-3">
              <div className="flex flex-wrap items-center gap-3">
                <label className="flex min-w-[220px] flex-1 items-center gap-2 rounded-lg border border-[var(--cp-border)] bg-[var(--cp-surface)] px-2">
                  <Search size={14} aria-hidden className="text-[var(--cp-muted)]" />
                  <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder={t('myContent.search')} aria-label={t('myContent.search')} data-testid="mc-search" className="min-w-0 flex-1 bg-transparent py-1.5 text-sm outline-none" />
                </label>
                <label className="flex items-center gap-2 text-sm">
                  <input type="checkbox" checked={includeHidden} onChange={(e) => setIncludeHidden(e.target.checked)} data-testid="mc-show-hidden" />
                  {t('myContent.showHidden')}
                </label>
              </div>
              {items && items.length === 0 && <p className="py-8 text-center text-sm text-[var(--cp-muted)]" data-testid="mc-empty">{t('myContent.empty')}</p>}
              {items && items.length > 0 && (
                <div className="overflow-x-auto rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface)]">
                  <table className="w-full text-sm" data-testid="mc-list">
                    <thead className="hidden text-left text-xs text-[var(--cp-muted)] md:table-header-group">
                      <tr>
                        <th className="px-3 py-2 font-normal">{t('myContent.col.name')}</th>
                        <th className="px-3 py-2 font-normal">{t('myContent.col.type')}</th>
                        <th className="px-3 py-2 font-normal">{t('myContent.col.access')}</th>
                        <th className="px-3 py-2 font-normal">{t('myContent.col.status')}</th>
                        <th className="px-3 py-2 text-right font-normal">{t('myContent.col.views')}</th>
                        <th className="px-3 py-2 font-normal">{t('myContent.col.updated')}</th>
                      </tr>
                    </thead>
                    <tbody>
                      {items.map((s) => (
                        <tr
                          key={s.share_id}
                          className="mc-row block md:table-row"
                          aria-selected={selected === s.share_id}
                          data-testid="mc-row"
                          data-share-id={s.share_id}
                          tabIndex={0}
                          onClick={() => setSelected(s.share_id)}
                          onKeyDown={(e) => { if (e.key === 'Enter') setSelected(s.share_id) }}
                        >
                          <td className="block px-3 pt-2 md:table-cell md:py-2">
                            <span className="flex min-w-0 items-center gap-2">
                              <KindIcon share={s} />
                              <span className="min-w-0">
                                <span className="block truncate font-medium">{s.title}{s.hidden ? ' ·' : ''}</span>
                                <span className="block truncate font-mono text-[11px] text-[var(--cp-muted)]">{displaySourcePath(s.source_path)}</span>
                              </span>
                            </span>
                          </td>
                          <td className="inline-block px-3 text-xs md:table-cell md:py-2">{s.kind === 'link' ? t('myContent.kind.link') : t('myContent.kind.app_delivery', '', { app: appName(s.target_app) })}</td>
                          <td className="inline-block px-3 text-xs md:table-cell md:py-2">{accessLabel(t, s)}</td>
                          <td className="inline-block px-3 md:table-cell md:py-2"><StatusChip status={s.status} /></td>
                          <td className="inline-block px-3 text-xs tabular-nums md:table-cell md:py-2 md:text-right">{s.kind === 'link' ? s.stats.views : '—'}</td>
                          <td className="inline-block px-3 pb-2 text-xs text-[var(--cp-muted)] md:table-cell md:py-2">{formatTime(s.updated_at, locale)}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
              {cursor && (
                <div className="flex justify-center">
                  <button type="button" className="min-h-9 rounded-lg border border-[var(--cp-border)] px-4 text-sm" data-testid="mc-load-more" onClick={() => void loadMore()}>{t('myContent.loadMore')}</button>
                </div>
              )}
            </section>
          )}
        </div>
      </div>
      {selected && service && (
        <div className="mc-detail-layer" onClick={(e) => { if (e.target === e.currentTarget) setSelected(null) }}>
          <ShareDetail key={selected} service={service} shareId={selected} onClose={() => setSelected(null)} onChanged={changed} onSelect={setSelected} />
        </div>
      )}
    </div>
  )
}
