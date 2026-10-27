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
