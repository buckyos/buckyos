import type {
  ProviderView,
  UsageEvent,
  LogicalModelConfig,
  LocalModel,
  UsageCategory,
} from './types'

// ========== Provider Seeds ==========

const snRouterProvider: ProviderView = {
  config: {
    id: 'sn-router-1',
    name: 'SN Router',
    provider_type: 'sn_router',
    auto_sync_models: true,
    created_at: '2025-11-01T08:00:00Z',
  },
  status: {
    provider_id: 'sn-router-1',
    is_connected: true,
    auth_status: 'ok',
    usage_supported: true,
    balance_supported: true,
    discovered_models: [
      'llm-chat-standard',
      'llm-chat-advanced',
      'llm-code',
      'txt2img-standard',
    ],
    model_sync_status: 'ok',
    last_verified_at: '2025-12-15T10:30:00Z',
    last_model_sync_at: '2025-12-15T10:30:00Z',
  },
  account: {
    provider_id: 'sn-router-1',
    usage_supported: true,
    cost_supported: false,
    balance_supported: true,
    balance_unit: 'credit',
    balance_value: 500,
    usage_value: 234567,
  },
}

const openaiProvider: ProviderView = {
  config: {
    id: 'openai-1',
    name: 'OpenAI',
    provider_type: 'openai',
    auth_mode: 'api_key',
    endpoint: 'https://api.openai.com',
    auto_sync_models: true,
    created_at: '2025-11-05T14:00:00Z',
  },
  status: {
    provider_id: 'openai-1',
    is_connected: true,
    auth_status: 'ok',
    usage_supported: true,
    balance_supported: true,
    discovered_models: [
      'gpt-4o',
      'gpt-4o-mini',
      'gpt-4-turbo',
      'gpt-3.5-turbo',
      'dall-e-3',
      'dall-e-2',
      'whisper-1',
      'tts-1',
      'tts-1-hd',
      'text-embedding-3-large',
      'text-embedding-3-small',
      'text-embedding-ada-002',
    ],
    model_sync_status: 'ok',
    last_verified_at: '2025-12-15T09:00:00Z',
    last_model_sync_at: '2025-12-15T09:00:00Z',
  },
  account: {
    provider_id: 'openai-1',
    usage_supported: true,
    cost_supported: true,
    balance_supported: true,
    balance_unit: 'usd',
    balance_value: 23.5,
    estimated_cost: 8.72,
    usage_value: 456789,
  },
}

const anthropicProvider: ProviderView = {
  config: {
    id: 'anthropic-1',
    name: 'Anthropic',
    provider_type: 'anthropic',
    auth_mode: 'api_key',
    endpoint: 'https://api.anthropic.com',
    auto_sync_models: true,
    created_at: '2025-11-10T09:00:00Z',
  },
  status: {
    provider_id: 'anthropic-1',
    is_connected: false,
    auth_status: 'expired',
    usage_supported: true,
    balance_supported: false,
    discovered_models: [
      'claude-3-opus',
      'claude-3-sonnet',
      'claude-3-haiku',
    ],
    model_sync_status: 'failed',
    last_verified_at: '2025-12-10T08:00:00Z',
    last_model_sync_at: '2025-12-10T08:00:00Z',
  },
  account: {
    provider_id: 'anthropic-1',
    usage_supported: true,
    cost_supported: true,
    balance_supported: false,
    estimated_cost: 3.21,
    usage_value: 123456,
  },
}

// ========== Usage Event Seeds ==========

function generateUsageEvents(): UsageEvent[] {
  const events: UsageEvent[] = []
  const now = Date.now()
  const day = 24 * 60 * 60 * 1000

  const providers = ['sn-router-1', 'openai-1', 'anthropic-1']
  const models: Record<string, string[]> = {
    'sn-router-1': ['llm-chat-standard', 'llm-chat-advanced'],
    'openai-1': ['gpt-4o', 'gpt-4o-mini', 'dall-e-3'],
    'anthropic-1': ['claude-3-sonnet', 'claude-3-haiku'],
  }
  const categories: UsageCategory[] = ['text', 'text', 'text', 'image']
  const apps = ['studio', 'codeassistant', 'messagehub', undefined]
  const agents = ['agent-coder', 'agent-writer', 'agent-analyst', undefined]

  for (let i = 0; i < 150; i++) {
    const daysAgo = Math.floor(Math.random() * 30)
    const providerId = providers[i % 3]
    const providerModels = models[providerId]
    const modelName = providerModels[i % providerModels.length]
    const category = categories[i % 4]
    const tokensIn = Math.floor(Math.random() * 2000) + 100
    const tokensOut = Math.floor(Math.random() * 3000) + 50
    const costPerToken = category === 'image' ? 0.00004 : 0.000003

    events.push({
      id: `evt-${i.toString().padStart(3, '0')}`,
      timestamp: new Date(now - daysAgo * day - Math.random() * day).toISOString(),
      provider_id: providerId,
      model_name: modelName,
      category,
      app_id: apps[i % 4],
      agent_id: agents[i % 4],
      session_id: `session-${Math.floor(i / 5)}`,
      tokens_in: tokensIn,
      tokens_out: tokensOut,
      estimated_cost: Number(((tokensIn + tokensOut) * costPerToken).toFixed(4)),
      status: i % 20 === 0 ? 'failed' : 'success',
    })
  }

  return events.sort((a, b) => new Date(b.timestamp).getTime() - new Date(a.timestamp).getTime())
}

// ========== Logical Model Seeds ==========

const populatedLogicalModels: LogicalModelConfig[] = [
  {
    name: 'llm.chat',
    candidates: [
      { provider_id: 'openai-1', model_name: 'gpt-4o', priority: 1, enabled: true },
      { provider_id: 'anthropic-1', model_name: 'claude-3-sonnet', priority: 2, enabled: true },
      { provider_id: 'sn-router-1', model_name: 'llm-chat-standard', priority: 3, enabled: true },
    ],
    resolved_model: 'OpenAI / gpt-4o',
  },
  {
    name: 'llm.plan',
    candidates: [
      { provider_id: 'openai-1', model_name: 'gpt-4o', priority: 1, enabled: true },
      { provider_id: 'sn-router-1', model_name: 'llm-chat-advanced', priority: 2, enabled: true },
    ],
    resolved_model: 'OpenAI / gpt-4o',
  },
  {
    name: 'llm.code',
    candidates: [
      { provider_id: 'openai-1', model_name: 'gpt-4o', priority: 1, enabled: true },
      { provider_id: 'sn-router-1', model_name: 'llm-code', priority: 2, enabled: true },
    ],
    resolved_model: 'OpenAI / gpt-4o',
  },
  {
    name: 'txt2img',
    candidates: [],
    resolved_model: undefined,
  },
]

// ========== Scenario Exports ==========

export interface SeedData {
  providers: ProviderView[]
  usageEvents: UsageEvent[]
  logicalModels: LogicalModelConfig[]
  localModels: LocalModel[]
}

export function getEmptySeed(): SeedData {
  return {
    providers: [],
    usageEvents: [],
    logicalModels: [],
    localModels: [],
  }
}

export function getPopulatedSeed(): SeedData {
  return {
    providers: [snRouterProvider, openaiProvider, anthropicProvider],
    usageEvents: generateUsageEvents(),
    logicalModels: populatedLogicalModels,
    localModels: [],
  }
}
