import { Button } from '@mui/material'
import { RefreshCw } from 'lucide-react'
import { useMemo, useState } from 'react'
import type { bucket } from 'buckyos'
import { useSudoByPassword } from '../../../components/sudo'
import { useI18n } from '../../../i18n/provider'
import { isMockRuntime } from '../../../runtime'
import { useSettingsResource, useSettingsStore } from '../hooks/use-settings-store'
import { ListCard, Section, SectionNote, StatusBadge } from '../components/shared/Section'
import { formatBytes, formatDateTime } from '../components/shared/format'

type Translate = ReturnType<typeof useI18n>['t']

const POLICY_LABELS: Record<bucket.DataPolicy, [string, string]> = {
  documents: ['settings.storage.policy.documents', 'Documents'],
  app_data: ['settings.storage.policy.appData', 'App data'],
  collected: ['settings.storage.policy.collected', 'Collected'],
  ambient: ['settings.storage.policy.ambient', 'Related content'],
  temp: ['settings.storage.policy.temp', 'Temporary'],
  cache: ['settings.storage.policy.cache', 'Cache'],
}

function policyLabel(t: Translate, policy: bucket.DataPolicy) {
  const [key, fallback] = POLICY_LABELS[policy] ?? [policy, policy]
  return t(key, fallback)
}

/** Documents and collections are the user's: deleting them takes a sudo confirmation. */
const userOwned = (policy: bucket.DataPolicy) => policy === 'documents' || policy === 'collected'

interface AppStorage {
  appid: string
  buckets: bucket.BucketInfo[]
  bytes: number
  /** Every bucket is an orphan: the app is no longer installed for this user. */
  uninstalled: boolean
}

/** Buckets are created when first used; an empty one of an installed app says nothing. */
function byApp(buckets: readonly bucket.BucketInfo[]): AppStorage[] {
  const apps = new Map<string, bucket.BucketInfo[]>()
  for (const item of buckets) {
    if (item.key_count === 0 && item.orphaned_at == null) continue
    apps.set(item.appid, [...(apps.get(item.appid) ?? []), item])
  }
  return [...apps.entries()]
    .map(([appid, items]) => ({
      appid,
      buckets: [...items].sort((a, b) => b.unique_bytes - a.unique_bytes || a.name.localeCompare(b.name)),
      bytes: items.reduce((sum, item) => sum + item.unique_bytes, 0),
      uninstalled: items.every((item) => item.orphaned_at != null),
    }))
    .sort((a, b) => b.bytes - a.bytes || a.appid.localeCompare(b.appid))
}

/** What each app keeps for the signed-in user (bucket `unique_bytes`), with the leftovers of uninstalled apps. */
export function StoragePage() {
  const { t, locale } = useI18n()
  const storage = useSettingsResource('storage')
  const store = useSettingsStore()
  const requestSudo = useSudoByPassword()
  const [confirming, setConfirming] = useState<string | null>(null)
  const [deleting, setDeleting] = useState<string | null>(null)
  const [failure, setFailure] = useState<string | null>(null)
  const apps = useMemo(() => byApp(storage.data?.buckets ?? []), [storage.data])
  const total = apps.reduce((sum, app) => sum + app.bytes, 0)

  const remove = async (item: bucket.BucketInfo) => {
    setFailure(null)
    let sudoToken: string | null = null
    if (userOwned(item.data_policy) && !isMockRuntime()) {
      const grant = await requestSudo({
        aud: 'nfs-server',
        title: t('settings.storage.sudoTitle', 'Delete documents for good'),
        description: t('settings.storage.sudoDescription', 'Confirm your password to delete {{name}} of {{app}}. This cannot be undone.', { name: item.name, app: item.appid }),
        reason: t('settings.storage.sudoReason', 'Delete the documents an uninstalled app left behind.'),
        confirmLabel: t('settings.storage.delete', 'Delete'),
      }).catch(() => null)
      if (!grant) return
      sudoToken = grant.sessionToken
    }
    setDeleting(item.bucket_id)
    try {
      await store.deleteBucket(item.bucket_id, sudoToken)
      setConfirming(null)
    } catch (error) {
      setFailure(error instanceof Error && error.message ? error.message : t('settings.storage.deleteFailed', 'The data could not be deleted.'))
    } finally {
      setDeleting(null)
    }
  }

  return (
    <div className="space-y-4" data-testid="settings-storage-page">
      <Section
        title={t('settings.storage.title', 'Storage by app')}
        description={t('settings.storage.description', 'What each app keeps for you. When an app is uninstalled, your documents and collections stay here until you delete them.')}
        actions={<Button size="small" startIcon={<RefreshCw size={14} />} onClick={() => void store.load('storage', true)}>{t('settings.storage.refresh', 'Refresh')}</Button>}
        testId="settings-storage"
      >
        {storage.status === 'error' && !storage.data && <SectionNote tone="danger">{t('settings.storage.unavailable', 'Storage usage could not be loaded.')}</SectionNote>}
        {!storage.data && storage.status !== 'error' && <SectionNote>{t('settings.common.loading', 'Loading…')}</SectionNote>}
        {storage.data && apps.length === 0 && <SectionNote>{t('settings.storage.empty', 'No app keeps data for you yet.')}</SectionNote>}
        {apps.length > 0 && (
          <p className="mb-3 text-sm" style={{ color: 'var(--cp-muted)' }} data-testid="settings-storage-total">
            {t('settings.storage.total', '{{size}} kept by {{count}} apps', { size: formatBytes(total), count: String(apps.length) })}
          </p>
        )}
        {failure && <SectionNote tone="danger">{failure}</SectionNote>}
        <div className="space-y-2">
          {apps.map((app) => (
            <ListCard key={app.appid} testId={`settings-storage-app-${app.appid}`}>
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div className="flex min-w-0 flex-wrap items-center gap-2">
                  <span className="break-all font-mono text-sm" style={{ color: 'var(--cp-text)' }}>{app.appid}</span>
                  {app.uninstalled && <StatusBadge status="warn" label={t('settings.storage.uninstalled', 'Uninstalled')} />}
                </div>
                <span className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{formatBytes(app.bytes)}</span>
              </div>
              <div className="mt-2 space-y-1.5">
                {app.buckets.map((item) => (
                  <div key={item.bucket_id} className="flex flex-wrap items-center justify-between gap-2 text-sm" data-testid={`settings-storage-bucket-${item.bucket_id}`}>
                    <div className="flex min-w-0 flex-wrap items-center gap-1.5">
                      <span style={{ color: 'var(--cp-text)' }}>{item.name}</span>
                      <StatusBadge status={userOwned(item.data_policy) ? 'info' : 'neutral'} label={policyLabel(t, item.data_policy)} />
                      {item.orphaned_at != null && !app.uninstalled && <StatusBadge status="warn" label={t('settings.storage.undeclared', 'No longer used by the app')} />}
                      <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>
                        {t('settings.storage.items', '{{count}} items', { count: String(item.key_count) })}
                        {item.orphaned_at != null && ` · ${t('settings.storage.orphanedAt', 'left {{when}}', { when: formatDateTime(Math.floor(item.orphaned_at / 1000), locale) })}`}
                      </span>
                    </div>
                    <div className="flex shrink-0 items-center gap-1">
                      <span style={{ color: 'var(--cp-muted)' }}>{formatBytes(item.unique_bytes)}</span>
                      {item.orphaned_at != null && confirming !== item.bucket_id && (
                        <Button size="small" color="error" onClick={() => setConfirming(item.bucket_id)} data-testid={`settings-storage-delete-${item.bucket_id}`}>
                          {t('settings.storage.delete', 'Delete')}
                        </Button>
                      )}
                    </div>
                    {confirming === item.bucket_id && (
                      <div className="flex w-full flex-wrap items-center justify-end gap-1" role="group">
                        <span className="mr-auto text-xs" style={{ color: 'var(--cp-danger)' }}>
                          {t('settings.storage.confirm', 'Delete everything in {{name}}? This cannot be undone.', { name: item.name })}
                        </span>
                        <Button size="small" onClick={() => setConfirming(null)} disabled={deleting === item.bucket_id}>
                          {t('common.cancel', 'Cancel')}
                        </Button>
                        <Button size="small" color="error" variant="contained" disabled={deleting === item.bucket_id} onClick={() => void remove(item)} data-testid={`settings-storage-confirm-${item.bucket_id}`}>
                          {t('settings.storage.deleteForGood', 'Delete for good')}
                        </Button>
                      </div>
                    )}
                  </div>
                ))}
              </div>
            </ListCard>
          ))}
        </div>
      </Section>
    </div>
  )
}
