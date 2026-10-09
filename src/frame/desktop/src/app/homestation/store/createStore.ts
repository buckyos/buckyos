import { createApiHomeStationStore } from '../api/store'
import { homeStationTransport } from '../api/transport'
import { createHomeStationStore, parseScenario } from '../mock/store'
import type { HomeStationStore } from './types'

export function createPageStore(): HomeStationStore {
  const transport = homeStationTransport()
  return transport ? createApiHomeStationStore(transport) : createHomeStationStore(parseScenario(window.location.search))
}
