import { FormControlLabel, Switch } from '@mui/material'
import { useEffect, useMemo, useState } from 'react'
import { fetchNotifyApps, saveNotifyApp, type NotifyApp } from '../../../api/notify_apps'
import { useI18n } from '../../../i18n/provider'
import { useDesktopApps } from '../../../models/DesktopUIDataModel'
import { useAppText } from '../../../presentation/appText'
import { didDisplayName, usableImageUrl } from '../../../presentation/model'
import { ObjectAvatar } from '../../../presentation/ObjectIcon'
import { formatDateTime } from '../components/shared/format'
import { ListCard, Section, SectionNote, StatusBadge } from '../components/shared/Section'
import { useSettingsSnapshot, useSettingsStore } from '../hooks/use-settings-store'

/** Which Apps may notify the user (System Notify §4), and what this device's status bar shows. */
export function NotificationsPage() {
  const { t } = useI18n()
  const { messagePreview } = useSettingsSnapshot()
  const store = useSettingsStore()

  return (
    <div className="space-y-4" data-testid="settings-notifications-page">
      <NotifyAppsSection />

      <Section title={t('settings.notifications.statusBar', 'Status bar')}>
        <FormControlLabel
          control={
            <Switch
              checked={messagePreview}
              onChange={(event) => store.setMessagePreview(event.target.checked)}
              data-testid="settings-message-preview"
            />
          }
          label={t('settings.appearance.messagePreview', 'Show message content in the status bar')}
        />
        <p className="mt-1 text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>
          {t('settings.appearance.messagePreviewHint', 'When off, the message list only says who wrote and how many messages are new. Applies to this device.')}
        </p>
      </Section>
    </div>
  )
}

function NotifyAppsSection() {
  const { t, locale } = useI18n()
  const desktopApps = useDesktopApps()
  const appText = useAppText()
  const [apps, setApps] = useState<NotifyApp[] | null>(null)
  const [failed, setFailed] = useState(false)
  const [saveFailed, setSaveFailed] = useState(false)
  const [saving, setSaving] = useState<ReadonlySet<string>>(new Set())

  useEffect(() => {
    let alive = true
    fetchNotifyApps().then(
      (list) => { if (alive) setApps(list) },
      (error: unknown) => {
        console.warn('notify.list_apps failed', error)
        if (alive) setFailed(true)
      },
    )
    return () => { alive = false }
  }, [])

  const installed = useMemo(() => new Map(desktopApps.map((app) => [app.appInstanceId, app])), [desktopApps])

  const toggle = async (app: NotifyApp, enabled: boolean) => {
    const id = app.app_instance_id
    const put = (next: NotifyApp) => setApps((list) => list?.map((item) => (item.app_instance_id === id ? next : item)) ?? null)
    setSaveFailed(false)
    setSaving((ids) => new Set(ids).add(id))
    put({ ...app, enabled })
    try {
      put(await saveNotifyApp(id, enabled))
    } catch (error) {
      console.warn('notify.set_app failed', error)
      put(app)
      setSaveFailed(true)
    } finally {
      setSaving((ids) => new Set([...ids].filter((item) => item !== id)))
    }
  }

  return (
    <Section
      title={t('settings.notifications.apps', 'Apps')}
      description={t('settings.notifications.appsDescription', 'Apps that declared they send notifications. What a turned-off app sends is refused. Your Agents and the system itself are not listed here.')}
      testId="settings-notify-apps"
    >
      {failed ? <SectionNote tone="danger">{t('settings.notifications.unavailable', 'Notification settings could not be loaded.')}</SectionNote> : null}
      {saveFailed ? <SectionNote tone="danger">{t('settings.notifications.saveFailed', 'The change was not saved.')}</SectionNote> : null}
      {!apps && !failed ? <SectionNote>{t('settings.notifications.loading', 'Loading…')}</SectionNote> : null}
      {apps?.length === 0 ? <SectionNote>{t('settings.notifications.empty', 'No app sends notifications.')}</SectionNote> : null}
      <div className="space-y-2">
        {apps?.map((app) => {
          const definition = installed.get(app.app_instance_id)
          const title = definition ? appText(definition).title : app.show_name ?? didDisplayName(app.app_id)
          const details = [
            app.last_sent_at_ms
              ? t('settings.notifications.lastSent', 'Last sent {{when}}', { when: formatDateTime(app.last_sent_at_ms / 1000, locale) })
              : t('settings.notifications.neverSent', 'Nothing sent yet'),
            ...(app.installed && !app.declared ? [t('settings.notifications.notDeclared', 'This version does not declare notifications, so what it sends is refused.')] : []),
          ]
          return (
            <ListCard key={app.app_instance_id} testId={`notify-app-${app.app_instance_id}`}>
              <div className="flex items-center gap-3">
                <ObjectAvatar title={title} icons={[definition?.iconUrl, usableImageUrl(app.app_icon_url)]} size={32} shape="square" decorative />
                <div className="min-w-0 flex-1">
                  <p className="truncate text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{title}</p>
                  <p className="text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>{details.join(' · ')}</p>
                </div>
                {app.installed ? (
                  <Switch
                    checked={app.enabled}
                    disabled={!app.declared || saving.has(app.app_instance_id)}
                    onChange={(event) => void toggle(app, event.target.checked)}
                    slotProps={{ input: { 'aria-label': t('settings.notifications.toggle', 'Notifications from {{app}}', { app: title }) } }}
                  />
                ) : (
                  <StatusBadge status="neutral" label={t('settings.notifications.uninstalled', 'Uninstalled')} />
                )}
              </div>
            </ListCard>
          )
        })}
      </div>
    </Section>
  )
}
