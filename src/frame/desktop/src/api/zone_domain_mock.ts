/* ── In-browser node answers for `zone.domain.*` ──
 *
 * The mock Zone is on its default hostname unless localStorage holds a (partial) state under
 * MOCK_ZONE_DOMAIN_STORAGE_KEY. Scripted outcomes: a token containing `REJECT` is refused; a certificate file
 * containing `PARENT` / `EXPIRED` / `NOKEY` is rejected, `SELF` warns, `PFX` needs the password `secret`;
 * `verify_waits` is how many verifications still find records waiting (with more than one left, the TXT as well). */

import { mockCurrentAccount, mockZoneOwner } from './control_panel_mock.ts'
import type {
  ApplyResult,
  CertificateInspection,
  CertificateSource,
  DnsProvider,
  DnsProviderField,
  DomainCheck,
  DomainCheckItem,
  DomainPlan,
  DomainStatus,
  HostingMode,
  HostCertStatus,
  InspectResult,
} from './zone_domain.ts'

export const MOCK_ZONE_DOMAIN_STORAGE_KEY = 'buckyos.mock.zone-domain.v1'

interface MockDomainState extends DomainStatus {
  /** Names that already carry another record. */
  taken: string[]
  verify_waits: number
  next_id: number
  warned_certificates: string[]
}

const nowSecs = () => Math.floor(Date.now() / 1000)
const RELAY = 'alice.web3.buckyos.ai'

function certificate(host: string, manager: 'acme' | 'manual', notAfter = nowSecs() + 61 * 86_400): HostCertStatus {
  return {
    host,
    loaded: true,
    not_after: notAfter,
    issuer: manager === 'acme' ? "Let's Encrypt E6" : 'Example CA DV G2',
    fingerprint: 'sha256:3f9a1c7e52b04d88216ce30f9b47a25d1ec87306bb942f60d5187ae9430c6b21',
    manager,
    acme: manager === 'acme' ? { state: 'ready', last_error: null, next_retry_at: null } : null,
  }
}

function defaults(): MockDomainState {
  return {
    hosting: null,
    hostname: RELAY,
    net_id: 'nat',
    certificates: [certificate(RELAY, 'acme'), certificate(`*.${RELAY}`, 'acme')],
    credential: null,
    check: null,
    problems: [],
    taken: [],
    verify_waits: 1,
    next_id: 1,
    warned_certificates: [],
  }
}

function loadState(): MockDomainState {
  try {
    return { ...defaults(), ...JSON.parse(localStorage.getItem(MOCK_ZONE_DOMAIN_STORAGE_KEY) ?? '{}') }
  } catch {
    return defaults()
  }
}

function saveState(state: MockDomainState) {
  try { localStorage.setItem(MOCK_ZONE_DOMAIN_STORAGE_KEY, JSON.stringify(state)) } catch { /* storage is optional */ }
}

const field = (id: string, label: string, options: Partial<DnsProviderField> = {}): DnsProviderField =>
  ({ id, label, secret: false, optional: false, multiline: false, options: [], ...options })

const provider = (id: string, name: string, fields: DnsProviderField[], options: Partial<DnsProvider> = {}): DnsProvider =>
  ({ id, name, fields, token_url: `https://${id}.example/tokens`, ns_suffixes: [], token_scope: 'domain', apex_cname: false, tested: false, sandbox_url: null, ...options })

const PROVIDERS: DnsProvider[] = [
  provider('cloudflare', 'Cloudflare', [field('api_token', 'API Token', { secret: true })], { token_url: 'https://dash.cloudflare.com/profile/api-tokens', apex_cname: true, tested: true }),
  provider('route53', 'Amazon Route 53', [field('access_key_id', 'Access key ID'), field('secret_access_key', 'Secret access key', { secret: true }), field('hosted_zone_id', 'Hosted zone ID', { optional: true })]),
  provider('ovh', 'OVHcloud', [
    field('region', 'API endpoint', { options: [['eu', 'Europe'], ['us', 'United States'], ['ca', 'Canada']] }),
    field('application_key', 'Application key'),
    field('application_secret', 'Application secret', { secret: true }),
    field('consumer_key', 'Consumer key', { secret: true }),
  ]),
  provider('hetzner', 'Hetzner', [field('api_token', 'API token', { secret: true })], { token_scope: 'account' }),
  provider('google_cloud', 'Google Cloud DNS', [field('service_account_json', 'Service account key (JSON)', { secret: true, multiline: true }), field('project_id', 'Project ID', { optional: true })]),
]

const text = (value: unknown) => (typeof value === 'string' ? value : '')

function fail(code: string): never {
  throw new Error(`rpc call error: [domain:${code}] mock`)
}

function setupOf(params: Record<string, unknown>): { mode: HostingMode; certificate: CertificateSource } {
  const mode = text(params.mode) as HostingMode
  const source = text(params.certificate) as CertificateSource
  if (mode === 'manual' && source !== 'upload') fail('invalid_setup')
  return { mode, certificate: source }
}

function planFor(state: MockDomainState, mode: HostingMode): DomainPlan {
  const domain = state.hostname
  const pkx = { id: 'pkx', phase: 'ownership' as const, name: `_pkx.${domain}`, type: 'TXT', values: ['PKX(T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8)'] }
  const records: DomainPlan['records'] = mode === 'sn'
    ? [pkx, { id: 'ns', phase: 'routing', name: domain, type: 'NS', values: ['dns.web3.buckyos.ai'] }]
    : [
        pkx,
        { id: 'entry', phase: 'routing', name: domain, type: 'CNAME', values: [RELAY] },
        { id: 'wildcard', phase: 'routing', name: `*.${domain}`, type: 'CNAME', values: [RELAY] },
      ]
  return {
    records,
    conflicts: records.filter((record) => state.taken.includes(record.name)).map((record) => ({
      name: record.name,
      existing: [{ type: 'A', values: ['198.51.100.4'] }],
      planned: { type: record.type, values: record.values },
    })),
  }
}

function checkOf(state: MockDomainState, setup: { mode: HostingMode; certificate: CertificateSource }, waiting: boolean, certificateReady: boolean): DomainCheck {
  const domain = state.hostname
  const item = (id: string, extra: Partial<DomainCheckItem> = {}): DomainCheckItem =>
    ({ id, status: 'pass', code: null, expected: [], actual: [], detail: null, targets: [], ...extra })
  const cname = `CNAME ${RELAY}`
  const pkx = ['TXT PKX(T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8)']
  const items = [item('ownership', waiting && state.verify_waits > 1
    ? { status: 'waiting', code: 'txt_missing', expected: pkx }
    : { expected: pkx, actual: pkx })]
  if (setup.mode === 'sn') {
    items.push(item('delegation', waiting
      ? { status: 'waiting', code: 'ns_missing', expected: ['NS dns.web3.buckyos.ai'], actual: ['NS ada.ns.cloudflare.com'] }
      : { expected: ['NS dns.web3.buckyos.ai'], actual: ['NS dns.web3.buckyos.ai'] }))
  }
  items.push(item('entry', waiting
    ? {
        status: 'fail',
        code: 'stale_record',
        expected: [cname],
        targets: [
          { host: domain, status: 'waiting', code: 'record_missing', expected: [cname], actual: [] },
          { host: `ood1.${domain}`, status: 'fail', code: 'stale_record', expected: [cname], actual: ['A 198.51.100.4'] },
        ],
      }
    : { expected: [cname], actual: [cname] }))
  if (setup.certificate === 'acme') {
    items.push(item('challenge', setup.mode === 'sn' ? { status: 'skipped', code: 'commit_time' } : {}))
    items.push(item('caa'))
  } else {
    items.push(item('certificate', certificateReady ? {} : { status: 'fail', code: 'certificate_missing' }))
  }
  return { ready: items.every((entry) => entry.status === 'pass' || entry.status === 'skipped'), bound: true, checked_at: nowSecs(), items }
}

function inspect(state: MockDomainState, params: Record<string, unknown>): InspectResult {
  const domain = state.hostname
  const content = (params.files as string[]).map((file) => {
    try { return atob(file) } catch { return '' }
  }).join('\n')
  const items: CertificateInspection['items'] = []
  const add = (code: string, level: 'pass' | 'warn' | 'reject' = 'pass', reason: string | null = null, detail: string | null = null) =>
    items.push({ code, level, reason, detail })
  const wrongPassword = content.includes('PFX') && params.pfx_password !== 'secret'
  const missingKey = content.includes('NOKEY')
  const parent = content.includes('PARENT')
  const expired = content.includes('EXPIRED')
  const selfSigned = content.includes('SELF')
  const apex = domain.split('.').slice(1).join('.')
  if (!content.trim()) add('readable', 'reject', 'empty')
  else if (wrongPassword) add('readable', 'reject', 'pfx_password_wrong')
  else if (missingKey) add('readable', 'reject', 'key_missing')
  else {
    add('readable')
    add('key_matches')
    if (parent) add('covers_domain', 'pass')
    else add('covers_domain')
    if (parent) add('covers_wildcard', 'reject', 'parent_wildcard', `*.${apex}`)
    else add('covers_wildcard')
    if (expired) add('validity', 'reject', 'expired', '2026-10-01')
    else add('validity')
    add('server_usage')
    add('not_ca')
    if (selfSigned) add('public_trust', 'warn', 'self_signed')
    else add('public_trust')
    add('key_algorithm')
  }
  const readable = items[0].level === 'pass'
  const rejected = items.some((entry) => entry.level === 'reject')
  let id: string | null = null
  if (!rejected) {
    id = `cert-${state.next_id++}`
    if (selfSigned) state.warned_certificates.push(id)
  }
  return {
    certificate_id: id,
    inspection: {
      domain,
      items,
      coverage: readable ? [{ name: domain, covered_by: parent ? `*.${apex}` : domain }, { name: `*.${domain}`, covered_by: parent ? null : `*.${domain}` }] : [],
      dns_names: readable ? (parent ? [`*.${apex}`] : [domain, `*.${domain}`]) : [],
      issuer: readable ? (selfSigned ? `CN=${domain}` : 'Example CA DV G2') : null,
      subject: readable ? `CN=${domain}` : null,
      not_before: readable ? nowSecs() - 86_400 : null,
      not_after: readable ? nowSecs() + (expired ? -30 : 198) * 86_400 : null,
      publicly_trusted: readable && !selfSigned,
      node_time: nowSecs(),
      fingerprint: readable ? 'sha256:aa11' : null,
    },
  }
}

function statusOf(state: MockDomainState): DomainStatus {
  return {
    hosting: state.hosting,
    hostname: state.hostname,
    net_id: state.net_id,
    certificates: state.certificates,
    credential: state.credential,
    check: state.check,
    problems: state.problems,
  }
}

function handle(state: MockDomainState, method: string, params: Record<string, unknown>): unknown {
  if (mockCurrentAccount()?.user_id !== mockZoneOwner()) fail('not_owner')
  if (method === 'status') return statusOf(state)
  if (method === 'restart_certificate') {
    const cert = state.certificates.find((item) => item.host === params.host)
    if (!cert?.acme) fail('certificate_not_automatic')
    cert.acme = { ...cert.acme, state: cert.loaded ? 'renewing' : 'ordering', next_retry_at: null }
    return statusOf(state)
  }
  if (!state.hosting) fail('default_hostname')
  const hosting = state.hosting
  switch (method) {
    case 'providers':
      return { providers: PROVIDERS }
    case 'check':
      state.check = checkOf(state, hosting, state.verify_waits > 0, true)
      return state.check
    case 'inspect_certificate':
      return inspect(state, params)
    case 'plan':
      return planFor(state, setupOf(params).mode)
    case 'apply': {
      const plan = planFor(state, setupOf(params).mode)
      const fields = Object.values((params.fields ?? {}) as Record<string, string>)
      if (fields.some((value) => value.includes('REJECT'))) {
        return {
          credential_id: null,
          results: plan.records.map(({ name, type }) => ({ name, type, status: 'failed', code: 'credential_rejected', detail: null })),
          conflicts: plan.conflicts,
        } satisfies ApplyResult
      }
      const confirm = (params.confirm ?? []) as string[]
      const open = plan.conflicts.filter((conflict) => !confirm.includes(conflict.name))
      state.taken = open.map((conflict) => conflict.name)
      const id = text(params.credential_id) || `home-example-com-${state.next_id++}`
      return {
        credential_id: id,
        results: plan.records.map(({ name, type }) => ({
          name, type, status: open.some((conflict) => conflict.name === name) ? 'needs_confirm' : 'written', code: null, detail: null,
        })),
        conflicts: open,
      } satisfies ApplyResult
    }
    case 'verify': {
      const setup = setupOf(params)
      const check = checkOf(state, setup, state.verify_waits > 0, Boolean(params.certificate_id) || hosting.certificate === 'upload')
      state.verify_waits = Math.max(0, state.verify_waits - 1)
      return check
    }
    case 'update': {
      const setup = setupOf(params)
      const certificateId = text(params.certificate_id)
      const credentialId = text(params.credential_id)
      if (setup.certificate === 'upload' && !certificateId && hosting.certificate !== 'upload') fail('certificate_missing')
      if (state.warned_certificates.includes(certificateId) && params.accept_warnings !== true) fail('certificate_warning_unconfirmed')
      if (setup.mode === 'dns_api' && !credentialId && !state.credential) fail('credential_missing')
      const refresh = setup.mode === hosting.mode && setup.certificate === hosting.certificate
      if (!refresh && state.verify_waits > 0) fail('domain_not_ready')
      const credential = setup.mode === 'dns_api'
        ? { id: credentialId || state.credential!.id, provider: state.credential?.provider ?? 'cloudflare' }
        : null
      state.hosting = {
        ...setup,
        domain: state.hostname,
        ...(credential ? { provider: credential.provider, credential_id: credential.id } : {}),
      }
      state.credential = credential
      state.problems = []
      const manager = setup.certificate === 'acme' ? 'acme' : 'manual'
      state.certificates = [state.hostname, `*.${state.hostname}`].map((host) => certificate(host, manager, certificateId ? nowSecs() + 198 * 86_400 : undefined))
      state.check = null
      return statusOf(state)
    }
    default:
      throw new Error(`Unknown method: zone.domain.${method}`)
  }
}

export async function callMockZoneDomain<T>(method: string, params: Record<string, unknown>): Promise<{ data: T | null; error: unknown }> {
  await new Promise((resolve) => window.setTimeout(resolve, 120))
  const state = loadState()
  try {
    const data = handle(state, method, params) as T
    saveState(state)
    return { data, error: null }
  } catch (error) {
    return { data: null, error }
  }
}
