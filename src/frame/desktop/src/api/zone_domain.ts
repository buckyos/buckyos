/* `zone.domain.*`: how the Zone's own domain is hosted and where its certificate comes from.
 * control-panel forwards to the node (buckyos-api `domain_hosting.rs`, `cyfs-identity-cert`). */

import { callRpc, type RpcCallOptions } from './rpc.ts'
import { isMockRuntime } from '../runtime.ts'

export type HostingMode = 'sn' | 'manual' | 'dns_api'
export type CertificateSource = 'acme' | 'upload'

export interface DomainHosting {
  mode: HostingMode
  certificate: CertificateSource
  domain: string
  provider?: string
  credential_id?: string
  acme_issuer?: string
}

/** Unix seconds, or a date text. */
export type TimeValue = number | string | null

export interface HostCertStatus {
  host: string
  loaded: boolean
  not_after: TimeValue
  issuer: string | null
  fingerprint: string | null
  manager: 'acme' | 'manual' | null
  acme: {
    state: 'none' | 'ordering' | 'ready' | 'renewing' | 'manual'
    last_error: { message: string; at: TimeValue } | null
    next_retry_at: TimeValue
  } | null
}

export type CheckStatus = 'pass' | 'waiting' | 'fail' | 'skipped'

export interface DomainCheckTarget {
  host: string
  status: CheckStatus
  code: string | null
  expected: string[]
  actual: string[]
}

export interface DomainCheckItem {
  id: string
  status: CheckStatus
  code: string | null
  expected: string[]
  actual: string[]
  detail: string | null
  targets: DomainCheckTarget[]
}

export interface DomainCheck {
  ready: boolean
  /** The SN binds the domain to the owner; null when that could not be asked. */
  bound: boolean | null
  checked_at: number
  items: DomainCheckItem[]
}

export interface DomainStatus {
  /** Null on the default hostname. */
  hosting: DomainHosting | null
  hostname: string
  net_id: string | null
  certificates: HostCertStatus[]
  credential: { id: string; provider: string } | null
  check: DomainCheck | null
  problems: string[]
}

export interface DnsProviderField {
  id: string
  label: string
  secret: boolean
  optional: boolean
  multiline: boolean
  options: Array<[string, string]>
}

export interface DnsProvider {
  id: string
  name: string
  fields: DnsProviderField[]
  token_url: string
  ns_suffixes: string[]
  token_scope: 'domain' | 'account'
  apex_cname: boolean
  tested: boolean
  sandbox_url: string | null
}

export interface RecordValues {
  type: string
  values: string[]
}

export interface PlannedRecord extends RecordValues {
  id: string
  phase: 'ownership' | 'routing'
  name: string
}

export interface RecordConflict {
  name: string
  existing: RecordValues[]
  planned: RecordValues
}

export interface DomainPlan {
  records: PlannedRecord[]
  conflicts: RecordConflict[]
}

export interface RecordResult {
  name: string
  type: string
  status: 'written' | 'unchanged' | 'needs_confirm' | 'failed'
  code: string | null
  detail: string | null
}

export interface ApplyResult {
  credential_id: string | null
  results: RecordResult[]
  conflicts: RecordConflict[]
}

export type CertificateCheckLevel = 'pass' | 'warn' | 'reject'

export interface CertificateInspection {
  domain: string
  items: Array<{ code: string; level: CertificateCheckLevel; reason: string | null; detail?: string | null }>
  coverage: Array<{ name: string; covered_by: string | null }>
  dns_names: string[]
  issuer: string | null
  subject: string | null
  not_before: number | null
  not_after: number | null
  publicly_trusted: boolean
  node_time: number
  fingerprint: string | null
}

export interface InspectResult {
  /** Null when a check rejects the certificate. */
  certificate_id: string | null
  inspection: CertificateInspection
}

export interface DomainSetup {
  mode: HostingMode
  certificate: CertificateSource
}

type DomainResult<T> = Promise<{ data: T | null; error: unknown }>

async function callDomain<T>(method: string, params: object = {}, options: RpcCallOptions = {}): DomainResult<T> {
  if (isMockRuntime()) {
    const { callMockZoneDomain } = await import('./zone_domain_mock.ts')
    return callMockZoneDomain<T>(method, params as Record<string, unknown>)
  }
  return callRpc<T>(`zone.domain.${method}`, params as Record<string, unknown>, options)
}

export const fetchDomainStatus = () => callDomain<DomainStatus>('status')

export const fetchDnsProviders = () => callDomain<{ providers: DnsProvider[] }>('providers')

export const checkDomain = (options: RpcCallOptions) => callDomain<DomainCheck>('check', {}, options)

export const inspectDomainCertificate = (params: { files: string[]; pfx_password?: string }, options: RpcCallOptions) =>
  callDomain<InspectResult>('inspect_certificate', params, options)

export const planDomain = (setup: DomainSetup, options: RpcCallOptions) => callDomain<DomainPlan>('plan', setup, options)

export type ApplyCredential = { provider: string; fields: Record<string, string> } | { credential_id: string }

export const applyDomain = (setup: DomainSetup, credential: ApplyCredential, confirm: string[], options: RpcCallOptions) =>
  callDomain<ApplyResult>('apply', { ...setup, ...credential, confirm }, options)

export interface SetupIds {
  credential_id?: string
  certificate_id?: string
}

export const verifyDomain = (setup: DomainSetup, ids: SetupIds, options: RpcCallOptions) =>
  callDomain<DomainCheck>('verify', { ...setup, ...ids }, options)

export const updateDomain = (setup: DomainSetup, ids: SetupIds, acceptWarnings: boolean, options: RpcCallOptions) =>
  callDomain<DomainStatus>('update', { ...setup, ...ids, accept_warnings: acceptWarnings }, options)

/** The `[domain:<code>]` / `[active:<code>]` a failed call carries, and its raw text. */
export function domainErrorOf(error: unknown): { code: string | null; detail: string } {
  const detail = error instanceof Error ? error.message : String(error ?? '')
  return { code: /\[(?:domain|active):([a-z0-9_]+)\]/.exec(detail)?.[1] ?? null, detail }
}
