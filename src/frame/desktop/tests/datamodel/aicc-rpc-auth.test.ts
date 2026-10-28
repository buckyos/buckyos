import { buckyos } from 'buckyos'
import { toAiccRpcCallOptions } from '../../src/api/aicc_rpc_options.ts'

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message)
}

Deno.test('AICC authentication is encoded in kRPC sys instead of business params', async () => {
  let request: Record<string, unknown> | undefined
  const client = new buckyos.kRPCClient('https://example.com/kapi/aicc', null, null, {
    fetcher: async (_input, init) => {
      request = JSON.parse(String(init?.body)) as Record<string, unknown>
      const sys = Array.isArray(request.sys) ? [request.sys[0]] : undefined
      return new Response(JSON.stringify({ result: {}, sys }), {
        headers: { 'content-type': 'application/json' },
      })
    },
  })

  await client.call('provider.catalog', {}, toAiccRpcCallOptions('session-token'))

  assert(request !== undefined, 'kRPC request must be captured')
  assert(Array.isArray(request.sys) && request.sys[1] === 'session-token', 'session token must use kRPC sys')
  assert(
    typeof request.params === 'object'
      && request.params !== null
      && !('session_token' in request.params),
    'session token must not use a business params field',
  )
})

Deno.test('AICC calls preserve the SDK token provider when no token is resolved', () => {
  assert(toAiccRpcCallOptions(null) === undefined, 'missing token must not override SDK call options')
})

Deno.test('provider inventory displays only direct LLM family mounts from the existing snapshot', async () => {
  const { createAICCMgr } = await import('../../src/api/aicc_mgr.ts')
  const sdk = buckyos as unknown as {
    getAccountInfo: () => Promise<unknown>
    getServiceRpcClient: (service: string) => unknown
  }
  const originalAccountInfo = sdk.getAccountInfo
  const originalClient = sdk.getServiceRpcClient
  const calls: string[] = []
  const exact = 'qwen3.8-max-0902:reasoning-medium@sn-main'
  const base = 'qwen3.8-max-0902@sn-main'
  const image = 'image@sn-main'
  const directory = {
    models: [
      { provider_instance_name: 'sn-main', model_driver_id: 'qwen', origin_model_id: 'qwen3.8-max', provider_model_id: 'qwen3.8-max-0902', exact_model: exact, api_types: ['llm'], logical_mounts: [] },
      { provider_instance_name: 'sn-main', model_driver_id: 'qwen', origin_model_id: 'qwen3.8-max', provider_model_id: 'qwen3.8-max-0902', exact_model: base, api_types: ['llm'], logical_mounts: ['llm.chat'] },
      { provider_instance_name: 'sn-main', model_driver_id: 'qwen', provider_model_id: 'image', exact_model: image, api_types: ['image.txt2img'], logical_mounts: ['image.general'] },
    ],
    directory: {
      'llm.chat': [{ name: 'direct', target: exact }],
      'llm.qwen-max': [{ name: 'preset', target: 'llm.qwen3-8-max:medium' }],
      'llm.qwen3-8-max': [{ name: exact, target: exact }],
      'image.general': [{ name: image, target: image }],
    },
    logical_definitions: [
      { path: 'llm.chat', kind: 'task', api_type: 'llm' },
      { path: 'llm.qwen-max', kind: 'spec', api_type: 'llm' },
      { path: 'llm.qwen3-8-max', kind: 'family', api_type: 'llm' },
      { path: 'image.general', kind: 'task', api_type: 'image.txt2img' },
    ],
  }
  sdk.getAccountInfo = async () => ({ session_token: 'test-session' })
  sdk.getServiceRpcClient = () => ({
    call: async (method: string) => {
      calls.push(method)
      if (method === 'models.list') return directory
      if (method === 'provider.list') return {
        settings_revision: 1,
        providers: [{ provider_instance_name: 'sn-main', provider_profile_id: 'sn', protocol_adapter_id: 'sn-openai', enabled: true, inventory: { state: 'loaded' }, health: { state: 'healthy' }, auth: { mode: 'api_key', configured: true } }],
      }
      return {}
    },
  })
  try {
    const manager = createAICCMgr({ useMock: false })
    await manager.refresh()
    const models = manager.getSnapshot().providers[0].inventory.models
    const preset = models.find((model) => model.exact_model === exact)!
    assert(preset.model_driver === 'qwen' && preset.provider_actual_model_id === 'qwen3.8-max', 'driver identity remains linked')
    assert(JSON.stringify(preset.logical_mounts) === JSON.stringify(['llm.qwen3-8-max']), 'only actual family membership is shown')
    assert(models.find((model) => model.exact_model === base)!.logical_mounts.length === 0, 'unmounted base must not inherit variant or legacy task mounts')
    assert(JSON.stringify(models.find((model) => model.exact_model === image)!.logical_mounts) === JSON.stringify(['image.general']), 'non-LLM mounts are preserved')
    assert(calls.filter((method) => method === 'models.list').length === 1, 'mounts use the existing model snapshot')
    assert(directory.models[0].logical_mounts.length === 0, 'the raw response is not mutated')
  } finally {
    sdk.getAccountInfo = originalAccountInfo
    sdk.getServiceRpcClient = originalClient
  }
})

Deno.test('custom catalog aliases and explicit OpenAI adapters survive the RPC boundary', async () => {
  const { createAICCMgr } = await import('../../src/api/aicc_mgr.ts')
  const sdk = buckyos as unknown as {
    getAccountInfo: () => Promise<unknown>
    getServiceRpcClient: (service: string) => unknown
  }
  const originalAccountInfo = sdk.getAccountInfo
  const originalClient = sdk.getServiceRpcClient
  const sent: Record<string, unknown>[] = []
  sdk.getAccountInfo = async () => ({ session_token: 'test-session' })
  sdk.getServiceRpcClient = () => ({
    call: async (method: string, params: Record<string, unknown>) => {
      if (method === 'provider.catalog') return { providers: [{
        provider_profile_id: 'deepseek', provider_rules_id: 'deepseek',
        model_driver_overrides: { 'deepseek-flash': 'deepseek/deepseek-v4.1-flash' },
      }] }
      if (method === 'protocol_adapter.list') return { adapters: [
        { protocol_family_id: 'openai', protocol_adapter_id: 'openai-chat-completions', custom_provider_selectable: true },
        { protocol_family_id: 'openai', protocol_adapter_id: 'openai-responses', custom_provider_selectable: true },
        { protocol_family_id: 'openai', protocol_adapter_id: 'private-adapter', custom_provider_selectable: false },
      ] }
      if (method === 'provider.validate') sent.push(params)
      return {}
    },
  })
  try {
    const manager = createAICCMgr({ useMock: false })
    const catalog = await manager.fetchProviderSetupCatalog()
    assert(catalog.providers[0].model_driver_overrides?.['deepseek-flash'] === 'deepseek/deepseek-v4.1-flash', 'preset aliases must be available to Connection')
    assert(catalog.protocol_families.length === 2, 'only selectable OpenAI adapters must be shown')
    for (const protocol_adapter_id of ['openai-chat-completions', 'openai-responses']) {
      assert(catalog.protocol_families.some((option) => option.protocol_adapter_id === protocol_adapter_id), 'OpenAI APIs must be separate choices')
      await manager.validateConnection({
        provider_profile_id: 'custom', protocol_family_id: 'openai', protocol_adapter_id,
        base_url: 'https://example.test/v1', operation_base_urls: {},
        display_name: 'custom', auth_mode: 'api_key', api_key: 'mock-key', auto_sync_models: true,
      })
      assert(sent.at(-1)?.protocol_adapter_id === protocol_adapter_id, 'validation must transmit the chosen adapter')
    }
  } finally {
    sdk.getAccountInfo = originalAccountInfo
    sdk.getServiceRpcClient = originalClient
  }
})

Deno.test('editing custom rules sends one complete CAS update without changing provider connection settings', async () => {
  const { createAICCMgr } = await import('../../src/api/aicc_mgr.ts')
  const { createCustomRulesDraft, providerRulesFile, rulesJson } = await import('../../src/api/custom_provider_rules.ts')
  const sdk = buckyos as unknown as {
    getAccountInfo: () => Promise<unknown>
    getServiceRpcClient: (service: string) => unknown
  }
  const originalAccountInfo = sdk.getAccountInfo
  const originalClient = sdk.getServiceRpcClient
  const updates: Record<string, unknown>[] = []
  let revision = 11
  sdk.getAccountInfo = async () => ({ session_token: 'test-session' })
  sdk.getServiceRpcClient = () => ({
    call: async (method: string, params: Record<string, unknown>) => {
      if (method === 'provider.update') { updates.push(params); return { ok: true, settings_revision: 12 } }
      if (method === 'provider.list') return { settings_revision: revision, providers: [{
        provider_instance_name: 'custom-test', provider_profile_id: 'custom',
        protocol_adapter_id: 'openai-responses', base_url: 'https://example.test/v1',
        enabled: true, auth: { mode: 'api_key', configured: true },
        inventory: { state: 'loaded' }, health: { state: 'healthy' },
        discovered_model_ids: ['hidden', 'local'],
        instance_rules: { exclude_models: ['hidden'], policy_region: 'unknown' },
      }] }
      return {}
    },
  })
  try {
    const manager = createAICCMgr({ useMock: false })
    await manager.refresh()
    const provider = manager.getSnapshot().providers[0]
    assert(provider.config.discovered_model_ids?.length === 2, 'raw discovered IDs must survive even without published models')
    const file = createCustomRulesDraft(provider.config)
    file.instance_rules.model_driver_overrides = { local: 'openai/gpt-6-astra' }
    revision = 22
    await manager.refresh()
    await manager.updateProviderRules(provider, { rules_config_json: rulesJson(providerRulesFile(file)), rules_settings_revision: 11 })
    assert(updates.length === 1, 'one edit session must issue one update')
    assert(updates[0].settings_revision === 11, 'save must use the captured settings revision')
    const rules = updates[0].instance_rules as Record<string, unknown>
    assert(rules.catalog !== undefined && rules.policy_region === 'unknown', 'full rules and unrelated policy must be retained')
    assert(!('base_url' in updates[0]) && !('credential' in updates[0]), 'rules updates must not overwrite connection settings')
  } finally {
    sdk.getAccountInfo = originalAccountInfo
    sdk.getServiceRpcClient = originalClient
  }
})

Deno.test('rule editor metadata uses finite tasks and adapter defaults from two batched queries', async () => {
  const { createAICCMgr } = await import('../../src/api/aicc_mgr.ts')
  const sdk = buckyos as unknown as { getAccountInfo: () => Promise<unknown>; getServiceRpcClient: (service: string) => unknown }
  const originalAccountInfo = sdk.getAccountInfo
  const originalClient = sdk.getServiceRpcClient
  const calls: string[] = []
  sdk.getAccountInfo = async () => ({ session_token: 'test-session' })
  sdk.getServiceRpcClient = () => ({
    call: async (method: string) => {
      calls.push(method)
      if (method === 'models.list') return {
        catalog: { revision: 1, vendors: [{ id: 'openai', revision: 1, specs: [], models: [{
          id: 'gpt', metadata: { api_types: ['llm'], llm: { supported_efforts: ['low', 'high'] } }, providers: [],
        }] }] },
        logical_definitions: [{ path: 'llm.chat', kind: 'task', api_type: 'llm' }, { path: 'llm.gpt', kind: 'family', api_type: 'llm' }],
      }
      if (method === 'protocol_adapter.list') return { adapters: [{
        protocol_adapter_id: 'openai-responses', default_operations: { llm: 'responses.create' },
        operations: [{ operation_id: 'responses.create', api_types: ['llm'] }],
      }] }
      throw new Error('unexpected per-model query')
    },
  })
  try {
    const options = await createAICCMgr({ useMock: false }).fetchProviderRuleOptions()
    assert(calls.length === 2, 'metadata must use two batched queries rather than per-model requests')
    assert(options.tasks.join(',') === 'llm.chat', 'families must not become task choices')
    assert(options.models[0].efforts.join(',') === 'low,high', 'supported_efforts must come from model metadata')
    assert(options.adapters[0].defaults.llm === 'responses.create', 'default operations must come from adapter metadata')
  } finally { sdk.getAccountInfo = originalAccountInfo; sdk.getServiceRpcClient = originalClient }
})
