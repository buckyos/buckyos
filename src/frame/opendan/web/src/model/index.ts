import type { OpenDanDataModel } from './datamodel'
import { createKrpcDataModel } from './krpc'
import { createMockDataModel } from './mock'

// `?data=mock` / `?data=krpc` picks the source. Without it the real service
// is used, except under `vite dev` with no backend proxy configured.
const pickSource = (): 'mock' | 'krpc' => {
  const requested = new URLSearchParams(window.location.search).get('data')
  if (requested === 'mock' || requested === 'krpc') return requested
  return import.meta.env.DEV && !import.meta.env.VITE_OPENDAN_PROXY ? 'mock' : 'krpc'
}

export const dataModel: OpenDanDataModel = pickSource() === 'mock' ? createMockDataModel() : createKrpcDataModel()

export { OpenDanError } from './datamodel'
export type { OpenDanDataModel } from './datamodel'
export type * from './types'
