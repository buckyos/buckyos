import { useEffect, useState } from 'react'
import { CloudDownload, X } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import type { CloudUpdateSettings } from '../../../../api/aicc_mgr'
import { useAICCStore } from '../../hooks/use-aicc-store'

const EMPTY_SETTINGS: CloudUpdateSettings = {
  enabled: false,
  sourceConfigured: false,
  intervalSecs: 3600,
}

export function CloudUpdateCard() {
  const { t } = useI18n()
  const store = useAICCStore()
  const [settings, setSettings] = useState<CloudUpdateSettings>(EMPTY_SETTINGS)
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [dialogOpen, setDialogOpen] = useState(false)
  const [sourceUrl, setSourceUrl] = useState('')
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    void store.getCloudUpdateSettings()
      .then((value) => {
        if (!cancelled) setSettings(value)
      })
      .catch((cause) => {
        if (!cancelled) setError(errorMessage(cause))
      })
      .finally(() => {
        if (!cancelled) setLoading(false)
      })
    return () => { cancelled = true }
  }, [store])

  const openDialog = () => {
    setSourceUrl(settings.sourceUrl?.includes('***') ? '' : settings.sourceUrl ?? '')
    setError(null)
    setDialogOpen(true)
  }

  const saveEnabled = async () => {
    const nextSource = sourceUrl.trim()
    if (!settings.sourceConfigured && !nextSource) {
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
    } catch (cause) {
      setError(errorMessage(cause))
    } finally {
      setSaving(false)
    }
  }

  return (
    <>
      <section
        className="mb-5 flex flex-col gap-4 rounded-xl p-4 sm:flex-row sm:items-center sm:justify-between"
        style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
      >
        <div className="flex min-w-0 items-start gap-3">
          <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg" style={{ background: 'color-mix(in oklch, var(--cp-accent), transparent 88%)', color: 'var(--cp-accent)' }}>
            <CloudDownload size={20} />
          </div>
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <h2 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
                {t('aiCenter.cloudUpdate.title', 'Cloud metadata updates')}
              </h2>
              <span
                className="rounded-full px-2 py-0.5 text-xs"
                style={{
                  background: settings.enabled ? 'color-mix(in oklch, var(--cp-success), transparent 86%)' : 'var(--cp-bg)',
                  color: settings.enabled ? 'var(--cp-success)' : 'var(--cp-muted)',
                }}
              >
                {loading
                  ? t('common.loading', 'Loading...')
                  : settings.enabled
                    ? t('aiCenter.home.enabled', 'Enabled')
                    : t('aiCenter.home.disabled', 'Disabled')}
              </span>
            </div>
            <p className="mt-1 text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>
              {t('aiCenter.cloudUpdate.desc', 'Keep provider model capabilities, pricing, and routing metadata current from a trusted publisher.')}
            </p>
            {error && !dialogOpen && <p className="mt-1 text-xs" style={{ color: 'var(--cp-danger)' }}>{error}</p>}
          </div>
        </div>
        <div className="flex shrink-0 gap-2">
          {settings.enabled && (
            <button type="button" disabled={saving} onClick={() => void disable()} className="min-h-10 rounded-lg px-3 text-sm disabled:opacity-60" style={{ color: 'var(--cp-muted)', border: '1px solid var(--cp-border)' }}>
              {t('aiCenter.cloudUpdate.disable', 'Disable')}
            </button>
          )}
          <button type="button" disabled={loading || saving} onClick={openDialog} className="min-h-10 rounded-lg px-4 text-sm font-medium disabled:opacity-60" style={{ background: 'var(--cp-accent)', color: '#fff' }}>
            {settings.enabled
              ? t('aiCenter.cloudUpdate.configure', 'Configure')
              : t('aiCenter.cloudUpdate.enable', 'Enable cloud updates')}
          </button>
        </div>
      </section>

      {dialogOpen && (
        <div className="fixed inset-0 z-50 flex items-end justify-center md:items-center" role="dialog" aria-modal="true" aria-labelledby="cloud-update-title">
          <button type="button" aria-label={t('common.close', 'Close')} className="absolute inset-0" style={{ background: 'rgba(0,0,0,0.4)' }} onClick={() => setDialogOpen(false)} />
          <div className="relative w-full rounded-t-xl p-6 shadow-lg md:mx-4 md:max-w-lg md:rounded-xl" style={{ background: 'var(--cp-surface)' }}>
            <button type="button" aria-label={t('common.close', 'Close')} onClick={() => setDialogOpen(false)} className="absolute right-4 top-4 p-1" style={{ color: 'var(--cp-muted)' }}><X size={18} /></button>
            <h3 id="cloud-update-title" className="pr-8 text-base font-semibold" style={{ color: 'var(--cp-text)' }}>
              {t('aiCenter.cloudUpdate.dialogTitle', 'Enable cloud metadata updates')}
            </h3>
            <p className="mt-2 text-sm leading-5" style={{ color: 'var(--cp-muted)' }}>
              {t('aiCenter.cloudUpdate.dialogDesc', 'Only use a publisher you trust. Updates can change model capabilities, pricing estimates, and routing behavior.')}
            </p>
            <label className="mt-5 block text-sm" style={{ color: 'var(--cp-text)' }}>
              {t('aiCenter.cloudUpdate.source', 'Metadata source URL')}
              <input
                type="url"
                value={sourceUrl}
                onChange={(event) => setSourceUrl(event.target.value)}
                placeholder="https://publisher.example/aicc/driver-metadata/index.json"
                className="mt-2 h-11 w-full rounded-lg px-3 text-sm outline-none"
                style={{ background: 'var(--cp-bg)', border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}
              />
            </label>
            {settings.sourceConfigured && (
              <p className="mt-2 text-xs" style={{ color: 'var(--cp-muted)' }}>
                {t('aiCenter.cloudUpdate.keepSource', 'Leave blank to keep the configured source.')}
              </p>
            )}
            {error && <p className="mt-3 text-xs" style={{ color: 'var(--cp-danger)' }}>{error}</p>}
            <div className="mt-6 flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
              <button type="button" disabled={saving} onClick={() => setDialogOpen(false)} className="min-h-11 rounded-lg px-4 text-sm" style={{ color: 'var(--cp-muted)' }}>
                {t('common.cancel', 'Cancel')}
              </button>
              <button type="button" disabled={saving} onClick={() => void saveEnabled()} className="min-h-11 rounded-lg px-4 text-sm font-medium disabled:opacity-60" style={{ background: 'var(--cp-accent)', color: '#fff' }}>
                {saving ? t('common.saving', 'Saving...') : t('aiCenter.cloudUpdate.enable', 'Enable cloud updates')}
              </button>
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

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
