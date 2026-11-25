import { useEffect, useSyncExternalStore } from 'react'
import { isZoneReachable, subscribeZoneReachable, watchZoneLink } from './zone_link'

export function useZoneReachable(): boolean {
  useEffect(watchZoneLink, [])
  return useSyncExternalStore(subscribeZoneReachable, isZoneReachable)
}
