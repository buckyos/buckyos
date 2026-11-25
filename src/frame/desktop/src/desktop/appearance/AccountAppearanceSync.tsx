import { useEffect, useMemo, useRef, useSyncExternalStore } from 'react'
import { globalSettingsStore } from '../../app/settings/data/store'
import { useI18n } from '../../i18n/provider'
import { useDesktopUIStore } from '../../models/DesktopUIDataModel'
import { useThemeMode } from '../../theme/provider'
import { useWallpaperSnapshot, wallpaperStore } from '../wallpaper/store'
import type { AccountAppearance } from './model'
import { AppearanceSync, desktopStateIo } from './sync'

/**
 * Makes the appearance settings follow the signed-in account: shows the account's on start and when the page
 * comes back to the foreground, and saves what is changed here. Mounted by the desktop shell, keyed by user.
 */
export function AccountAppearanceSync({ userId }: { userId: string }) {
  const { themeMode, setThemeMode } = useThemeMode()
  const { preferredLocale, setLocale } = useI18n()
  const desktopStore = useDesktopUIStore()
  const windowAppearance = useSyncExternalStore(
    desktopStore.subscribe,
    () => desktopStore.getSnapshot().syncData.appearance.windowAppearance,
  )
  const iconSize = useSyncExternalStore(
    globalSettingsStore.subscribe,
    () => globalSettingsStore.getSnapshot().desktopIconSize,
  )
  const accent = useSyncExternalStore(globalSettingsStore.subscribe, () => globalSettingsStore.getSnapshot().accent)
  const mobileFeed = useSyncExternalStore(globalSettingsStore.subscribe, () => globalSettingsStore.getSnapshot().mobileFeed)
  const wallpaper = useWallpaperSnapshot().account

  const local = useMemo<AccountAppearance>(
    () => ({ theme: themeMode, locale: preferredLocale, iconSize, window: windowAppearance, wallpaper, accent, mobileFeed }),
    [themeMode, preferredLocale, iconSize, windowAppearance, wallpaper, accent, mobileFeed],
  )

  const localRef = useRef(local)
  const setters = useRef({ setThemeMode, setLocale })
  const engine = useRef<AppearanceSync | null>(null)

  useEffect(() => {
    setters.current = { setThemeMode, setLocale }
  }, [setThemeMode, setLocale])

  useEffect(() => {
    localRef.current = local
    engine.current?.update(local)
  }, [local])

  useEffect(() => {
    const sync = new AppearanceSync(userId, localRef.current, desktopStateIo, (patch) => {
      if (patch.theme) setters.current.setThemeMode(patch.theme)
      if (patch.locale) setters.current.setLocale(patch.locale)
      if (patch.iconSize) globalSettingsStore.setDesktopIconSize(patch.iconSize)
      if (patch.window) desktopStore.setWindowAppearance(patch.window)
      if (patch.wallpaper) wallpaperStore.applyAccount(patch.wallpaper)
      if (patch.accent !== undefined) globalSettingsStore.setAccent(patch.accent)
      if (patch.mobileFeed) globalSettingsStore.setMobileFeed(patch.mobileFeed)
    })
    engine.current = sync
    void sync.refresh(true)

    const onVisibility = () => {
      if (document.visibilityState === 'visible') void sync.refresh()
      else void sync.flush()
    }
    const onFocus = () => void sync.refresh()
    document.addEventListener('visibilitychange', onVisibility)
    window.addEventListener('focus', onFocus)
    return () => {
      document.removeEventListener('visibilitychange', onVisibility)
      window.removeEventListener('focus', onFocus)
      void sync.flush()
      sync.stop()
      engine.current = null
    }
  }, [userId, desktopStore])

  return null
}
