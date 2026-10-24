/* ── Hybrid SSO calls of Control Panel (contract doc/arch/verify-hub/混合SSO技术契约.md) ──
 *
 * Expected outcomes come back as `status` envelopes, denials as `AccessDenial`; nothing here reads
 * error text. Public methods go without a session token (the visitor may hold none), session
 * methods through the SDK's control-panel client. The mock runtime answers from the in-browser
 * control panel.
 */

import {
  buckyos,
  parseAccessDenial,
  type AccessDenial,
  type CurrentIdentity,
  type EntryDescriptor,
  type EntryService,
  type SsoPublicConfig,
} from 'buckyos'
import { callRpc } from './rpc.ts'
import { isMockRuntime } from '../runtime.ts'

type RpcResult<T> = Promise<{ data: T | null; error: unknown }>

async function callMock<T>(method: string, params: Record<string, unknown>): RpcResult<T> {
  const { callMockControlPanel } = await import('./control_panel_mock.ts')
  return callMockControlPanel<T>(method, params)
}

/** A method open to visitors without a session. */
async function callPublic<T>(method: string, params: Record<string, unknown>): RpcResult<T> {
  if (isMockRuntime()) return callMock<T>(method, params)
  try {
    const client = new buckyos.kRPCClient('/kapi/control-panel')
    const result = await client.call(method, params)
    if (!result || typeof result !== 'object') throw new Error(`Invalid ${method} response`)
    return { data: result as T, error: null }
  } catch (error) {
    return { data: null, error }
  }
}

async function callSession<T>(method: string, params: Record<string, unknown> = {}): RpcResult<T> {
  if (isMockRuntime()) return callMock<T>(method, params)
  return callRpc<T>(method, params)
}

type Envelope = Record<string, unknown> & { status?: unknown }

/** Accepts only the listed statuses; a `denied` envelope must carry a valid denial. */
async function enveloped<T>(call: RpcResult<Envelope>, statuses: readonly string[]): RpcResult<T> {
  const { data, error } = await call
  if (!data) return { data: null, error }
  const status = typeof data.status === 'string' ? data.status : ''
  if (!statuses.includes(status)) return { data: null, error: new Error('unexpected response status') }
  if (status === 'denied') {
    const denial = parseAccessDenial(data.denial)
    if (!denial) return { data: null, error: new Error('malformed denial') }
    return { data: { ...data, denial } as unknown as T, error: null }
  }
  return { data: data as unknown as T, error: null }
}

type Denied = { status: 'denied'; denial: AccessDenial }

// ── Zone switches and entries ──

/** `system/sso/settings` as admins edit it; an empty `jump_base_url` means the default. */
export interface SsoSettings {
  self_register_enabled: boolean
  did_login_enabled: boolean
  jump_base_url?: string | null
}

export const fetchSsoConfig = (): RpcResult<SsoPublicConfig> =>
  callPublic<SsoPublicConfig>('auth.sso_config', {})

export const fetchSsoSettings = (): RpcResult<SsoSettings> => callSession<SsoSettings>('sso.settings.get')

export const saveSsoSettings = (settings: SsoSettings): RpcResult<SsoSettings> => {
  const jump = settings.jump_base_url?.trim()
  return callSession<SsoSettings>('sso.settings.set', {
    self_register_enabled: settings.self_register_enabled,
    did_login_enabled: settings.did_login_enabled,
    // An empty field returns the Zone to the default jump entry.
    jump_base_url: jump || null,
  })
}

export const ENTRY_SERVICES: readonly EntryService[] = ['aiworkspace', 'msg-center']

export function parseEntryService(value: string | null): EntryService | null {
  return (ENTRY_SERVICES as readonly string[]).includes(value ?? '') ? (value as EntryService) : null
}

export const describeEntry = (service: EntryService, entryId: string): RpcResult<EntryDescriptor> =>
  callPublic<EntryDescriptor>('auth.entry.describe', { service, entry_id: entryId })

export type RegisterResult = { status: 'created'; user_id: string } | { status: 'name_taken' } | Denied

/** Self-registration on an entry; `passwordHash` is `hashPassword(userId, password)` without a nonce. */
export const registerAccount = (input: {
  service: EntryService
  entryId: string
  userId: string
  showName?: string
  passwordHash: string
}): RpcResult<RegisterResult> =>
  enveloped<RegisterResult>(
    callPublic<Envelope>('auth.register', {
      service: input.service,
      entry_id: input.entryId,
      user_id: input.userId,
      ...(input.showName ? { show_name: input.showName } : {}),
      password_hash: input.passwordHash,
    }),
    ['created', 'name_taken', 'denied'],
  )

/** After signing in on an AI Workspace entry: the entry grants the caller its template. */
export async function joinWorkspaceEntry(entryId: string): RpcResult<Record<string, unknown>> {
  if (isMockRuntime()) return { data: { ok: true, entry_id: entryId }, error: null }
  try {
    const result = await buckyos.getServiceRpcClient('aiworkspace').call('ws.join_entry', { entry_id: entryId })
    return { data: (result ?? {}) as Record<string, unknown>, error: null }
  } catch (error) {
    return { data: null, error }
  }
}

// ── DID login ──

export type DidBeginResult = { status: 'redirect'; url: string } | { status: 'canonical_unavailable' } | Denied

/** Starts a DID login on this Zone; the browser then goes to the DID's canonical Zone. */
export const beginDidLogin = (did: string, redirectUrl: string): RpcResult<DidBeginResult> =>
  enveloped<DidBeginResult>(
    callPublic<Envelope>('auth.did.begin', { did, redirect_url: redirectUrl }),
    ['redirect', 'canonical_unavailable', 'denied'],
  )

/** What the target Zone sent to the canonical consent page (`/sso/federate?…`). */
export interface FederationRequest {
  did: string
  client_zone: string
  redirect_uri: string
  state: string
  nonce: string
  code_challenge: string
}

export const FEDERATION_REQUEST_KEYS = ['did', 'client_zone', 'redirect_uri', 'state', 'nonce', 'code_challenge'] as const

export type FederationConsent = {
  status: 'consent'
  client_zone: string
  client_host: string
  profile: { show_name: string; avatar?: string | null }
  already_granted: boolean
}

export type FederationDescribeResult = FederationConsent | Denied
export type FederationDecision = { status: 'redirect'; url: string } | Denied

export const describeFederation = (request: FederationRequest): RpcResult<FederationDescribeResult> =>
  enveloped<FederationDescribeResult>(
    callSession<Envelope>('auth.federation.describe', { ...request }),
    ['consent', 'denied'],
  )

export const approveFederation = (request: FederationRequest, remember: boolean): RpcResult<FederationDecision> =>
  enveloped<FederationDecision>(
    callSession<Envelope>('auth.federation.approve', { ...request, remember }),
    ['redirect', 'denied'],
  )

export const denyFederation = (request: FederationRequest): RpcResult<FederationDecision> =>
  enveloped<FederationDecision>(
    callSession<Envelope>('auth.federation.deny', { ...request }),
    ['redirect', 'denied'],
  )

// ── Exclusive invites: visitor side ──

export type InviteDescribeResult =
  | { status: 'active'; user_id: string; show_name: string; resource_title?: string | null; return_path: string }
  | (Denied & { user_id?: string; return_path?: string })

export type InviteLoginResult = { status: 'ready'; callback_url: string } | Denied
export type InviteResendResult = { status: 'submitted' | 'rate_limited' | 'not_applicable' }

export const describeInvite = (inviteId: string, secret: string): RpcResult<InviteDescribeResult> =>
  enveloped<InviteDescribeResult>(
    callPublic<Envelope>('auth.invite.describe', { invite_id: inviteId, secret }),
    ['active', 'denied'],
  )

export const loginByInvite = (inviteId: string, secret: string): RpcResult<InviteLoginResult> =>
  enveloped<InviteLoginResult>(
    callPublic<Envelope>('auth.invite.login', { invite_id: inviteId, secret }),
    ['ready', 'denied'],
  )

export const requestInviteResend = (inviteId: string): RpcResult<InviteResendResult> =>
  enveloped<InviteResendResult>(
    callPublic<Envelope>('auth.invite.request_resend', { invite_id: inviteId }),
    ['submitted', 'rate_limited', 'not_applicable'],
  )

/** First password of an exclusive invite account; the caller is that account. */
export const setFirstPassword = (userId: string, password: string): RpcResult<{ status: 'ok' } | Denied> =>
  enveloped<{ status: 'ok' } | Denied>(
    callSession<Envelope>('auth.invite.set_password', { new_password_hash: buckyos.hashPassword(userId, password) }),
    ['ok', 'denied'],
  )

export const fetchCurrentIdentityFromServer = (): RpcResult<CurrentIdentity> =>
  callSession<CurrentIdentity>('auth.current_identity')

// ── Exclusive invites: admin side ──

export interface InviteChannel {
  /** `email`, `sms` or another channel the issuer sends through. */
  kind: string
  /** Masked address; the full one stays with the issuer. */
  target_hint: string
}

export type InviteResource =
  | { kind: 'group_session'; group_did: string; session_id: string; title: string }
  | { kind: 'workspace'; workspace_id: string; title: string }

export type InviteIssuer = { kind: 'user'; user_id: string } | { kind: 'app'; app_instance_id: string }

export interface ExclusiveInvite {
  invite_id: string
  user_id: string
  issuer: InviteIssuer
  channel: InviteChannel
  resource?: InviteResource | null
  resource_state: 'done' | 'pending' | 'none'
  return_path: string
  state: 'active' | 'revoked'
  credential: {
    version: number
    issued_at: number
    expires_at: number
    state: 'active' | 'superseded' | 'consumed_by_password'
  }
  resend?: { requested_at: number; count: number } | null
  created_at: number
  updated_at: number
}

/** The link is in this answer only; it is never readable again. */
export interface ExclusiveInviteLink {
  invite: ExclusiveInvite
  link: string
}

export interface ExclusiveInviteCreateInput {
  account: { kind: 'new'; userId: string; showName?: string } | { kind: 'existing'; userId: string }
  channel: InviteChannel
  resource?: InviteResource
  returnPath?: string
}

export const createExclusiveInvite = (input: ExclusiveInviteCreateInput): RpcResult<ExclusiveInviteLink> =>
  callSession<ExclusiveInviteLink>('user.exclusive_invite.create', {
    ...(input.account.kind === 'new'
      ? { new_user: { user_id: input.account.userId, ...(input.account.showName ? { show_name: input.account.showName } : {}) } }
      : { user_id: input.account.userId }),
    channel: input.channel,
    ...(input.resource ? { resource: input.resource } : {}),
    ...(input.returnPath ? { return_path: input.returnPath } : {}),
  })

export const listExclusiveInvites = (userId?: string): RpcResult<{ invites: ExclusiveInvite[] }> =>
  callSession<{ invites: ExclusiveInvite[] }>('user.exclusive_invite.list', userId ? { user_id: userId } : {})

export const revokeExclusiveInvite = (inviteId: string): RpcResult<Record<string, unknown>> =>
  callSession<Record<string, unknown>>('user.exclusive_invite.revoke', { invite_id: inviteId })

export const rotateExclusiveInvite = (inviteId: string): RpcResult<ExclusiveInviteLink> =>
  callSession<ExclusiveInviteLink>('user.exclusive_invite.rotate', { invite_id: inviteId })

export const retryExclusiveInviteResource = (inviteId: string): RpcResult<Record<string, unknown>> =>
  callSession<Record<string, unknown>>('user.exclusive_invite.retry_resource', { invite_id: inviteId })

// ── App 代登录 (EXT-09 / EXT-11): the App owner's switch per App instance ──

export interface AppLoginSettings {
  enabled: boolean
  updated_by: string
  updated_at: number
}

export interface AppLoginState {
  settings: AppLoginSettings
  /** Limited accounts this App instance has signed in so far. */
  bound_accounts: number
}

export const fetchAppLoginState = (appInstanceId: string): RpcResult<AppLoginState> =>
  callSession<AppLoginState>('apps.app_login.get', { app_instance_id: appInstanceId })

export const setAppLoginEnabled = (appInstanceId: string, enabled: boolean): RpcResult<{ settings: AppLoginSettings; families_revoked: number }> =>
  callSession<{ settings: AppLoginSettings; families_revoked: number }>('apps.app_login.set', { app_instance_id: appInstanceId, enabled })
