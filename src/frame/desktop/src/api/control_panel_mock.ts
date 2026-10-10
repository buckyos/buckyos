/* ── In-browser control panel for the mock runtime ──
 *
 * Answers the Agent (`agent.*`) and own-profile (`user.get`, `user.*_msg_tunnel`)
 * calls with the shapes of the Agent init contract §3. State lives in
 * localStorage under MOCK_CONTROL_PANEL_STORAGE_KEY and is re-read on every
 * call, so e2e tests can seed or change it (a partial object is merged over
 * the defaults).
 *
 * Scripted outcomes: an Agent name starting with `fail-runtime` / `fail-start`
 * fails that creation step once; a bot token containing `FAIL` fails the
 * tunnel step once.
 */

import type {
  AgentCreateRequest,
  AgentCreateStep,
  AgentEntry,
  AgentNameCheck,
  AgentProfile,
  AgentSettings,
  AgentStatus,
  AgentTemplate,
  AgentTemplateBinding,
  UserTunnelBinding,
} from './user_mgr.ts'

export const MOCK_CONTROL_PANEL_STORAGE_KEY = 'buckyos.mock.control-panel.v1'

interface MockAccount {
  user_id: string
  user_name: string
  user_type: string
  did: string
}

interface MockAgent {
  agent_id: string
  agent_did: string
  owner_user_id: string
  owner_did: string
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
  agents: MockAgent[]
  step_ms: number
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
    agents: [
      {
        agent_id: 'assistant.alice',
        agent_did: 'did:bns:assistant.alice',
        owner_user_id: 'alice',
        owner_did: 'did:bns:alice',
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
  return account.user_type === 'limited' || account.user_type === 'guest'
}

function isAdmin(account: MockAccount) {
  return account.user_type === 'admin' || account.user_type === 'root'
}

function requireCreator(state: MockState) {
  if (isLimited(state.account)) throw new MockRpcError('limited_user: limited users cannot create agents')
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

function userDetail(state: MockState) {
  const { account } = state
  const contact = { did: account.did, bindings: state.user_bindings.map((binding) => ({ ...binding })) }
  return {
    user_id: account.user_id,
    show_name: account.user_name,
    user_type: account.user_type,
    state: 'active',
    res_pool_id: 'default',
    is_local: false,
    profile: { did: account.did, display_name: account.user_name },
    local_profile: {
      did: account.did,
      display_name: account.user_name,
      private_extra: { system_contact: contact },
    },
  }
}

function createAgent(state: MockState, params: Record<string, unknown>) {
  requireCreator(state)
  const request = params as unknown as AgentCreateRequest
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
    name,
    profile: {
      ...(request.profile.display_name?.trim() ? { display_name: request.profile.display_name.trim() } : {}),
      ...(request.profile.avatar ? { avatar: request.profile.avatar } : {}),
      ...(request.profile.bio?.trim() ? { bio: request.profile.bio.trim() } : {}),
    },
    settings: {
      ...defaultSettings(),
      allow_group: Boolean(request.allow_group),
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

function handle(state: MockState, method: string, params: Record<string, unknown>): unknown {
  switch (method) {
    case 'agent.check_name':
      requireCreator(state)
      return checkName(state, stringParam(params, 'name'))
    case 'agent.list_templates':
      return { templates: state.templates.map((template) => ({ ...template })) }
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

export function mockCurrentAccount() {
  const { account } = loadState()
  return { user_id: account.user_id, user_name: account.user_name, user_type: account.user_type }
}

/** For the MessageHub mock: whether a control-panel Agent may join groups (null when it is not one). */
export function mockAgentGroupAllowed(agentDid: string): boolean | null {
  const agent = liveAgents(loadState()).find((item) => item.agent_did === agentDid)
  return agent ? agent.settings.allow_group : null
}
