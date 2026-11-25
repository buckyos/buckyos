import { useSyncExternalStore } from 'react'
import { useAgentBrowserStore } from '../store/context'
import { siteControllerOf, type SiteController, type SiteSnapshot } from './controller'

export function useSiteController(): SiteController {
  return siteControllerOf(useAgentBrowserStore())
}

/** The part of the personal site's state a component shows; it re-renders when that part changes. */
export function useSite<T>(select: (snapshot: SiteSnapshot) => T): T {
  const controller = useSiteController()
  return useSyncExternalStore(controller.subscribe, () => select(controller.getSnapshot()))
}
