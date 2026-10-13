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

type RpcResult<T> = Promise<{ data: T | null; error: unknown }>

/** Settings reads and writes: kRPC to control-panel, or the in-browser mock runtime. */
async function callSettingsRpc<T>(
  method: string,
  params: Record<string, unknown> = {},
  options: RpcCallOptions = {},
): RpcResult<T> {
  if (isMockRuntime()) {
    const { callMockSettings } = await import('./settings_mock.ts')
    return callMockSettings<T>(method, params)
  }
  return callRpc<T>(method, params, options)
}

export const fetchBuckyOSInfo = async (): RpcResult<BuckyOSInfo> =>
  callSettingsRpc<BuckyOSInfo>('system.buckyos_info.get')

export interface BuckyOSDevConfig {
  schema_version: number
  enabled: boolean
  enabled_at?: number | null
  enabled_by?: string | null
}

export const fetchBuckyOSDevConfig = async (): RpcResult<BuckyOSDevConfig> =>
  callSettingsRpc<BuckyOSDevConfig>('system.dev_mode.get')

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
): RpcResult<BuckyOSDevConfig> =>
  callSettingsRpc<BuckyOSDevConfig>('system.dev_mode.set', { enabled }, options)

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

// ---- Zone topology (control-panel cluster_mgr / zone_mgr) ----

export interface NetworkEndpoint {
  ip?: string | null
  /** `global`, `private`, `link_local`, `loopback`, … as classified by node_daemon. */
  scope?: string | null
}

/** One device of `zone.devices.list`: `devices/{name}/info` as its node_daemon reports it. */
export interface ZoneDevice {
  name: string
  did?: string
  device_type?: string
  /** `ood` / `gateway` for nodes named by the ZoneDocument, otherwise the device type. */
  role: string
  is_ood: boolean
  /** The node this control-panel runs on. */
  is_local: boolean
  online: boolean
  state?: string | null
  /** Unix seconds of the last report. */
  last_seen?: number | null
  hostname?: string | null
  arch?: string | null
  os?: string | null
  os_version?: string | null
  net_id?: string | null
  ips: string[]
  endpoints?: NetworkEndpoint[]
  ipv6_state?: string | null
  rtcp_port?: number | null
  cpu?: { model?: string | null; cores?: number | null; mhz?: number | null; usage_percent?: number | null }
  memory?: { total_bytes?: number | null; used_bytes?: number | null }
  storage?: { total_bytes?: number | null; used_bytes?: number | null }
  gpu?: {
    model: string
    total_mem_bytes?: number | null
    used_mem_bytes?: number | null
    load_percent?: number | null
    tflops?: number | null
  } | null
  /** Set when the node never reported or its report could not be read. */
  info_error?: string | null
}

export interface ZoneDevicesResponse {
  zone_did: string
  hostname?: string | null
  owner_did?: string | null
  /** SN host when the Zone is reached through an SN. */
  sn?: string | null
  local_device: string
  online_window_secs: number
  devices: ZoneDevice[]
}

export const fetchZoneDevices = async (): RpcResult<ZoneDevicesResponse> =>
  callSettingsRpc<ZoneDevicesResponse>('zone.devices.list')

export type ZoneCertificateState = 'valid' | 'expired' | 'not_yet_valid' | 'unusable' | 'unknown'

export interface ZoneCertificate {
  host: string
  match_type?: string | null
  /** `acme` when the gateway issues and renews it; anything else was installed by hand. */
  source: string
  issuer?: string | null
  subject?: string | null
  serial_number?: string | null
  not_before?: string | null
  not_after?: string | null
  days_remaining?: number | null
  dns_names: string[]
  fingerprint_sha256?: string | null
  last_renewed_at?: string | null
  state: ZoneCertificateState
  cert_pem?: string | null
}

export interface ZoneCertificatesResponse {
  hostname: string
  /** The node whose identity store was read. */
  node?: string | null
  certificates: ZoneCertificate[]
}

export const fetchZoneCertificates = async (): RpcResult<ZoneCertificatesResponse> =>
  callSettingsRpc<ZoneCertificatesResponse>('zone.certificates.list')

/** `zone.overview`: local Zone files plus an SN DNS self-check. */
export interface ZoneOverviewResponse {
  etcDir: string
  zone: { name: string; domain: string; did: string; ownerDid: string; userName: string; zoneIat: number }
  device: { name: string; did: string; type: string; netId: string }
  sn: {
    url: string
    username: string
    host: string
    ip: string
    dnsARecords: string[]
    dnsTxtRecords: string[]
    digError: string
    selfCertState: boolean
    selfCertStateSource: string
  }
  notes: string[]
}

export const fetchZoneOverview = async (): RpcResult<ZoneOverviewResponse> =>
  callSettingsRpc<ZoneOverviewResponse>('zone.overview')

// ---- Developer tools ----

export interface ConfigFileSummary {
  name: string
  path: string
  exists: boolean
  sizeBytes: number
  modifiedAt: string
}

export interface GatewayOverviewResponse {
  mode: string
  etcDir: string
  files: ConfigFileSummary[]
  tlsDomains: string[]
  notes: string[]
}

export const fetchGatewayOverview = async (): RpcResult<GatewayOverviewResponse> =>
  callSettingsRpc<GatewayOverviewResponse>('gateway.overview')

export interface GatewayFileResponse {
  name: string
  path: string
  sizeBytes: number
  modifiedAt: string
  content: string
}

export const fetchGatewayFile = async (name: string): RpcResult<GatewayFileResponse> =>
  callSettingsRpc<GatewayFileResponse>('gateway.file.get', { name })

export interface LogService {
  id: string
  label: string
}

export const fetchLogServices = async (): RpcResult<{ services: LogService[] }> =>
  callSettingsRpc<{ services: LogService[] }>('system.logs.list')

export interface LogDownloadTicket {
  /** Same-origin GET URL, valid for `expiresInSec`. */
  url: string
  expiresInSec: number
  filename: string
}

/** Packs the logs of `services` written since `since` (RFC 3339) into a zip and returns its link. */
export const requestLogDownload = async (
  services: string[],
  since: string,
): RpcResult<LogDownloadTicket> =>
  callSettingsRpc<LogDownloadTicket>('system.logs.download', { services, since, mode: 'filtered' })
