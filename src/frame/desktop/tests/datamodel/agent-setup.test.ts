import type { AgentEntry, AgentStatus, UserDetail } from '../../src/api/user_mgr.ts'
import type { AppSummary } from '../../src/api/app_mgr.ts'
import {
  agentDisplayName,
  agentHomeTarget,
  buildAgentCreateRequest,
  canCancelCreation,
  classifyAgentError,
  computeAgentGuideState,
  creationStepPhases,
  isTelegramBotToken,
  isValidAgentName,
  newAgentSetupDraft,
  ownerChannelBinding,
  parseAgentSetupDraft,
  applyTemplateToDraft,
  pickTemplate,
} from '../../src/app/agent-setup/model.ts'
import { agentStateToStatus, toAgentEntity } from '../../src/app/users-agents/datamodel/transforms.ts'
import { createBackendAppDefinitionMapper, launcherDefinitions, withAccountBuiltins } from '../../src/app/backend-apps.ts'
import type { AppDefinition } from '../../src/models/ui.ts'

function assertEquals<T>(actual: T, expected: T, message: string) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(`${message}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`)
  }
}

function status(overrides: Partial<AgentStatus> = {}): AgentStatus {
  return {
    agent_id: 'xiaobai.test.buckyos.io',
    agent_did: 'did:web:xiaobai.test.buckyos.io',
    owner_user_id: 'devtest',
    state: 'ready',
    step: 'done',
    tunnel_state: 'none',
    created_at: 100,
    updated_at: 100,
    ...overrides,
  }
}

function entry(overrides: Partial<AgentEntry> = {}, install: Partial<AgentStatus> = {}): AgentEntry {
  return {
    agent_id: 'xiaobai.test.buckyos.io',
    agent_did: 'did:web:xiaobai.test.buckyos.io',
    owner_user_id: 'devtest',
    owner_did: 'did:web:devtest.test.buckyos.io',
    name: 'xiaobai',
    display_name: '小白',
    profile: { display_name: '小白', avatar: 'data:image/jpeg;base64,AAAA' },
    settings: {
      enabled: true,
      auto_start: true,
      allow_other_users: false,
      allow_group: false,
      role_supplement: '',
      template_auto_update: true,
      desktop_entry: 'jarvis_guide',
      msg_tunnels: [],
    },
    install: status(install),
    template: { template_id: 'bundled:jarvis.buckyos.bns.did', source: 'bundled', app_did: 'did:bns:jarvis.buckyos', version: '0.7.0' },
    runtime: { app_instance_id: 'xiaobai.test.buckyos.io@devtest', app_host_name: 'xiaobai', has_web: true, state: 'running' },
    ...overrides,
  }
}

Deno.test('display name is the nickname, else the user name', () => {
  assertEquals(agentDisplayName(entry()), '小白', 'nickname')
  assertEquals(agentDisplayName(entry({ display_name: '', profile: { display_name: '  ' } })), 'xiaobai', 'blank nickname falls back to the user name')
})

Deno.test('the desktop guide follows only the caller\'s own guide Agent', () => {
  assertEquals(computeAgentGuideState([], 'devtest').kind, 'none', 'nothing created')
  const others = entry({ owner_user_id: 'bob' })
  assertEquals(computeAgentGuideState([others], 'devtest').kind, 'none', 'another user\'s Agent never links my guide')
  const fromUsersAgents = entry({ settings: { ...entry().settings, desktop_entry: null } })
  assertEquals(computeAgentGuideState([fromUsersAgents], 'devtest').kind, 'none', 'an Agent created elsewhere does not take the guide')
  assertEquals(computeAgentGuideState([entry({}, { state: 'provisioning', step: 'runtime' })], 'devtest').kind, 'creating', 'in progress')
  assertEquals(computeAgentGuideState([entry({}, { state: 'bound', step: 'start' })], 'devtest').kind, 'creating', 'bound is still creating')
  assertEquals(computeAgentGuideState([entry({}, { state: 'failed', step: 'runtime' })], 'devtest').kind, 'failed', 'failed')
  assertEquals(computeAgentGuideState([entry({}, { state: 'removed' })], 'devtest').kind, 'none', 'a removed Agent returns the guide')
  const stopped = entry({ runtime: { app_instance_id: 'x@devtest', app_host_name: 'x', has_web: true, state: 'stopped' } })
  assertEquals(computeAgentGuideState([stopped], 'devtest').kind, 'linked', 'a stopped Agent stays linked')
  const failedOld = entry({ agent_id: 'old' }, { state: 'failed', created_at: 50 })
  const ready = entry({}, { created_at: 10 })
  const linked = computeAgentGuideState([failedOld, ready], 'devtest')
  assertEquals(linked.kind === 'linked' ? linked.agent.agent_id : null, ready.agent_id, 'a ready Agent wins over a failed one')
})

Deno.test('an Agent home is its App Web entry, else its details', () => {
  assertEquals(agentHomeTarget(entry()), { kind: 'app', appId: 'xiaobai.test.buckyos.io@devtest' }, 'web entry')
  assertEquals(agentHomeTarget(entry({ runtime: null })), { kind: 'details' }, 'no App yet')
  assertEquals(agentHomeTarget(entry({ runtime: { app_instance_id: 'a@b', app_host_name: 'a', has_web: false } })), { kind: 'details' }, 'no web')
})

Deno.test('agent errors are classified by their contract prefix anywhere in the RPC message', () => {
  assertEquals(classifyAgentError(new Error('RPC call error: Failed due to reason: name_conflict: xiaobai is taken')), { kind: 'name_conflict', detail: 'xiaobai is taken' }, 'name conflict')
  assertEquals(classifyAgentError(new Error('RPC call error: Failed due to reason: limited_user: nope')).kind, 'limited_user', 'limited')
  assertEquals(classifyAgentError(new Error('RPC call error: owner_identity_missing: add telegram')).kind, 'owner_identity_missing', 'owner identity')
  assertEquals(classifyAgentError(new Error('RPC call error: Token expired:abc')).kind, 'session_expired', 'expired token')
  assertEquals(classifyAgentError(new Error('RPC call failed: Failed to fetch')).kind, 'network', 'network')
  assertEquals(classifyAgentError(new Error('RPC call error: Failed due to reason: boom')), { kind: 'other', detail: 'boom' }, 'other keeps the reason')
  assertEquals(classifyAgentError(new Error('RPC call error: Failed due to reason: invalid_bot_token: bad')).kind, 'invalid_bot_token', 'bot token')
  assertEquals(classifyAgentError(new Error('RPC call error: Failed due to reason: template_invalid: broken')).kind, 'template_unavailable', 'template invalid')
  assertEquals(classifyAgentError(new Error('RPC call error: Failed due to reason: agent_not_found: x')).kind, 'not_found', 'deleted agent')
})

Deno.test('names, tokens and Owner identities are validated like the contract', () => {
  assertEquals(['xiaobai', 'a', 'devtest-jarvis-2'].map(isValidAgentName), [true, true, true], 'valid labels')
  assertEquals(['', '-a', 'a-', 'Xiaobai', 'a_b', 'a'.repeat(64)].map(isValidAgentName), [false, false, false, false, false, false], 'invalid labels')
  assertEquals(isTelegramBotToken('123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw'), true, 'bot token')
  assertEquals(isTelegramBotToken('123456789:short'), false, 'short token')
  const detail = { local_profile: { private_extra: { system_contact: { bindings: [{ platform: 'email', account_id: 'a@b' }, { platform: 'telegram', account_id: '42424242' }] } } } } as unknown as UserDetail
  assertEquals(ownerChannelBinding(detail, 'telegram')?.account_id, '42424242', 'telegram identity')
  assertEquals(ownerChannelBinding({ local_profile: {} } as unknown as UserDetail, 'telegram'), null, 'missing identity')
})

Deno.test('creation steps follow the status, with the tunnel only when one was submitted', () => {
  assertEquals(creationStepPhases(status({ state: 'provisioning', step: 'runtime' })).map((item) => item.phase), ['active', 'pending', 'pending'], 'runtime running')
  assertEquals(creationStepPhases(status({ state: 'bound', step: 'tunnel', tunnel_state: 'pending' })).map((item) => item.phase), ['done', 'done', 'active', 'pending'], 'tunnel running')
  const failed = status({ state: 'failed', step: 'tunnel', tunnel_state: 'failed', last_error: { step: 'tunnel', code: 'x', message: 'y', retryable: true } })
  assertEquals(creationStepPhases(failed).map((item) => item.phase), ['done', 'done', 'failed', 'pending'], 'tunnel failed')
  assertEquals(creationStepPhases(status({ tunnel_state: 'skipped' })).map((item) => item.phase), ['done', 'done', 'skipped', 'done'], 'skipped tunnel')
  assertEquals(canCancelCreation(status({ state: 'failed', step: 'runtime', last_error: { step: 'runtime', code: 'x', message: 'y', retryable: true } })), true, 'runtime failure can be cancelled')
  assertEquals(canCancelCreation(failed), false, 'after the spec is written it cannot')
})

Deno.test('drafts restore only for their own entry and never keep secrets', () => {
  const draft = { ...newAgentSetupDraft('jarvis_guide', 'key-1'), page: 'channel' as const, channel: 'telegram' as const, botToken: 'secret' }
  const restored = parseAgentSetupDraft(JSON.parse(JSON.stringify(draft)), 'jarvis_guide')
  assertEquals(restored?.page, 'channel', 'page restored')
  assertEquals(restored && 'botToken' in restored, false, 'no token field')
  assertEquals(parseAgentSetupDraft(draft, 'users_agents'), null, 'other entry')
  assertEquals(newAgentSetupDraft('jarvis_guide', 'k').name, 'jarvis', 'guide prefill')
  assertEquals(newAgentSetupDraft('users_agents', 'k').displayName, '', 'Add Agent starts blank')
})

Deno.test('a template fills only an untouched bio, and the create request follows the contract', () => {
  const templates = [
    { template_id: 'bundled:jarvis', source: 'bundled' as const, app_id: 'jarvis', app_did: 'did:bns:jarvis', name: 'jarvis', show_name: 'Jarvis', description: 'Base', version: '0.7.0', loader: 'opendan', is_default: true },
    { template_id: 'installed:coder', source: 'installed' as const, app_id: 'coder', app_did: 'did:bns:coder', name: 'coder', show_name: 'Coder', description: 'Code', version: '1.0.0', loader: 'opendan', is_default: false },
  ]
  assertEquals(pickTemplate(templates, null)?.template_id, 'bundled:jarvis', 'default template')
  const draft = { ...newAgentSetupDraft('jarvis_guide', 'k'), displayName: '小白', roleSupplement: '日本語で', templateId: 'installed:coder' }
  assertEquals(applyTemplateToDraft(draft, templates[1]).bio, 'Code', 'untouched bio follows')
  const edited = applyTemplateToDraft({ ...draft, bio: 'Mine', bioEdited: true }, templates[0])
  assertEquals([edited.bio, edited.displayName, edited.roleSupplement], ['Mine', '小白', '日本語で'], 'edits are kept')
  const request = buildAgentCreateRequest({ ...edited, name: 'xiaobai', channel: 'telegram' }, ' 1:token ')
  assertEquals(request, {
    idempotency_key: 'k',
    name: 'xiaobai',
    profile: { display_name: '小白', bio: 'Mine' },
    role_supplement: '日本語で',
    allow_group: false,
    allow_other_users: false,
    template_id: 'bundled:jarvis',
    template_auto_update: true,
    desktop_entry: 'jarvis_guide',
    msg_tunnel: { platform: 'telegram', bot_token: '1:token' },
  }, 'request')
  const plain = buildAgentCreateRequest({ ...newAgentSetupDraft('users_agents', 'k2'), name: 'b', templateId: 't' }, 'ignored')
  assertEquals(['desktop_entry' in plain, 'msg_tunnel' in plain], [false, false], 'Add Agent sends no desktop entry and a skipped channel sends no tunnel')
})

Deno.test('unknown Agent states map to unknown, never to an error', () => {
  assertEquals(agentStateToStatus('ready', 'running'), 'running', 'running')
  assertEquals(agentStateToStatus('ready', 'stopped'), 'stopped', 'stopped')
  assertEquals(agentStateToStatus('ready', 'something-new'), 'unknown', 'unknown runtime state')
  assertEquals(agentStateToStatus('ready', null), 'unknown', 'no runtime yet')
  assertEquals(agentStateToStatus('provisioning', null), 'creating', 'creating')
  assertEquals(agentStateToStatus('failed', 'running'), 'failed', 'failed creation')
  assertEquals(agentStateToStatus('weird', 'running'), 'unknown', 'unknown install state')
  const entity = toAgentEntity(entry({ settings: { ...entry().settings, msg_tunnels: [{ platform: 'telegram', bot_account_id: 'xiaobai_bot' }] } }, { tunnel_state: 'bound' }))
  assertEquals([entity.id, entity.displayName, entity.createdFromGuide, entity.socialAccounts[0]?.displayId], ['xiaobai.test.buckyos.io', '小白', true, '@xiaobai_bot'], 'entity mapping')
})

Deno.test('constructed Agent Apps keep a definition but no launcher icon; the guide follows the account', () => {
  const summary = {
    app_id: 'xiaobai.test.buckyos.io', app_instance_id: 'xiaobai.test.buckyos.io@devtest', app_did: 'did:web:xiaobai.test.buckyos.io',
    runtime_type: 'agent', owner_user_id: 'devtest', availability_match: null, show_name: '小白', version: '0.7.0', app_icon_url: null,
    icon_res_url: '', author: '', app_index: 1, enable: true, state: 'running', expected_instance_count: 1, spec_path: '', web_hosts: ['xiaobai'],
  } as unknown as AppSummary
  const definition = createBackendAppDefinitionMapper([])(summary)
  assertEquals([definition.id, definition.manifest.showInLauncher, definition.webHosts], ['xiaobai.test.buckyos.io@devtest', false, ['xiaobai']], 'agent app')
  assertEquals(launcherDefinitions([definition]).length, 0, 'not on the launcher')
  const guide = { id: 'agent-guide' } as AppDefinition
  assertEquals(withAccountBuiltins([guide], { agentGuide: false }).length, 0, 'hidden for limited users')
  assertEquals(withAccountBuiltins([guide], { agentGuide: true }).length, 1, 'shown otherwise')
})
