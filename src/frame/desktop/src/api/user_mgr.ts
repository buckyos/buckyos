import { callRpc, type RpcCallOptions } from './rpc.ts'

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/** Possible user types. Mirrors `UserType` in `buckyos-api/src/control_panel.rs`. */
export type UserType = 'admin' | 'user' | 'root' | 'limited' | 'guest'

/**
 * User state as serialized by the backend. Active/Deleted are plain strings;
 * Suspended/Banned serialize as `"suspended:{reason}"` / `"banned:{reason}"`.
 */
export type UserStateString =
  | 'active'
  | 'pending'
  | 'deleted'
  | `suspended:${string}`
  | `banned:${string}`
  | string

/** Single binding between a user/agent and an external message tunnel. */
export interface UserTunnelBinding {
  platform: string
  account_id: string
  display_id?: string | null
  tunnel_instance_id?: string | null
  status?: string | null
  last_sync_at?: number | null
  meta?: Record<string, string>
}

/**
 * System-level contact settings stored in `UserPrivateProfile.private_extra`. Full
 * contact/friend management lives in MessageCenter; this block only holds
 * the account-level binding info needed for user management UIs.
 */
export interface UserContactSettings {
  did?: string | null
  note?: string | null
  groups?: string[]
  tags?: string[]
  bindings?: UserTunnelBinding[]
}

export interface ProfileLink {
  label: string
  url: string
}

export type ProfileContact = {
  platform: string
  [key: string]: unknown
}

export interface UserProfile {
  did: string
  name?: string | null
  display_name?: string | null
  avatar?: string | null
  meta?: unknown
  headline?: string | null
  title?: string | null
  bio?: string | null
  location?: string | null
  organization?: string | null
  birthday?: string | null
  tags?: string[]
  bkg_image?: string | null
  links?: Record<string, ProfileLink>
  public_contacts?: Record<string, ProfileContact>
  [key: string]: unknown
}

export interface UserPrivateProfile extends UserProfile {
  privacy?: Record<string, unknown>
  private_contacts?: Record<string, ProfileContact>
  private_meta?: unknown
  private_extra?: Record<string, unknown>
}

/** Minimal user listing entry as returned by `user.list`. */
export interface UserInfo {
  user_id: string
  show_name: string
  user_type: UserType | string
  state: UserStateString
  is_local?: boolean
  allow_password_change?: boolean
}

export interface UsersListResponse {
  total: number
  users: UserInfo[]
}

/** Full user detail as returned by `user.get`. */
export interface UserDetail {
  user_id: string
  show_name?: string | null
  user_type: UserType | string
  state: UserStateString
  res_pool_id: string
  is_local?: boolean
  profile?: UserProfile | Record<string, unknown> | null
  local_profile?: UserPrivateProfile | null
  allow_password_change?: boolean
  /** Compatibility field; source of truth is `local_profile.private_extra.system_contact`. */
  contact?: UserContactSettings
  /** Optional DID document loaded from `users/{uid}/doc`. */
  did_document?: Record<string, unknown>
}

export interface SimpleOkResponse {
  ok: boolean
  user_id?: string
  [key: string]: unknown
}

export interface UserCreateResponse extends SimpleOkResponse {
  created: boolean
  rbac_refreshed: boolean
  user_id: string
  user_type: UserType | string
  state: UserStateString
  warning?: string
}

export interface UserProfileResponse {
  user_id: string
  profile: UserProfile | Record<string, unknown>
  local_profile?: UserPrivateProfile | null
  did_profile?: Record<string, unknown> | null
  is_local?: boolean
}

export interface UserInviteRecord {
  invite_id: string
  created_by: string
  created_at: number
  expires_at?: number | null
  state: 'pending' | 'accepted' | string
  target_user_id?: string | null
  target_did?: string | null
  show_name?: string | null
  default_user_type: UserType | string
  groups?: string[]
  accepted_at?: number | null
  accepted_user_id?: string | null
}

export interface UserInviteResponse {
  invite: UserInviteRecord
  expired?: boolean
  zone_did?: string
  zone_host?: string
  invite_url?: string
  ok?: boolean
  user_id?: string
  state?: UserStateString
}

/** Agent listing entry as returned by `agent.list`. */
export interface AgentInfo {
  agent_id: string
  [key: string]: unknown
}

export interface AgentsListResponse {
  total: number
  agents: AgentInfo[]
}

/** Full agent detail as returned by `agent.get`. */
export interface AgentDetail {
  agent_id: string
  /** Optional settings block merged in from `agents/{agent_id}/settings`. */
  settings?: Record<string, unknown>
  runtime?: Record<string, unknown>
  [key: string]: unknown
}

export interface AgentTunnelBindingResponse {
  ok: boolean
  agent_id: string
  platform: string
  total_bindings?: number
  remaining_bindings?: number
}

export interface AgentProfileResponse {
  agent_id: string
  profile: Record<string, unknown>
  local_profile?: Record<string, unknown> | null
  did_profile?: Record<string, unknown> | null
}

// ---------------------------------------------------------------------------
// User management RPC
// ---------------------------------------------------------------------------

/** List all users visible to the caller. */
export const fetchUserList = async (): Promise<{
  data: UsersListResponse | null
  error: unknown
}> => callRpc<UsersListResponse>('user.list', {})

/** Get a single user's detail. Defaults to the caller when `user_id` omitted. */
export const fetchUserDetail = async (
  options: { userId?: string } = {},
): Promise<{ data: UserDetail | null; error: unknown }> => {
  const params: Record<string, unknown> = {}
  if (options.userId) params.user_id = options.userId
  return callRpc<UserDetail>('user.get', params)
}

/** Create a new user. Admin-only. */
export const createUser = async (input: {
  userId: string
  passwordHash: string
  showName?: string
  userType?: Exclude<UserType, 'root'>
  allowPasswordChange?: boolean
}, options: RpcCallOptions = {}): Promise<{
  data: UserCreateResponse | null
  error: unknown
}> => {
  const params: Record<string, unknown> = {
    user_id: input.userId,
    password_hash: input.passwordHash,
  }
  if (input.showName !== undefined) params.show_name = input.showName
  if (input.userType !== undefined) params.user_type = input.userType
  if (input.allowPasswordChange !== undefined) {
    params.allow_password_change = input.allowPasswordChange
  }
  return callRpc<UserCreateResponse>('user.create', params, options)
}

/** Update basic user profile fields (currently `display_name`, via `show_name` RPC param). */
export const updateUser = async (input: {
  userId?: string
  showName?: string
}): Promise<{ data: SimpleOkResponse | null; error: unknown }> => {
  const params: Record<string, unknown> = {}
  if (input.userId) params.user_id = input.userId
  if (input.showName !== undefined) params.show_name = input.showName
  return callRpc<SimpleOkResponse>('user.update', params)
}

/**
 * Update the system-level contact profile data for a user. Partial update — only
 * fields present in `input` are written. For full contact/friend management
 * use the MessageCenter RPCs.
 */
export const updateUserContact = async (input: {
  userId?: string
  did?: string
  note?: string
  groups?: string[]
  tags?: string[]
  bindings?: UserTunnelBinding[]
}): Promise<{
  data: (SimpleOkResponse & { contact?: UserContactSettings }) | null
  error: unknown
}> => {
  const params: Record<string, unknown> = {}
  if (input.userId) params.user_id = input.userId
  if (input.did !== undefined) params.did = input.did
  if (input.note !== undefined) params.note = input.note
  if (input.groups !== undefined) params.groups = input.groups
  if (input.tags !== undefined) params.tags = input.tags
  if (input.bindings !== undefined) params.bindings = input.bindings
  return callRpc<SimpleOkResponse & { contact?: UserContactSettings }>(
    'user.update_contact',
    params,
  )
}

export const fetchUserProfile = async (
  userId?: string,
): Promise<{ data: UserProfileResponse | null; error: unknown }> => {
  const params: Record<string, unknown> = {}
  if (userId) params.user_id = userId
  return callRpc<UserProfileResponse>('user.profile.get', params)
}

export const setUserProfile = async (input: {
  userId?: string
  profile?: UserPrivateProfile
  name?: string
  displayName?: string
  avatar?: string
  avatarUrl?: string
  headline?: string
  title?: string
  bio?: string
  location?: string
  organization?: string
  birthday?: string
  bkgImage?: string
  tags?: string[]
  website?: string
  email?: string
  phone?: string
  privateExtra?: Record<string, unknown>
  extra?: Record<string, unknown>
}): Promise<{ data: (SimpleOkResponse & { profile?: UserPrivateProfile }) | null; error: unknown }> => {
  const params: Record<string, unknown> = {}
  if (input.userId) params.user_id = input.userId
  if (input.profile !== undefined) params.profile = input.profile
  if (input.name !== undefined) params.name = input.name
  if (input.displayName !== undefined) params.display_name = input.displayName
  if (input.avatar !== undefined) params.avatar = input.avatar
  if (input.avatarUrl !== undefined) params.avatar = input.avatarUrl
  if (input.headline !== undefined) params.headline = input.headline
  if (input.title !== undefined) params.title = input.title
  if (input.bio !== undefined) params.bio = input.bio
  if (input.location !== undefined) params.location = input.location
  if (input.organization !== undefined) params.organization = input.organization
  if (input.birthday !== undefined) params.birthday = input.birthday
  if (input.bkgImage !== undefined) params.bkg_image = input.bkgImage
  if (input.tags !== undefined) params.tags = input.tags
  if (input.website !== undefined) params.website = input.website
  if (input.email !== undefined) params.email = input.email
  if (input.phone !== undefined) params.phone = input.phone
  if (input.privateExtra !== undefined) params.private_extra = input.privateExtra
  if (input.extra !== undefined) params.extra = input.extra
  return callRpc<SimpleOkResponse & { profile?: UserPrivateProfile }>('user.profile.set', params)
}

export const setUserMsgTunnel = async (input: {
  userId?: string
  platform: string
  accountId: string
  displayId?: string
  tunnelId?: string
  status?: string
  lastSyncAt?: number
  meta?: Record<string, string>
}): Promise<{
  data: (SimpleOkResponse & {
    platform?: string
    total_bindings?: number
    contact?: UserContactSettings
  }) | null
  error: unknown
}> => {
  const params: Record<string, unknown> = {
    platform: input.platform,
    account_id: input.accountId,
  }
  if (input.userId) params.user_id = input.userId
  if (input.displayId !== undefined) params.display_id = input.displayId
  if (input.tunnelId !== undefined) params.tunnel_instance_id = input.tunnelId
  if (input.status !== undefined) params.status = input.status
  if (input.lastSyncAt !== undefined) params.last_sync_at = input.lastSyncAt
  if (input.meta !== undefined) params.meta = input.meta
  return callRpc<
    SimpleOkResponse & {
      platform?: string
      total_bindings?: number
      contact?: UserContactSettings
    }
  >('user.set_msg_tunnel', params)
}

export const removeUserMsgTunnel = async (input: {
  userId?: string
  platform: string
}): Promise<{
  data: (SimpleOkResponse & {
    platform?: string
    remaining_bindings?: number
    contact?: UserContactSettings
  }) | null
  error: unknown
}> => {
  const params: Record<string, unknown> = { platform: input.platform }
  if (input.userId) params.user_id = input.userId
  return callRpc<
    SimpleOkResponse & {
      platform?: string
      remaining_bindings?: number
      contact?: UserContactSettings
    }
  >('user.remove_msg_tunnel', params)
}

export const createUserInvite = async (input: {
  inviteId?: string
  userId?: string
  targetDid?: string
  showName?: string
  userType?: Exclude<UserType, 'root'>
  expiresAt?: number
  ttlSecs?: number
  groups?: string[]
}): Promise<{ data: UserInviteResponse | null; error: unknown }> => {
  const params: Record<string, unknown> = {}
  if (input.inviteId !== undefined) params.invite_id = input.inviteId
  if (input.userId !== undefined) params.user_id = input.userId
  if (input.targetDid !== undefined) params.target_did = input.targetDid
  if (input.showName !== undefined) params.show_name = input.showName
  if (input.userType !== undefined) params.user_type = input.userType
  if (input.expiresAt !== undefined) params.expires_at = input.expiresAt
  if (input.ttlSecs !== undefined) params.ttl_secs = input.ttlSecs
  if (input.groups !== undefined) params.groups = input.groups
  return callRpc<UserInviteResponse>('user.invite.create', params)
}

export const fetchUserInvite = async (
  inviteId: string,
): Promise<{ data: UserInviteResponse | null; error: unknown }> =>
  callRpc<UserInviteResponse>('user.invite.get', { invite_id: inviteId })

export const acceptUserInvite = async (input: {
  inviteId: string
  ownerConfig: Record<string, unknown> | string
  passwordHash?: string
}): Promise<{ data: UserInviteResponse | null; error: unknown }> => {
  const params: Record<string, unknown> = {
    invite_id: input.inviteId,
    owner_config: input.ownerConfig,
  }
  if (input.passwordHash !== undefined) params.password_hash = input.passwordHash
  return callRpc<UserInviteResponse>('user.invite.accept', params)
}

/** Soft-delete a user (state → `deleted`). Admin-only; cannot delete root or self. */
export const deleteUser = async (
  userId: string,
): Promise<{ data: SimpleOkResponse | null; error: unknown }> =>
  callRpc<SimpleOkResponse>('user.delete', { user_id: userId })

/** Change a user's password hash. Allowed for self or admin. */
export const changeUserPassword = async (input: {
  userId?: string
  newPasswordHash: string
}): Promise<{ data: SimpleOkResponse | null; error: unknown }> => {
  const params: Record<string, unknown> = {
    new_password_hash: input.newPasswordHash,
  }
  if (input.userId) params.user_id = input.userId
  return callRpc<SimpleOkResponse>('user.change_password', params)
}

/** Change a user's state. Admin-only. `state` uses the raw encoding (e.g. `"suspended:abuse"`). */
export const changeUserState = async (input: {
  userId: string
  state: UserStateString
}): Promise<{ data: SimpleOkResponse | null; error: unknown }> =>
  callRpc<SimpleOkResponse>('user.change_state', {
    user_id: input.userId,
    state: input.state,
  })

/** Change a user's type. Admin-only; cannot promote to root or change root's type. */
export const changeUserType = async (input: {
  userId: string
  userType: Exclude<UserType, 'root'>
}): Promise<{ data: SimpleOkResponse | null; error: unknown }> =>
  callRpc<SimpleOkResponse>('user.change_type', {
    user_id: input.userId,
    user_type: input.userType,
  })

// ---------------------------------------------------------------------------
// Agent management RPC
// ---------------------------------------------------------------------------

/** List all agents in the zone. */
export const fetchAgentList = async (): Promise<{
  data: AgentsListResponse | null
  error: unknown
}> => callRpc<AgentsListResponse>('agent.list', {})

export const fetchAgentListWithRuntime = async (): Promise<{
  data: AgentsListResponse | null
  error: unknown
}> => callRpc<AgentsListResponse>('agent.list', { include_runtime: true })

/** Get the full detail for a single agent (doc + optional settings). */
export const fetchAgentDetail = async (
  agentId: string,
): Promise<{ data: AgentDetail | null; error: unknown }> =>
  callRpc<AgentDetail>('agent.get', { agent_id: agentId })

export const createAgent = async (input: {
  agentId: string
  displayName?: string
  ownerUserId?: string
  agentDid?: string
  description?: string
  profile?: Record<string, unknown>
  settings?: Record<string, unknown>
}): Promise<{ data: (SimpleOkResponse & AgentDetail) | null; error: unknown }> => {
  const params: Record<string, unknown> = { agent_id: input.agentId }
  if (input.displayName !== undefined) params.display_name = input.displayName
  if (input.ownerUserId !== undefined) params.owner_user_id = input.ownerUserId
  if (input.agentDid !== undefined) params.agent_did = input.agentDid
  if (input.description !== undefined) params.description = input.description
  if (input.profile !== undefined) params.profile = input.profile
  if (input.settings !== undefined) params.settings = input.settings
  return callRpc<SimpleOkResponse & AgentDetail>('agent.create', params)
}

export const updateAgent = async (input: {
  agentId: string
  displayName?: string
  description?: string
  state?: string
  profile?: Record<string, unknown>
  settings?: Record<string, unknown>
}): Promise<{ data: (SimpleOkResponse & { settings?: Record<string, unknown> }) | null; error: unknown }> => {
  const params: Record<string, unknown> = { agent_id: input.agentId }
  if (input.displayName !== undefined) params.display_name = input.displayName
  if (input.description !== undefined) params.description = input.description
  if (input.state !== undefined) params.state = input.state
  if (input.profile !== undefined) params.profile = input.profile
  if (input.settings !== undefined) params.settings = input.settings
  return callRpc<SimpleOkResponse & { settings?: Record<string, unknown> }>(
    'agent.update',
    params,
  )
}

export const deleteAgent = async (
  agentId: string,
): Promise<{ data: SimpleOkResponse | null; error: unknown }> =>
  callRpc<SimpleOkResponse>('agent.delete', { agent_id: agentId })

export const fetchAgentProfile = async (
  agentId: string,
): Promise<{ data: AgentProfileResponse | null; error: unknown }> =>
  callRpc<AgentProfileResponse>('agent.profile.get', { agent_id: agentId })

export const setAgentProfile = async (input: {
  agentId: string
  profile: Record<string, unknown>
}): Promise<{ data: (SimpleOkResponse & { profile?: Record<string, unknown> }) | null; error: unknown }> =>
  callRpc<SimpleOkResponse & { profile?: Record<string, unknown> }>('agent.profile.set', {
    agent_id: input.agentId,
    profile: input.profile,
  })

/**
 * Set (add or replace) a message-tunnel binding for an agent. Admin-only.
 * Bindings are keyed by `platform` — passing the same platform replaces the
 * existing binding.
 */
export const setAgentMsgTunnel = async (input: {
  agentId: string
  platform: string
  accountId: string
  displayId?: string
  tunnelId?: string
  status?: string
  lastSyncAt?: number
  meta?: Record<string, string>
}): Promise<{ data: AgentTunnelBindingResponse | null; error: unknown }> => {
  const params: Record<string, unknown> = {
    agent_id: input.agentId,
    platform: input.platform,
    account_id: input.accountId,
  }
  if (input.displayId !== undefined) params.display_id = input.displayId
  if (input.tunnelId !== undefined) params.tunnel_instance_id = input.tunnelId
  if (input.status !== undefined) params.status = input.status
  if (input.lastSyncAt !== undefined) params.last_sync_at = input.lastSyncAt
  if (input.meta !== undefined) params.meta = input.meta
  return callRpc<AgentTunnelBindingResponse>('agent.set_msg_tunnel', params)
}

/** Remove a specific platform binding from an agent. Admin-only. */
export const removeAgentMsgTunnel = async (input: {
  agentId: string
  platform: string
}): Promise<{ data: AgentTunnelBindingResponse | null; error: unknown }> =>
  callRpc<AgentTunnelBindingResponse>('agent.remove_msg_tunnel', {
    agent_id: input.agentId,
    platform: input.platform,
  })
