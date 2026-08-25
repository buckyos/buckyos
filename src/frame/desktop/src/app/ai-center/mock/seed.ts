import type {
  ApiType,
  LocalModel,
  LogicalNode,
  ModelMetadata,
  ProviderType,
  ProviderView,
  RouteTrace,
  SchedulerProfile,
  GlobalRoutingView,
  UsageEvent,
} from './types'

function model(
  providerModelId: string,
  providerInstanceName: string,
  mounts: string[],
  apiTypes: ApiType[],
  overrides: Partial<ModelMetadata> = {},
): ModelMetadata {
  const exactModel = `${providerModelId}@${providerInstanceName}`

  return {
    provider_model_id: providerModelId,
    exact_model: exactModel,
    model_driver: providerInstanceName,
    logical_mounts: mounts,
    api_types: apiTypes,
    capabilities: {
      streaming: apiTypes.some((t) => t.startsWith('llm.')),
      tool_call: apiTypes.some((t) => t.startsWith('llm.')),
      json_schema: apiTypes.some((t) => t.startsWith('llm.')),
      web_search: false,
      vision: apiTypes.some((t) => t.startsWith('vision.') || t === 'llm'),
      max_context_tokens: 128000,
      max_output_tokens: 8192,
    },
    attributes: {
      local: providerInstanceName === 'local',
      privacy: providerInstanceName === 'local' ? 'local' : 'public_cloud',
      quality_score: 82,
      latency_class: 'normal',
      cost_class: 'medium',
      tier: 'mid',
    },
    pricing: {
      input_token_usd: 0.000003,
      output_token_usd: 0.000012,
    },
    health: {
      status: 'available',
      p50_latency_ms: 760,
      p95_latency_ms: 2100,
      error_rate_5m: 0.006,
      quota_state: 'normal',
    },
    ...overrides,
  }
}

const snModels = [
  model('sn-router-balanced', 'sn-router', ['llm', 'llm.fallback'], ['llm'], {
    attributes: {
      local: false,
      privacy: 'private_safe',
      quality_score: 78,
      latency_class: 'fast',
      cost_class: 'low',
      tier: 'mid',
    },
    health: {
      status: 'available',
      p50_latency_ms: 420,
      p95_latency_ms: 980,
      error_rate_5m: 0.004,
      quota_state: 'normal',
    },
  }),
  model('sn-router-image', 'sn-router', ['image.txt2img'], ['image.txt2img'], {
    attributes: {
      local: false,
      privacy: 'private_safe',
      quality_score: 74,
      latency_class: 'normal',
      cost_class: 'low',
      tier: 'mid',
    },
  }),
]

const openaiModels = [
  model('gpt-5.1', 'openai-main', ['llm.gpt-standard', 'llm.openai.gpt-5-1'], ['llm'], {
    parameter_scale: 'frontier',
    capabilities: {
      streaming: true,
      tool_call: true,
      json_schema: true,
      web_search: true,
      vision: true,
      max_context_tokens: 256000,
      max_output_tokens: 32768,
    },
    attributes: {
      local: false,
      privacy: 'public_cloud',
      quality_score: 93,
      latency_class: 'normal',
      cost_class: 'high',
      tier: 'flagship',
    },
    pricing: {
      input_token_usd: 0.00001,
      output_token_usd: 0.00004,
      cache_input_token_usd: 0.0000025,
    },
  }),
  model('gpt-5.1-mini', 'openai-main', ['llm.gpt-mini', 'llm.openai.gpt-5-1-mini'], ['llm'], {
    attributes: {
      local: false,
      privacy: 'public_cloud',
      quality_score: 82,
      latency_class: 'fast',
      cost_class: 'low',
      tier: 'nano',
    },
    pricing: {
      input_token_usd: 0.0000015,
      output_token_usd: 0.000006,
      cache_input_token_usd: 0.0000004,
    },
  }),
  model('text-embedding-3-large', 'openai-main', ['embedding.large'], ['embedding.text'], {
    capabilities: {
      streaming: false,
      tool_call: false,
      json_schema: false,
      web_search: false,
      vision: false,
      max_context_tokens: 8192,
    },
    attributes: {
      local: false,
      privacy: 'public_cloud',
      quality_score: 88,
      latency_class: 'fast',
      cost_class: 'low',
      tier: 'mid',
    },
  }),
]

const anthropicModels = [
  model('claude-opus-4.7', 'anthropic-work', ['llm.opus'], ['llm'], {
    parameter_scale: 'frontier',
    attributes: {
      local: false,
      privacy: 'public_cloud',
      quality_score: 95,
      latency_class: 'slow',
      cost_class: 'high',
      tier: 'flagship',
    },
    health: {
      status: 'degraded',
      p50_latency_ms: 1340,
      p95_latency_ms: 4800,
      error_rate_5m: 0.036,
      quota_state: 'near_limit',
    },
  }),
  model('claude-sonnet-4.5', 'anthropic-work', ['llm.sonnet', 'llm.anthropic.claude-sonnet-4-5'], ['llm'], {
    attributes: {
      local: false,
      privacy: 'public_cloud',
      quality_score: 90,
      latency_class: 'normal',
      cost_class: 'medium',
      tier: 'mid',
    },
    health: {
      status: 'available',
      p50_latency_ms: 920,
      p95_latency_ms: 2400,
      error_rate_5m: 0.01,
      quota_state: 'normal',
    },
  }),
]

const localModels: LocalModel[] = [
  {
    id: 'local-qwen-coder',
    name: 'Qwen Coder 32B',
    size_bytes: 21.6 * 1024 * 1024 * 1024,
    status: 'ready',
    last_used_at: '2026-05-29T18:20:00Z',
    ...model('qwen2.5-coder-32b', 'local', ['llm.qwen-coder', 'llm.local.qwen2-5-coder-32b'], ['llm'], {
      parameter_scale: '32B',
      attributes: {
        local: true,
        privacy: 'local',
        quality_score: 78,
        latency_class: 'fast',
        cost_class: 'low',
        tier: 'mid',
      },
      pricing: {},
      health: {
        status: 'available',
        p50_latency_ms: 180,
        p95_latency_ms: 520,
        error_rate_5m: 0.002,
        quota_state: 'normal',
      },
    }),
  },
  {
    id: 'local-embed',
    name: 'BGE M3 Embedding',
    size_bytes: 2.2 * 1024 * 1024 * 1024,
    status: 'loading',
    ...model('bge-m3', 'local', ['embedding.local'], ['embedding.text'], {
      parameter_scale: '567M',
      capabilities: {
        streaming: false,
        tool_call: false,
        json_schema: false,
        web_search: false,
        vision: false,
        max_context_tokens: 8192,
      },
      attributes: {
        local: true,
        privacy: 'local',
        quality_score: 76,
        latency_class: 'fast',
        cost_class: 'low',
        tier: 'nano',
      },
      pricing: {},
      health: {
        status: 'degraded',
        p50_latency_ms: 210,
        p95_latency_ms: 900,
        error_rate_5m: 0.018,
        quota_state: 'unknown',
      },
    }),
  },
]

function provider(
  id: string,
  name: string,
  providerType: ProviderType,
  instanceName: string,
  driver: string,
  models: ModelMetadata[],
  options: {
    connected?: boolean
    authStatus?: 'ok' | 'expired' | 'invalid' | 'unknown'
    balanceUnit?: 'usd' | 'credit'
    balanceValue?: number
    estimatedCost?: number
    pricingMode?: 'per_token' | 'subscription' | 'free_quota' | 'unknown'
    endpoint?: string
    syncStatus?: 'ok' | 'syncing' | 'failed'
  } = {},
): ProviderView {
  return {
    config: {
      id,
      name,
      provider_type: providerType,
      provider_instance_name: instanceName,
      provider_runtime_type: 'cloud_api',
      provider_driver: driver,
      provider_origin: providerType === 'sn_router' ? 'system_config' : 'user_config',
      auth_mode: providerType === 'sn_router' ? undefined : 'api_key',
      endpoint: options.endpoint,
      auto_sync_models: true,
      created_at: '2026-05-20T08:00:00Z',
    },
    inventory: {
      provider_instance_name: instanceName,
      provider_type: 'cloud_api',
      provider_driver: driver,
      provider_origin: providerType === 'sn_router' ? 'system_config' : 'user_config',
      inventory_revision: `${instanceName}-rev-20260529`,
      version: '2026.05',
      models,
    },
    status: {
      provider_id: id,
      is_connected: options.connected ?? true,
      auth_status: options.authStatus ?? 'ok',
      usage_supported: true,
      balance_supported: options.balanceValue !== undefined,
      discovered_models: models,
      model_sync_status: options.syncStatus ?? 'ok',
      last_verified_at: '2026-05-30T09:16:00Z',
      last_model_sync_at: '2026-05-30T09:12:00Z',
    },
    account: {
      provider_instance_name: instanceName,
      usage_supported: true,
      cost_supported: options.estimatedCost !== undefined,
      balance_supported: options.balanceValue !== undefined,
      pricing_mode: options.pricingMode ?? 'per_token',
      balance_unit: options.balanceUnit,
      balance_value: options.balanceValue,
      estimated_cost: options.estimatedCost,
      usage_value: 448920,
    },
  }
}

const providers = [
  provider('sn-router-1', 'SN Router', 'sn_router', 'sn-ai-provider-default', 'sn-ai-provider', snModels),
  provider('openai-1', 'OpenAI Main Key', 'openai', 'openai-main', 'openai', openaiModels, {
    balanceUnit: 'usd',
    balanceValue: 38.25,
    estimatedCost: 12.84,
    endpoint: 'https://api.openai.com',
  }),
  provider('anthropic-1', 'Anthropic Work', 'anthropic', 'anthropic-work', 'anthropic', anthropicModels, {
    connected: true,
    authStatus: 'ok',
    estimatedCost: 18.4,
    endpoint: 'https://api.anthropic.com',
    syncStatus: 'failed',
  }),
]

function exactNode(path: string, label: string, apiType: ApiType, resolved: boolean): LogicalNode {
  return {
    path,
    label,
    level: 'L1',
    api_type: apiType,
    exact_model_weights: {},
    policy: { profile: 'balanced' },
    resolved_exact_model: resolved ? path : undefined,
  }
}

function mountNode(
  path: string,
  label: string,
  apiType: ApiType,
  resolvedExactModel: string | undefined,
  children: LogicalNode[] = [],
): LogicalNode {
  return {
    path,
    label,
    level: 'L2',
    api_type: apiType,
    exact_model_weights: {},
    fallback: { mode: 'strict' },
    policy: { profile: 'balanced', allow_fallback: false },
    resolved_exact_model: resolvedExactModel,
    children,
  }
}

function buildLogicalTree(withPhysicalModels: boolean): LogicalNode[] {
  const opusExact = 'claude-opus-4.7@anthropic-work'
  const sonnetExact = 'claude-sonnet-4.5@anthropic-work'
  const gptExact = 'gpt-5.1@openai-main'
  const miniExact = 'gpt-5.1-mini@openai-main'
  const localCodeExact = 'qwen2.5-coder-32b@local'
  const embeddingExact = 'text-embedding-3-large@openai-main'
  const localEmbeddingExact = 'bge-m3@local'
  const imageExact = 'sn-router-image@sn-router'

  return [
    {
      path: 'llm',
      label: 'LLM namespace',
      level: 'L3',
      api_type: 'llm',
      exact_model_weights: {},
      fallback: { mode: 'strict' },
      policy: { profile: 'balanced', allow_fallback: true, runtime_failover: true },
      resolved_exact_model: withPhysicalModels ? 'sn-router-balanced@sn-router' : undefined,
      children: [
        {
          path: 'llm.plan',
          label: 'Planning',
          level: 'L3',
          api_type: 'llm',
          items: {
            opus: { target: 'llm.opus', weight: 2.5 },
            gemini: { target: 'llm.gemini-pro', weight: 2.4 },
            gpt: { target: 'llm.gpt-standard', weight: 2.2 },
            local_code: { target: 'llm.qwen-coder', weight: 1.1 },
          },
          exact_model_weights: {},
          fallback: { mode: 'parent' },
          policy: { profile: 'quality_first', allow_fallback: true, runtime_failover: true },
          resolved_exact_model: withPhysicalModels ? opusExact : undefined,
          children: [
            mountNode('llm.opus', 'Provider flagship planning mount', 'llm', withPhysicalModels ? opusExact : undefined, withPhysicalModels ? [
              exactNode(opusExact, 'Anthropic Opus exact model', 'llm', true),
            ] : []),
            mountNode('llm.gemini-pro', 'Gemini planning mount', 'llm', undefined),
            mountNode('llm.gpt-standard', 'OpenAI GPT standard family mount', 'llm', withPhysicalModels ? gptExact : undefined, withPhysicalModels ? [
              exactNode(gptExact, 'OpenAI frontier exact model', 'llm', true),
            ] : []),
            mountNode('llm.qwen-coder', 'Qwen coder family mount', 'llm', withPhysicalModels ? localCodeExact : undefined, withPhysicalModels ? [
              exactNode(localCodeExact, 'Local Qwen coder exact model', 'llm', true),
            ] : []),
          ],
        },
        {
          path: 'llm.code',
          label: 'Code generation',
          level: 'L3',
          api_type: 'llm',
          items: {
            sonnet: { target: 'llm.sonnet', weight: 2.4 },
            gpt: { target: 'llm.gpt-standard', weight: 2.1 },
            local: { target: 'llm.qwen-coder', weight: 1.8 },
          },
          exact_model_weights: withPhysicalModels ? { [localCodeExact]: 1.2 } : {},
          fallback: { mode: 'target_logical', target: 'llm.fallback' },
          policy: { profile: 'local_first', allow_fallback: true, required_features: ['tool_call'] },
          resolved_exact_model: withPhysicalModels ? sonnetExact : undefined,
          children: [
            mountNode('llm.sonnet', 'Balanced coding mount', 'llm', withPhysicalModels ? sonnetExact : undefined, withPhysicalModels ? [
              exactNode(sonnetExact, 'Anthropic Sonnet exact model', 'llm', true),
            ] : []),
            mountNode('llm.gpt-standard', 'OpenAI GPT standard family mount', 'llm', withPhysicalModels ? gptExact : undefined, withPhysicalModels ? [
              exactNode(gptExact, 'OpenAI frontier exact model', 'llm', true),
            ] : []),
            mountNode('llm.qwen-coder', 'Qwen coder family mount', 'llm', withPhysicalModels ? localCodeExact : undefined, withPhysicalModels ? [
              exactNode(localCodeExact, 'Local Qwen coder exact model', 'llm', true),
            ] : []),
          ],
        },
        {
          path: 'llm.swift',
          label: 'Fast responses',
          level: 'L3',
          api_type: 'llm',
          items: {
            mini: { target: 'llm.gpt-mini', weight: 2 },
            sn: { target: 'llm.sn-balanced', weight: 1.7 },
          },
          exact_model_weights: {},
          fallback: { mode: 'parent' },
          policy: { profile: 'latency_first', allow_fallback: true },
          resolved_exact_model: withPhysicalModels ? miniExact : undefined,
          children: [
            mountNode('llm.gpt-mini', 'OpenAI GPT mini family mount', 'llm', withPhysicalModels ? miniExact : undefined, withPhysicalModels ? [
              exactNode(miniExact, 'OpenAI mini exact model', 'llm', true),
            ] : []),
            mountNode('llm.sn-balanced', 'SN balanced mount', 'llm', withPhysicalModels ? 'sn-router-balanced@sn-router' : undefined, withPhysicalModels ? [
              exactNode('sn-router-balanced@sn-router', 'SN balanced exact model', 'llm', true),
            ] : []),
          ],
        },
        {
          path: 'llm.reason',
          label: 'Deep reasoning',
          level: 'L3',
          api_type: 'llm',
          items: {
            reasoner: { target: 'llm.reasoner', weight: 2.8 },
          },
          exact_model_weights: {},
          fallback: { mode: 'disabled' },
          policy: { profile: 'quality_first', allow_fallback: false },
          resolved_exact_model: undefined,
          children: [
            mountNode('llm.reasoner', 'Reasoning mount without inventory match', 'llm', undefined),
          ],
        },
      ],
    },
    {
      path: 'embedding',
      label: 'Embedding namespace',
      level: 'L3',
      api_type: 'embedding.text',
      items: {
        cloud: { target: 'embedding.large', weight: 1.8 },
        local: { target: 'embedding.local', weight: 1.4 },
      },
      exact_model_weights: {},
      fallback: { mode: 'parent' },
      policy: { profile: 'cost_first', allow_fallback: true },
      resolved_exact_model: withPhysicalModels ? embeddingExact : undefined,
      children: [
        mountNode('embedding.large', 'Cloud embedding mount', 'embedding.text', withPhysicalModels ? embeddingExact : undefined, withPhysicalModels ? [
          exactNode(embeddingExact, 'OpenAI embedding exact model', 'embedding.text', true),
        ] : []),
        mountNode('embedding.local', 'Local embedding mount', 'embedding.text', withPhysicalModels ? localEmbeddingExact : undefined, withPhysicalModels ? [
          exactNode(localEmbeddingExact, 'Local embedding exact model', 'embedding.text', true),
        ] : []),
      ],
    },
    {
      path: 'image.txt2img',
      label: 'Text to image',
      level: 'L3',
      api_type: 'image.txt2img',
      items: {
        sn: { target: 'image.sn-txt2img', weight: 1 },
        local: { target: 'image.local-txt2img', weight: 0.8 },
      },
      exact_model_weights: {},
      fallback: { mode: 'disabled' },
      policy: { profile: 'balanced', allow_fallback: false },
      resolved_exact_model: withPhysicalModels ? imageExact : undefined,
      locked: true,
      children: [
        mountNode('image.sn-txt2img', 'SN image mount', 'image.txt2img', withPhysicalModels ? imageExact : undefined, withPhysicalModels ? [
          exactNode(imageExact, 'SN text-to-image exact model', 'image.txt2img', true),
        ] : []),
        mountNode('image.local-txt2img', 'Local image mount without installed model', 'image.txt2img', undefined),
      ],
    },
    {
      path: 'vision.ocr',
      label: 'OCR',
      level: 'L3',
      api_type: 'vision.ocr',
      items: {
        cloud: { target: 'vision.ocr-cloud', weight: 1 },
      },
      exact_model_weights: {},
      fallback: { mode: 'strict' },
      policy: { profile: 'balanced', allow_fallback: false, required_features: ['vision'] },
      resolved_exact_model: undefined,
      children: [
        mountNode('vision.ocr-cloud', 'OCR mount without discovered provider model', 'vision.ocr', undefined),
      ],
    },
    {
      path: 'audio.tts',
      label: 'Text to speech',
      level: 'L3',
      api_type: 'audio.tts',
      items: {
        voice: { target: 'audio.voice', weight: 1 },
      },
      exact_model_weights: {},
      fallback: { mode: 'strict' },
      policy: { profile: 'latency_first', allow_fallback: false },
      resolved_exact_model: undefined,
      children: [
        mountNode('audio.voice', 'TTS mount without discovered provider model', 'audio.tts', undefined),
      ],
    },
  ]
}

function buildGlobalRoutingView(withPhysicalModels: boolean): GlobalRoutingView {
  return {
    revision: withPhysicalModels ? 'mock-aicc-router-v2-with-inventory' : 'mock-aicc-router-v2-directory-only',
    global_exact_model_weights: withPhysicalModels
      ? {
          'qwen2.5-coder-32b@local': 1.2,
        }
      : {},
    provider_weights: {
      'openai-main': 0.3,
    },
    policy: {
      profile: 'balanced',
      allow_fallback: true,
      runtime_failover: true,
    },
    logical_tree: buildLogicalTree(withPhysicalModels),
  }
}

function generateUsageEvents(): UsageEvent[] {
  const events: UsageEvent[] = []
  const now = Date.now()
  const day = 24 * 60 * 60 * 1000
  const rows = [
    { provider: 'openai-main', exact: 'gpt-5.1@openai-main', requested: 'llm.code', api: 'llm' as ApiType, cost: 0.000028 },
    { provider: 'anthropic-work', exact: 'claude-opus-4.7@anthropic-work', requested: 'llm.plan', api: 'llm' as ApiType, cost: 0.000042 },
    { provider: 'openai-main', exact: 'text-embedding-3-large@openai-main', requested: 'embedding', api: 'embedding.text' as ApiType, cost: 0.000001 },
    { provider: 'sn-router', exact: 'sn-router-image@sn-router', requested: 'image.txt2img', api: 'image.txt2img' as ApiType, cost: 0.00005 },
    { provider: 'local', exact: 'qwen2.5-coder-32b@local', requested: 'llm.code', api: 'llm' as ApiType, cost: 0 },
  ]
  const apps = ['studio', 'codeassistant', 'messagehub', 'jarvis']
  const agents = ['agent-coder', 'agent-planner', 'agent-analyst', 'jarvis-default']

  for (let i = 0; i < 140; i++) {
    const daysAgo = Math.floor((i * 7) % 30)
    const row = rows[i % rows.length]
    const tokensIn = row.api === 'image.txt2img' ? undefined : 220 + ((i * 131) % 3100)
    const tokensOut = row.api === 'image.txt2img' ? undefined : 120 + ((i * 97) % 2400)
    const tokenEquivalent = row.api === 'image.txt2img' ? 1600 + ((i * 71) % 1200) : undefined
    const billableTokens = tokenEquivalent ?? (tokensIn ?? 0) + (tokensOut ?? 0)
    const estimatedCost = Number((billableTokens * row.cost).toFixed(4))

    events.push({
      id: `evt-${i.toString().padStart(3, '0')}`,
      timestamp: new Date(now - daysAgo * day - ((i * 37) % 24) * 60 * 60 * 1000).toISOString(),
      provider_instance_name: row.provider,
      exact_model: row.exact,
      requested_model: row.requested,
      api_type: row.api,
      tenant_id: 'devtest',
      app_id: apps[i % apps.length],
      agent_id: agents[i % agents.length],
      session_id: `session-${Math.floor(i / 4).toString().padStart(2, '0')}`,
      tokens_in: tokensIn,
      tokens_out: tokensOut,
      token_equivalent: tokenEquivalent,
      estimated_cost: estimatedCost,
      finance_snapshot: {
        amount: estimatedCost,
        currency: 'USD',
        provider_trace_id: `trace-${i.toString().padStart(3, '0')}`,
      },
      status: i % 23 === 0 ? 'failed' : 'success',
    })
  }

  return events.sort((a, b) => new Date(b.timestamp).getTime() - new Date(a.timestamp).getTime())
}

function scoreInputs(cost: number, latency: number, reliability: number, quality: number, preference: number, local: number) {
  return {
    cost,
    latency,
    reliability,
    quality,
    preference,
    cache: 0,
    local,
  }
}

const routeTraces: RouteTrace[] = [
  {
    request_id: 'trace-llm-plan-001',
    session_id: 'session-04',
    api_type: 'llm',
    requested_model: 'llm.plan',
    requested_model_type: 'logical',
    resolved_logical_path: 'llm.plan',
    selected_exact_model: 'claude-opus-4.7@anthropic-work',
    selected_provider_instance_name: 'anthropic-work',
    pricing_snapshot: {
      input_token_usd: 0.00001,
      output_token_usd: 0.00004,
      cache_input_token_usd: 0.0000025,
      estimated_cost_usd: 0.023,
    },
    ranked_candidates: [
      {
        exact_model: 'claude-opus-4.7@anthropic-work',
        final_score: 0.94,
        selected: true,
        pricing_snapshot: {
          input_token_usd: 0.00001,
          output_token_usd: 0.00004,
          cache_input_token_usd: 0.0000025,
          estimated_cost_usd: 0.023,
        },
        exact_model_weight: 1,
        provider_weight: 1,
        preference_score_inputs: {
          exact_model_weight: 1,
          provider_weight: 1,
          combined_weight: 1,
          preference_penalty: 1,
          exact_model_weight_effect: 'neutral',
          provider_weight_effect: 'neutral',
        },
        score_inputs: scoreInputs(0.6, 0.4, 0.08, 0.02, 0.5, 1),
      },
      {
        exact_model: 'gpt-5.1@openai-main',
        final_score: 0.9,
        selected: false,
        pricing_snapshot: {
          input_token_usd: 0.000003,
          output_token_usd: 0.000012,
          cache_input_token_usd: 0.00000075,
          estimated_cost_usd: 0.008,
        },
        exact_model_weight: 1,
        provider_weight: 0.3,
        preference_score_inputs: {
          exact_model_weight: 1,
          provider_weight: 0.3,
          combined_weight: 0.3,
          preference_penalty: 3.3333333333333335,
          exact_model_weight_effect: 'neutral',
          provider_weight_effect: 'downweighted',
        },
        score_inputs: scoreInputs(0.2, 0.35, 0.03, 0.04, 1, 1),
      },
      {
        exact_model: 'qwen2.5-coder-32b@local',
        final_score: 0.62,
        selected: false,
        pricing_snapshot: {
          estimated_cost_usd: 0.0,
        },
        exact_model_weight: 1.2,
        provider_weight: 1,
        preference_score_inputs: {
          exact_model_weight: 1.2,
          provider_weight: 1,
          combined_weight: 1.2,
          preference_penalty: 0.8333333333333334,
          exact_model_weight_effect: 'upweighted',
          provider_weight_effect: 'neutral',
        },
        score_inputs: scoreInputs(0, 0.1, 0.01, 0.18, 0.3, 0),
      },
    ],
    filtered_candidates: [
      { exact_model: 'sn-router-balanced@sn-router', reason: 'quality score below llm.plan policy threshold' },
    ],
    fallback_applied: false,
    fallback_chain: [],
    session_sticky_hit: false,
    scheduler_profile: 'quality_first' satisfies SchedulerProfile,
    runtime_failover_count: 0,
    user_summary: {
      display_name: 'Planning request',
      model_family: 'Claude Opus',
      provider_origin: 'cloud',
      reason_short: 'Selected the high-quality planning role target with no fallback.',
      was_fallback: false,
      was_failover: false,
    },
    warnings: ['Anthropic quota is near limit.'],
  },
  {
    request_id: 'trace-llm-code-018',
    session_id: 'session-18',
    api_type: 'llm',
    requested_model: 'llm.code',
    requested_model_type: 'logical',
    resolved_logical_path: 'llm.code',
    selected_exact_model: 'qwen2.5-coder-32b@local',
    selected_provider_instance_name: 'local',
    ranked_candidates: [
      { exact_model: 'qwen2.5-coder-32b@local', final_score: 0.88, selected: true, score_inputs: scoreInputs(0, 0.1, 0.01, 0.12, 0.4, 0) },
      { exact_model: 'claude-sonnet-4.5@anthropic-work', final_score: 0.86, selected: false, score_inputs: scoreInputs(0.7, 0.45, 0.05, 0.04, 0.6, 1) },
      { exact_model: 'gpt-5.1@openai-main', final_score: 0.81, selected: false, score_inputs: scoreInputs(0.35, 0.3, 0.03, 0.08, 0.6, 1) },
    ],
    filtered_candidates: [],
    fallback_applied: false,
    fallback_chain: [],
    session_sticky_hit: true,
    scheduler_profile: 'local_first' satisfies SchedulerProfile,
    runtime_failover_count: 0,
    user_summary: {
      display_name: 'Code generation',
      model_family: 'Qwen Coder',
      provider_origin: 'local',
      reason_short: 'Local-first policy chose the local coder model for privacy and latency.',
      was_fallback: false,
      was_failover: false,
    },
    warnings: [],
  },
]

const directoryOnlyRouteTraces: RouteTrace[] = [
  {
    request_id: 'trace-directory-only-llm-reason',
    api_type: 'llm',
    requested_model: 'llm.reason',
    requested_model_type: 'logical',
    resolved_logical_path: 'llm.reason',
    ranked_candidates: [],
    filtered_candidates: [
      { exact_model: 'llm.reasoner', reason: 'logical mount exists, but no provider inventory supplies an exact model' },
    ],
    fallback_applied: false,
    fallback_chain: [],
    session_sticky_hit: false,
    scheduler_profile: 'quality_first' satisfies SchedulerProfile,
    runtime_failover_count: 0,
    user_summary: {
      display_name: 'Reasoning request',
      model_family: 'Unresolved logical mount',
      provider_origin: 'proxy_unknown',
      reason_short: 'The logical role exists in the directory, but no physical model is currently mounted under llm.reasoner.',
      was_fallback: false,
      was_failover: false,
    },
    warnings: ['AICC_ROUTE_NO_CANDIDATE'],
  },
]

export interface SeedData {
  providers: ProviderView[]
  usageEvents: UsageEvent[]
  routingView: GlobalRoutingView
  routeTraces: RouteTrace[]
  localModels: LocalModel[]
}

export function getEmptySeed(): SeedData {
  return {
    providers: [],
    usageEvents: [],
    routingView: buildGlobalRoutingView(false),
    routeTraces: directoryOnlyRouteTraces,
    localModels: [],
  }
}

export function getPopulatedSeed(): SeedData {
  return {
    providers,
    usageEvents: generateUsageEvents(),
    routingView: buildGlobalRoutingView(true),
    routeTraces,
    localModels,
  }
}

export { model }
