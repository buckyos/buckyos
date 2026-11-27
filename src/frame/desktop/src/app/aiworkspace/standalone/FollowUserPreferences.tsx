import { useEffect } from 'react'
import { buckyos, type UserPreferences } from 'buckyos'
import { useI18n } from '../../../i18n/provider'
import { matchSupportedLocale } from '../../../models/ui'
import { useThemeMode } from '../../../theme/provider'

/** The account's theme and language as they change (Settings on the Desktop, another device): read again whenever
 * this tab comes back to the foreground. `startup` is what the first paint used, null when it did not wait for it. */
export function FollowUserPreferences({ startup }: { startup: UserPreferences | null }) {
  const { setThemeMode } = useThemeMode()
  const { setLocale } = useI18n()
  useEffect(() => {
    const show = (preferences: UserPreferences) => {
      if (preferences.theme) setThemeMode(preferences.theme)
      const locale = matchSupportedLocale(preferences.locale)
      if (locale) setLocale(locale)
    }
    if (!startup) void buckyos.getUserPreferences().then(show)
    return buckyos.onUserPreferencesChanged(show)
  }, [startup, setThemeMode, setLocale])
  return null
}
