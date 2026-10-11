import { buckyos } from 'buckyos'
import { callRpc, type RpcCallOptions } from './rpc.ts'
import { isMockRuntime } from '../runtime.ts'

export interface BuckyOSInfo {
  schema_version: number
  version: string
  build_version?: string | null
  release_channel: string
  target: string
  installed_at: number
  updated_at: number
}

export const fetchBuckyOSInfo = async (): Promise<{
  data: BuckyOSInfo | null
  error: unknown
}> => callRpc<BuckyOSInfo>('system.buckyos_info.get', {})

export interface BuckyOSDevConfig {
  schema_version: number
  enabled: boolean
  enabled_at?: number | null
  enabled_by?: string | null
}

export const fetchBuckyOSDevConfig = async (): Promise<{
  data: BuckyOSDevConfig | null
  error: unknown
}> => callRpc<BuckyOSDevConfig>('system.dev_mode.get', {})

export const fetchBuckyOSDevModeEnabled = async (): Promise<{
  data: boolean | null
  error: unknown
}> => {
  const { data, error } = await fetchBuckyOSDevConfig()
  return { data: data?.enabled ?? null, error }
}

export const setBuckyOSDevMode = async (
  enabled: boolean,
  options: RpcCallOptions,
): Promise<{
  data: BuckyOSDevConfig | null
  error: unknown
}> => callRpc<BuckyOSDevConfig>('system.dev_mode.set', { enabled }, options)

export interface ZonePresentationFields {
  display_name?: string | null
  description?: string | null
  icon?: string | null
}

/** `zone.presentation.get` (public): the administrator's settings and the ZoneDocument candidates. */
export interface ZonePresentationResponse {
  zone_did: string
  hostname?: string | null
  settings: ZonePresentationFields
  document: ZonePresentationFields
}

/** A field left out is kept; `null` clears it back to the document / DID default. */
export type ZonePresentationPatch = Partial<Record<keyof ZonePresentationFields, string | null>>

export const fetchZonePresentation = async (): Promise<{
  data: ZonePresentationResponse | null
  error: unknown
}> => {
  if (isMockRuntime()) {
    const { callMockControlPanel } = await import('./control_panel_mock.ts')
    return callMockControlPanel<ZonePresentationResponse>('zone.presentation.get', {})
  }
  try {
    // Signed-out pages (the login page) read it too, so no session token is attached.
    const client = new buckyos.kRPCClient('/kapi/control-panel')
    const result = await client.call('zone.presentation.get', {})
    if (!result || typeof result !== 'object') throw new Error('Invalid zone.presentation.get response')
    return { data: result as ZonePresentationResponse, error: null }
  } catch (error) {
    return { data: null, error }
  }
}

export const setZonePresentation = async (
  patch: ZonePresentationPatch,
  options: RpcCallOptions,
): Promise<{ data: ZonePresentationResponse | null; error: unknown }> => {
  if (isMockRuntime()) {
    const { callMockControlPanel } = await import('./control_panel_mock.ts')
    return callMockControlPanel<ZonePresentationResponse>('zone.presentation.set', patch)
  }
  return callRpc<ZonePresentationResponse>('zone.presentation.set', patch, options)
}
