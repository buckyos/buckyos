/* ── Where My Network hands over: MessageHub conversations and group management, HomeStation pages ── */

import { didDisplayName } from '../../../presentation/model'
import { desktopUIStore } from '../../../models/DesktopUIDataModel'
import type { LookupKind } from '../../../components/network/types'

/** Opens MessageHub on this person's or group's conversation (its group panel holds group management). */
export async function openMessageHub(entityId: string, sessionId?: string) {
  const { getMessageHubStore } = await import('../../messagehub/store')
  const hub = getMessageHubStore()
  try { await hub.initialize() } catch { /* the view reports load errors itself */ }
  const payload = { kind: 'messagehub', entityId, ...(sessionId ? { sessionId } : {}), context: hub.defaultContext() }
  desktopUIStore.openAppWindow('messagehub', { launch: { requestId: crypto.randomUUID(), payload } })
}

export function openHomeStation() {
  desktopUIStore.openAppWindow('homestation')
}

/**
 * The person's HomeStation portal page (`https://<zone>/homestation/<user>`). msg-center does not say
 * whether someone publishes one, so it is offered for people whose Zone is known.
 */
export function homePageUrl(person: { did: string; kind?: LookupKind; zone?: string; local?: boolean }): string | null {
  if (person.kind && person.kind !== 'user') return null
  const user = encodeURIComponent(didDisplayName(person.did))
  if (person.local) return `/homestation/${user}`
  return person.zone ? `https://${person.zone}/homestation/${user}` : null
}
