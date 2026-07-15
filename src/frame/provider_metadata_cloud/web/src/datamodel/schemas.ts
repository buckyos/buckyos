import { z } from 'zod'

export const serviceRoleSchema = z.enum(['tech', 'ops'])
export const supportedProtocolFamilies = ['openai-compatible', 'anthropic', 'google-gemini', 'fal', 'minimax'] as const
const providerNameSchema = z.string().trim().min(2).max(80).regex(/^[A-Za-z0-9 _-]+$/, 'Use English letters, numbers, spaces, underscores, and hyphens')
const providerDriverSchema = z.string().trim().min(2).max(80).regex(/^[a-z0-9_-]+$/, 'Driver is derived from the lowercase provider name')

export const tableFilterSchema = z.object({
  search: z.string().trim().max(128),
  providerKey: z.string().trim().max(96),
  apiType: z.string().trim().max(96),
  capability: z.string().trim().max(96),
})

export type TableFilterInput = z.infer<typeof tableFilterSchema>

export const editSessionActionSchema = z.object({
  role: serviceRoleSchema,
  reason: z.string().trim().min(4).max(180),
})

export type EditSessionActionInput = z.infer<typeof editSessionActionSchema>

export const providerInputSchema = z.object({
  provider_key: z.string().trim().min(2).max(64).regex(/^[a-z0-9][a-z0-9-]*$/),
  name: providerNameSchema,
  provider_driver: providerDriverSchema,
  base_url: z.string().trim().url().max(180),
  provider_kind: z.enum(['origin', 'aggregator']),
  protocol_family: z.enum(supportedProtocolFamilies),
  template_provider_key: z.string().trim().max(96),
})

export type ProviderInput = z.infer<typeof providerInputSchema>

export const modelRuleInputSchema = z.object({
  rule_key: z.string().trim().min(2).max(96).regex(/^[a-z0-9][a-z0-9-]*$/),
  match_type: z.enum(['exact', 'pattern', 'default']),
  provider_key: z.string().trim().max(96),
  original_provider: z.string().trim().max(96),
  model_id_selector: z.string().trim().max(128),
  priority: z.number().int().min(1).max(999),
  model_driver: z.string().trim().min(1).max(96),
  api_types: z.array(z.string().trim().min(1).max(96)).min(1),
  capabilities: z.array(z.string().trim().min(1).max(96)).min(1),
  capability_values: z.record(z.string(), z.union([z.boolean(), z.number()])).optional(),
  max_context_tokens: z.number().int().min(1).max(10000000).optional(),
  estimated_cost_usd: z.number().min(0).max(1000).optional(),
  estimated_latency_ms: z.number().int().min(0).max(3600000).optional(),
  quality_score: z.number().min(0).max(1).optional(),
  latency_class: z.enum(['fast', 'normal', 'slow']).optional(),
  cost_class: z.enum(['low', 'medium', 'high']).optional(),
  logical_mounts: z.array(z.string().trim().min(1).max(128)).min(1),
  scope: z.enum(['global', 'provider']),
  exclude: z.boolean(),
}).refine((value) => value.match_type === 'default' || value.model_id_selector.length > 0, {
  path: ['model_id_selector'],
  message: 'Selector is required for exact and pattern rules',
})

export type ModelRuleInput = z.infer<typeof modelRuleInputSchema>

export const deleteModelRuleInputSchema = z.object({
  rule_key: z.string().trim().min(1).max(96),
})

export type DeleteModelRuleInput = z.infer<typeof deleteModelRuleInputSchema>

export const selectionRuleInputSchema = z.object({
  rule_key: z.string().trim().min(2).max(96).regex(/^[a-z0-9][a-z0-9-]*$/),
  provider_key: z.string().trim().min(1).max(96),
  rule_type: z.enum(['include_origin', 'exclude_origin', 'include_pattern', 'exclude_pattern']),
  selector: z.string().trim().min(1).max(128),
  priority: z.number().int().min(1).max(999),
})

export type SelectionRuleInput = z.infer<typeof selectionRuleInputSchema>

export const nickRuleInputSchema = z.object({
  nick_key: z.string().trim().min(2).max(96).regex(/^[a-z0-9][a-z0-9-]*$/),
  provider_key: z.string().trim().min(1).max(96),
  original_provider: z.string().trim().max(96),
  model_id: z.string().trim().min(1).max(128),
  nick: z.string().trim().min(1).max(128),
  selector_type: z.enum(['exact', 'pattern']),
  priority: z.number().int().min(1).max(999),
})

export type NickRuleInput = z.infer<typeof nickRuleInputSchema>

const resolverRuleBaseSchema = z.object({
  rule_key: z.string().trim().min(2).max(96).regex(/^[a-z0-9][a-z0-9-]*$/),
  scope: z.enum(['global', 'provider']),
  provider_key: z.string().trim().max(96),
  original_provider: z.string().trim().max(96),
  api_type: z.string().trim().min(1).max(96),
  capability: z.string().trim().min(1).max(96),
  model_driver: z.string().trim().min(1).max(96),
})

export const exactResolverRuleInputSchema = resolverRuleBaseSchema.extend({
  match_type: z.literal('exact'),
  model_id_selector: z.string().trim().min(1).max(128),
})

export const patternResolverRuleInputSchema = resolverRuleBaseSchema.extend({
  match_type: z.literal('pattern'),
  model_id_selector: z.string().trim().min(1).max(128),
  priority: z.number().int().min(1).max(999),
})

export const defaultResolverRuleInputSchema = resolverRuleBaseSchema.extend({
  match_type: z.literal('default'),
})

export const resolverRuleInputSchema = z.discriminatedUnion('match_type', [
  exactResolverRuleInputSchema,
  patternResolverRuleInputSchema,
  defaultResolverRuleInputSchema,
])

export type ExactResolverRuleInput = z.infer<typeof exactResolverRuleInputSchema>
export type PatternResolverRuleInput = z.infer<typeof patternResolverRuleInputSchema>
export type DefaultResolverRuleInput = z.infer<typeof defaultResolverRuleInputSchema>
export type ResolverRuleInput = z.infer<typeof resolverRuleInputSchema>

export const metadataVariantInputSchema = z.object({
  variant_key: z.string().trim().min(2).max(96).regex(/^[a-z0-9][a-z0-9-]*$/),
  provider_key: z.string().trim().max(96),
  selector_type: z.enum(['exact', 'pattern']),
  original_provider: z.string().trim().max(96),
  model_id_selector: z.string().trim().min(1).max(128),
  priority: z.number().int().min(1).max(999),
  nick: z.string().trim().max(128),
  mount_suffix: z.string().trim().max(64),
  logical_mounts: z.array(z.string().trim().min(1).max(128)),
  capabilities: z.array(z.string().trim().min(1).max(96)),
  capability_values: z.record(z.string(), z.union([z.boolean(), z.number()])).optional(),
  provider_options_json: z.string().trim().max(4000),
  content_json: z.string().trim().max(4000),
})

export type MetadataVariantInput = z.infer<typeof metadataVariantInputSchema>

export const metadataVersionRuleInputSchema = z.object({
  version_rule_key: z.string().trim().min(2).max(96).regex(/^[a-z0-9][a-z0-9-]*$/),
  provider_key: z.string().trim().max(96),
  selector_type: z.enum(['exact', 'pattern']),
  original_provider: z.string().trim().max(96),
  model_id_selector: z.string().trim().min(1).max(128),
  priority: z.number().int().min(1).max(999),
  nick: z.string().trim().max(128),
  family: z.string().trim().min(1).max(64),
  tier: z.string().trim().min(1).max(64),
  model_pattern: z.string().trim().max(180),
  tier_tokens: z.array(z.string().trim().min(1).max(96)),
  exclude_tier_tokens: z.array(z.string().trim().min(1).max(96)),
  version_rank_prefix: z.string().trim().max(96),
  stability_unstable_tokens: z.array(z.string().trim().min(1).max(96)),
  stability_current_requires_stable: z.boolean(),
  current_mount: z.string().trim().max(128),
  version_mount: z.string().trim().max(128),
  exclude_snapshot_date_suffix: z.boolean(),
  auto_mounts: z.array(z.string().trim().min(1).max(128)),
  capabilities: z.array(z.string().trim().min(1).max(96)),
  capability_values: z.record(z.string(), z.union([z.boolean(), z.number()])).optional(),
  content_json: z.string().trim().max(4000),
})

export type MetadataVersionRuleInput = z.infer<typeof metadataVersionRuleInputSchema>

export const logicalDirectoryInputSchema = z.object({
  directory_key: z.string().trim().min(2).max(96).regex(/^[a-z0-9][a-z0-9-]*$/),
  path: z.string().trim().min(1).max(160).regex(/^\/[a-z0-9/_-]*$/),
  title: z.string().trim().min(1).max(80),
  parent_key: z.string().trim().max(96),
})

export type LogicalDirectoryInput = z.infer<typeof logicalDirectoryInputSchema>

export const logicalDirectoryMountInputSchema = z.object({
  directory_key: z.string().trim().min(1).max(96),
  model_rule_keys: z.array(z.string().trim().min(1).max(96)).min(1),
})

export type LogicalDirectoryMountInput = z.infer<typeof logicalDirectoryMountInputSchema>

export const dictionaryItemInputSchema = z.object({
  key: z.string().trim().min(2).max(96).regex(/^[a-z0-9][a-z0-9._-]*$/),
  label: z.string().trim().min(2).max(80),
  kind: z.enum(['api_type', 'capability']),
  value_type: z.enum(['boolean', 'number']),
})

export type DictionaryItemInput = z.infer<typeof dictionaryItemInputSchema>

export const dictionaryBulkApplyInputSchema = z.object({
  kind: z.enum(['api_type', 'capability']),
  key: z.string().trim().min(1).max(96),
  value_type: z.enum(['boolean', 'number']),
  boolean_value: z.boolean(),
  number_value: z.number().min(0).max(10000000).optional(),
  model_rule_keys: z.array(z.string().trim().min(1).max(96)).min(1),
}).refine((value) => value.kind !== 'capability' || value.value_type !== 'number' || value.number_value !== undefined, {
  path: ['number_value'],
  message: 'Number value is required',
})

export type DictionaryBulkApplyInput = z.infer<typeof dictionaryBulkApplyInputSchema>

export const providerWizardModelRuleDraftSchema = z.object({
  draft_key: z.string().trim().min(2).max(96),
  match_type: z.enum(['exact', 'pattern', 'default']),
  original_provider: z.string().trim().max(96),
  model_id_selector: z.string().trim().max(128),
  priority: z.number().int().min(1).max(999),
  source_rule_key: z.string().trim().max(96),
  model_driver: z.string().trim().min(1).max(96),
  api_types: z.array(z.string().trim().min(1).max(96)),
  capabilities: z.array(z.string().trim().min(1).max(96)),
  capability_values: z.record(z.string(), z.union([z.boolean(), z.number()])).optional(),
  max_context_tokens: z.number().int().min(1).max(10000000).optional(),
  estimated_cost_usd: z.number().min(0).max(1000).optional(),
  estimated_latency_ms: z.number().int().min(0).max(3600000).optional(),
  quality_score: z.number().min(0).max(1).optional(),
  latency_class: z.enum(['fast', 'normal', 'slow']).optional(),
  cost_class: z.enum(['low', 'medium', 'high']).optional(),
  logical_mounts: z.array(z.string().trim().min(1).max(128)),
  exclude: z.boolean(),
}).refine((value) => value.match_type === 'default' || value.model_id_selector.length > 0, {
  path: ['model_id_selector'],
  message: 'Selector is required for exact and pattern rules',
}).refine((value) => value.exclude || value.api_types.length > 0, {
  path: ['api_types'],
  message: 'At least one API type is required for enabled model rules',
})

export type ProviderWizardModelRuleDraft = z.infer<typeof providerWizardModelRuleDraftSchema>

export const providerWizardResolverRuleDraftSchema = z.object({
  draft_key: z.string().trim().min(2).max(96),
  rule_kind: z.enum(['variant', 'version_rule']),
  selector_type: z.enum(['exact', 'pattern']),
  original_provider: z.string().trim().min(1).max(96),
  model_id_selector: z.string().trim().min(1).max(128),
  priority: z.number().int().min(1).max(999),
  nick: z.string().trim().max(128),
  mount_suffix: z.string().trim().max(128).optional(),
  provider_options_json: z.string().trim().max(4000).optional(),
  family: z.string().trim().max(128).optional(),
  tier: z.string().trim().max(128).optional(),
  model_pattern: z.string().trim().max(180).optional(),
  tier_tokens: z.array(z.string().trim().min(1).max(96)).optional(),
  exclude_tier_tokens: z.array(z.string().trim().min(1).max(96)).optional(),
  version_rank_prefix: z.string().trim().max(96).optional(),
  stability_unstable_tokens: z.array(z.string().trim().min(1).max(96)).optional(),
  stability_current_requires_stable: z.boolean().optional(),
  current_mount: z.string().trim().max(180).optional(),
  version_mount: z.string().trim().max(180).optional(),
  exclude_snapshot_date_suffix: z.boolean().optional(),
  capabilities: z.array(z.string().trim().min(1).max(96)).optional(),
  capability_values: z.record(z.string(), z.union([z.boolean(), z.number()])).optional(),
  source_rule_key: z.string().trim().max(128),
  logical_mounts: z.array(z.string().trim().min(1).max(128)),
})

export type ProviderWizardResolverRuleDraft = z.infer<typeof providerWizardResolverRuleDraftSchema>

export const providerWizardInputSchema = z.object({
  provider_key: z.string().trim().min(2).max(64).regex(/^[a-z0-9][a-z0-9-]*$/),
  name: providerNameSchema,
  provider_driver: providerDriverSchema,
  base_url: z.union([z.string().trim().url().max(180), z.literal('')]),
  provider_kind: z.enum(['origin', 'aggregator']),
  protocol_family: z.enum(supportedProtocolFamilies),
  template_provider_key: z.string().trim().max(96),
  selected_origins: z.array(z.string().trim().min(1).max(96)).min(1),
  selected_model_ids: z.array(z.string().trim().min(1).max(128)),
  selected_resolver_rule_keys: z.array(z.string().trim().min(1).max(128)),
  nick_rules: z.array(z.object({
    draft_key: z.string().trim().min(2).max(96),
    original_provider: z.string().trim().min(1).max(96),
    selector_type: z.enum(['exact', 'pattern']),
    model_id: z.string().trim().min(1).max(128),
    nick: z.string().trim().min(1).max(128),
    priority: z.number().int().min(1).max(999),
  })),
  selected_api_types: z.array(z.string().trim().min(1).max(96)),
  selected_capabilities: z.array(z.string().trim().min(1).max(96)),
  selected_logical_mounts: z.array(z.string().trim().min(1).max(128)),
  model_rule_drafts: z.array(providerWizardModelRuleDraftSchema).min(1),
  resolver_rule_drafts: z.array(providerWizardResolverRuleDraftSchema),
})

export type ProviderWizardInput = z.infer<typeof providerWizardInputSchema>

export const importPlanInputSchema = z.object({
  title: z.string().trim().min(2).max(80),
  text: z.string().trim().min(12).max(12000),
})

export type ImportPlanInput = z.infer<typeof importPlanInputSchema>

export const importPlanDraftInputSchema = importPlanInputSchema.extend({
  plan_id: z.string().trim().min(1).max(96),
})

export type ImportPlanDraftInput = z.infer<typeof importPlanDraftInputSchema>

export const publishWizardInputSchema = z.object({
  release_note: z.string().trim().min(8).max(280),
  confirm_key_risk: z.boolean(),
  confirm_stale_publish: z.boolean(),
  confirm_final_publish: z.boolean(),
}).refine((value) => value.confirm_key_risk, {
  path: ['confirm_key_risk'],
  message: 'Confirm key field risks before publish',
}).refine((value) => value.confirm_final_publish, {
  path: ['confirm_final_publish'],
  message: 'Confirm final publish before continuing',
})

export type PublishWizardInput = z.infer<typeof publishWizardInputSchema>

const optionalNumberInput = z.preprocess((value) => {
  if (value === '' || value === null || value === undefined) {
    return undefined
  }
  return value
}, z.coerce.number().optional())

export const techSourceInputSchema = z.object({
  service_url: z.string().trim().url().max(180),
})

export type TechSourceInput = z.infer<typeof techSourceInputSchema>

export const recommendationLevelSchema = z.enum(['featured', 'preferred', 'standard', 'limited'])

export const providerOpsInputSchema = z.object({
  provider_key: z.string().trim().min(1).max(96),
  disabled: z.boolean(),
  recommendation_level: recommendationLevelSchema,
  display_priority: z.number().int().min(0).max(999),
  routing_policy_tag: z.string().trim().max(64),
  ops_note: z.string().trim().max(240),
})

export type ProviderOpsInput = z.infer<typeof providerOpsInputSchema>

export const modelOpsInputSchema = z.object({
  rule_key: z.string().trim().min(1).max(96),
  pricing_input: z.number().min(0).max(1000),
  pricing_output: z.number().min(0).max(1000),
  routing_weight: z.number().int().min(0).max(100),
  cost_class: z.enum(['low', 'medium', 'high']),
  latency_class: z.enum(['fast', 'normal', 'slow']),
  quality_score: z.number().int().min(0).max(100),
  recommendation_level: recommendationLevelSchema,
  display_priority: z.number().int().min(0).max(999),
  rollout_strategy: z.enum(['stable', 'canary', 'hold']),
  ops_note: z.string().trim().max(240),
})

export type ModelOpsInput = z.infer<typeof modelOpsInputSchema>

export const resolverOpsOverlayInputSchema = z.object({
  target_type: z.enum(['model_param_rule', 'variants', 'version_rules']),
  target_key: z.string().trim().min(1).max(96),
  disabled: z.boolean(),
  routing_policy_tag: z.string().trim().max(64),
  ops_note: z.string().trim().max(240),
})

export type ResolverOpsOverlayInput = z.infer<typeof resolverOpsOverlayInputSchema>

export const opsBulkOperationInputSchema = z.object({
  provider_key: z.string().trim().max(96),
  original_provider: z.string().trim().max(96),
  model_id_pattern: z.string().trim().max(128),
  api_type: z.string().trim().max(96),
  capability: z.string().trim().max(96),
  recommendation_level: z.union([recommendationLevelSchema, z.literal('')]),
  price_min: optionalNumberInput,
  price_max: optionalNumberInput,
  routing_weight_min: optionalNumberInput,
  routing_weight_max: optionalNumberInput,
  action: z.enum([
    'set_recommendation',
    'set_display_priority',
    'adjust_price_percent',
    'set_price',
    'set_routing_weight',
    'clear_pricing',
  ]),
  target_recommendation_level: recommendationLevelSchema,
  display_priority: z.coerce.number().int().min(0).max(999),
  price_percent: z.coerce.number().min(-95).max(500),
  pricing_input: z.coerce.number().min(0).max(1000),
  pricing_output: z.coerce.number().min(0).max(1000),
  routing_weight: z.coerce.number().int().min(0).max(100),
}).refine((value) => value.price_min === undefined || value.price_max === undefined || value.price_min <= value.price_max, {
  path: ['price_max'],
  message: 'Price range is invalid',
}).refine((value) => {
  return value.routing_weight_min === undefined || value.routing_weight_max === undefined || value.routing_weight_min <= value.routing_weight_max
}, {
  path: ['routing_weight_max'],
  message: 'Routing weight range is invalid',
})

export type OpsBulkOperationInput = z.infer<typeof opsBulkOperationInputSchema>
export type OpsBulkOperationFormInput = z.input<typeof opsBulkOperationInputSchema>
