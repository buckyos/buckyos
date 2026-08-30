import { callRpc, type RpcCallOptions } from './rpc.ts'

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
