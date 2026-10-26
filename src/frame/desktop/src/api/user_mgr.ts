import { callRpc, type RpcCallOptions } from './rpc.ts'
import type { AssignableUserType, ContactProjectionRule, UserType } from './user_type.ts'
import { isMockRuntime } from '../runtime.ts'

/** Agent and own-profile calls; the mock runtime answers them from an in-browser control panel. */
async function callControlPanel<T>(
  method: string,
  params: Record<string, unknown>,
  options: RpcCallOptions = {},
): Promise<{ data: T | null; error: unknown }> {
  if (!isMockRuntime()) return callRpc<T>(method, params, options)
  const { callMockControlPanel } = await import('./control_panel_mock.ts')
  return callMockControlPanel<T>(method, params)
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type { AssignableUserType, ContactProjectionRule, UserType }

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

/** Minimal user listing entry as returned by `user.list`; `did`/`avatar` are the public profile's. */
export interface UserInfo {
  user_id: string
  show_name: string
  did?: string
  avatar?: string
  user_type: UserType | string
  state: UserStateString
  is_local?: boolean
  /** Raw per-account override; null/absent means the type default. */
  allow_password_change?: boolean | null
  allow_contact_edit?: boolean | null
  /** Effective values. */
  can_change_password?: boolean
  can_edit_contacts?: boolean
  /** System user group ids. */
  groups?: string[]
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
  /** Effective: the account may change its own password. */
  allow_password_change?: boolean
  /** Effective: the account may edit its own address book. */
  allow_contact_edit?: boolean
  /** System user group ids. */
  groups?: string[]
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

// ---------------------------------------------------------------------------
// Agent types (control_panel `agent.*`, see the Agent init contract §3)
// ---------------------------------------------------------------------------

export type AgentInstallState = 'provisioning' | 'bound' | 'ready' | 'failed' | 'removed'
export type AgentCreateStep = 'runtime' | 'bind' | 'tunnel' | 'start' | 'done'
export type AgentTunnelState = 'none' | 'pending' | 'bound' | 'failed' | 'skipped'
export type AgentTemplateSource = 'bundled' | 'installed'
export type AgentNameUnavailableReason = 'invalid' | 'reserved' | 'user_exists' | 'agent_exists' | 'host_taken'
export type AgentDesktopEntry = 'jarvis_guide'
export type AgentPermissionMode = 'read_write' | 'read_only'

export interface AgentCreateError {
  step: AgentCreateStep
  code: string
  message: string
  retryable: boolean
}

export interface AgentRuntimeProgress {
  phase?: string | null
  percent?: number | null
  message?: string | null
}

/** Creation status; also the `install` field of an `AgentEntry`. */
export interface AgentStatus {
  agent_id: string
  agent_did: string
  owner_user_id: string
  state: AgentInstallState
  step: AgentCreateStep
  last_error?: AgentCreateError | null
  tunnel_state: AgentTunnelState
  runtime_task_id?: string | null
  runtime_progress?: AgentRuntimeProgress | null
  created_at: number
  updated_at: number
}

export interface AgentProfile {
  display_name?: string | null
  /** Data URL, at most 192px. */
  avatar?: string | null
  bio?: string | null
}

/** A message tunnel of an Agent as listed; never carries the bot token. */
export interface AgentMsgTunnelSummary {
  platform: string
  bot_account_id?: string | null
}

export interface AgentSettings {
  enabled: boolean
  auto_start: boolean
  allow_other_users: boolean
  allow_group: boolean
  role_supplement: string
  template_auto_update: boolean
  desktop_entry?: string | null
  /** The Agent has a HomeStation of its own and follows / is followed by its owner and the zone's Agents. */
  homestation?: boolean
  /** After a user task ends, the Agent posts a privacy-free note about it to its HomeStation. */
  auto_share_results?: boolean
  msg_tunnels: AgentMsgTunnelSummary[]
  /** Execution runtimes beyond the Agent's container, by runtime id (`host` = the owner's desktop). */
  runtimes?: Record<string, AgentRuntimeSetting>
  /** The runtime new sessions start in (`container` when none is chosen). */
  default_runtime_id?: string
}

export type ComputerAccess = 'none' | 'observe' | 'control'

/** What the owner declares a runtime provides: decides which cmd_skills its sessions take. */
export type RuntimeBaseline = 'standard' | 'extended'

export interface AgentRuntimeSetting {
  enabled: boolean
  computer: ComputerAccess
  target: { kind: 'node'; node_id: string; node_did: string } | { kind: 'executor'; executor_id: string }
  baseline?: RuntimeBaseline | null
}

/** One machine the owner's Agents may run on (`agent.runtime_directory`). */
export interface AgentRuntimeOffer {
  runtime_id: string
  kind: 'node' | 'executor'
  node_id?: string
  node_did?: string
  device_type?: string
  name?: string
  computer_max: ComputerAccess
  workdir_roots?: string[]
  /** The policy exists but the administrator closed it. */
  closed: boolean
  /** `linux | windows | apple` as the node reports it. */
  os?: string
  /** The node no longer offers it (the Agent's setting still names it). */
  withdrawn?: boolean
  enabled: boolean
  computer: ComputerAccess
  baseline: RuntimeBaseline
}

export interface AgentRuntimeDirectory {
  runtimes: AgentRuntimeOffer[]
  default_runtime_id: string | null
  container: string
  /** A node of this zone runs Agent containers. */
  container_available?: boolean
  /** A new Agent runs on `host` unless the owner chooses its container
   * (Windows / macOS hosts; on Linux the container is the default). */
  host_by_default?: boolean
  /** How this Agent runs its Agent Service: natively on the node of its
   * host default runtime, or in its container. */
  agent_service?: { mode: 'native' | 'container'; node_id?: string }
}

export interface AgentTemplateBinding {
  template_id: string
  source: AgentTemplateSource
  app_did: string
  version: string
  loaded_version?: string | null
}

/** The Agent's constructed App; null until that App is installed. */
export interface AgentRuntimeBinding {
  app_instance_id: string
  app_host_name: string
  has_web: boolean
  state?: string | null
}

export interface AgentEntry {
  agent_id: string
  agent_did: string
  owner_user_id: string
  owner_did: string
  permission_mode: AgentPermissionMode
  name: string
  display_name: string
  profile: AgentProfile
  settings: AgentSettings
  install: AgentStatus
  template?: AgentTemplateBinding | null
  runtime?: AgentRuntimeBinding | null
}

export interface AgentsListResponse {
  agents: AgentEntry[]
}

export interface AgentTemplate {
  /** `bundled:<app_id>` or `installed:<app_id>`. */
  template_id: string
  source: AgentTemplateSource
  app_id: string
  app_did: string
  name: string
  show_name: string
  description: string
  version: string
  icon?: string | null
  loader: string
  is_default: boolean
}

export interface AgentTemplatesResponse {
  templates: AgentTemplate[]
}

export interface AgentNameCheck {
  name: string
  available: boolean
  agent_id: string
  agent_did: string
  reason?: AgentNameUnavailableReason | string | null
  message?: string | null
  suggestion?: string | null
}

export interface AgentMsgTunnelInput {
  platform: 'telegram'
  bot_token: string
}

export interface AgentCreateRequest {
  idempotency_key: string
  name: string
  profile: AgentProfile
  role_supplement: string
  permission_mode: AgentPermissionMode
  allow_group: boolean
  allow_other_users: boolean
  homestation?: boolean
  auto_share_results?: boolean
  template_id: string
  template_auto_update: boolean
  desktop_entry?: AgentDesktopEntry
  msg_tunnel?: AgentMsgTunnelInput
  runtimes?: Record<string, { enabled?: boolean; computer?: ComputerAccess } | null>
  default_runtime_id?: string | null
}

export interface AgentCreateResponse {
  agent_id: string
  agent_did: string
  status: AgentStatus
}

// ---------------------------------------------------------------------------
// User management RPC
// ---------------------------------------------------------------------------

/** List all users visible to the caller. */
export const fetchUserList = async (): Promise<{
  data: UsersListResponse | null
  error: unknown
}> => callControlPanel<UsersListResponse>('user.list', {})

/** Get a single user's detail. Defaults to the caller when `user_id` omitted. */
export const fetchUserDetail = async (
  options: { userId?: string } = {},
): Promise<{ data: UserDetail | null; error: unknown }> => {
  const params: Record<string, unknown> = {}
  if (options.userId) params.user_id = options.userId
  return callControlPanel<UserDetail>('user.get', params)
}

/** Create a new user (User or Limited). Admin-only. */
export const createUser = async (input: {
  userId: string
  passwordHash: string
  showName?: string
  userType?: Exclude<AssignableUserType, 'admin'>
  groups?: string[]
  allowPasswordChange?: boolean
  allowContactEdit?: boolean
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
  if (input.groups !== undefined) params.groups = input.groups
  if (input.allowPasswordChange !== undefined) {
    params.allow_password_change = input.allowPasswordChange
  }
  if (input.allowContactEdit !== undefined) {
    params.allow_contact_edit = input.allowContactEdit
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

/**
 * `user.profile.set`. A text field left `undefined` is kept; `null` (or a blank string) clears the
 * local value so the DID document's value, if any, shows again. `avatar` must be a base64
 * PNG/JPEG/WebP data URL of at most 192px.
 */
export const setUserProfile = async (input: {
  userId?: string
  profile?: UserPrivateProfile
  name?: string | null
  displayName?: string | null
  avatar?: string | null
  headline?: string | null
  title?: string | null
  bio?: string | null
  location?: string | null
  organization?: string | null
  birthday?: string | null
  bkgImage?: string | null
  tags?: string[]
  website?: string | null
  email?: string | null
  phone?: string | null
  privateExtra?: Record<string, unknown>
  extra?: Record<string, unknown>
}): Promise<{ data: (SimpleOkResponse & { profile?: UserPrivateProfile }) | null; error: unknown }> => {
  const params: Record<string, unknown> = {}
  if (input.userId) params.user_id = input.userId
  if (input.profile !== undefined) params.profile = input.profile
  if (input.name !== undefined) params.name = input.name
  if (input.displayName !== undefined) params.display_name = input.displayName
  if (input.avatar !== undefined) params.avatar = input.avatar
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
  return callControlPanel<SimpleOkResponse & { profile?: UserPrivateProfile }>('user.profile.set', params)
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
  return callControlPanel<
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
  return callControlPanel<
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
  userType?: AssignableUserType
  expiresAt?: number
  ttlSecs?: number
  /** System user group ids. */
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
  callControlPanel<UserInviteResponse>('user.invite.get', { invite_id: inviteId }, { sessionToken: null })

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
  return callControlPanel<UserInviteResponse>('user.invite.accept', params)
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

/** Change a user's type between Admin, User and Limited. Admin-only; the owner record cannot change. */
export const changeUserType = async (input: {
  userId: string
  userType: AssignableUserType
}): Promise<{ data: SimpleOkResponse | null; error: unknown }> =>
  callControlPanel<SimpleOkResponse>('user.change_type', {
    user_id: input.userId,
    user_type: input.userType,
  })

export interface UserAccountOptionsResponse extends SimpleOkResponse {
  user_id: string
  allow_password_change: boolean
  allow_contact_edit: boolean
}

/** Per-account overrides; `null` restores the account type's default. Admin-only. */
export const setUserAccountOptions = async (input: {
  userId: string
  allowPasswordChange?: boolean | null
  allowContactEdit?: boolean | null
}): Promise<{ data: UserAccountOptionsResponse | null; error: unknown }> => {
  const params: Record<string, unknown> = { user_id: input.userId }
  if (input.allowPasswordChange !== undefined) params.allow_password_change = input.allowPasswordChange
  if (input.allowContactEdit !== undefined) params.allow_contact_edit = input.allowContactEdit
  return callControlPanel<UserAccountOptionsResponse>('user.set_account_options', params)
}

// ---------------------------------------------------------------------------
// System user groups and contact projection (admin-only)
// ---------------------------------------------------------------------------

export interface UserGroup {
  group_id: string
  name: string
  description?: string | null
  updated_by: string
  created_at: number
  updated_at: number
}

export interface UserGroupListItem extends UserGroup {
  members: string[]
}

export interface ContactProjections {
  rules: ContactProjectionRule[]
  /** No rules were saved; the Zone uses the default rules. */
  is_default: boolean
  updated_by?: string
  updated_at?: number
}

export interface UserGroupListResponse {
  groups: UserGroupListItem[]
  projections: ContactProjections
}

export const fetchUserGroups = async (): Promise<{ data: UserGroupListResponse | null; error: unknown }> =>
  callControlPanel<UserGroupListResponse>('user_group.list', {})

export const createUserGroup = async (input: {
  groupId: string
  name: string
  description?: string
}): Promise<{ data: { ok: boolean; group: UserGroup } | null; error: unknown }> => {
  const params: Record<string, unknown> = { group_id: input.groupId, name: input.name }
  if (input.description !== undefined) params.description = input.description
  return callControlPanel<{ ok: boolean; group: UserGroup }>('user_group.create', params)
}

export const updateUserGroup = async (input: {
  groupId: string
  name?: string
  description?: string
}): Promise<{ data: { ok: boolean; group: UserGroup } | null; error: unknown }> => {
  const params: Record<string, unknown> = { group_id: input.groupId }
  if (input.name !== undefined) params.name = input.name
  if (input.description !== undefined) params.description = input.description
  return callControlPanel<{ ok: boolean; group: UserGroup }>('user_group.update', params)
}

export interface UserGroupDeleteResponse {
  ok: boolean
  group_id: string
  removed_members: number
  removed_rules: number
}

export const deleteUserGroup = async (
  groupId: string,
): Promise<{ data: UserGroupDeleteResponse | null; error: unknown }> =>
  callControlPanel<UserGroupDeleteResponse>('user_group.delete', { group_id: groupId })

export const setUserGroupMembers = async (input: {
  groupId: string
  add?: string[]
  remove?: string[]
}): Promise<{ data: { ok: boolean; group_id: string; members: string[] } | null; error: unknown }> => {
  const params: Record<string, unknown> = { group_id: input.groupId }
  if (input.add !== undefined) params.add = input.add
  if (input.remove !== undefined) params.remove = input.remove
  return callControlPanel<{ ok: boolean; group_id: string; members: string[] }>('user_group.set_members', params)
}

export const setContactProjections = async (
  rules: ContactProjectionRule[],
): Promise<{ data: { ok: boolean; projections: ContactProjections } | null; error: unknown }> =>
  callControlPanel<{ ok: boolean; projections: ContactProjections }>('user_group.set_projections', { rules })

export const resetContactProjections = async (): Promise<{
  data: { ok: boolean; projections: ContactProjections } | null
  error: unknown
}> => callControlPanel<{ ok: boolean; projections: ContactProjections }>('user_group.reset_projections', {})

// ---------------------------------------------------------------------------
// Agent management RPC
// ---------------------------------------------------------------------------

export const checkAgentName = async (
  name: string,
): Promise<{ data: AgentNameCheck | null; error: unknown }> =>
  callControlPanel<AgentNameCheck>('agent.check_name', { name })

/** Templates the caller can build an Agent from; an error never falls back to a fixed list. */
export const fetchAgentTemplates = async (): Promise<{
  data: AgentTemplatesResponse | null
  error: unknown
}> => callControlPanel<AgentTemplatesResponse>('agent.list_templates', {})

export const createAgent = async (
  request: AgentCreateRequest,
): Promise<{ data: AgentCreateResponse | null; error: unknown }> =>
  callControlPanel<AgentCreateResponse>('agent.create', { ...request })

export const fetchAgentCreateStatus = async (
  agentId: string,
): Promise<{ data: AgentStatus | null; error: unknown }> =>
  callControlPanel<AgentStatus>('agent.create.status', { agent_id: agentId })

export const retryAgentCreate = async (input: {
  agentId: string
  skipTunnel?: boolean
}): Promise<{ data: AgentStatus | null; error: unknown }> =>
  callControlPanel<AgentStatus>('agent.create.retry', {
    agent_id: input.agentId,
    ...(input.skipTunnel ? { skip_tunnel: true } : {}),
  })

/** Only valid before the Agent's spec is written (runtime / bind failures). */
export const cancelAgentCreate = async (
  agentId: string,
): Promise<{ data: SimpleOkResponse | null; error: unknown }> =>
  callControlPanel<SimpleOkResponse>('agent.create.cancel', { agent_id: agentId })

/** The caller's own Agents (Admin / Root: every Agent), including unfinished creations. */
export const fetchAgentList = async (): Promise<{
  data: AgentsListResponse | null
  error: unknown
}> => callControlPanel<AgentsListResponse>('agent.list', {})

export const fetchAgentDetail = async (
  agentId: string,
): Promise<{ data: AgentEntry | null; error: unknown }> =>
  callControlPanel<AgentEntry>('agent.get', { agent_id: agentId })

export const updateAgent = async (input: {
  agentId: string
  allowGroup?: boolean
  homestation?: boolean
  autoShareResults?: boolean
  /** `{ "<id>": { enabled?, computer?, baseline? } | null }`; targets come from the directory, never from here. */
  runtimes?: Record<string, { enabled?: boolean; computer?: ComputerAccess; baseline?: RuntimeBaseline | null } | null>
  /** `null` or `'container'`: the Agent's own environment. */
  defaultRuntimeId?: string | null
}): Promise<{ data: AgentEntry | null; error: unknown }> => {
  const params: Record<string, unknown> = { agent_id: input.agentId }
  if (input.allowGroup !== undefined) params.allow_group = input.allowGroup
  if (input.homestation !== undefined) params.homestation = input.homestation
  if (input.autoShareResults !== undefined) params.auto_share_results = input.autoShareResults
  if (input.runtimes !== undefined) params.runtimes = input.runtimes
  if (input.defaultRuntimeId !== undefined) params.default_runtime_id = input.defaultRuntimeId
  return callControlPanel<AgentEntry>('agent.update', params)
}

/** The machines the caller's Agents may run on; with `agentId`, that Agent's choice too. */
export const fetchAgentRuntimeDirectory = async (
  agentId?: string,
): Promise<{ data: AgentRuntimeDirectory | null; error: unknown }> =>
  callControlPanel<AgentRuntimeDirectory>('agent.runtime_directory', agentId ? { agent_id: agentId } : {})

export const fetchAgentProfile = async (
  agentId: string,
): Promise<{ data: { profile: AgentProfile } | null; error: unknown }> =>
  callControlPanel<{ profile: AgentProfile }>('agent.profile.get', { agent_id: agentId })

export const setAgentProfile = async (input: {
  agentId: string
  displayName?: string
  avatar?: string
  bio?: string
}): Promise<{ data: { profile: AgentProfile } | null; error: unknown }> => {
  const params: Record<string, unknown> = { agent_id: input.agentId }
  if (input.displayName !== undefined) params.display_name = input.displayName
  if (input.avatar !== undefined) params.avatar = input.avatar
  if (input.bio !== undefined) params.bio = input.bio
  return callControlPanel<{ profile: AgentProfile }>('agent.profile.set', params)
}

export const deleteAgent = async (
  agentId: string,
): Promise<{ data: SimpleOkResponse | null; error: unknown }> =>
  callControlPanel<SimpleOkResponse>('agent.delete', { agent_id: agentId })
