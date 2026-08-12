import { useEffect, useState } from 'react'
import { CloudDownload, Copy, Settings2, X } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import type { CloudUpdateSettings } from '../../../../api/aicc_mgr'
import { useAICCStore } from '../../hooks/use-aicc-store'

const EMPTY_SETTINGS: CloudUpdateSettings = {
  enabled: false,
  sourceConfigured: false,
  intervalSecs: 3600,
  status: 'disabled',
  consecutiveFailures: 0,
}

const ACTIVE_POLL_INTERVAL_MS = 5_000
const DEGRADED_POLL_INTERVAL_MS = 15_000
const STABLE_POLL_INTERVAL_MS = 60_000

export function CloudUpdateCard() {
  const { t } = useI18n()
  const store = useAICCStore()
  const [settings, setSettings] = useState<CloudUpdateSettings>(EMPTY_SETTINGS)
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [dialogOpen, setDialogOpen] = useState(false)
  const [editingSource, setEditingSource] = useState(false)
  const [sourceCopied, setSourceCopied] = useState(false)
  const [sourceUrl, setSourceUrl] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [refreshError, setRefreshError] = useState<string | null>(null)
  const statusLabel = cloudUpdateStatusLabel(settings.status, t)
  const statusColor = settings.status === 'healthy'
    ? 'var(--cp-success)'
    : settings.status === 'error'
      ? 'var(--cp-danger)'
      : settings.status === 'degraded'
        ? 'var(--cp-warning)'
        : settings.status === 'updating'
          ? 'var(--cp-accent)'
          : 'var(--cp-muted)'

  useEffect(() => {
    if (!dialogOpen) return

    let cancelled = false
    let timer: number | undefined
    let refreshing = false
    let refreshFailures = 0

    const scheduleRefresh = (delay: number): void => {
      if (cancelled || document.visibilityState === 'hidden') return
      timer = window.setTimeout(() => void refresh(), delay)
    }

    const refresh = async (): Promise<void> => {
      if (refreshing || document.visibilityState === 'hidden') return
      refreshing = true
      let nextDelay = ACTIVE_POLL_INTERVAL_MS
      try {
        const value = await store.getCloudUpdateSettings()
        refreshFailures = 0
        nextDelay = cloudUpdatePollInterval(value.status)
        if (!cancelled) {
          setSettings(value)
          setRefreshError(null)
        }
      } catch (cause) {
        refreshFailures += 1
        nextDelay = Math.min(
          STABLE_POLL_INTERVAL_MS,
          ACTIVE_POLL_INTERVAL_MS * (2 ** Math.min(refreshFailures - 1, 4)),
        )
        if (!cancelled) setRefreshError(errorMessage(cause))
      } finally {
        refreshing = false
        if (!cancelled) {
          setLoading(false)
          scheduleRefresh(nextDelay)
        }
      }
    }

    const handleVisibilityChange = () => {
      if (timer !== undefined) {
        window.clearTimeout(timer)
        timer = undefined
      }
      if (document.visibilityState === 'visible') void refresh()
    }

    document.addEventListener('visibilitychange', handleVisibilityChange)
    void refresh()
    return () => {
      cancelled = true
      if (timer !== undefined) window.clearTimeout(timer)
      document.removeEventListener('visibilitychange', handleVisibilityChange)
    }
  }, [dialogOpen, store])

  const openDialog = () => {
    setSourceUrl('')
    setEditingSource(false)
    setSourceCopied(false)
    setError(null)
    setLoading(true)
    setDialogOpen(true)
  }

  const saveEnabled = async () => {
    const nextSource = sourceUrl.trim()
    if ((!settings.sourceConfigured || editingSource) && !nextSource) {
      setError(t('aiCenter.cloudUpdate.sourceRequired', 'Enter a metadata source URL.'))
      return
    }
    if (nextSource && !isValidSourceUrl(nextSource)) {
      setError(t('aiCenter.cloudUpdate.sourceInvalid', 'Use an HTTPS URL ending in /aicc/driver-metadata/index.json.'))
      return
    }
    setSaving(true)
    setError(null)
    try {
      const next = await store.setCloudUpdateSettings({
        enabled: true,
        sourceUrl: nextSource || undefined,
      })
      setSettings(next)
      setDialogOpen(false)
    } catch (cause) {
      setError(errorMessage(cause))
    } finally {
      setSaving(false)
    }
  }

  const disable = async () => {
    setSaving(true)
    setError(null)
    try {
      setSettings(await store.setCloudUpdateSettings({ enabled: false }))
      setDialogOpen(false)
    } catch (cause) {
      setError(errorMessage(cause))
    } finally {
      setSaving(false)
    }
  }

  const copyCurrentSource = async () => {
    if (!settings.sourceUrl) return
    try {
      await navigator.clipboard.writeText(settings.sourceUrl)
      setSourceCopied(true)
    } catch (cause) {
      setError(errorMessage(cause))
    }
  }

  return (
    <>
      <button
        type="button"
        onClick={openDialog}
        className="flex min-h-9 w-full items-center gap-2 rounded-md px-3 text-left text-xs transition-colors hover:bg-black/5 dark:hover:bg-white/5"
        style={{ color: 'var(--cp-muted)' }}
      >
        <Settings2 size={15} />
        {t('aiCenter.cloudUpdate.advancedSettings', 'Advanced settings')}
      </button>

      {dialogOpen && (
        <div className="fixed inset-0 z-50 flex items-end justify-center md:items-center" role="dialog" aria-modal="true" aria-labelledby="cloud-update-title">
          <button type="button" aria-label={t('common.close', 'Close')} className="absolute inset-0" style={{ background: 'rgba(0,0,0,0.4)' }} onClick={() => setDialogOpen(false)} />
          <div className="relative w-full rounded-t-xl p-6 shadow-lg md:mx-4 md:max-w-lg md:rounded-xl" style={{ background: 'var(--cp-surface)' }}>
            <button type="button" aria-label={t('common.close', 'Close')} onClick={() => setDialogOpen(false)} className="absolute right-4 top-4 p-1" style={{ color: 'var(--cp-muted)' }}><X size={18} /></button>
            <div className="flex items-start gap-3 pr-8">
              <div className="mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-lg" style={{ background: 'var(--cp-bg)', color: 'var(--cp-muted)' }}>
                <CloudDownload size={18} />
              </div>
              <div>
                <h3 id="cloud-update-title" className="text-base font-semibold" style={{ color: 'var(--cp-text)' }}>
                  {t('aiCenter.cloudUpdate.settingsTitle', 'Provider metadata settings')}
                </h3>
                <span className="mt-1 inline-block rounded-full px-2 py-0.5 text-xs" style={{ background: 'var(--cp-bg)', color: statusColor }}>
                  {loading ? t('common.loading', 'Loading...') : statusLabel}
                </span>
              </div>
            </div>
            <p className="mt-2 text-sm leading-5" style={{ color: 'var(--cp-muted)' }}>
              {t('aiCenter.cloudUpdate.dialogDesc', 'Only use a publisher you trust. Updates can change model capabilities, pricing estimates, and routing behavior.')}
            </p>
            {(error || refreshError) && <p className="mt-3 text-xs" style={{ color: 'var(--cp-danger)' }}>{error || refreshError}</p>}
            {!error && !refreshError && settings.lastError && (
              <p className="mt-3 text-xs" style={{ color: settings.status === 'error' ? 'var(--cp-danger)' : 'var(--cp-warning)' }}>
                {settings.lastError}
              </p>
            )}
            {settings.activeRevision !== undefined && (
              <p className="mt-2 text-xs" style={{ color: 'var(--cp-muted)' }}>
                {t('aiCenter.cloudUpdate.activeRevision', 'Active revision')}: {settings.activeRevision}
              </p>
            )}
            <div className="mt-5">
              <div className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>
                {t('aiCenter.cloudUpdate.source', 'Metadata source URL')}
              </div>
              {settings.sourceConfigured && settings.sourceUrl ? (
                <>
                  <div className="mt-2 text-[11px] font-medium uppercase tracking-wide" style={{ color: 'var(--cp-muted)' }}>
                    {t('aiCenter.cloudUpdate.currentSource', 'Current source')}
                  </div>
                  <div className="mt-1 flex min-w-0 items-center gap-2">
                    <div
                      className="min-w-0 flex-1 truncate font-mono text-sm"
                      title={settings.sourceUrl}
                      style={{ color: 'var(--cp-text)' }}
                    >
                      {settings.sourceUrl}
                    </div>
                    <button
                      type="button"
                      onClick={() => void copyCurrentSource()}
                      className="flex shrink-0 items-center gap-1 rounded-md px-2 py-1 text-xs"
                      style={{ color: 'var(--cp-muted)' }}
                    >
                      <Copy size={13} aria-hidden="true" />
                      {sourceCopied
                        ? t('aiCenter.cloudUpdate.copied', 'Copied')
                        : t('aiCenter.cloudUpdate.copy', 'Copy')}
                    </button>
                  </div>
                  {!editingSource && (
                    <button
                      type="button"
                      disabled={loading || saving}
                      onClick={() => {
                        setSourceUrl('')
                        setError(null)
                        setEditingSource(true)
                      }}
                      className="mt-3 text-sm font-medium disabled:opacity-60"
                      style={{ color: 'var(--cp-accent)' }}
                    >
                      {t('aiCenter.cloudUpdate.changeSource', 'Change source')}
                    </button>
                  )}
                </>
              ) : (
                <>
                  <p className="mt-2 text-sm" style={{ color: 'var(--cp-muted)' }}>
                    {t('aiCenter.cloudUpdate.notConfigured', 'No source configured')}
                  </p>
                  {!editingSource && (
                    <button
                      type="button"
                      disabled={loading || saving}
                      onClick={() => {
                        setSourceUrl('')
                        setError(null)
                        setEditingSource(true)
                      }}
                      className="mt-3 text-sm font-medium disabled:opacity-60"
                      style={{ color: 'var(--cp-accent)' }}
                    >
                      {t('aiCenter.cloudUpdate.setSource', 'Set source')}
                    </button>
                  )}
                </>
              )}
              {editingSource && (
                <div className="mt-4">
                  <label htmlFor="cloud-update-source-url" className="block text-[11px] font-medium uppercase tracking-wide" style={{ color: 'var(--cp-muted)' }}>
                    {settings.sourceConfigured
                      ? t('aiCenter.cloudUpdate.newSource', 'New source URL')
                      : t('aiCenter.cloudUpdate.source', 'Metadata source URL')}
                  </label>
                  <input
                    id="cloud-update-source-url"
                    type="url"
                    value={sourceUrl}
                    disabled={loading || saving}
                    onChange={(event) => setSourceUrl(event.target.value)}
                    placeholder={settings.sourceConfigured
                      ? t('aiCenter.cloudUpdate.newSourcePlaceholder', 'Enter a new source URL')
                      : t('aiCenter.cloudUpdate.sourcePlaceholder', 'Enter a source URL')}
                    className="mt-2 h-11 w-full rounded-lg px-3 text-sm outline-none"
                    style={{ background: 'var(--cp-bg)', border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}
                  />
                  <p className="mt-2 text-xs" style={{ color: 'var(--cp-muted)' }}>
                    {settings.sourceConfigured
                      ? t('aiCenter.cloudUpdate.replacementHint', 'This will replace the current source after saving.')
                      : t('aiCenter.cloudUpdate.sourceRequiredHint', 'A source URL is required to enable cloud updates.')}
                  </p>
                  <button
                    type="button"
                    disabled={saving}
                    onClick={() => {
                      setSourceUrl('')
                      setError(null)
                      setEditingSource(false)
                    }}
                    className="mt-2 text-xs disabled:opacity-60"
                    style={{ color: 'var(--cp-muted)' }}
                  >
                    {t('aiCenter.cloudUpdate.cancelChange', 'Cancel change')}
                  </button>
                </div>
              )}
            </div>
            <div className="mt-6 flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
              <button type="button" disabled={saving} onClick={() => setDialogOpen(false)} className="min-h-11 rounded-lg px-4 text-sm" style={{ color: 'var(--cp-muted)' }}>
                {t('common.cancel', 'Cancel')}
              </button>
              {settings.enabled && (
                <button type="button" disabled={loading || saving} onClick={() => void disable()} className="min-h-11 rounded-lg px-4 text-sm disabled:opacity-60" style={{ color: 'var(--cp-danger)' }}>
                  {t('aiCenter.cloudUpdate.disable', 'Disable')}
                </button>
              )}
              {(editingSource || (!settings.enabled && settings.sourceConfigured)) && (
                <button
                  type="button"
                  disabled={loading || saving || (editingSource && !isValidSourceUrl(sourceUrl.trim()))}
                  onClick={() => void saveEnabled()}
                  className="min-h-11 rounded-lg px-4 text-sm font-medium disabled:opacity-60"
                  style={{ background: 'var(--cp-accent)', color: '#fff' }}
                >
                  {saving
                    ? t('common.saving', 'Saving...')
                    : settings.enabled
                      ? t('common.save', 'Save')
                      : t('aiCenter.cloudUpdate.enable', 'Enable cloud updates')}
                </button>
              )}
            </div>
          </div>
        </div>
      )}
    </>
  )
}

function isValidSourceUrl(value: string): boolean {
  try {
    const url = new URL(value)
    return url.protocol === 'https:'
      && Boolean(url.hostname)
      && url.pathname === '/aicc/driver-metadata/index.json'
      && !url.search
      && !url.hash
  } catch {
    return false
  }
}

function cloudUpdatePollInterval(status: CloudUpdateSettings['status']): number {
  if (status === 'updating') return ACTIVE_POLL_INTERVAL_MS
  if (status === 'degraded' || status === 'error') return DEGRADED_POLL_INTERVAL_MS
  return STABLE_POLL_INTERVAL_MS
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

function cloudUpdateStatusLabel(
  status: CloudUpdateSettings['status'],
  t: (key: string, fallback?: string) => string,
): string {
  switch (status) {
    case 'healthy': return t('aiCenter.cloudUpdate.statusHealthy', 'Healthy')
    case 'updating': return t('aiCenter.cloudUpdate.statusUpdating', 'Updating')
    case 'degraded': return t('aiCenter.cloudUpdate.statusDegraded', 'Attention needed')
    case 'error': return t('aiCenter.cloudUpdate.statusError', 'Update failed')
    case 'idle': return t('aiCenter.cloudUpdate.statusIdle', 'Waiting for first update')
    default: return t('aiCenter.home.disabled', 'Disabled')
  }
}
