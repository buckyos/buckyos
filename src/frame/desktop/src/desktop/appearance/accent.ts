import { useSyncExternalStore } from 'react'
import { globalSettingsStore } from '../../app/settings/data/store'

/** The Zone's colour as apps get it (`frame.init` `theme.accent`); undefined leaves them their own. */
export function zoneAccent(): string | undefined {
  return globalSettingsStore.getSnapshot().accent ?? undefined
}

export function useZoneAccent(): string | undefined {
  return useSyncExternalStore(globalSettingsStore.subscribe, zoneAccent)
}
