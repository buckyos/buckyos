import { Monitor, Smartphone, Globe, Copy as CopyIcon } from 'lucide-react'
import { Button, TextField } from '@mui/material'
import { useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import { useSettingsSnapshot, useSettingsStore } from '../hooks/use-settings-store'
import { Section, InfoRow } from '../components/shared/Section'
import { SettingsPageIntro } from '../components/shared/SettingsPageIntro'
import { localeLabels } from '../../../mock/data'
import type { AppContentLoaderProps } from '../../types'
import type { FontSize } from '../mock/types'

const fontSizeOptions: { value: FontSize; label: string }[] = [
  { value: 'small', label: 'Small' },
  { value: 'medium', label: 'Medium' },
  { value: 'large', label: 'Large' },
]

const wallpaperOptions = [
  { value: 'wallpaper_01', label: 'Default Blue' },
  { value: 'wallpaper_02', label: 'Mountain' },
  { value: 'wallpaper_03', label: 'Ocean' },
  { value: 'wallpaper_04', label: 'Forest' },
  { value: 'wallpaper_05', label: 'Abstract' },
]

interface AppearancePageProps {
  appProps: AppContentLoaderProps
}

export function AppearancePage({ appProps }: AppearancePageProps) {
  const { t } = useI18n()
  const { session } = useSettingsSnapshot()
  const store = useSettingsStore()
  const { appearance } = session
  const [renaming, setRenaming] = useState(false)
  const [newName, setNewName] = useState(session.session.name)

  const environmentIcon = {
    desktop: Monitor,
    mobile: Smartphone,
    browser: Globe,
  }[session.session.environment] ?? Monitor

  const EnvironmentIcon = environmentIcon

  const handleRename = () => {
    if (renaming && newName.trim()) {
      store.renameSession(newName.trim())
    }
    setRenaming(!renaming)
  }

  const handleClone = () => {
    store.cloneToDeviceSession()
  }

  return (
    <div className="space-y-4">
      <SettingsPageIntro
        page="appearance"
        title={t('settings.appearance.title', 'Appearance')}
        description={t(
          'settings.appearance.description',
          'Customize the look and feel for your current session.',
        )}
      />

      <Section title={t('settings.appearance.currentSession', 'Current Session')}>
        <div className="space-y-0.5">
          <InfoRow label={t('settings.appearance.sessionName', 'Session Name')} value={session.session.name} />
          <InfoRow
            label={t('settings.appearance.sessionType', 'Session Type')}
            value={
              <span
                className="px-2 py-0.5 rounded-full text-xs font-medium"
                style={{
                  color: session.session.type === 'shared' ? 'var(--cp-accent)' : 'var(--cp-success)',
                  background: session.session.type === 'shared'
                    ? 'color-mix(in srgb, var(--cp-accent) 14%, transparent)'
                    : 'color-mix(in srgb, var(--cp-success) 14%, transparent)',
                }}
              >
                {session.session.type === 'shared' ? 'Shared' : 'Device'}
              </span>
            }
          />
          <InfoRow
            label={t('settings.appearance.environment', 'Environment')}
            value={
              <span className="inline-flex items-center gap-1.5">
                <EnvironmentIcon size={14} />
                {session.session.environment.charAt(0).toUpperCase() + session.session.environment.slice(1)}
              </span>
            }
          />
        </div>
      </Section>

      {/* Session Management */}
      <Section
        title={t('settings.appearance.sessionManagement', 'Session Management')}
      >
        <div className="space-y-3">
          <div className="flex items-center gap-2">
            {renaming ? (
              <TextField
                size="small"
                value={newName}
                onChange={(e) => setNewName(e.target.value)}
                className="flex-1"
              />
            ) : (
              <span className="flex-1 text-sm" style={{ color: 'var(--cp-text)' }}>
                {session.session.name}
              </span>
            )}
            <Button size="small" variant="outlined" onClick={handleRename}>
              {renaming
                ? t('common.save', 'Save')
                : t('settings.appearance.rename', 'Rename')}
            </Button>
          </div>

          {session.session.type === 'shared' && (
            <div
              className="rounded-lg p-3"
              style={{ background: 'color-mix(in srgb, var(--cp-surface) 82%, transparent)' }}
            >
              <p className="text-xs mb-2" style={{ color: 'var(--cp-muted)' }}>
                {t('settings.appearance.cloneHint', 'Clone this session to create a device-specific configuration that won\'t sync with other devices.')}
              </p>
              <Button size="small" variant="outlined" onClick={handleClone}>
                <CopyIcon size={14} className="mr-1.5" />
                {t('settings.appearance.cloneToDevice', 'Clone to Device Session')}
              </Button>
            </div>
          )}
        </div>
      </Section>

      {/* Theme & Display */}
      <Section title={t('settings.appearance.themeDisplay', 'Theme & Display')}>
        <div className="grid gap-3 sm:grid-cols-2">
          <TextField
            select
            size="small"
            label={t('common.theme', 'Theme')}
            value={appearance.theme}
            onChange={(e) => {
              store.setTheme(e.target.value as 'light' | 'dark')
              appProps.onSaveSettings({
                locale: appearance.language as 'en',
                theme: e.target.value as 'light' | 'dark',
                runtimeContainer: appProps.runtimeContainer as 'browser',
                deadZoneTop: appProps.layoutState.deadZone.top,
                deadZoneBottom: appProps.layoutState.deadZone.bottom,
                deadZoneLeft: appProps.layoutState.deadZone.left,
                deadZoneRight: appProps.layoutState.deadZone.right,
                titleBarOpacity: appProps.windowAppearance.titleBarOpacity,
                backgroundOpacity: appProps.windowAppearance.backgroundOpacity,
              })
            }}
            SelectProps={{ native: true }}
          >
            <option value="light">{t('common.light', 'Light')}</option>
            <option value="dark">{t('common.dark', 'Dark')}</option>
          </TextField>

          <TextField
            select
            size="small"
            label={t('common.language', 'Language')}
            value={appearance.language}
            onChange={(e) => {
              store.setLanguage(e.target.value)
              appProps.onSaveSettings({
                locale: e.target.value as 'en',
                theme: appearance.theme,
                runtimeContainer: appProps.runtimeContainer as 'browser',
                deadZoneTop: appProps.layoutState.deadZone.top,
                deadZoneBottom: appProps.layoutState.deadZone.bottom,
                deadZoneLeft: appProps.layoutState.deadZone.left,
                deadZoneRight: appProps.layoutState.deadZone.right,
                titleBarOpacity: appProps.windowAppearance.titleBarOpacity,
                backgroundOpacity: appProps.windowAppearance.backgroundOpacity,
              })
            }}
            SelectProps={{ native: true }}
          >
            {Object.entries(localeLabels).map(([value, label]) => (
              <option key={value} value={value}>
                {label}
              </option>
            ))}
          </TextField>

          <TextField
            select
            size="small"
            label={t('settings.appearance.fontSize', 'Font Size')}
            value={appearance.fontSize}
            onChange={(e) => store.setFontSize(e.target.value as FontSize)}
            SelectProps={{ native: true }}
          >
            {fontSizeOptions.map((opt) => (
              <option key={opt.value} value={opt.value}>
                {t(`settings.appearance.fontSize.${opt.value}`, opt.label)}
              </option>
            ))}
          </TextField>

          <TextField
            select
            size="small"
            label={t('settings.appearance.wallpaper', 'Wallpaper')}
            value={appearance.wallpaper}
            onChange={(e) => store.setWallpaper(e.target.value)}
            SelectProps={{ native: true }}
          >
            {wallpaperOptions.map((opt) => (
              <option key={opt.value} value={opt.value}>
                {opt.label}
              </option>
            ))}
          </TextField>
        </div>
      </Section>

      {/* Desktop Layout (read-only info) */}
      <Section
        title={t('settings.appearance.desktopLayout', 'Desktop Layout')}
        description={t('settings.appearance.desktopLayoutDesc', 'Window positions and layout state are managed automatically by the system.')}
      >
        <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>
          {t('settings.appearance.desktopLayoutReadonly', 'Desktop layout settings are saved as part of your session and restored when you reconnect.')}
        </p>
      </Section>
    </div>
  )
}
