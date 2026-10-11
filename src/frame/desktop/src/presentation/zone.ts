/* ── The current Zone's presentation (name, description, icon), readable while signed out ── */

import { buckyos } from 'buckyos'
import { fetchZonePresentation } from '../api/settings'
import { isMockRuntime } from '../runtime'
import { presentZone, ZONE_PRESENTATION_KEY, type ObjectPresentation, type ZonePresentationSource } from './model'
import { presentationStore, usePresentation } from './store'

/** The Zone host this page belongs to (`sys.` stripped), for scoping and as the last-resort name. */
export function currentZoneHost(): string {
  if (!isMockRuntime()) {
    try {
      const host = buckyos.getZoneHostName()?.trim()
      if (host) return host
    } catch {
      // fall back to the page host
    }
  }
  const host = window.location.hostname.toLowerCase()
  return host.startsWith('sys.') ? host.slice(4) : host
}

async function loadZonePresentation(): Promise<ObjectPresentation> {
  const fallbackHost = currentZoneHost()
  const { data } = await fetchZonePresentation()
  return data
    ? presentZone(data, { fallbackHost })
    : presentZone(null, { fallbackHost, status: 'failed' })
}

export function useZonePresentation(): ObjectPresentation {
  const value = usePresentation(ZONE_PRESENTATION_KEY, loadZonePresentation)
  return value ?? presentZone(null, { fallbackHost: currentZoneHost(), status: 'loading' })
}

/** After a save: every view of the Zone (login card, title, favicon, settings) shows the stored result. */
export function publishZonePresentation(source: ZonePresentationSource) {
  presentationStore.put(presentZone(source, { fallbackHost: currentZoneHost() }))
}

/** Scopes cached presentations to this Zone and the signed-in account (`''` when signed out). */
export function setPresentationViewer(userId: string | null | undefined) {
  presentationStore.setScope(`${currentZoneHost()}|${userId ?? ''}`)
}
