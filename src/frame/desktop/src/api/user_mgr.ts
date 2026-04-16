import { callRpc } from './rpc.ts'

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
  | 'deleted'
  | `suspended:${string}`
  | `banned:${string}`
  | string

/** Single binding between a user/agent and an external message tunnel. */
export interface UserTunnelBinding {
  platform: string
  account_id: string
  display_id?: string | null
  tunnel_id?: string | null
  meta?: Record<string, string>
}

/**
 * System-level contact settings stored alongside a user account. Full
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

/** Minimal user listing entry as returned by `user.list`. */
export interface UserInfo {
  user_id: string
  show_name: string
  user_type: UserType | string
  state: UserStateString
}

export interface UsersListResponse {
  total: number
  users: UserInfo[]
}

/** Full user detail as returned by `user.get`. */
export interface UserDetail {
  user_id: string
  show_name: string
  user_type: UserType | string
  state: UserStateString
  res_pool_id: string
  /** Only present if caller is the user themselves or an admin. */
  contact?: UserContactSettings
  /** Optional DID document loaded from `users/{uid}/doc`. */
  did_document?: Record<string, unknown>
}

export interface SimpleOkResponse {
  ok: boolean
  user_id?: string
  [key: string]: unknown
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
  [key: string]: unknown
}

export interface AgentTunnelBindingResponse {
  ok: boolean
  agent_id: string
  platform: string
  total_bindings?: number
  remaining_bindings?: number
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
}): Promise<{ data: SimpleOkResponse | null; error: unknown }> => {
  const params: Record<string, unknown> = {
    user_id: input.userId,
    password_hash: input.passwordHash,
  }
  if (input.showName !== undefined) params.show_name = input.showName
  if (input.userType !== undefined) params.user_type = input.userType
  return callRpc<SimpleOkResponse>('user.create', params)
}

/** Update basic user fields (currently `show_name`). */
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
 * Update the system-level contact settings for a user. Partial update — only
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

/** Get the full detail for a single agent (doc + optional settings). */
export const fetchAgentDetail = async (
  agentId: string,
): Promise<{ data: AgentDetail | null; error: unknown }> =>
  callRpc<AgentDetail>('agent.get', { agent_id: agentId })

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
  meta?: Record<string, string>
}): Promise<{ data: AgentTunnelBindingResponse | null; error: unknown }> => {
  const params: Record<string, unknown> = {
    agent_id: input.agentId,
    platform: input.platform,
    account_id: input.accountId,
  }
  if (input.displayId !== undefined) params.display_id = input.displayId
  if (input.tunnelId !== undefined) params.tunnel_id = input.tunnelId
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
