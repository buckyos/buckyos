import type { ZoneChannel } from '../channel.ts'
import type { ZonePresentationFields } from '../presentation/model.ts'
import { callControlPanel } from './call.ts'

export type { ZonePresentationFields }

/** `zone.presentation.get` (public): the administrator's settings and the ZoneDocument candidates. */
export interface ZonePresentationResponse {
  zone_did: string
  hostname?: string | null
  settings: ZonePresentationFields
  document: ZonePresentationFields
}

/** Readable while signed out: the channel may carry no session. `presentZone` turns it into a title and icons. */
export function readZonePresentation(channel: ZoneChannel): Promise<ZonePresentationResponse> {
  return callControlPanel<ZonePresentationResponse>(channel, 'zone.presentation.get')
}
