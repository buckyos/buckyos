/* ── In-browser control panel for the mock runtime ──
 *
 * Answers the Agent (`agent.*`), own-profile (`user.get`, `user.profile.set`,
 * `user.*_msg_tunnel`), account management (`user.list`, `user.change_type`,
 * `user.set_account_options`), system user group (`user_group.*`) and Zone
 * display (`zone.presentation.*`) calls with the shapes of the Agent init
 * contract §3 and control_panel. State lives in
 * localStorage under MOCK_CONTROL_PANEL_STORAGE_KEY and is re-read on every
 * call, so e2e tests can seed or change it (a partial object is merged over
 * the defaults).
 *
 * Scripted outcomes: an Agent name starting with `fail-runtime` / `fail-start`
 * fails that creation step once; a bot token containing `FAIL` fails the
 * tunnel step once.
 *
 * Hybrid SSO (`auth.*`, `sso.settings.*`, `user.exclusive_invite.*`): `session`
 * says whom the browser speaks for (the account, an External DID or nobody);
 * `sso` holds the Zone switches, share entries, exclusive invite accounts and
 * invites (with their secret, mock only). A DID containing `offline` has an
 * unreachable home Zone; a group DID containing `offline` leaves the invite's
 * Session invitation pending.
 */

import {
  DEFAULT_JUMP_BASE_URL,
  RPCError,
  type AccessDenial,
  type AuthMethod,
  type CurrentIdentity,
  type DenyAdjust,
  type DenyReason,
  type EntryDescriptor,
  type EntryMethod,
  type EntryMethods,
  type EntryResource,
  type EntryService,
  type SessionScope,
} from 'buckyos'
import {
  ASSIGNABLE_USER_TYPES,
  USER_GROUP_ID_PATTERN,
  defaultCanChangePassword,
  defaultCanEditContacts,
  defaultContactProjectionRules,
  isPrivilegedUserType,
  parseUserGroupRef,
  parseUserType,
  type ContactProjectionRule,
} from './user_type.ts'
import type {
  AgentCreateRequest,
  AgentCreateStep,
  AgentEntry,
  AgentNameCheck,
  AgentPermissionMode,
  AgentProfile,
  AgentRuntimeOffer,
  AgentSettings,
  AgentStatus,
  AgentTemplate,
  AgentTemplateBinding,
  UserTunnelBinding,
  UserInviteRecord,
} from './user_mgr.ts'
import type { ExclusiveInvite, InviteResource } from './sso.ts'

export const MOCK_CONTROL_PANEL_STORAGE_KEY = 'buckyos.mock.control-panel.v1'

interface MockAccount {
  user_id: string
  user_name: string
  user_type: string
  did: string
  /** Fields saved through `user.profile.set` (display_name, avatar, bio, …). */
  profile?: Record<string, string>
}

/** Per-account facts of the Zone's users; `null` overrides mean the type default. */
interface MockZoneUser {
  user_type: string
  allow_password_change?: boolean | null
  allow_contact_edit?: boolean | null
}

interface MockUserGroup {
  group_id: string
  name: string
  description?: string
  updated_by: string
  created_at: number
  updated_at: number
  members: string[]
}

type MockZonePresentation = Partial<Record<'display_name' | 'description' | 'icon', string>>

interface MockAgent {
  agent_id: string
  agent_did: string
  owner_user_id: string
  owner_did: string
  permission_mode: AgentPermissionMode
  name: string
  profile: AgentProfile
  settings: AgentSettings
  bot_token: string | null
  template: AgentTemplateBinding
  install: AgentStatus
  idempotency_key: string
  fail_steps: AgentCreateStep[]
  step_started_at: number
  installed: boolean
}

interface MockState {
  zone: string
  zone_owner: string
  account: MockAccount
  user_bindings: UserTunnelBinding[]
  user_names: string[]
  host_names: string[]
  templates: AgentTemplate[]
  runtime_offers: AgentRuntimeOffer[]
  /** A node of the zone runs Agent containers (default true). */
  container_available?: boolean
  runtime_directory_error?: string
  agents: MockAgent[]
  step_ms: number
  zone_presentation?: MockZonePresentation
  /** Keyed by user_id; every name in `user_names` has one. */
  zone_users: Record<string, MockZoneUser>
  user_groups: MockUserGroup[]
  /** Unset until rules are saved; the defaults apply meanwhile. */
  projections?: { rules: ContactProjectionRule[]; updated_by: string; updated_at: number } | null
  /** Whom the browser session speaks for; unset means the account above. */
  session?: MockSession
  /** Hybrid SSO state; a partial seed is merged over the defaults. */
  sso?: Partial<MockSso>
  user_invites?: Record<string, UserInviteRecord>
  /** `apps.app_login.*`: the owner's switch per App instance (EXT-09 / EXT-11). */
  app_logins?: Record<string, { enabled: boolean; updated_by: string; updated_at: number }>
}

type MockSession =
  | { kind: 'account'; auth_method?: AuthMethod }
  | { kind: 'external'; did: string; show_name: string }
  | { kind: 'anonymous' }

/** Limited accounts made by exclusive invites or self-registration. */
interface MockSsoAccount {
  show_name: string
  password_set: boolean
  state: 'active' | 'disabled'
}

interface MockEntry {
  service: EntryService
  entry_id: string
  state: 'active' | 'unavailable'
  resource?: EntryResource
  methods: EntryMethods
}

type MockInvite = ExclusiveInvite & { secret: string }

interface MockSso {
  settings: { self_register_enabled: boolean; did_login_enabled: boolean; jump_base_url?: string }
  entries: MockEntry[]
  accounts: Record<string, MockSsoAccount>
  invites: MockInvite[]
  /** Target Zones the signed-in account agreed to (canonical side). */
  consents: string[]
  did_logins?: Record<string, { did: string; redirect_url: string }>
}

const RESERVED_NAMES = new Set(['root', 'system', 'admin', 'guest', '_', 'www', 'sys', 'homestation'])
const DNS_LABEL = /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/

const jarvisTemplate: AgentTemplate = {
  template_id: 'bundled:jarvis.buckyos.bns.did',
  source: 'bundled',
  app_id: 'jarvis.buckyos.bns.did',
  app_did: 'did:bns:jarvis.buckyos',
  name: 'jarvis',
  show_name: 'Jarvis',
  description: 'A personal assistant that helps you manage files, messages and tasks in your Zone.',
  version: '0.7.0',
  icon: null,
  loader: 'opendan',
  is_default: true,
}

function defaultSettings(): AgentSettings {
  return {
    enabled: true,
    auto_start: true,
    allow_other_users: false,
    allow_group: false,
    role_supplement: '',
    template_auto_update: true,
    desktop_entry: null,
    homestation: true,
    auto_share_results: true,
    msg_tunnels: [],
  }
}

function defaultState(): MockState {
  const now = Date.UTC(2026, 3, 1) / 1000
  return {
    zone: 'alice',
    zone_owner: 'alice',
    account: { user_id: 'alice', user_name: 'Alice', user_type: 'admin', did: 'did:bns:alice' },
    user_bindings: [
      { platform: 'email', account_id: 'alice@example.com', display_id: 'alice@example.com', status: 'active' },
    ],
    user_names: ['alice', 'bob', 'carol', 'dave'],
    host_names: ['messagehub', 'files', 'control-panel'],
    templates: [jarvisTemplate],
    runtime_offers: [{
      runtime_id: 'host', kind: 'node', node_id: 'desktop', node_did: 'did:dev:desktop',
      device_type: 'desktop', name: 'Alice’s computer', computer_max: 'control', os: 'windows',
      workdir_roots: ['/home/alice'], closed: false, enabled: false, computer: 'none', baseline: 'standard',
    }],
    agents: [
      {
        agent_id: 'assistant.alice',
        agent_did: 'did:bns:assistant.alice',
        owner_user_id: 'alice',
        owner_did: 'did:bns:alice',
        permission_mode: 'read_write',
        name: 'assistant',
        profile: { display_name: 'BuckyOS Assistant', bio: 'Your personal AI assistant on BuckyOS.' },
        settings: { ...defaultSettings(), msg_tunnels: [{ platform: 'telegram', bot_account_id: 'bucky_bot' }] },
        bot_token: null,
        template: { template_id: jarvisTemplate.template_id, source: 'bundled', app_did: jarvisTemplate.app_did, version: '0.7.0', loaded_version: '0.7.0' },
        install: {
          agent_id: 'assistant.alice',
          agent_did: 'did:bns:assistant.alice',
          owner_user_id: 'alice',
          state: 'ready',
          step: 'done',
          tunnel_state: 'bound',
          created_at: now,
          updated_at: now,
        },
        idempotency_key: 'seed-assistant',
        fail_steps: [],
        step_started_at: 0,
        installed: true,
      },
    ],
    step_ms: 700,
    zone_users: {
      alice: { user_type: 'admin' },
      bob: { user_type: 'user' },
      carol: { user_type: 'user' },
      dave: { user_type: 'limited' },
    },
    user_groups: [],
    projections: null,
  }
}

let memoryState: MockState | null = null

function loadState(): MockState {
  const defaults = defaultState()
  try {
    const raw = window.localStorage.getItem(MOCK_CONTROL_PANEL_STORAGE_KEY)
    if (raw) return { ...defaults, ...(JSON.parse(raw) as Partial<MockState>) }
  } catch {
    // Storage unavailable or corrupted: fall back to the in-memory copy.
  }
  return memoryState ?? defaults
}

function saveState(state: MockState) {
  memoryState = state
  try {
    window.localStorage.setItem(MOCK_CONTROL_PANEL_STORAGE_KEY, JSON.stringify(state))
  } catch {
    // Keep the in-memory copy only.
  }
}

class MockRpcError extends Error {
  constructor(reason: string) {
    super(`RPC call error: Failed due to reason: ${reason}`)
    this.name = 'RPCError'
  }
}

function stringParam(params: Record<string, unknown>, key: string): string {
  const value = params[key]
  if (typeof value !== 'string' || !value.trim()) throw new MockRpcError(`missing param ${key}`)
  return value.trim()
}

function isLimited(account: MockAccount) {
  return parseUserType(account.user_type) === 'limited'
}

function isAdmin(account: MockAccount) {
  return isPrivilegedUserType(account.user_type)
}

function requireCreator(state: MockState) {
  if (isLimited(state.account)) throw new MockRpcError('limited_user: limited users cannot create agents')
}

function requireAdmin(state: MockState) {
  if (!isAdmin(state.account)) throw new MockRpcError('Admin privileges required')
}

function liveAgents(state: MockState) {
  return state.agents.filter((agent) => agent.install.state !== 'removed')
}

function nameTaken(state: MockState, name: string): 'reserved' | 'user_exists' | 'agent_exists' | 'host_taken' | null {
  if (RESERVED_NAMES.has(name)) return 'reserved'
  if (state.user_names.includes(name)) return 'user_exists'
  if (liveAgents(state).some((agent) => agent.name === name)) return 'agent_exists'
  if (state.host_names.includes(name)) return 'host_taken'
  return null
}

const reasonMessages = {
  invalid: 'Use 1-63 lowercase letters, digits or hyphens; it cannot start or end with a hyphen.',
  reserved: 'This name is reserved by the system.',
  user_exists: 'A user already uses this name.',
  agent_exists: 'An Agent already uses this name.',
  host_taken: 'An app already uses this name as its host name.',
}

function checkName(state: MockState, rawName: string): AgentNameCheck {
  const name = rawName.trim()
  const agentId = `${name}.${state.zone}`
  const base = { name, agent_id: agentId, agent_did: `did:bns:${agentId}` }
  if (!DNS_LABEL.test(name)) {
    return { ...base, available: false, reason: 'invalid', message: reasonMessages.invalid }
  }
  const reason = nameTaken(state, name)
  if (!reason) return { ...base, available: true }
  const owner = state.account.user_id
  let suggestion: string | undefined
  for (let index = 1; index < 50 && !suggestion; index += 1) {
    const candidate = index === 1 ? `${owner}-${name}` : `${owner}-${name}-${index}`
    if (DNS_LABEL.test(candidate) && !nameTaken(state, candidate)) suggestion = candidate
  }
  return { ...base, available: false, reason, message: reasonMessages[reason], ...(suggestion ? { suggestion } : {}) }
}

function nextStep(agent: MockAgent): AgentCreateStep {
  switch (agent.install.step) {
    case 'runtime':
      return 'bind'
    case 'bind':
      return agent.bot_token ? 'tunnel' : 'start'
    case 'tunnel':
      return 'start'
    default:
      return 'done'
  }
}

const failureCodes: Record<AgentCreateStep, string> = {
  runtime: 'app_install_failed',
  bind: 'bind_failed',
  tunnel: 'telegram_bot_invalid',
  start: 'start_timeout',
  done: '',
}

const failureMessages: Record<AgentCreateStep, string> = {
  runtime: 'The Agent App could not be installed: image pull timed out.',
  bind: 'The Agent identity could not be bound to its App.',
  tunnel: 'Telegram rejected the bot token (401 Unauthorized).',
  start: 'The Agent did not report ready within 5 minutes.',
  done: '',
}

function advance(state: MockState, agent: MockAgent, now: number) {
  while (agent.install.state === 'provisioning' || agent.install.state === 'bound') {
    const step = agent.install.step
    const duration = step === 'runtime' ? state.step_ms * 2 : state.step_ms
    const elapsed = now - agent.step_started_at
    if (step === 'runtime') {
      agent.install.runtime_task_id = `task-${agent.name}`
      agent.install.runtime_progress = {
        phase: 'install',
        percent: Math.min(99, Math.round((elapsed / duration) * 100)),
        message: 'Installing the Agent App',
      }
    }
    if (elapsed < duration) return
    agent.install.updated_at = Math.floor(now / 1000)
    if (agent.fail_steps.includes(step)) {
      agent.fail_steps = agent.fail_steps.filter((item) => item !== step)
      agent.install.state = 'failed'
      agent.install.last_error = { step, code: failureCodes[step], message: failureMessages[step], retryable: true }
      if (step === 'tunnel') agent.install.tunnel_state = 'failed'
      return
    }
    agent.step_started_at += duration
    if (step === 'runtime') {
      agent.installed = true
      agent.install.runtime_progress = { phase: 'install', percent: 100, message: 'Installed' }
    }
    if (step === 'tunnel') agent.install.tunnel_state = 'bound'
    const next = nextStep(agent)
    agent.install.step = next
    if (step === 'bind') agent.install.state = 'bound'
    if (next === 'done') agent.install.state = 'ready'
  }
}

function advanceAll(state: MockState) {
  const now = Date.now()
  for (const agent of state.agents) advance(state, agent, now)
}

function visibleAgents(state: MockState) {
  return liveAgents(state).filter((agent) => isAdmin(state.account) || agent.owner_user_id === state.account.user_id)
}

function findAgent(state: MockState, agentId: string) {
  const agent = visibleAgents(state).find((item) => item.agent_id === agentId)
  if (!agent) throw new MockRpcError(`agent_not_found: ${agentId}`)
  return agent
}

function displayName(agent: MockAgent) {
  return agent.profile.display_name?.trim() || agent.name
}

function toEntry(state: MockState, agent: MockAgent): AgentEntry {
  const hostName = agent.owner_user_id === state.zone_owner ? agent.name : `${agent.name}-${agent.owner_user_id}`
  return {
    agent_id: agent.agent_id,
    agent_did: agent.agent_did,
    owner_user_id: agent.owner_user_id,
    owner_did: agent.owner_did,
    permission_mode: agent.permission_mode,
    name: agent.name,
    display_name: displayName(agent),
    profile: { ...agent.profile },
    settings: { ...agent.settings, msg_tunnels: agent.settings.msg_tunnels.map((tunnel) => ({ ...tunnel })) },
    install: { ...agent.install },
    template: { ...agent.template },
    runtime: agent.installed
      ? {
        app_instance_id: `${agent.agent_id}@${agent.owner_user_id}`,
        app_host_name: hostName,
        has_web: true,
        state: agent.install.state === 'ready' ? 'running' : 'new',
      }
      : null,
  }
}

function zoneUser(state: MockState, userId: string): MockZoneUser {
  const user = state.zone_users[userId]
  if (!user) throw new MockRpcError(`User '${userId}' not found`)
  return user
}

function groupsOf(state: MockState, userId: string) {
  return state.user_groups.filter((group) => group.members.includes(userId)).map((group) => group.group_id).sort()
}

function effectiveOptions(user: MockZoneUser) {
  return {
    allow_password_change: user.allow_password_change ?? defaultCanChangePassword(user.user_type),
    allow_contact_edit: user.allow_contact_edit ?? defaultCanEditContacts(user.user_type),
  }
}

function userDetail(state: MockState) {
  const { account } = state
  const contact = { did: account.did, bindings: state.user_bindings.map((binding) => ({ ...binding })) }
  const profile = { did: account.did, display_name: account.user_name, ...account.profile }
  const facts = state.zone_users[account.user_id] ?? { user_type: account.user_type }
  return {
    user_id: account.user_id,
    show_name: profile.display_name,
    user_type: account.user_type,
    state: 'active',
    res_pool_id: 'default',
    is_local: false,
    groups: groupsOf(state, account.user_id),
    ...effectiveOptions({ ...facts, user_type: account.user_type }),
    profile,
    local_profile: {
      ...profile,
      private_extra: { system_contact: contact },
    },
  }
}

function userList(state: MockState) {
  const users = state.user_names
    .filter((userId) => state.zone_users[userId])
    .map((userId) => {
      const user = zoneUser(state, userId)
      const effective = effectiveOptions(user)
      return {
        user_id: userId,
        show_name: userId === state.account.user_id ? state.account.user_name : userId,
        user_type: user.user_type,
        state: 'active',
        is_local: false,
        groups: groupsOf(state, userId),
        allow_password_change: user.allow_password_change ?? null,
        allow_contact_edit: user.allow_contact_edit ?? null,
        can_change_password: effective.allow_password_change,
        can_edit_contacts: effective.allow_contact_edit,
      }
    })
  return { total: users.length, users }
}

function optionParam(params: Record<string, unknown>, key: string): boolean | null | undefined {
  if (!(key in params)) return undefined
  const value = params[key]
  if (value === null || typeof value === 'boolean') return value
  throw new MockRpcError(`${key} must be a boolean or null`)
}

function stringListParam(params: Record<string, unknown>, key: string): string[] {
  const value = params[key]
  if (value === undefined) return []
  if (!Array.isArray(value) || value.some((item) => typeof item !== 'string')) throw new MockRpcError(`${key} must be a list of strings`)
  return value as string[]
}

function projections(state: MockState) {
  if (!state.projections) return { rules: defaultContactProjectionRules(), is_default: true, updated_by: '', updated_at: 0 }
  return { rules: state.projections.rules.map((rule) => ({ ...rule })), is_default: false, updated_by: state.projections.updated_by, updated_at: state.projections.updated_at }
}

function groupView(group: MockUserGroup) {
  return { ...group, members: [...group.members].sort() }
}

function findGroup(state: MockState, params: Record<string, unknown>) {
  const groupId = stringParam(params, 'group_id')
  const group = state.user_groups.find((item) => item.group_id === groupId)
  if (!group) throw new MockRpcError(`user group '${groupId}' not found`)
  return group
}

function validRules(state: MockState, raw: unknown): ContactProjectionRule[] {
  if (!Array.isArray(raw)) throw new MockRpcError('rules must be a list')
  const seen = new Set<string>()
  return raw.map((item) => {
    const rule = item as Partial<ContactProjectionRule>
    for (const ref of [rule.source, rule.target]) {
      const parsed = typeof ref === 'string' ? parseUserGroupRef(ref) : null
      if (!parsed) throw new MockRpcError(`invalid user group reference \`${String(ref)}\``)
      if (parsed.kind === 'group' && !state.user_groups.some((group) => group.group_id === parsed.groupId)) {
        throw new MockRpcError(`projection rule refers to undefined group \`${ref}\``)
      }
    }
    const key = `${rule.source}->${rule.target}`
    if (seen.has(key)) throw new MockRpcError(`duplicate projection rule ${rule.source} -> ${rule.target}`)
    seen.add(key)
    return { source: rule.source as string, target: rule.target as string }
  })
}

function handleUserGroup(state: MockState, method: string, params: Record<string, unknown>): unknown {
  requireAdmin(state)
  const now = Math.floor(Date.now() / 1000)
  switch (method) {
    case 'user_group.list':
      return { groups: state.user_groups.map(groupView), projections: projections(state) }
    case 'user_group.create': {
      const groupId = stringParam(params, 'group_id')
      if (!USER_GROUP_ID_PATTERN.test(groupId)) throw new MockRpcError(`invalid user group id \`${groupId}\``)
      if (state.user_groups.some((group) => group.group_id === groupId)) throw new MockRpcError(`user group '${groupId}' already exists`)
      const group: MockUserGroup = {
        group_id: groupId,
        name: stringParam(params, 'name'),
        ...(typeof params.description === 'string' && params.description.trim() ? { description: params.description.trim() } : {}),
        updated_by: state.account.user_id,
        created_at: now,
        updated_at: now,
        members: [],
      }
      state.user_groups.push(group)
      return { ok: true, group: groupView(group) }
    }
    case 'user_group.update': {
      const group = findGroup(state, params)
      if (typeof params.name === 'string') group.name = stringParam(params, 'name')
      if (typeof params.description === 'string') {
        if (params.description.trim()) group.description = params.description.trim()
        else delete group.description
      }
      group.updated_by = state.account.user_id
      group.updated_at = now
      return { ok: true, group: groupView(group) }
    }
    case 'user_group.delete': {
      const group = findGroup(state, params)
      const ref = `group:${group.group_id}`
      state.user_groups = state.user_groups.filter((item) => item !== group)
      let removedRules = 0
      if (state.projections) {
        const kept = state.projections.rules.filter((rule) => rule.source !== ref && rule.target !== ref)
        removedRules = state.projections.rules.length - kept.length
        state.projections = { ...state.projections, rules: kept }
      }
      return { ok: true, group_id: group.group_id, removed_members: group.members.length, removed_rules: removedRules }
    }
    case 'user_group.set_members': {
      const group = findGroup(state, params)
      const add = stringListParam(params, 'add')
      const remove = stringListParam(params, 'remove')
      for (const userId of add) {
        if (zoneUser(state, userId).user_type === 'root') throw new MockRpcError('the owner record cannot join user groups')
      }
      const members = new Set(group.members)
      for (const userId of add) members.add(userId)
      for (const userId of remove) members.delete(userId)
      group.members = [...members].sort()
      group.updated_at = now
      return { ok: true, group_id: group.group_id, members: [...group.members] }
    }
    case 'user_group.set_projections':
      state.projections = { rules: validRules(state, params.rules), updated_by: state.account.user_id, updated_at: now }
      return { ok: true, projections: projections(state) }
    case 'user_group.reset_projections':
      state.projections = null
      return { ok: true, projections: projections(state) }
    default:
      throw new MockRpcError(`Unknown method: ${method}`)
  }
}

const PROFILE_TEXT_FIELDS = ['display_name', 'avatar', 'bio', 'title', 'location', 'organization', 'website', 'email', 'phone'] as const

/** A field left out is kept; null or a blank string clears it. */
function applyTextPatch(target: Record<string, string>, params: Record<string, unknown>, keys: readonly string[]) {
  for (const key of keys) {
    if (!(key in params)) continue
    const value = params[key]
    if (value !== null && typeof value !== 'string') throw new MockRpcError(`${key} must be a string or null`)
    const text = value?.trim()
    if (text) target[key] = text
    else delete target[key]
  }
}

function zonePresentation(state: MockState) {
  return {
    zone_did: `did:bns:${state.zone}`,
    hostname: `${state.zone}.web3.buckyos.ai`,
    settings: { ...state.zone_presentation },
    document: {},
  }
}

function createAgent(state: MockState, params: Record<string, unknown>) {
  requireCreator(state)
  const request = params as unknown as AgentCreateRequest
  const permissionMode = request.permission_mode ?? 'read_write'
  if (permissionMode !== 'read_write' && permissionMode !== 'read_only') {
    throw new MockRpcError('invalid_permission_mode: expected read_write or read_only')
  }
  const existing = state.agents.find((agent) => agent.idempotency_key === request.idempotency_key && agent.install.state !== 'removed')
  if (existing) {
    return { agent_id: existing.agent_id, agent_did: existing.agent_did, status: { ...existing.install } }
  }
  if (request.allow_other_users) throw new MockRpcError('sharing_unsupported: sharing an Agent with other users is not available yet')
  const check = checkName(state, String(request.name ?? ''))
  if (!check.available) {
    if (check.reason === 'invalid') throw new MockRpcError(`invalid_name: ${check.message ?? ''}`)
    throw new MockRpcError(`name_conflict: ${check.message ?? ''}`)
  }
  const template = state.templates.find((item) => item.template_id === request.template_id)
  if (!template) throw new MockRpcError(`template_not_found: ${request.template_id}`)
  if (request.msg_tunnel && !/^\d+:[A-Za-z0-9_-]{30,}$/.test(request.msg_tunnel.bot_token)) {
    throw new MockRpcError('invalid_bot_token: the token does not look like a Telegram bot token')
  }
  if (request.msg_tunnel && !state.user_bindings.some((binding) => binding.platform === request.msg_tunnel?.platform)) {
    throw new MockRpcError('owner_identity_missing: add your Telegram account to your profile first')
  }
  const now = Date.now()
  const name = check.name
  const failSteps: AgentCreateStep[] = []
  if (name.startsWith('fail-runtime')) failSteps.push('runtime')
  if (name.startsWith('fail-start')) failSteps.push('start')
  if (request.msg_tunnel?.bot_token.includes('FAIL')) failSteps.push('tunnel')
  const agent: MockAgent = {
    agent_id: check.agent_id,
    agent_did: check.agent_did,
    owner_user_id: state.account.user_id,
    owner_did: state.account.did,
    permission_mode: permissionMode,
    name,
    profile: {
      ...(request.profile.display_name?.trim() ? { display_name: request.profile.display_name.trim() } : {}),
      ...(request.profile.avatar ? { avatar: request.profile.avatar } : {}),
      ...(request.profile.bio?.trim() ? { bio: request.profile.bio.trim() } : {}),
    },
    settings: {
      ...defaultSettings(),
      allow_group: Boolean(request.allow_group),
      homestation: request.homestation !== false,
      auto_share_results: request.auto_share_results !== false,
      role_supplement: request.role_supplement ?? '',
      template_auto_update: request.template_auto_update !== false,
      desktop_entry: request.desktop_entry ?? null,
      msg_tunnels: request.msg_tunnel ? [{ platform: request.msg_tunnel.platform, bot_account_id: null }] : [],
    },
    bot_token: request.msg_tunnel?.bot_token ?? null,
    template: { template_id: template.template_id, source: template.source, app_did: template.app_did, version: template.version, loaded_version: null },
    install: {
      agent_id: check.agent_id,
      agent_did: check.agent_did,
      owner_user_id: state.account.user_id,
      state: 'provisioning',
      step: 'runtime',
      tunnel_state: request.msg_tunnel ? 'pending' : 'none',
      created_at: Math.floor(now / 1000),
      updated_at: Math.floor(now / 1000),
    },
    idempotency_key: request.idempotency_key,
    fail_steps: failSteps,
    step_started_at: now,
    installed: false,
  }
  const host = state.account.user_id === state.zone_owner
    ? state.runtime_offers.find((offer) => offer.runtime_id === 'host' && !offer.closed)
    : undefined
  const choice = request.runtimes?.host
  const enableHost = host && choice !== null && choice?.enabled !== false
  if (choice?.enabled && !host) throw new MockRpcError('runtime_not_offered: host is not open to this owner')
  agent.settings.runtimes = enableHost ? {
    host: {
      enabled: true,
      computer: choice?.computer ?? host.computer_max,
      target: { kind: 'node', node_id: host.node_id!, node_did: host.node_did! },
    },
  } : {}
  agent.settings.default_runtime_id = request.default_runtime_id ?? (enableHost ? 'host' : 'container')
  if (agent.settings.default_runtime_id === 'container' && state.container_available === false) {
    throw new MockRpcError('container_unavailable: no node of this zone runs Agent containers')
  }
  state.agents.push(agent)
  return { agent_id: agent.agent_id, agent_did: agent.agent_did, status: { ...agent.install } }
}

function retryCreate(state: MockState, params: Record<string, unknown>) {
  const agent = findAgent(state, stringParam(params, 'agent_id'))
  if (agent.install.state !== 'failed' || !agent.install.last_error) throw new MockRpcError('not_failed: nothing to retry')
  const step = agent.install.last_error.step
  agent.install.last_error = null
  agent.step_started_at = Date.now()
  if (step === 'tunnel' && params.skip_tunnel === true) {
    agent.install.tunnel_state = 'skipped'
    agent.bot_token = null
    agent.settings.msg_tunnels = []
    agent.install.step = 'start'
  } else {
    agent.install.step = step
    if (step === 'tunnel') agent.install.tunnel_state = 'pending'
  }
  agent.install.state = step === 'runtime' || step === 'bind' ? 'provisioning' : 'bound'
  return { ...agent.install }
}

function cancelCreate(state: MockState, params: Record<string, unknown>) {
  const agent = findAgent(state, stringParam(params, 'agent_id'))
  const step = agent.install.state === 'failed' ? agent.install.last_error?.step : agent.install.step
  const beforeSpec = agent.install.state === 'provisioning' || (agent.install.state === 'failed' && (step === 'runtime' || step === 'bind'))
  if (!beforeSpec) throw new MockRpcError('cancel_unavailable: the Agent is already bound; delete it instead')
  state.agents = state.agents.filter((item) => item !== agent)
  return { ok: true }
}

// ── Hybrid SSO ──

const nowSecs = () => Math.floor(Date.now() / 1000)
const INVITE_TTL_SECS = 48 * 3600
const RESEND_INTERVAL_SECS = 10 * 60
const ALL_SCOPES: SessionScope[] = ['account_self', 'msg_group', 'aiworkspace', 'homestation', 'aicc']
const EXTERNAL_SCOPES: SessionScope[] = ['msg_group', 'aiworkspace', 'homestation']
const USER_ID_PATTERN = /^[a-z0-9_.-]{1,64}$/

const DENY_ADJUST: Record<DenyReason, DenyAdjust> = {
  session_kind_not_allowed: { party: 'product' },
  session_ceiling_exceeded: { party: 'self', location: 'reauth' },
  resource_grant_missing: { party: 'sharer', location: 'share_panel' },
  entry_unavailable: { party: 'sharer', location: 'share_panel' },
  zone_method_disabled: { party: 'admin', location: 'cp:sso_settings' },
  account_state: { party: 'admin', location: 'cp:users' },
  credential_state: { party: 'self', location: 'credential' },
  source_revoked: { party: 'admin', location: 'cp:exclusive_invites' },
  authorization_unavailable: { party: 'none' },
}

function denialFor(reason: DenyReason, detail?: string, method?: string): AccessDenial {
  return { reason, adjust: DENY_ADJUST[reason], service: 'control-panel', ...(method ? { method } : {}), ...(detail ? { detail } : {}) }
}

/** A thrown denial, as the kRPC client surfaces a `code: access_denied` failure. */
function deny(reason: DenyReason, method: string, detail?: string): never {
  throw new RPCError(`access denied: ${reason}`, 'access_denied', denialFor(reason, detail, method))
}

function randomToken(): string {
  const bytes = new Uint8Array(16)
  crypto.getRandomValues(bytes)
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('')
}

function mockInvite(input: {
  invite_id: string
  user_id: string
  secret: string
  issued_at: number
  credential_state?: ExclusiveInvite['credential']['state']
  resource?: InviteResource
  return_path: string
  target_hint: string
}): MockInvite {
  return {
    invite_id: input.invite_id,
    user_id: input.user_id,
    issuer: { kind: 'user', user_id: 'alice' },
    channel: { kind: 'email', target_hint: input.target_hint },
    ...(input.resource ? { resource: input.resource } : {}),
    resource_state: input.resource ? 'done' : 'none',
    return_path: input.return_path,
    state: 'active',
    credential: {
      version: 1,
      issued_at: input.issued_at,
      expires_at: input.issued_at + INVITE_TTL_SECS,
      state: input.credential_state ?? 'active',
    },
    created_at: input.issued_at,
    updated_at: input.issued_at,
    secret: input.secret,
  }
}

function defaultSso(): MockSso {
  const now = nowSecs()
  const allMethods = { guest: true, password: true, self_register: true, did_login: true }
  return {
    settings: { self_register_enabled: true, did_login_enabled: true },
    entries: [
      {
        service: 'aiworkspace',
        entry_id: 'se-dj',
        state: 'active',
        resource: { kind: 'workspace', title: 'DJ Radio', owner_name: 'DJ', path: '/workspace/ws-dj' },
        methods: allMethods,
      },
      {
        service: 'msg-center',
        entry_id: 'did:bns:support.alice',
        state: 'active',
        resource: { kind: 'group', title: 'Acme Support', owner_name: 'Acme', path: '/messagehub/guest?group=did%3Abns%3Asupport.alice' },
        methods: { ...allMethods, guest: false },
      },
      { service: 'aiworkspace', entry_id: 'se-closed', state: 'unavailable', methods: { guest: false, password: false, self_register: false, did_login: false } },
    ],
    accounts: {
      'acme-c1': { show_name: 'Acme Customer', password_set: false, state: 'active' },
      'acme-c2': { show_name: 'Bolt Customer', password_set: false, state: 'active' },
      'acme-c3': { show_name: 'Cove Customer', password_set: true, state: 'active' },
    },
    invites: [
      mockInvite({
        invite_id: 'ei-active',
        user_id: 'acme-c1',
        secret: 'mock-secret-active',
        issued_at: now - 3600,
        resource: { kind: 'group_session', group_did: 'did:bns:support.alice', session_id: 't-123', title: 'Ticket #123' },
        return_path: '/?invite=ei-active',
        target_hint: 'a***@acme.com',
      }),
      mockInvite({ invite_id: 'ei-expired', user_id: 'acme-c2', secret: 'mock-secret-expired', issued_at: now - 3 * 86_400, return_path: '/', target_hint: 'b***@bolt.io' }),
      mockInvite({
        invite_id: 'ei-password',
        user_id: 'acme-c3',
        secret: 'mock-secret-password',
        issued_at: now - 86_400,
        credential_state: 'consumed_by_password',
        return_path: '/?invite=ei-password',
        target_hint: '138****5678',
      }),
    ],
    consents: [],
  }
}

function ssoOf(state: MockState): MockSso {
  const sso = { ...defaultSso(), ...state.sso }
  state.sso = sso
  return sso
}

function sessionOf(state: MockState): MockSession {
  return state.session ?? { kind: 'account' }
}

function zoneHostOf(state: MockState) {
  return `${state.zone}.web3.buckyos.ai`
}

function identityOf(state: MockState): CurrentIdentity {
  const session = sessionOf(state)
  if (session.kind === 'anonymous') return { kind: 'anonymous' }
  if (session.kind === 'external') {
    return {
      kind: 'external',
      did: session.did,
      show_name: session.show_name,
      zone_host: zoneHostOf(state),
      auth_method: 'did_federation',
      restricted: true,
      scopes: EXTERNAL_SCOPES,
    }
  }
  const { account } = state
  const restricted = isLimited(account)
  return {
    kind: 'account',
    user_id: account.user_id,
    did: account.did,
    user_type: parseUserType(account.user_type) ?? 'user',
    show_name: account.user_name,
    zone_host: zoneHostOf(state),
    auth_method: session.auth_method ?? 'password',
    restricted,
    ...(restricted ? { scopes: ALL_SCOPES } : {}),
    password_set: ssoOf(state).accounts[account.user_id]?.password_set ?? true,
  }
}

function signInAs(state: MockState, userId: string, userType: string, showName: string, authMethod: AuthMethod) {
  if (state.account.user_id !== userId) {
    state.account = { user_id: userId, user_name: showName, user_type: userType, did: `did:bns:${userId}` }
  }
  state.session = { kind: 'account', auth_method: authMethod }
}

function requireSignedIn(state: MockState, method: string) {
  if (sessionOf(state).kind === 'anonymous') throw new MockRpcError(`${method}: no session`)
}

/** Admin methods: a non-restricted administrator account. */
function requireSsoAdmin(state: MockState, method: string) {
  requireSignedIn(state, method)
  if (sessionOf(state).kind !== 'account' || isLimited(state.account)) deny('session_kind_not_allowed', method)
  requireAdmin(state)
}

function effectiveEntry(state: MockState, service: string, entryId: string): EntryDescriptor {
  const sso = ssoOf(state)
  const entry = sso.entries.find((item) => item.service === service && item.entry_id === entryId)
  const ref = { service: service as EntryService, entry_id: entryId }
  if (!entry || entry.state !== 'active') {
    return { entry: ref, state: 'unavailable', methods: { guest: false, password: false, self_register: false, did_login: false } }
  }
  const methods = { ...entry.methods }
  const zoneDisabled: EntryMethod[] = []
  if (methods.self_register && !sso.settings.self_register_enabled) {
    methods.self_register = false
    zoneDisabled.push('self_register')
  }
  if (methods.did_login && !sso.settings.did_login_enabled) {
    methods.did_login = false
    zoneDisabled.push('did_login')
  }
  if (methods.self_register) methods.password = true
  return { entry: ref, state: 'active', ...(entry.resource ? { resource: { ...entry.resource } } : {}), methods, ...(zoneDisabled.length ? { zone_disabled: zoneDisabled } : {}) }
}

function userTaken(state: MockState, userId: string) {
  return RESERVED_NAMES.has(userId) || Boolean(state.zone_users[userId]) || Boolean(ssoOf(state).accounts[userId])
}

/** The login decision for an invite link (contract §5.3), or the account it signs in. */
function inviteDecision(state: MockState, inviteId: string, secret: string): { invite: MockInvite; account: MockSsoAccount } | AccessDenial {
  const sso = ssoOf(state)
  const invite = sso.invites.find((item) => item.invite_id === inviteId)
  if (!invite || invite.state === 'revoked') return denialFor('source_revoked', 'revoked')
  const account = sso.accounts[invite.user_id]
  if (!account || account.state !== 'active') return denialFor('account_state', 'account_inactive')
  if (account.password_set || invite.credential.state === 'consumed_by_password') return denialFor('credential_state', 'password_required')
  if (invite.secret !== secret || invite.credential.state !== 'active') return denialFor('credential_state', 'link_replaced')
  if (nowSecs() > invite.credential.expires_at) return denialFor('credential_state', 'expired')
  return { invite, account }
}

function publicInvite(invite: MockInvite): ExclusiveInvite {
  const { secret: _secret, ...rest } = invite
  void _secret
  return { ...rest, credential: { ...rest.credential } }
}

function inviteLink(invite: MockInvite) {
  return `${window.location.origin}/sso/invite/${invite.invite_id}#s=${invite.secret}`
}

function findInvite(state: MockState, params: Record<string, unknown>) {
  const inviteId = stringParam(params, 'invite_id')
  const invite = ssoOf(state).invites.find((item) => item.invite_id === inviteId)
  if (!invite) throw new MockRpcError(`invite '${inviteId}' not found`)
  return invite
}

function inviteResourceParam(raw: unknown): InviteResource | undefined {
  if (raw === undefined || raw === null) return undefined
  const value = raw as Record<string, unknown>
  const title = typeof value.title === 'string' ? value.title.trim() : ''
  if (value.kind === 'group_session') {
    return { kind: 'group_session', group_did: stringParam(value, 'group_did'), session_id: stringParam(value, 'session_id'), title }
  }
  if (value.kind === 'workspace') return { kind: 'workspace', workspace_id: stringParam(value, 'workspace_id'), title }
  throw new MockRpcError('unknown invite resource kind')
}

function defaultReturnPath(resource: InviteResource | undefined) {
  if (resource?.kind === 'group_session') {
    return `/messagehub/guest?group=${encodeURIComponent(resource.group_did)}&session=${encodeURIComponent(resource.session_id)}`
  }
  if (resource?.kind === 'workspace') return `/workspace/${encodeURIComponent(resource.workspace_id)}`
  return '/'
}

function federationChecks(state: MockState, method: string, params: Record<string, unknown>) {
  requireSignedIn(state, method)
  if (sessionOf(state).kind !== 'account' || isLimited(state.account)) deny('session_kind_not_allowed', method)
  if (stringParam(params, 'did') !== state.account.did) deny('session_ceiling_exceeded', method)
  const redirectUri = stringParam(params, 'redirect_uri')
  let clientHost: string
  try {
    const url = new URL(redirectUri)
    if (url.pathname !== '/sso/did_callback') throw new Error('path')
    clientHost = url.host
  } catch {
    throw new MockRpcError('invalid redirect_uri')
  }
  return { clientZone: stringParam(params, 'client_zone'), clientHost, redirectUri, flowState: stringParam(params, 'state') }
}

/** `apps.app_login.get/set`: the App owner's per-instance switch; the mock does not track bound accounts. */
function handleAppLogin(state: MockState, method: string, params: Record<string, unknown>): unknown {
  requireSignedIn(state, method)
  const appInstanceId = stringParam(params, 'app_instance_id')
  state.app_logins ??= {}
  const current = state.app_logins[appInstanceId] ?? { enabled: false, updated_by: '', updated_at: 0 }
  switch (method) {
    case 'apps.app_login.get':
      return { settings: current, bound_accounts: 0 }
    case 'apps.app_login.set': {
      if (typeof params.enabled !== 'boolean') throw new MockRpcError('missing param enabled')
      const settings = { enabled: params.enabled, updated_by: state.account.user_id, updated_at: nowSecs() }
      state.app_logins[appInstanceId] = settings
      return { settings, families_revoked: 0 }
    }
    default:
      throw new MockRpcError(`Unknown method: ${method}`)
  }
}

function handleSso(state: MockState, method: string, params: Record<string, unknown>): unknown {
  const sso = ssoOf(state)
  const now = nowSecs()
  switch (method) {
    case 'auth.sso_config':
      return {
        self_register_enabled: sso.settings.self_register_enabled,
        did_login_enabled: sso.settings.did_login_enabled,
        jump_base_url: sso.settings.jump_base_url || DEFAULT_JUMP_BASE_URL,
      }
    case 'auth.current_identity':
      requireSignedIn(state, method)
      return identityOf(state)
    case 'auth.entry.describe':
      return effectiveEntry(state, stringParam(params, 'service'), stringParam(params, 'entry_id'))
    case 'auth.register': {
      const service = stringParam(params, 'service')
      const entryId = stringParam(params, 'entry_id')
      const entry = effectiveEntry(state, service, entryId)
      if (entry.state !== 'active') return { status: 'denied', denial: denialFor('entry_unavailable', undefined, method) }
      if (!entry.methods.self_register) {
        const closed = entry.zone_disabled?.includes('self_register')
        return { status: 'denied', denial: denialFor(closed ? 'zone_method_disabled' : 'entry_unavailable', undefined, method) }
      }
      const userId = stringParam(params, 'user_id')
      if (!USER_ID_PATTERN.test(userId)) throw new MockRpcError('user_id contains invalid characters')
      stringParam(params, 'password_hash')
      if (userTaken(state, userId)) return { status: 'name_taken' }
      const showName = typeof params.show_name === 'string' && params.show_name.trim() ? params.show_name.trim() : userId
      sso.accounts[userId] = { show_name: showName, password_set: true, state: 'active' }
      return { status: 'created', user_id: userId }
    }
    case 'auth.login': {
      const username = stringParam(params, 'username')
      stringParam(params, 'password')
      const ssoAccount = sso.accounts[username]
      const zoneAccount = state.zone_users[username]
      if (!ssoAccount && !zoneAccount) throw new MockRpcError(`user not found: ${username}`)
      if (ssoAccount?.state === 'disabled') deny('account_state', method)
      if (ssoAccount && !ssoAccount.password_set) deny('credential_state', method, 'password_unset')
      signInAs(state, username, ssoAccount ? 'limited' : zoneAccount.user_type, ssoAccount?.show_name ?? (state.account.user_id === username ? state.account.user_name : username), 'password')
      return {
        session_token: `mock-session-${username}`,
        refresh_token: 'mock-refresh',
        sso_nonce: 0,
        user_info: { user_id: state.account.user_id, user_name: state.account.user_name, user_type: state.account.user_type },
        identity: identityOf(state),
      }
    }
    case 'auth.invite.describe': {
      const decision = inviteDecision(state, stringParam(params, 'invite_id'), stringParam(params, 'secret'))
      if ('reason' in decision) {
        // A valid link of an account that now has a password: its address stays an entry to password sign-in.
        const invite = sso.invites.find((item) => item.invite_id === params.invite_id)
        const passwordEntry = decision.detail === 'password_required' && invite?.secret === params.secret
        return { status: 'denied', denial: decision, ...(passwordEntry && invite ? { user_id: invite.user_id, return_path: invite.return_path } : {}) }
      }
      const { invite, account } = decision
      return {
        status: 'active',
        user_id: invite.user_id,
        show_name: account.show_name,
        ...(invite.resource?.title ? { resource_title: invite.resource.title } : {}),
        return_path: invite.return_path,
      }
    }
    case 'auth.invite.login': {
      const decision = inviteDecision(state, stringParam(params, 'invite_id'), stringParam(params, 'secret'))
      if ('reason' in decision) return { status: 'denied', denial: decision }
      signInAs(state, decision.invite.user_id, 'limited', decision.account.show_name, 'exclusive_invite')
      return { status: 'ready', callback_url: decision.invite.return_path }
    }
    case 'auth.invite.request_resend': {
      const invite = sso.invites.find((item) => item.invite_id === params.invite_id)
      const account = invite ? sso.accounts[invite.user_id] : undefined
      const expired = invite?.credential.state === 'active' && now > invite.credential.expires_at
      if (!invite || invite.state !== 'active' || !account || account.password_set || !expired) return { status: 'not_applicable' }
      if (invite.resend && now - invite.resend.requested_at < RESEND_INTERVAL_SECS) return { status: 'rate_limited' }
      invite.resend = { requested_at: now, count: (invite.resend?.count ?? 0) + 1 }
      invite.updated_at = now
      return { status: 'submitted' }
    }
    case 'auth.invite.set_password': {
      requireSignedIn(state, method)
      stringParam(params, 'new_password_hash')
      const account = sessionOf(state).kind === 'account' ? sso.accounts[state.account.user_id] : undefined
      if (!account) deny('session_kind_not_allowed', method)
      if (account.password_set) deny('credential_state', method, 'password_required')
      account.password_set = true
      for (const invite of sso.invites) {
        if (invite.user_id === state.account.user_id && invite.credential.state === 'active') {
          invite.credential.state = 'consumed_by_password'
          invite.updated_at = now
        }
      }
      return { status: 'ok' }
    }
    case 'auth.did.begin': {
      const did = stringParam(params, 'did')
      const redirectUrl = stringParam(params, 'redirect_url')
      if (!sso.settings.did_login_enabled) return { status: 'denied', denial: denialFor('zone_method_disabled', undefined, method) }
      if (!did.startsWith('did:')) throw new MockRpcError('invalid did')
      if (did.includes('offline')) return { status: 'canonical_unavailable' }
      const canonicalHost = `${did.split(':').pop()}.web3.buckyos.ai`
      const url = new URL(`https://${canonicalHost}/sso/federate`)
      url.searchParams.set('did', did)
      url.searchParams.set('client_zone', `did:bns:${state.zone}`)
      url.searchParams.set('redirect_uri', `${window.location.origin}/sso/did_callback`)
      const flowState = `mock-state-${encodeURIComponent(redirectUrl).length}`
      sso.did_logins ??= {}
      sso.did_logins[flowState] = { did, redirect_url: redirectUrl }
      url.searchParams.set('state', flowState)
      url.searchParams.set('nonce', 'mock-nonce')
      url.searchParams.set('code_challenge', 'mock-challenge')
      return { status: 'redirect', url: url.toString() }
    }
    case 'auth.federation.describe': {
      const { clientZone, clientHost } = federationChecks(state, method, params)
      return {
        status: 'consent',
        client_zone: clientZone,
        client_host: clientHost,
        profile: { show_name: state.account.user_name, avatar: state.account.profile?.avatar ?? null },
        already_granted: sso.consents.includes(clientZone),
      }
    }
    case 'auth.federation.approve': {
      const { clientZone, redirectUri, flowState } = federationChecks(state, method, params)
      if (params.remember === true && !sso.consents.includes(clientZone)) sso.consents.push(clientZone)
      const url = new URL(redirectUri)
      url.searchParams.set('code', 'fc-mock')
      url.searchParams.set('state', flowState)
      return { status: 'redirect', url: url.toString() }
    }
    case 'auth.federation.deny': {
      const { redirectUri, flowState } = federationChecks(state, method, params)
      const url = new URL(redirectUri)
      url.searchParams.set('error', 'access_denied')
      url.searchParams.set('state', flowState)
      return { status: 'redirect', url: url.toString() }
    }
    case 'sso.settings.get':
      requireSsoAdmin(state, method)
      return { ...sso.settings }
    case 'sso.settings.set': {
      requireSsoAdmin(state, method)
      if (typeof params.self_register_enabled !== 'boolean' || typeof params.did_login_enabled !== 'boolean') {
        throw new MockRpcError('self_register_enabled and did_login_enabled must be booleans')
      }
      const jump = typeof params.jump_base_url === 'string' ? params.jump_base_url.trim() : ''
      if (jump && !/^https:\/\/[^/?#]+\/?$/.test(jump)) throw new MockRpcError('jump_base_url must be an https origin')
      sso.settings = {
        self_register_enabled: params.self_register_enabled,
        did_login_enabled: params.did_login_enabled,
        ...(jump ? { jump_base_url: jump } : {}),
      }
      return { ...sso.settings }
    }
    case 'user.exclusive_invite.create': {
      requireSsoAdmin(state, method)
      const channel = params.channel as Record<string, unknown> | undefined
      if (!channel || typeof channel.kind !== 'string' || !channel.kind.trim()) throw new MockRpcError('channel.kind is required')
      const resource = inviteResourceParam(params.resource)
      const newUser = params.new_user as Record<string, unknown> | undefined
      let userId: string
      if (newUser && params.user_id === undefined) {
        userId = stringParam(newUser, 'user_id')
        if (!USER_ID_PATTERN.test(userId)) throw new MockRpcError('user_id contains invalid characters')
        if (userTaken(state, userId)) throw new MockRpcError(`user '${userId}' already exists`)
        const showName = typeof newUser.show_name === 'string' && newUser.show_name.trim() ? newUser.show_name.trim() : userId
        sso.accounts[userId] = { show_name: showName, password_set: false, state: 'active' }
      } else if (!newUser && typeof params.user_id === 'string') {
        userId = params.user_id
        const limited = sso.accounts[userId] || parseUserType(state.zone_users[userId]?.user_type) === 'limited'
        if (!limited) throw new MockRpcError(`'${userId}' is not a Limited account`)
      } else {
        throw new MockRpcError('give either user_id or new_user')
      }
      const invite: MockInvite = {
        ...mockInvite({
          invite_id: `ei-${randomToken().slice(0, 10)}`,
          user_id: userId,
          secret: randomToken(),
          issued_at: now,
          resource,
          return_path: typeof params.return_path === 'string' && params.return_path.startsWith('/') ? params.return_path : defaultReturnPath(resource),
          target_hint: typeof channel.target_hint === 'string' ? channel.target_hint : '',
        }),
        issuer: { kind: 'user', user_id: state.account.user_id },
        channel: { kind: channel.kind.trim(), target_hint: typeof channel.target_hint === 'string' ? channel.target_hint : '' },
        resource_state: resource ? (resource.kind === 'group_session' && resource.group_did.includes('offline') ? 'pending' : 'done') : 'none',
      }
      sso.invites.push(invite)
      return { invite: publicInvite(invite), link: inviteLink(invite) }
    }
    case 'user.exclusive_invite.list': {
      requireSsoAdmin(state, method)
      const userId = typeof params.user_id === 'string' ? params.user_id : null
      return {
        invites: sso.invites
          .filter((invite) => !userId || invite.user_id === userId)
          .sort((left, right) => right.created_at - left.created_at)
          .map(publicInvite),
      }
    }
    case 'user.exclusive_invite.get':
      requireSsoAdmin(state, method)
      return { invite: publicInvite(findInvite(state, params)) }
    case 'user.exclusive_invite.revoke': {
      requireSsoAdmin(state, method)
      const invite = findInvite(state, params)
      invite.state = 'revoked'
      invite.updated_at = now
      return { invite: publicInvite(invite) }
    }
    case 'user.exclusive_invite.rotate': {
      requireSsoAdmin(state, method)
      const invite = findInvite(state, params)
      if (invite.state === 'revoked') deny('source_revoked', method, 'revoked')
      if (sso.accounts[invite.user_id]?.password_set) deny('credential_state', method, 'password_required')
      invite.secret = randomToken()
      invite.credential = { version: invite.credential.version + 1, issued_at: now, expires_at: now + INVITE_TTL_SECS, state: 'active' }
      invite.resend = null
      invite.updated_at = now
      return { invite: publicInvite(invite), link: inviteLink(invite) }
    }
    case 'user.exclusive_invite.retry_resource': {
      requireSsoAdmin(state, method)
      const invite = findInvite(state, params)
      if (invite.resource_state === 'pending') invite.resource_state = 'done'
      invite.updated_at = now
      return { invite: publicInvite(invite) }
    }
    default:
      throw new MockRpcError(`Unknown method: ${method}`)
  }
}

function handle(state: MockState, method: string, params: Record<string, unknown>): unknown {
  switch (method) {
    case 'user.invite.get':
    case 'user.invite.accept': {
      const invite = state.user_invites?.[stringParam(params, 'invite_id')]
      if (!invite) throw new MockRpcError('Invitation not found')
      const expired = invite.expires_at != null && invite.expires_at < nowSecs()
      if (method === 'user.invite.get') {
        return { invite, expired, zone_did: `did:bns:${state.zone}`, zone_host: zoneHostOf(state) }
      }
      const identity = identityOf(state)
      if (identity.kind === 'anonymous' || identity.auth_method !== 'did_federation') deny('session_kind_not_allowed', method)
      const document = typeof params.owner_config === 'string' ? JSON.parse(params.owner_config) : params.owner_config
      if (!document || typeof document !== 'object' || Array.isArray(document)) throw new MockRpcError('Invalid owner_config')
      const owner = document as Record<string, unknown>
      if (owner.id !== identity.did || (invite.target_did && invite.target_did !== identity.did)) {
        deny('session_ceiling_exceeded', method)
      }
      if (expired || invite.state !== 'pending') throw new MockRpcError('Invitation is no longer pending')
      const userId = invite.target_user_id || String(owner.name || '').trim().toLowerCase()
      if (!userId) throw new MockRpcError('Account name is required')
      invite.state = 'accepted'
      invite.accepted_at = nowSecs()
      invite.accepted_user_id = userId
      state.zone_users[userId] = { user_type: invite.default_user_type }
      if (!state.user_names.includes(userId)) state.user_names.push(userId)
      return { ok: true, invite, user_id: userId, state: 'active' }
    }
    case 'agent.check_name':
      requireCreator(state)
      return checkName(state, stringParam(params, 'name'))
    case 'agent.list_templates':
      return { templates: state.templates.map((template) => ({ ...template })) }
    case 'agent.runtime_directory': {
      if (state.runtime_directory_error) throw new MockRpcError(state.runtime_directory_error)
      const agent = typeof params.agent_id === 'string' ? findAgent(state, params.agent_id) : null
      const owner = agent?.owner_user_id ?? state.account.user_id
      const containerAvailable = state.container_available !== false
      const host = owner === state.zone_owner ? state.runtime_offers.find((offer) => offer.runtime_id === 'host') : undefined
      const defaultTarget = agent?.settings.runtimes?.[agent.settings.default_runtime_id ?? '']?.target
      return {
        container_available: containerAvailable,
        host_by_default: Boolean(host && !host.closed && (host.os !== 'linux' || !containerAvailable)),
        agent_service: owner === state.zone_owner && defaultTarget?.kind === 'node'
          ? { mode: 'native', node_id: defaultTarget.node_id }
          : { mode: 'container' },
        runtimes: owner === state.zone_owner ? state.runtime_offers.map((offer) => ({
          ...offer,
          enabled: agent?.settings.runtimes?.[offer.runtime_id]?.enabled ?? false,
          computer: agent?.settings.runtimes?.[offer.runtime_id]?.computer ?? 'none',
        })) : [],
        default_runtime_id: agent?.settings.default_runtime_id ?? null,
        container: 'container',
      }
    }
    case 'agent.create':
      return createAgent(state, params)
    case 'agent.create.status':
      return { ...findAgent(state, stringParam(params, 'agent_id')).install }
    case 'agent.create.retry':
      return retryCreate(state, params)
    case 'agent.create.cancel':
      return cancelCreate(state, params)
    case 'agent.list':
      return { agents: visibleAgents(state).map((agent) => toEntry(state, agent)) }
    case 'agent.get':
      return toEntry(state, findAgent(state, stringParam(params, 'agent_id')))
    case 'agent.update': {
      const agent = findAgent(state, stringParam(params, 'agent_id'))
      if (typeof params.allow_group === 'boolean') agent.settings.allow_group = params.allow_group
      if (typeof params.homestation === 'boolean') agent.settings.homestation = params.homestation
      if (typeof params.auto_share_results === 'boolean') agent.settings.auto_share_results = params.auto_share_results
      return toEntry(state, agent)
    }
    case 'agent.profile.get':
      return { profile: { ...findAgent(state, stringParam(params, 'agent_id')).profile } }
    case 'agent.profile.set': {
      const agent = findAgent(state, stringParam(params, 'agent_id'))
      for (const key of ['display_name', 'avatar', 'bio'] as const) {
        const value = params[key]
        if (typeof value === 'string') agent.profile[key] = value || null
      }
      return { profile: { ...agent.profile } }
    }
    case 'agent.delete': {
      const agent = findAgent(state, stringParam(params, 'agent_id'))
      state.agents = state.agents.filter((item) => item !== agent)
      return { ok: true }
    }
    case 'user.get':
      return userDetail(state)
    case 'user.list':
      return userList(state)
    case 'user.change_type': {
      requireAdmin(state)
      const userId = stringParam(params, 'user_id')
      const user = zoneUser(state, userId)
      const target = parseUserType(params.user_type)
      if (!target || !(ASSIGNABLE_USER_TYPES as readonly string[]).includes(target)) throw new MockRpcError(`Invalid user_type: ${String(params.user_type)}`)
      if (user.user_type === 'root') throw new MockRpcError('Cannot change root user type')
      user.user_type = target
      if (userId === state.account.user_id) state.account.user_type = target
      return { ok: true, user_id: userId, user_type: target }
    }
    case 'user.set_account_options': {
      requireAdmin(state)
      const userId = stringParam(params, 'user_id')
      const user = zoneUser(state, userId)
      const password = optionParam(params, 'allow_password_change')
      const contacts = optionParam(params, 'allow_contact_edit')
      if (password !== undefined) user.allow_password_change = password
      if (contacts !== undefined) user.allow_contact_edit = contacts
      return { ok: true, user_id: userId, ...effectiveOptions(user) }
    }
    case 'user.profile.set': {
      const profile = { ...state.account.profile }
      applyTextPatch(profile, params, PROFILE_TEXT_FIELDS)
      if (profile.avatar && !profile.avatar.startsWith('data:image/')) throw new MockRpcError('invalid avatar: expect a base64 image data URL')
      state.account.profile = profile
      return { ok: true, user_id: state.account.user_id, profile: userDetail(state).local_profile }
    }
    case 'zone.presentation.get':
      return zonePresentation(state)
    case 'zone.presentation.set': {
      if (!isAdmin(state.account)) {
        throw new MockRpcError('only Zone administrators can change the Zone display settings')
      }
      const settings: Record<string, string> = { ...state.zone_presentation }
      applyTextPatch(settings, params, ['display_name', 'description', 'icon'])
      state.zone_presentation = settings
      return zonePresentation(state)
    }
    case 'user.set_msg_tunnel': {
      const platform = stringParam(params, 'platform')
      const binding: UserTunnelBinding = {
        platform,
        account_id: stringParam(params, 'account_id'),
        ...(typeof params.display_id === 'string' ? { display_id: params.display_id } : {}),
        status: 'active',
      }
      const index = state.user_bindings.findIndex((item) => item.platform === platform)
      if (index >= 0) state.user_bindings[index] = binding
      else state.user_bindings.push(binding)
      return { ok: true, user_id: state.account.user_id, platform, total_bindings: state.user_bindings.length, contact: userDetail(state).local_profile.private_extra.system_contact }
    }
    case 'user.remove_msg_tunnel': {
      const platform = stringParam(params, 'platform')
      const before = state.user_bindings.length
      state.user_bindings = state.user_bindings.filter((item) => item.platform !== platform)
      if (state.user_bindings.length === before) throw new MockRpcError(`No binding for platform '${platform}' found on user '${state.account.user_id}'`)
      return { ok: true, user_id: state.account.user_id, platform, remaining_bindings: state.user_bindings.length, contact: userDetail(state).local_profile.private_extra.system_contact }
    }
    default:
      if (method.startsWith('user_group.')) return handleUserGroup(state, method, params)
      if (method.startsWith('auth.') || method.startsWith('sso.') || method.startsWith('user.exclusive_invite.')) return handleSso(state, method, params)
      if (method.startsWith('apps.app_login.')) return handleAppLogin(state, method, params)
      throw new MockRpcError(`Unknown method: ${method}`)
  }
}

export async function callMockControlPanel<T>(
  method: string,
  params: Record<string, unknown>,
): Promise<{ data: T | null; error: unknown }> {
  await new Promise((resolve) => window.setTimeout(resolve, 120))
  const state = loadState()
  advanceAll(state)
  try {
    const data = handle(state, method, params) as T
    saveState(state)
    return { data, error: null }
  } catch (error) {
    saveState(state)
    return { data: null, error }
  }
}

/** For the network mock: whether the signed-in mock account may edit its address book. */
export function mockContactEditAllowed(): boolean {
  const state = loadState()
  const facts = state.zone_users[state.account.user_id] ?? { user_type: state.account.user_type }
  return effectiveOptions({ ...facts, user_type: state.account.user_type }).allow_contact_edit
}

/** The signed-in account; null when the session is an External DID or there is none. */
export function mockCurrentAccount() {
  const state = loadState()
  if (sessionOf(state).kind !== 'account') return null
  const { account } = state
  return { user_id: account.user_id, user_name: account.user_name, user_type: account.user_type }
}

export function mockCurrentIdentity(): CurrentIdentity {
  return identityOf(loadState())
}

/** `/sso_logout` of the mock: the browser speaks for nobody until the next sign-in. */
export function mockEndSession() {
  const state = loadState()
  state.session = { kind: 'anonymous' }
  saveState(state)
}

/** For the MessageHub mock: whether a control-panel Agent may join groups (null when it is not one). */
export function mockAgentGroupAllowed(agentDid: string): boolean | null {
  const agent = liveAgents(loadState()).find((item) => item.agent_did === agentDid)
  return agent ? agent.settings.allow_group : null
}
