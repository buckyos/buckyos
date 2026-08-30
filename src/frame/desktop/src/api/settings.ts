import { callRpc } from './rpc.ts'

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
