import claudeSeed from '../../../../aicc/driver_metadata/claude.json'
import falSeed from '../../../../aicc/driver_metadata/fal.json'
import geminiSeed from '../../../../aicc/driver_metadata/gemini.json'
import minimaxSeed from '../../../../aicc/driver_metadata/minimax.json'
import openaiSeed from '../../../../aicc/driver_metadata/openai.json'
import type {
  MetadataVariantRecord,
  MetadataVersionRuleRecord,
  ModelParamRuleRecord,
  ProviderRecord,
} from '../datamodel/types'

interface DriverRule extends Record<string, unknown> {
  id?: string
  pattern?: string
  model_driver?: string
  api_types?: string[]
  logical_mounts?: string[]
  capabilities?: Record<string, boolean | number | string>
  estimated_cost_usd?: number
  estimated_latency_ms?: number
  quality_score?: number
  latency_class?: string
  cost_class?: string
  exclude?: boolean
}

interface DriverMetadata {
  provider_driver: string
  revision: string
  models?: DriverRule[]
  patterns?: DriverRule[]
  defaults?: DriverRule
  variants?: Array<Record<string, unknown> & { name?: string }>
  version_rules?: Array<Record<string, unknown> & { family?: string; tier?: string; model_pattern?: string }>
}

const metadataSeeds = [
  openaiSeed,
  claudeSeed,
  geminiSeed,
  falSeed,
  minimaxSeed,
] as DriverMetadata[]

const timestamp = Date.UTC(2026, 6, 10, 8, 0, 0)

function providerName(driver: string) {
  const names: Record<string, string> = {
    openai: 'OpenAI',
    claude: 'Anthropic Claude',
    gemini: 'Google Gemini',
    fal: 'FAL',
    minimax: 'MiniMax',
  }
  return names[driver] ?? driver
}

function protocolFamily(driver: string) {
  if (driver === 'openai') {
    return 'openai-compatible'
  }
  if (driver === 'claude') {
    return 'anthropic'
  }
  if (driver === 'gemini') {
    return 'google-gemini'
  }
  return driver
}

function ruleFromDriver(
  driver: string,
  revision: string,
  rule: DriverRule,
  index: number,
  matchType: 'exact' | 'pattern' | 'default',
): ModelParamRuleRecord {
  const selector = matchType === 'exact' ? rule.id : matchType === 'pattern' ? rule.pattern : null
  return {
    rule_key: `${driver}-${matchType}-${index + 1}`,
    provider_key: null,
    source_rule_key: null,
    match_type: matchType,
    original_provider: driver,
    model_id_selector: selector ?? null,
    priority: matchType === 'pattern' ? index + 1 : null,
    model_driver: rule.model_driver ?? driver,
    api_types: rule.api_types ?? [],
    logical_mounts: rule.logical_mounts ?? [],
    capabilities: rule.capabilities ?? {},
    attributes: {
      source_rule: rule,
      quality_score: rule.quality_score ?? null,
      latency_class: rule.latency_class ?? null,
      cost_class: rule.cost_class ?? null,
      source_revision: revision,
    },
    context_limits: null,
    pricing: rule.estimated_cost_usd
      ? {
          estimated_cost_usd: rule.estimated_cost_usd,
          estimated_latency_ms: rule.estimated_latency_ms ?? null,
        }
      : null,
    exclude: Boolean(rule.exclude),
    enabled: true,
    created_at: timestamp,
    updated_at: timestamp,
  }
}

export function buildDriverMetadataTables() {
  const providers: ProviderRecord[] = []
  const modelParamRules: ModelParamRuleRecord[] = []
  const variants: MetadataVariantRecord[] = []
  const versionRules: MetadataVersionRuleRecord[] = []

  metadataSeeds.forEach((seed, seedIndex) => {
    providers.push({
      provider_key: seed.provider_driver,
      provider_driver: seed.provider_driver,
      name: providerName(seed.provider_driver),
      base_url: null,
      provider_kind: 'origin',
      protocol_family: protocolFamily(seed.provider_driver),
      enabled: true,
      owner_service: 'tech',
      revision: seed.revision,
      created_at: timestamp + seedIndex,
      updated_at: timestamp + seedIndex,
    })

    seed.models?.forEach((rule, index) => {
      modelParamRules.push(ruleFromDriver(seed.provider_driver, seed.revision, rule, index, 'exact'))
    })
    seed.patterns?.forEach((rule, index) => {
      modelParamRules.push(ruleFromDriver(seed.provider_driver, seed.revision, rule, index, 'pattern'))
    })
    if (seed.defaults) {
      modelParamRules.push(ruleFromDriver(seed.provider_driver, seed.revision, seed.defaults, 0, 'default'))
    }
    seed.variants?.forEach((variant, index) => {
      variants.push({
        variant_key: `${seed.provider_driver}-variant-${variant.name ?? index + 1}`,
        provider_key: null,
        source_variant_key: null,
        selector_type: 'pattern',
        original_provider: seed.provider_driver,
        model_id_selector: '*',
        priority: index + 1,
        nick: typeof variant.name === 'string' ? variant.name : null,
        content: variant,
        enabled: true,
        created_at: timestamp,
        updated_at: timestamp,
      })
    })
    seed.version_rules?.forEach((rule, index) => {
      versionRules.push({
        version_rule_key: `${seed.provider_driver}-version-${rule.family ?? 'rule'}-${rule.tier ?? index + 1}`,
        provider_key: null,
        source_version_rule_key: null,
        selector_type: 'pattern',
        original_provider: seed.provider_driver,
        model_id_selector: rule.model_pattern ?? '*',
        priority: index + 1,
        nick: `${rule.family ?? 'model'}.${rule.tier ?? 'standard'}`,
        content: rule,
        enabled: true,
        created_at: timestamp,
        updated_at: timestamp,
      })
    })
  })

  return {
    providers,
    modelParamRules,
    variants,
    versionRules,
  }
}
