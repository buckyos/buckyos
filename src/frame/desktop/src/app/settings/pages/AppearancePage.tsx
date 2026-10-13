import { FormControlLabel, Switch, Slider, TextField } from '@mui/material'
import { useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import { useSettingsSnapshot, useSettingsStore } from '../hooks/use-settings-store'
import { Section } from '../components/shared/Section'
import { localeLabels } from '../../../mock/data'
import type { AppContentLoaderProps } from '../../types'
import type { GridDensity } from '../../../models/layout'
import type { SupportedLocale, SystemPreferencesInput, ThemeMode } from '../../../models/ui'

const iconSizeOptions: { value: GridDensity; label: string }[] = [
  { value: 'small', label: 'Small' },
  { value: 'medium', label: 'Medium' },
  { value: 'large', label: 'Large' },
]

interface AppearancePageProps {
  appProps: AppContentLoaderProps
}

/** The desktop's current preferences, so one changed field is saved with all the others intact. */
function currentPreferences(appProps: AppContentLoaderProps): SystemPreferencesInput {
  return {
    locale: appProps.locale as SupportedLocale,
    theme: appProps.themeMode === 'dark' ? 'dark' : 'light',
    runtimeContainer: appProps.runtimeContainer as SystemPreferencesInput['runtimeContainer'],
    deadZoneTop: appProps.layoutState.deadZone.top,
    deadZoneBottom: appProps.layoutState.deadZone.bottom,
    deadZoneLeft: appProps.layoutState.deadZone.left,
    deadZoneRight: appProps.layoutState.deadZone.right,
    titleBarOpacity: appProps.windowAppearance.titleBarOpacity,
    backgroundOpacity: appProps.windowAppearance.backgroundOpacity,
  }
}

function OpacitySlider({
  label,
  value,
  onCommit,
  testId,
}: {
  label: string
  value: number
  onCommit: (value: number) => void
  testId: string
}) {
  const [draft, setDraft] = useState<number | null>(null)
  const shown = draft ?? value
  return (
    <div>
      <div className="flex items-center justify-between text-sm">
        <span style={{ color: 'var(--cp-text)' }}>{label}</span>
        <span style={{ color: 'var(--cp-muted)' }}>{shown}%</span>
      </div>
      <Slider
        size="small"
        min={0}
        max={100}
        value={shown}
        onChange={(_, next) => setDraft(next as number)}
        onChangeCommitted={(_, next) => {
          setDraft(null)
          onCommit(next as number)
        }}
        slotProps={{ input: { 'aria-label': label, 'data-testid': testId } as never }}
      />
    </div>
  )
}

export function AppearancePage({ appProps }: AppearancePageProps) {
  const { t } = useI18n()
  const { desktopIconSize, messagePreview } = useSettingsSnapshot()
  const store = useSettingsStore()
  const save = (patch: Partial<SystemPreferencesInput>) => appProps.onSaveSettings({ ...currentPreferences(appProps), ...patch })

  return (
    <div className="space-y-4" data-testid="settings-appearance-page">
      <Section title={t('settings.appearance.themeLanguage', 'Theme & Language')}>
        <div className="grid gap-3 sm:grid-cols-2">
          <TextField
            select
            size="small"
            label={t('common.theme', 'Theme')}
            value={appProps.themeMode === 'dark' ? 'dark' : 'light'}
            onChange={(event) => save({ theme: event.target.value as ThemeMode })}
            SelectProps={{ native: true }}
            inputProps={{ 'data-testid': 'settings-theme' }}
          >
            <option value="light">{t('common.light', 'Light')}</option>
            <option value="dark">{t('common.dark', 'Dark')}</option>
          </TextField>

          <TextField
            select
            size="small"
            label={t('common.language', 'Language')}
            value={appProps.locale}
            onChange={(event) => save({ locale: event.target.value as SupportedLocale })}
            SelectProps={{ native: true }}
            inputProps={{ 'data-testid': 'settings-language' }}
          >
            {Object.entries(localeLabels).map(([value, label]) => (
              <option key={value} value={value}>
                {label}
              </option>
            ))}
          </TextField>
        </div>
      </Section>

      <Section title={t('settings.appearance.desktop', 'Desktop')}>
        <TextField
          select
          size="small"
          className="w-full sm:w-1/2"
          label={t('settings.appearance.iconSize', 'Desktop icon size')}
          value={desktopIconSize}
          onChange={(event) => store.setDesktopIconSize(event.target.value as GridDensity)}
          SelectProps={{ native: true }}
          inputProps={{ 'data-testid': 'settings-icon-size' }}
        >
          {iconSizeOptions.map((option) => (
            <option key={option.value} value={option.value}>
              {t(`settings.appearance.iconSize.${option.value}`, option.label)}
            </option>
          ))}
        </TextField>
      </Section>

      <Section title={t('settings.appearance.notifications', 'Notifications')}>
        <FormControlLabel
          control={
            <Switch
              checked={messagePreview}
              onChange={(event) => store.setMessagePreview(event.target.checked)}
              inputProps={{ 'aria-label': t('settings.appearance.messagePreview', 'Show message content in the status bar') }}
              data-testid="settings-message-preview"
            />
          }
          label={t('settings.appearance.messagePreview', 'Show message content in the status bar')}
        />
        <p className="mt-1 text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>
          {t('settings.appearance.messagePreviewHint', 'When off, the message list only says who wrote and how many messages are new. Applies to this device.')}
        </p>
      </Section>

      <Section title={t('settings.appearance.windows', 'Windows')}>
        <div className="space-y-3 sm:w-1/2">
          <OpacitySlider
            label={t('settings.titleBarOpacity', 'Window title bar opacity')}
            value={appProps.windowAppearance.titleBarOpacity}
            onCommit={(value) => save({ titleBarOpacity: value })}
            testId="settings-titlebar-opacity"
          />
          <OpacitySlider
            label={t('settings.backgroundOpacity', 'Window background opacity')}
            value={appProps.windowAppearance.backgroundOpacity}
            onCommit={(value) => save({ backgroundOpacity: value })}
            testId="settings-background-opacity"
          />
        </div>
      </Section>
    </div>
  )
}
