import { Button, Dialog, DialogActions, DialogContent, DialogTitle, IconButton } from '@mui/material'
import { Info, RefreshCw } from 'lucide-react'
import { useEffect, useState } from 'react'
import useSWR from 'swr'
import { fetchCurrentAccount } from '../../api/account'
import { fetchBuckyOSInfo, fetchZonePresentation } from '../../api/settings'
import { parseUserType } from '../../api/user_type'
import { onOwnProfileChanged } from '../../app/agent-setup/events'
import { CopyButton } from '../../app/settings/components/shared/CopyButton'
import { useI18n } from '../../i18n/provider'
import { useDesktopUIStore } from '../../models/DesktopUIDataModel'

const options = { refreshInterval: 60_000, focusThrottleInterval: 5_000, shouldRetryOnError: false }

async function required<T>(request: Promise<{ data: T | null; error: unknown }>): Promise<T> {
  const { data, error } = await request
  if (error || !data) throw new Error('unavailable')
  return data
}

export function SystemInfoWidget() {
  const { t } = useI18n()
  const desktop = useDesktopUIStore()
  const [details, setDetails] = useState(false)
  const account = useSWR('desktop-system-info-account', fetchCurrentAccount, options)
  const current = account.error ? null : account.data
  const userId = current?.user_id
  const software = useSWR(userId ? ['desktop-system-info-software', userId] : null, () => required(fetchBuckyOSInfo()), options)
  const zone = useSWR(userId ? ['desktop-system-info-zone', userId] : null, () => required(fetchZonePresentation()), options)
  const { mutate: refreshAccount } = account
  const { mutate: refreshSoftware } = software
  const { mutate: refreshZone } = zone

  useEffect(() => {
    const refresh = () => {
      if (document.visibilityState !== 'visible') return
      void refreshAccount()
      void refreshSoftware()
      void refreshZone()
    }
    document.addEventListener('visibilitychange', refresh)
    const stop = onOwnProfileChanged(refresh)
    return () => { document.removeEventListener('visibilitychange', refresh); stop() }
  }, [refreshAccount, refreshSoftware, refreshZone])

  const info = userId && !software.error ? software.data : null
  const location = userId && !zone.error ? zone.data : null
  const name = location?.settings.display_name?.trim() || location?.document.display_name?.trim() || location?.hostname || location?.zone_did
  const loading = t('systemInfo.loading', 'Loading…')
  const unavailable = t('systemInfo.unavailable', 'Unavailable')
  const softwareMissing = software.isLoading || account.isLoading ? loading : unavailable
  const zoneMissing = zone.isLoading || account.isLoading ? loading : unavailable
  const accountMissing = account.isLoading ? loading : account.error ? unavailable : t('systemInfo.signedOut', 'Not signed in')
  const type = parseUserType(current?.user_type)
  const userType = type ? t(`systemInfo.type.${type}`, type) : userId ? t('systemInfo.unknownType', 'Unknown') : accountMissing
  const refreshing = account.isValidating || software.isValidating || zone.isValidating
  const failed = Boolean(account.error || software.error || zone.error)

  const summary = () => JSON.stringify({
    buckyos: info ? { version: info.version, build: info.build_version ?? null, channel: info.release_channel, target: info.target } : null,
    account: current ? { user_id: current.user_id, user_type: current.user_type } : null,
    zone: location ? { name, hostname: location.hostname ?? null, did: location.zone_did } : null,
    status: { account: account.error ? 'unavailable' : current ? 'ready' : account.isLoading ? 'loading' : 'signed_out', software: info ? 'ready' : software.isLoading ? 'loading' : 'unavailable', zone: location ? 'ready' : zone.isLoading ? 'loading' : 'unavailable' },
    captured_at: new Date().toISOString(),
  }, null, 2)

  const rows = [
    [t('systemInfo.version', 'BuckyOS'), info?.version || softwareMissing],
    [t('systemInfo.build', 'Build'), info ? info.build_version?.trim() || '—' : softwareMissing],
    [t('systemInfo.user', 'User ID'), userId || accountMissing],
    [t('systemInfo.accountType', 'Account'), userType],
    [t('systemInfo.zone', 'Zone'), name || zoneMissing],
    ...(location?.hostname && location.hostname !== name ? [[t('systemInfo.hostname', 'Domain'), location.hostname]] : []),
  ]

  return (
    <section
      className="flex h-full min-h-0 flex-col gap-2 rounded-[22px] bg-[var(--cp-surface-2)] p-3 text-[color:var(--cp-text)]"
      aria-label={t('widgets.systemInfo', 'System Info')}
      data-testid="system-info-widget"
    >
      <header className="flex shrink-0 items-center gap-1.5">
        <Info size={15} className="shrink-0 text-[color:var(--cp-muted)]" />
        <h2 className="min-w-0 flex-1 truncate text-sm font-semibold">{t('widgets.systemInfo', 'System Info')}</h2>
        <IconButton
          size="small"
          className="widget-interactive"
          aria-label={t('systemInfo.refresh', 'Refresh system info')}
          disabled={refreshing}
          onClick={() => { void refreshAccount(); void refreshSoftware(); void refreshZone() }}
          data-testid="system-info-refresh"
        >
          <RefreshCw size={14} className={refreshing ? 'animate-spin' : ''} />
        </IconButton>
      </header>
      <div className="widget-interactive min-h-0 flex-1 overflow-auto overscroll-contain" data-testid="system-info-fields">
        <dl className="space-y-2 text-xs leading-4">
          {rows.map(([label, value]) => (
            <div key={label} className="grid grid-cols-[3.75rem_minmax(0,1fr)] gap-1.5">
              <dt className="text-[10px] text-[color:var(--cp-muted)]">{label}</dt>
              <dd className="m-0 break-words [overflow-wrap:anywhere]">{value}</dd>
            </div>
          ))}
        </dl>
        {failed && <p className="mt-2 text-xs text-[color:var(--cp-warning)]" role="status">{t('systemInfo.partialError', 'Some information could not be loaded. Try refreshing.')}</p>}
      </div>
      <footer className="widget-interactive flex shrink-0 flex-wrap items-center gap-1">
        <CopyButton label={t('systemInfo.copy', 'Copy')} text={summary} testId="system-info-copy" />
        <Button size="small" onClick={() => setDetails(true)} data-testid="system-info-details">{t('systemInfo.details', 'Details')}</Button>
      </footer>
      <Dialog open={details} onClose={() => setDetails(false)} fullWidth maxWidth="xs" aria-labelledby="system-info-title">
        <DialogTitle id="system-info-title">{t('widgets.systemInfo', 'System Info')}</DialogTitle>
        <DialogContent>
          <dl className="space-y-3 text-sm">
            {[...rows, [t('systemInfo.zoneDid', 'Zone DID'), location?.zone_did || zoneMissing], [t('systemInfo.channel', 'Channel'), info?.release_channel || softwareMissing], [t('systemInfo.platform', 'Platform'), info?.target || softwareMissing]].map(([label, value]) => (
              <div key={label}>
                <dt className="text-xs text-[color:var(--cp-muted)]">{label}</dt>
                <dd className="m-0 break-words [overflow-wrap:anywhere]">{value}</dd>
              </div>
            ))}
          </dl>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => { setDetails(false); desktop.openAppWindow('settings') }}>{t('systemInfo.settings', 'Open settings')}</Button>
          <Button onClick={() => setDetails(false)}>{t('common.close', 'Close')}</Button>
        </DialogActions>
      </Dialog>
    </section>
  )
}
