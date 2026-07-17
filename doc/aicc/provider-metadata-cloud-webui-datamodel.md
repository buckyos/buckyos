# Provider Metadata Cloud WebUI DataModel

Status: mock-first UI DataModel for `src/frame/provider_metadata_cloud/web/`.

Related documents:

- `product/ai_center/Provider_Metadata_Cloud_WebUI_PRD.md`
- `doc/aicc/provider-driver-cloud-update-design.md`
- `proposals/provider-metadata-cloud-webui/plan.md`
- `src/frame/provider_metadata_cloud/web/src/datamodel/types.ts`
- `src/frame/provider_metadata_cloud/web/src/datamodel/schemas.ts`

## 1. Overview

Provider Metadata Cloud WebUI is an independent mock-first console for maintaining provider metadata cloud data. It is not registered into desktop and does not call a real backend. Pages read and mutate data through `useProviderMetadataStore()`. The store imports functions from `mock/api.ts`; that file owns the in-memory mock workspace and simulates latency, empty/error/stale scenarios, import parsing, pending changes, revision conflicts, and publish preview.

The current UI DataModel intentionally uses table-like records that match the mock persistence shape:

- `providers`
- `model_param_rules`
- `metadata_variants`
- `metadata_version_rules`
- provider source object references for models/patterns/defaults/variants/version rules
- `model_nicks` as Nick rewrite authoring records
- `origin_provider_aliases`
- `origin_mapping_rules`
- `logical_directories`
- `dictionaries`
- `ops_overlays`
- workflow records such as edit session, pending changes, change logs, warnings, import plan draft, and publish preview

This document describes the UI-facing data shape, not a backend DTO or KRPC protocol. Future backend integration should preserve the page -> store -> API adapter boundary and may map these records to real service objects behind the store.

`published_json` in `PublishPreview` is the client delivery view, not the WebUI storage shape. It must be materialized as AICC driver metadata documents compatible with `doc/aicc/driver_metadata_schema.md`. Provider `name`, `provider_driver`, `protocol_family`, `base_url`, `origin_provider_aliases`, and `origin_mappings` are client-facing metadata and are part of the materialized document. Authoring records such as `provider_key`, source-object references, model_nicks storage rows, logical directory records, dictionaries, and operations overlay fields are consumed by the materializer and must not appear as top-level storage fields in the client driver metadata JSON.

## 2. Supported Pages and Views

| Page | Route | View role | Main data |
| --- | --- | --- | --- |
| Dashboard | `/` | tech, ops | aggregate counts from `ProviderCloudSeed` |
| Tech Source | `/tech-source` | ops | `TechSourceRecord` |
| Providers | `/providers` | tech, ops | `ProviderRecord[]`, provider ops overlays |
| Provider Wizard | `/providers/wizard` | tech | `ProviderWizardInput`, provider/source model/source resolver/nick changes |
| Models | `/models` | tech, ops | `ModelParamRuleRecord[]` exact/pattern/default rules, model ops overlays |
| Nick Rules | `/nick-rules` | tech | `ModelNickRecord[]` for Nick rewrite, plus `OriginProviderAliasRecord[]` and `OriginMappingRuleRecord[]` for final origin identity materialization |
| Resolver Rules | `/resolver-rules` | tech, ops | variants, version rules, resolver ops overlays; original provider is always explicit in authoring forms |
| Logical Directory | `/logical-directory` | tech | `LogicalDirectoryRecord[]` |
| Dictionaries | `/dictionaries` | tech | `DictionaryItem[]` and bulk dictionary tagging |
| Import Plan | `/import-plan` | tech | `ImportPlanInput`, `ImportPlanParseResult`, draft workflow |
| Bulk Operations | `/bulk-operations` | ops | `OpsBulkOperationInput`, bulk preview rows |
| Warnings | `/warnings` | ops | stored warnings plus generated diagnostics |
| Publish Preview | `/publish` | tech, ops | `PublishPreview`, `PublishWizardInput` |
| Change Logs | `/change-logs` | tech, ops | `ChangeLogRecord[]` |

`ServiceRole = 'tech' | 'ops'` controls which navigation entries and edit forms are available. `ViewMode = 'browse' | 'edit'` controls whether write forms are shown. Mobile view is browse-oriented; desktop edit flows use dense tables.

## 3. Runtime Boundary

Current call boundary:

```text
pages / layout / workflow components
  -> useProviderMetadataStore()
  -> ProviderMetadataStoreProvider
  -> mock/api.ts exported functions
  -> in-memory ProviderCloudSeed
```

Rules:

- Pages and layout components must not import `mock/api.ts`.
- Pages and layout components must not call `fetch`, `axios`, `XMLHttpRequest`, or real backend APIs.
- `ProviderMetadataStoreProvider` is the only UI layer that imports `../mock/api`.
- `mock/api.ts` returns cloned `ProviderCloudSeed` snapshots or `PublishPreview` snapshots.
- A future backend adapter can replace `mock/api.ts` functions or be wrapped by the store without large page changes.

Current store actions exposed to pages:

```typescript
reload()
enterEdit()
runPublishPreview()
upsertProvider(input)
runProviderWizard(input)
upsertModelRule(input)
removeModelRule(input)
upsertNickRule(input)
upsertMetadataVariant(input)
upsertMetadataVersionRule(input)
upsertLogicalDirectory(input)
applyDirectoryMounts(input)
upsertDictionaryItem(input)
applyDictionaryTag(input)
importPlan(input)
saveImportDraft(input)
restoreImportDraft()
discardImportDraft()
simulateConflict()
refreshPublishBase()
completePublish(input)
configureTechSource(input)
testTechSource()
syncSource()
upsertProviderOps(input)
upsertModelOps(input)
upsertResolverOps(input)
applyBulkOperation(input)
markTechSourceStale()
```

## 4. State Definitions

The store uses a small generic loading state:

```typescript
export type LoadingStatus = 'idle' | 'loading' | 'success' | 'error'

export interface DataState<T> {
  status: LoadingStatus
  data: T | null
  error: string | null
}
```

State rules:

| State | Meaning | UI handling |
| --- | --- | --- |
| `idle` | Store created but not loaded | short initial state before `reload()` |
| `loading` | Mock workspace is loading | shell renders loading state |
| `success` | Workspace snapshot is available | pages render tables/forms/detail panels |
| `error` | Workspace load failed | shell renders error + retry |

There is no `empty` status in the type. Empty data is represented as `success` with empty collections when `?mockState=empty` is used. Stale is a business state, represented by `TechSourceRecord.stale`, `source_stale` in publish preview, and warning diagnostics.

Mock query states:

| Query | Result |
| --- | --- |
| `?mockState=empty` | major collections are empty, shell still returns `success` |
| `?mockState=stale` | `tech_source.stale = true` and previous cache revision remains available |
| `?mockState=error-once&mockErrorKey=<key>` | workspace load fails for the first two attempts for that key |

## 5. TypeScript Interfaces

The canonical implementation source is `src/frame/provider_metadata_cloud/web/src/datamodel/types.ts`. The following excerpts are the UI interfaces currently consumed by the prototype.

### Core Types

```typescript
export type ServiceRole = 'tech' | 'ops'
export type ViewMode = 'browse' | 'edit'
export type MatchType = 'exact' | 'pattern' | 'default'
export type WarningSeverity = 'info' | 'warning' | 'blocked'
export type LoadingStatus = 'idle' | 'loading' | 'success' | 'error'
```

### Provider and Model Rule Records

```typescript
export interface ProviderRecord {
  provider_key: string
  provider_driver: string
  name: string
  base_url: string | null
  provider_kind: 'origin' | 'aggregator'
  protocol_family: string | null
  enabled: boolean
  owner_service: ServiceRole
  revision: string
  created_at: number
  updated_at: number
}

export interface ModelParamRuleRecord {
  rule_key: string
  provider_key: string | null
  source_rule_key: string | null
  match_type: MatchType
  original_provider: string | null
  model_id_selector: string | null
  priority: number | null
  model_driver: string | null
  api_types: string[]
  logical_mounts: string[]
  capabilities: Record<string, boolean | number | string>
  attributes: Record<string, unknown> | null
  context_limits: Record<string, number> | null
  pricing: Record<string, unknown> | null
  exclude: boolean
  enabled: boolean
  created_at: number
  updated_at: number
}
```

`ProviderRecord.name` is the user-facing provider display name. The Add Provider
wizard requires it to be unique ignoring case and limits it to English letters,
digits, underscores, and hyphens. `provider_driver` is the unique driver metadata
id and delivery filename stem; the wizard derives it from `name.toLowerCase()`.
`protocol_family` is the selected client wire protocol family and must be one of
the currently supported protocol adapters.

`model_param_rules` stores exact, pattern, and default model parameter rules in one collection. `match_type` is the discriminator. Published JSON still materializes them as separate `models`, `patterns`, and `defaults` fields.

### Variants, Version Rules, and Nick Rules

```typescript
export interface MetadataVariantRecord {
  variant_key: string
  provider_key: string | null
  source_variant_key: string | null
  selector_type: 'exact' | 'pattern'
  original_provider: string | null
  model_id_selector: string
  priority: number
  nick: string | null
  content: Record<string, unknown>
  enabled: boolean
  created_at: number
  updated_at: number
}

export interface MetadataVersionRuleRecord {
  version_rule_key: string
  provider_key: string | null
  source_version_rule_key: string | null
  selector_type: 'exact' | 'pattern'
  original_provider: string | null
  model_id_selector: string
  priority: number
  nick: string | null
  content: Record<string, unknown>
  enabled: boolean
  created_at: number
  updated_at: number
}

export interface ModelNickRecord {
  nick_key: string
  provider_key: string
  original_provider: string | null
  model_id: string
  nick: string
  selector_type: 'exact' | 'pattern'
  priority: number
  created_at: number
  updated_at: number
}

export interface OriginProviderAliasRecord {
  alias_key: string
  provider_key: string
  alias: string
  driver: string
  created_at: number
  updated_at: number
}
```

Variants and version rules stay as independent record collections. They are not stored as page-only aggregated JSON.
`ModelNickRecord` is the Nick rewrite authoring record. It constructs provider-native selectors from source metadata when a provider reuses another provider's source rules. It does not generate final driver metadata `origin_mappings`.
`OriginProviderAliasRecord` materializes to the final JSON `origin_provider_aliases` table for the same provider. `OriginMappingRuleRecord` materializes to final JSON `origin_mappings`.

### Directory, Dictionary, Ops, and Source Records

```typescript
export interface LogicalDirectoryRecord {
  directory_key: string
  path: string
  title: string
  parent_key: string | null
  model_rule_keys: string[]
  created_at: number
  updated_at: number
}

export interface DictionaryItem {
  key: string
  label: string
  kind: 'api_type' | 'capability'
  value_type: 'boolean' | 'number'
  referenced_by: number
}

export interface OpsOverlayRecord {
  overlay_key: string
  target_type: 'provider' | 'model_param_rule' | 'variants' | 'version_rules'
  target_key: string
  disabled: boolean
  ops_patch: Record<string, unknown>
  created_at: number
  updated_at: number
}

export interface TechSourceRecord {
  service_url: string
  source_revision: string
  ops_revision: string
  last_sync_at: number | null
  last_success_at: number | null
  last_error: string | null
  stale: boolean
  cache_revision: string
}
```

`OpsOverlayRecord.ops_patch` is the only editable operations overlay container. Operations pages may display technical fields but must only write `ops_overlays`.

Logical directory records are explicit directory objects, but the UI directory tree is not limited to those records. The tree should also be materialized from `ModelParamRuleRecord.logical_mounts` and generated mounts from variants/version rules. Explicit `logical_directories` provide titles, parent relationships, and curated model memberships; mount strings provide additional browse paths and mounted model references.

### Workflow Records

```typescript
export interface EditSessionRecord {
  session_id: string
  service_role: ServiceRole
  operator_id: string
  base_revision: string
  status: 'editing' | 'previewed' | 'approved' | 'published' | 'discarded'
  pending_change_count: number
  created_at: number
  updated_at: number
}

export interface PendingChangeRecord {
  change_key: string
  target_type: string
  target_key: string
  action: 'create' | 'update' | 'delete' | 'disable'
  summary: string
  risk: WarningSeverity
}

export interface WarningRecord {
  warning_key: string
  severity: WarningSeverity
  target_type: string
  target_key: string
  message_key: string
  detail?: string
  created_at: number
}

export interface ChangeLogRecord {
  change_id: string
  service_role: ServiceRole
  operator_id: string
  from_revision: string
  to_revision: string
  source_revision: string | null
  source_stale: boolean
  summary: string
  created_at: number
}
```

### Import Plan Records

```typescript
export type ImportPlanActionName =
  | 'upsert_provider'
  | 'disable_provider'
  | 'upsert_model_param_rule'
  | 'delete_model_param_rule'
  | 'include_models'
  | 'exclude_models'
  | 'set_model_nick'
  | 'upsert_variant'
  | 'upsert_version_rule'
  | 'set_logical_mounts'
  | 'upsert_logical_directory'
  | 'delete_logical_directory'
  | 'move_logical_directory'
  | 'set_api_types'
  | 'upsert_api_type'
  | 'delete_api_type'
  | 'set_capabilities'
  | 'upsert_capability'
  | 'delete_capability'

export interface ImportPlanActionRecord {
  action_key: string
  action: ImportPlanActionName | 'unsupported'
  raw_action: string
  target_type: string
  target_key: string
  selector: string | null
  match_type: MatchType | null
  priority: number | null
  hit_count: number
  samples: string[]
  affected_count: number
  reference_samples: string[]
  published_selector: string | null
  fallback_behavior: string | null
  source_record: string | null
  field_changes: Array<{ field: string; before: string; after: string }>
  risk: WarningSeverity
  summary: string
  errors: string[]
}

export interface ImportPlanParseResult {
  plan_id: string
  title: string
  action_count: number
  supported_count: number
  error_count: number
  actions: ImportPlanActionRecord[]
  warnings: WarningRecord[]
}

export interface ImportPlanDraftRecord {
  draft_id: string
  title: string
  text: string
  parse_result: ImportPlanParseResult | null
  workspace: ProviderCloudSeed
  saved_at: number
}
```

Unsupported import actions are explicit records with `action = 'unsupported'` and validation errors; they are not silently ignored.

### Dataset Root and Publish Preview

```typescript
export interface ProviderCloudSeed {
  published_revision: string
  source_revision: string
  ops_revision: string
  tech_source: TechSourceRecord
  providers: ProviderRecord[]
  model_param_rules: ModelParamRuleRecord[]
  metadata_variants: MetadataVariantRecord[]
  metadata_version_rules: MetadataVersionRuleRecord[]
  model_nicks: ModelNickRecord[]
  origin_provider_aliases: OriginProviderAliasRecord[]
  logical_directories: LogicalDirectoryRecord[]
  dictionaries: DictionaryItem[]
  ops_overlays: OpsOverlayRecord[]
  edit_session: EditSessionRecord | null
  pending_changes: PendingChangeRecord[]
  change_logs: ChangeLogRecord[]
  warnings: WarningRecord[]
  import_plan_result: ImportPlanParseResult | null
  import_plan_draft: ImportPlanDraftRecord | null
  revision_conflict: boolean
}

export interface DriverMetadataRule {
  id?: string
  pattern?: string
  model_driver?: string
  exclude?: boolean
  parameter_scale?: string
  api_types?: string[]
  logical_mounts?: string[]
  capabilities?: Record<string, boolean | number | string>
  context_limits?: Record<string, number>
  pricing?: {
    price: number
    currency: string
    unit?: string
  }
  estimated_cost_usd?: number
  estimated_latency_ms?: number
  quality_score?: number
  latency_class?: string
  cost_class?: string
}

export interface DriverMetadataDocument {
  schema_version: 2
  provider_driver: string
  name?: string | null
  protocol_family: string
  base_url?: string | null
  revision: string
  origin_provider_aliases?: Record<string, string>
  origin_mappings?: Array<{
    mapping_key: string
    priority: number
    match: {
      source: 'provider_model_id'
      regex: string
    }
    transforms?: {
      driver?: Array<{ op: 'trim' | 'lowercase' | 'alias'; table?: string; on_missing?: 'keep' | 'empty' | 'error' }>
      model?: Array<{ op: 'trim' | 'lowercase' | 'alias'; table?: string; on_missing?: 'keep' | 'empty' | 'error' }>
    }
  }>
  models: DriverMetadataRule[]
  patterns: DriverMetadataRule[]
  defaults: DriverMetadataRule
  variants: Array<Record<string, unknown>>
  version_rules: Array<Record<string, unknown>>
  signature: Record<string, unknown> | null
}

export interface PublishPreview {
  revision: string
  provider_count: number
  model_rule_count: number
  pending_change_count: number
  warnings: WarningRecord[]
  impact: {
    providers: number
    model_rules: number
    dictionaries: number
  }
  ops_impact: {
    visible_providers: number
    visible_models: number
    disabled_providers: number
    disabled_models: number
    overlay_hits: number
    discarded_technical_fields: number
    source_stale: boolean
  }
  published_json: DriverMetadataDocument[]
}
```

## 6. Input Zod Schemas

The canonical schema source is `src/frame/provider_metadata_cloud/web/src/datamodel/schemas.ts`. React forms derive their input types from these schemas through `z.infer` or `z.input`.

| Schema | Purpose | Key constraints |
| --- | --- | --- |
| `tableFilterSchema` | shared search/provider/api/capability filters | strings trimmed, max 96-128 |
| `editSessionActionSchema` | edit session action reason | role enum, reason 4..180 |
| `providerInputSchema` | direct technical provider upsert | current schema includes `provider_key`; Provider Wizard presents the key as auto-assigned/read-only |
| `modelRuleInputSchema` | exact/pattern/default model rule create/edit | lowercase `rule_key`, `match_type`, controlled provider/api/capability fields, selector required for exact/pattern, `exclude` only meaningful for exact/pattern |
| `deleteModelRuleInputSchema` | model rule delete | non-empty `rule_key` |
| `nickRuleInputSchema` | origin identity mapping authoring | exact/pattern selector, priority 1..999, origin template or regex |
| `resolverRuleInputSchema` | compatibility schema for model_param_rule writes | model rule UI uses `modelRuleInputSchema`; variants/version_rules use their own schemas |
| `metadataVariantInputSchema` | variant record edit | exact/pattern selector, priority 1..999, mount suffix, logical mounts, capabilities, JSON content patch |
| `metadataVersionRuleInputSchema` | version rule edit | `content.model_pattern` exact/pattern selector, priority 1..999, family/tier, current/version mounts, auto mounts, capabilities, JSON content patch |
| `logicalDirectoryInputSchema` | directory create/edit | key slug, path starts with `/`, title, parent key |
| `dictionaryItemInputSchema` | api type/capability dictionary | key regex `[a-z0-9._-]`, kind enum, value type enum |
| `dictionaryBulkApplyInputSchema` | apply dictionary item to models | existing dictionary key path in UI, one or more model rule keys |
| `providerWizardModelRuleDraftSchema` | provider wizard model rule draft | exact/pattern/default, selector, priority, api_types, capabilities, logical_mounts, exclude |
| `providerWizardResolverRuleDraftSchema` | provider wizard variant/version draft | immutable `variant` or `version_rule` type, selector, original provider, model selector, nick, type-specific fields such as mount suffix or family/tier/current/version mount, capabilities, source key, and a shared internal `logical_mounts` array. The Add Provider Logical mounts step applies this array only to `version_rule` drafts and publishes it as `auto_mounts`; variant draft mounts are preserved from source/default draft data. |
| `providerWizardInputSchema` | provider wizard happy path | provider fields, existing source match-rule selection, `model_rule_drafts[]`, resolver source selections, `resolver_rule_drafts[]`, multi-origin nick rewrite rows, pattern order, and per-draft mount arrays. The Logical mounts step applies user path edits to model rule drafts and version-rule drafts only. |
| `importPlanInputSchema` | parse import plan | title 2..80, text 12..12000 |
| `importPlanDraftInputSchema` | save draft | import plan input plus `plan_id` |
| `publishWizardInputSchema` | publish confirmation | release note 8..280 and required confirmations |
| `techSourceInputSchema` | source service URL | URL, max 180 |
| `providerOpsInputSchema` | provider operations overlay | provider key, disabled, recommendation, priority, routing tag, note |
| `modelOpsInputSchema` | model operations overlay | pricing/routing/class/quality/recommendation/rollout/note |
| `resolverOpsOverlayInputSchema` | resolver/variant/version ops overlay | target type/key, disabled, routing tag, note |
| `opsBulkOperationInputSchema` | bulk operations | filters, action enum, numeric range checks |

Representative schemas:

```typescript
export const providerInputSchema = z.object({
  provider_key: z.string().trim().min(2).max(64).regex(/^[a-z0-9][a-z0-9-]*$/),
  name: z.string().trim().min(2).max(80).regex(/^[A-Za-z0-9_-]+$/),
  provider_driver: z.string().trim().min(2).max(80).regex(/^[a-z0-9_-]+$/),
  base_url: z.string().trim().url().max(180),
  provider_kind: z.enum(['origin', 'aggregator']),
  protocol_family: z.enum(['openai-compatible', 'anthropic', 'google-gemini', 'fal', 'minimax']),
  template_provider_key: z.string().trim().max(96),
})

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
  logical_mounts: z.array(z.string().trim().min(1).max(128)).min(1),
  scope: z.enum(['global', 'provider']),
  exclude: z.boolean(),
}).refine((value) => value.match_type === 'default' || value.model_id_selector.length > 0, {
  path: ['model_id_selector'],
  message: 'Selector is required for exact and pattern rules',
})

export const providerWizardModelRuleDraftSchema = z.object({
  draft_key: z.string().trim().min(2).max(96),
  match_type: z.enum(['exact', 'pattern', 'default']),
  original_provider: z.string().trim().max(96),
  model_id_selector: z.string().trim().max(128),
  priority: z.number().int().min(1).max(999),
  source_rule_key: z.string().trim().max(96),
  model_driver: z.string().trim().min(1).max(96),
  api_types: z.array(z.string().trim().min(1).max(96)).min(1),
  capabilities: z.array(z.string().trim().min(1).max(96)),
  max_context_tokens: z.number().int().min(1).max(10000000).optional(),
  logical_mounts: z.array(z.string().trim().min(1).max(128)),
  exclude: z.boolean(),
})

export const providerWizardResolverRuleDraftSchema = z.object({
  draft_key: z.string().trim().min(2).max(96),
  rule_kind: z.enum(['variant', 'version_rule']),
  selector_type: z.enum(['exact', 'pattern']),
  original_provider: z.string().trim().min(1).max(96),
  model_id_selector: z.string().trim().min(1).max(128),
  priority: z.number().int().min(1).max(999),
  nick: z.string().trim().max(128),
  source_rule_key: z.string().trim().max(128),
  logical_mounts: z.array(z.string().trim().min(1).max(128)),
})

export const providerWizardInputSchema = z.object({
  provider_key: z.string().trim().min(2).max(64).regex(/^[a-z0-9][a-z0-9-]*$/),
  name: z.string().trim().min(2).max(80).regex(/^[A-Za-z0-9_-]+$/),
  provider_driver: z.string().trim().min(2).max(80).regex(/^[a-z0-9_-]+$/),
  base_url: z.string().trim().url().max(180),
  provider_kind: z.enum(['origin', 'aggregator']),
  protocol_family: z.enum(['openai-compatible', 'anthropic', 'google-gemini', 'fal', 'minimax']),
  template_provider_key: z.string().trim().max(96),
  selected_origins: z.array(z.string().trim().min(1).max(96)).min(1),
  selected_model_ids: z.array(z.string().trim().min(1).max(128)).min(1),
  selected_resolver_rule_keys: z.array(z.string().trim().min(1).max(128)),
  nick_rules: z.array(providerWizardNickRuleSchema).min(1),
  selected_api_types: z.array(z.string().trim().min(1).max(96)).min(1),
  selected_capabilities: z.array(z.string().trim().min(1).max(96)).min(1),
  selected_logical_mounts: z.array(z.string().trim().min(1).max(128)).min(1),
  model_rule_drafts: z.array(providerWizardModelRuleDraftSchema),
  resolver_rule_drafts: z.array(providerWizardResolverRuleDraftSchema),
})

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
    'enable',
    'disable',
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
})
```

## 7. Pagination Strategy

Current pagination is client-side offset pagination after filtering. The initial mock workspace is loaded as one `ProviderCloudSeed` snapshot.

| View | Page size / row cap | Source |
| --- | --- | --- |
| Providers | 8 | `pages/providers/index.tsx` |
| Models exact/pattern/default | 10 | `pages/models/index.tsx` |
| Resolver variants | 10 | `components/data-table/DataTable.tsx` |
| Resolver version rules | 10 | `components/data-table/DataTable.tsx` |
| Dictionary model tagging preview | 10 displayed from first 24 filtered models | `pages/dictionaries/index.tsx` |
| Bulk operation preview | 10 sample rows | `previewOpsBulkOperation()` |

Every collection that can grow in real operation must support pagination, search/filter state, and stable total counts. Some prototype tables still cap or render sample rows because the mock seed is small; backend integration should use offset or cursor pagination for providers, model rules, nick rules, variants, version rules, logical directory model lists, dictionaries, import plan actions, warnings, and change logs.

Logical directory mounted model lists should use a searchable paged list or equivalent virtualized selector. A compact native multi-select is not a sufficient primary control for real model counts.

## 8. Filters and Sorting

Current filter fields:

| Entity/view | Filters |
| --- | --- |
| Providers | free text over `provider_key`, `name`, `provider_driver`, `base_url`, `provider_kind` |
| Model rules | free text over `rule_key`, `original_provider`, `model_id_selector`, `match_type`, `model_driver`; plus provider, api type, capability |
| Model rules | `match_type`, source order; pattern rules sorted by ascending `priority`; exact/pattern can be filtered by `exclude` |
| Resolver variants/version rules | target type, provider scope, original provider, selector type, variant `model_id_selector` or version rule `content.model_pattern`, priority |
| Ops bulk | provider, original provider, wildcard model id pattern, api type, capability, recommendation, price range, routing weight range |
| Dictionaries | dictionary kind, model rule filters, dictionary references |
| Logical directory | path browsing mode or search mode, mutually exclusive in UI |
| Warnings | service role diagnostics, target object link |
| Change logs | current `serviceRole` |

Wildcard selectors use `*` converted to an anchored regular expression. Pattern resolver rules are displayed and published by ascending `priority`.

## 9. Derived and Aggregated Fields

Selectors live in `src/frame/provider_metadata_cloud/web/src/datamodel/selectors.ts`.

| Derived value | Function | Rule |
| --- | --- | --- |
| Provider model count | `getProviderModelCount()` | currently white-listed source model rules for that provider, excluding disabled/excluded rules; never a fixed display constant |
| Provider warnings | `getProviderWarnings()` | warnings with matching `target_key` |
| Ops provider preview | `buildOpsProviderPreview()` | provider enabled flag plus provider overlay |
| Ops model preview | `buildOpsModelPreview()` | rule enabled flag plus model overlay |
| Effective pricing/routing/recommendation | internal selectors | ops overlay value with stable fallback |
| Model rule filters | `filterModelRules()` | text/provider/api/capability conjunction |
| Bulk hit set | `filterOpsBulkModelRules()` | all bulk filters plus numeric ranges |
| Bulk preview | `previewOpsBulkOperation()` | first 10 samples plus changed counts |
| Resolver hits | `previewResolverHits()` | exact match, wildcard pattern, or default fallback miss |
| Resolver diagnostics | `getResolverWarnings()` | duplicate exact keys and empty pattern hits |
| Variant/version hits | `previewVariantHits()` | origin + exact/pattern selector |
| Directory items | `materializeDirectoryItems()` | child directories plus mounted model rules |
| Directory diagnostics | `getLogicalDirectoryWarnings()` | duplicate path/key, empty directory, broken model refs |
| Dictionary references | `getDictionaryReferenceCount()` | api type/capability usage in model rules |
| Nick preview | `previewNickRewrite()` | provider origin identity rules sorted by priority |
| Provider wizard nick drafts | `saveProviderWizard()` | `nick_rules[]` materializes to multiple `ModelNickRecord` rows and rewrites provider-native selectors only. |
| Provider wizard origin mapping drafts | `saveProviderWizard()` | `origin_mapping_rules[]` materializes to multiple `OriginMappingRuleRecord` rows and generates final `origin_mappings`. |
| Provider wizard variant/version drafts | `saveProviderWizard()` | source selections materialize to provider-scoped `metadata_variants` and `metadata_version_rules` rows; Variant/version params uses distinct type-specific panels with the same multi-select edit session as Model params, does not edit priority, edits variant `provider_options`, edits version-rule predicates and single-value `current_mount`/`version_mount`, and leaves version-rule `auto_mounts` to the dedicated Logical mounts step |
| Tech diagnostics | `buildTechDiagnostics()` | provider duplicates, empty selections, missing dictionary refs, nick conflicts |
| Ops diagnostics | `buildOpsDiagnostics()` | stale/sync failure, missing overlay target, polluted technical fields, invalid pricing/routing |
| Client driver metadata | `buildPublishedProviderJson()` | materializes `schema_version`, `provider_driver`, `name`, `protocol_family`, `base_url`, `revision`, `origin_provider_aliases`, `origin_mappings`, `models`, sorted `patterns`, `defaults`, `variants`, `version_rules`, and `signature` |
| Publish preview | `buildPublishPreview()` | pending changes, diagnostics, ops impact, JSON sample |

## 10. Technical vs Operations Ownership

The UI enforces ownership through service role, page routing, and schema choice.

Technical service owns:

- `ProviderRecord` identity and technical source fields: `provider_key`, `provider_driver`, `name`, `base_url`, `provider_kind`, `protocol_family`, `enabled`, `revision`.
- `ModelParamRuleRecord` technical fields: provider scope, `match_type`, selectors, original provider, driver, api types, logical mounts, capabilities, attributes, context limits, pricing, exclude/enabled.
- Source-object references, origin identity authoring rules, origin provider aliases, metadata variants, metadata version rules.
- Logical directory and dictionary records.
- Technical import plan actions and generated technical pending changes.

Operations service owns:

- `TechSourceRecord` sync source and stale/cache state.
- `OpsOverlayRecord` for providers, model rules, variants, and version rules.
- Provider operations fields stored in `ops_patch`: `recommendation_level`, `display_priority`, `routing_policy_tag`, `ops_note`.
- Model operations fields stored in `ops_patch`: `pricing_override`, `routing_weight`, `cost_class`, `latency_class`, `quality_score`, `recommendation_level`, `display_priority`, `rollout_strategy`, `ops_note`.
- Bulk operations and stale publish confirmation.

Rules:

- Operations pages may display technical fields read-only.
- Operations updates must write overlays, not mutate technical records.
- `buildOpsDiagnostics()` treats overlay attempts to write technical fields such as `provider_key`, `provider_driver`, `model_id_selector`, `match_type`, `api_types`, `capabilities`, or `logical_mounts` as pollution.
- Operations overlay fields are merge inputs only. Final client driver metadata may reflect valid metadata fields such as `pricing`, `estimated_cost_usd`, `quality_score`, `latency_class`, or `cost_class`, but it must not include `routing_weight`, `recommendation_level`, `display_priority`, `routing_policy_tag`, or `ops_note`.
- Technical pages do not edit operations overlay fields.

## 11. Field Stability Classification

| Field group | Stability | Notes |
| --- | --- | --- |
| `ProviderCloudSeed` root collection names | Frozen | Pages, selectors, and store depend on these names |
| Record key fields such as `provider_key`, `rule_key`, `variant_key`, `directory_key` | Frozen | Used for references, table row keys, pending changes, and overlays |
| `ServiceRole`, `ViewMode`, `MatchType` | Frozen | Drive routing, editing, and resolver behavior |
| `model_param_rules.match_type` unified storage | Frozen | Required for exact/pattern/default storage and publish materialization |
| `OpsOverlayRecord.target_type/target_key/ops_patch` | Frozen | Ownership boundary between tech and ops |
| `DataState<T>` shape | Frozen | Shared loading/error contract |
| `ImportPlanActionName` supported action names | Frozen for current mock plan | Parser and UI action table depend on explicit names |
| `DriverMetadataDocument.models/patterns/defaults/variants/version_rules` | Frozen | Client delivery shape must stay compatible with driver metadata schema |
| `DriverMetadataDocument.origin_mappings` | Frozen | Runtime resolver uses this to derive `{driver}` and `{model}` for mount expansion |
| Warning severity values | Extensible | New severities need i18n and visual handling |
| Recommendation/cost/latency/rollout enum values | Extensible | Additive values require filters and labels |
| `ops_patch` subfields | Extensible | New ops knobs can be added behind schema and diagnostics |
| `attributes`, `context_limits`, `pricing`, variant/version `content` | Extensible | Structured technical payloads can grow |
| `WarningRecord.message_key/detail` text | Volatile | Mock diagnostics text can change |
| Mock revision string formats | Volatile | Real service should define revision format |
| `mock/api.ts` function names | Extensible | Store can adapt to a real API adapter |

## 12. Mock Data Contract

Mock data source and behavior:

- Technical seed derives from `src/frame/aicc/driver_metadata/*.json` through `mock/driverMetadataSeed.ts`.
- Cloud seed and overlays live in `mock/providerCloudSeed.ts`.
- Async behavior lives in `mock/latency.ts`.
- Mutations and workflows live in `mock/api.ts`.
- Selectors calculate page counts, previews, diagnostics, and published JSON from normalized seed collections.

Mock data requirements:

- Include enough providers and model rules to exercise pagination.
- Keep exact/pattern/default model parameter rules in `model_param_rules`.
- Manage exact/pattern/default only through the Models UI path; Resolver Rules manages variants/version_rules.
- Keep `exclude` as an exact/pattern model rule attribute. When `exclude=true`, other model parameters are stored but ignored for current publish semantics so the rule can be restored later.
- Provider authoring is a white-list workflow. Models, patterns, defaults, variants, and version rules are selected as source objects; disabling an included model rule uses its own `exclude=true` value. The system has no Selection Rules route, storage record, or workflow step.
- `selected_resolver_rule_keys[]` identifies selected existing variants/version rules as `variant:<key>` or `version_rule:<key>`. The wizard persists their source key in `source_variant_key` or `source_version_rule_key`; manual resolver drafts leave it empty.
- Model parameter and variant/version parameter batch edits are field-level overlays over selected drafts: only fields explicitly applied by the user replace those fields, and every other field retains the individual source value. Model params edits `model_driver`, api types, capabilities, estimates, classes, and related model rule fields; matching identity fields are editable only for a single target. Variant/version params uses separate Variant and Version tabs, supports multi-select pending edit targets, and does not edit priority or bulk logical mounts. Variant drafts edit `provider_options` as JSON. Version-rule drafts edit `family`, `tier`, `model_pattern`, `tier_tokens`, `exclude_tier_tokens`, `version_rank.prefix`, `stability.unstable_tokens`, `stability.current_requires_stable`, `exclude_snapshot_date_suffix`, `current_mount`, and `version_mount`; token arrays are free-text token inputs, and `current_mount`/`version_mount` are single selections from the materialized logical directory tree. Logical mounts use the same materialized tree and a cross-object `mountTargetKeys[]` selection in the dedicated Logical mounts step for models, patterns, defaults, and version rules.
- Provider Wizard does not create a separate wizard-only storage shape. Its Models step selects existing models, patterns, and defaults; Model params creates or edits provider-scoped `model_param_rules`; Variants / Version rules and Variant/version params create provider-scoped `metadata_variants` and `metadata_version_rules`; Logical mounts reuses the same selector/mount vocabulary for models, patterns, defaults, and version rules; Nick rewrite writes multiple `model_nicks` records. Existing providers enter this same wizard with immutable `provider_key`.
- Existing-provider edit is a full scoped-set replacement: initial drafts are reconstructed from that provider's `model_param_rules`, `metadata_variants`, `metadata_version_rules`, and `model_nicks`; saving removes stale records scoped to that provider before inserting the edited set.
- Provider Wizard `Kind` values are `origin` and `aggregator`. `Name` is entered by the user, restricted to English letters, digits, underscores, and hyphens, and must be unique ignoring case. `provider_driver` is derived from `Name.toLowerCase()` and is used as the delivery filename stem and final JSON `provider_driver`. `protocol_family` describes the client-facing wire protocol it speaks, is selected from the currently supported protocol list, and may be reused by multiple providers.
- Nick rules remain Nick rewrite authoring mappings, not copies of source metadata. They support origin-prefix templates such as `openai/{model}` and exact/pattern mapping. When a provider reuses source rules from another provider, Nick rules may construct provider-native `models[].id`, `patterns[].pattern`, variant selectors, and version-rule `content.model_pattern`. Separate Origin mapping rules then tell the runtime how to map those provider-native ids back to physical `{driver}` and `{model}`. A change or deletion of an object referenced by another provider produces a synchronization-review warning for that dependent provider.
- Resolver Rules authoring forms require an explicit `original_provider`; they must not use an `All` option as a persisted value.
- Keep variants and version rules as independent records.
- Include dictionaries for controlled api type/capability choices.
- Include normal, stale, empty, and error scenarios.
- Include technical and operations pending changes.
- Keep mutations in memory/session scope only; no real persistence.

## 12.1 Authoring Interaction Contract

- `provider_key`, `nick_key`, `variant_key`, and `version_rule_key` are automatically assigned. Existing provider editing opens the same wizard as creation with `provider_key` immutable. Template selection is either one concrete provider or no template (`Start from scratch`); `All` is invalid.
- The provider wizard order is `Basic -> Models -> Model params -> Pattern order -> Variants / Version rules -> Variant/version params -> Logical mounts -> Nick rewrite -> Preview`. There is no Selection Rules step or Selection Rules page.
- Models and Variants / Version rules are source-selection steps. They share a three-column layout: source-provider list, type-tabbed available records, and type-tabbed selected records. Their tabs show selected/total counts; empty provider groups and tabs are hidden. Models includes exact, pattern, and default source rules.
- Model params and Variant/version params are separate two-area edit sessions: the upper area selects targets using the same provider/type/selected layout; the lower area edits parameters. A common field displays a value only when all selected targets agree. Apply changes selected fields only and clears the target selection; Discard restores the current session snapshot. Newly created records appear under the current target provider. Variants and version rules use independent tabs and type-specific panels; their type is immutable, and Variant/version params does not edit priority or bulk logical mounts. Version-rule token arrays are free text, while `current_mount` and `version_mount` are selected from the materialized directory tree and stored as mount strings.
- Model params can change `original_provider` only to the target provider when turning a source rule into a provider-scoped object; when all selected targets already belong to the target provider, the field is read-only. Model selector is assigned per target and never batch-edited. Model params does not edit priority; pattern priority is authored only in Pattern order. A changed source record becomes a provider-scoped copy only when its values actually differ from the source.
- Logical mounts is the Add Provider wizard's only bulk `logical_mounts`/`auto_mounts` editor. It uses the same target selector over non-empty Models/Patterns/Defaults/Version rules tabs; variants are not configured in this step. The tree is tri-state across selected targets: all selected is checked, a subset is indeterminate, none is unchecked. Indeterminate -> checked -> unchecked clicks are deterministic. Selected paths lists only paths checked for every selected target. Apply adds checked paths to all targets, removes unchecked paths from all targets, and leaves indeterminate paths unchanged. Discard clears un-applied path overrides and restores the tree to the state before path selection while keeping the target selection. Version-rule `current_mount` and `version_mount` are single-value fields selected in Variant/version params, not members of this bulk mount set.
- Every source-reference mutation must locate reverse references through `source_rule_key`, `source_variant_key`, and `source_version_rule_key`. Updating or deleting a source object warns administrators to synchronize all dependent providers.
- Browse mode inspectors are read-only. In edit mode Providers, Models, Nick Rules, Resolver Rules, and Dictionaries expose row-level edit/delete actions. Resolver Rules uses separate Variants and Version rules tabs, explicit provider filtering, and immutable provider/original-provider keys after creation. Nick Rules uses the same Nick rewrite editor with a provider filter; its provider can only be set when creating a rule.

Valid input examples:

```typescript
const providerInput = {
  name: 'OpenRouter Lab',
  provider_driver: 'openrouter',
  base_url: 'https://openrouter.ai/api/v1',
  provider_kind: 'aggregator',
  protocol_family: 'openai-compatible',
  template_provider_key: 'openrouter',
}

const modelOpsInput = {
  rule_key: 'openrouter-gpt-4o',
  disabled: false,
  pricing_input: 0.000005,
  pricing_output: 0.000015,
  routing_weight: 80,
  cost_class: 'medium',
  latency_class: 'fast',
  quality_score: 92,
  recommendation_level: 'featured',
  display_priority: 20,
  rollout_strategy: 'stable',
  ops_note: 'Promote for default chat route.',
}
```

Invalid input examples:

| Input | Expected UI validation |
| --- | --- |
| client supplies `provider_key` on create | rejected; the service assigns the key |
| `base_url = "openrouter.local"` | fails URL validation |
| `rule_key = "gpt 4o"` | fails lowercase slug regex |
| exact model rule without `model_id_selector` | fails model rule schema |
| pattern model rule without `priority` | fails model rule schema |
| logical directory `path = "llm/chat"` | fails leading slash regex |
| dictionary `key = "chat model"` | fails dictionary key regex |
| import plan text shorter than 12 characters | fails import plan minimum length |
| publish without `confirm_key_risk` | fails publish refinement |
| bulk operation `price_min > price_max` | fails range refinement |
| bulk operation `routing_weight_min > routing_weight_max` | fails range refinement |

## 13. Text Import and Export Contract

JSON, YAML, and Markdown payloads can be much longer than the surrounding UI. View components should expose copy and download actions for:

- object JSON shown in the inspector,
- published provider JSON,
- publish preview JSON,
- diff reports,
- import plan parse results,
- warning or diagnostics JSON.

Input components for import plans and other long structured text should support both paste and local file upload. Upload reads UTF-8 text into the same validated form model as pasted text. Import Plan accepts `.yaml`, `.yml`, and `.md`; unsupported extensions and empty files should produce visible validation errors.

JSON viewers are inspection tools. Structured forms remain the primary write path for providers, model rules, variants, version rules, dictionaries, and logical directory records.

## 14. Future API/KRPC Mapping Notes

This section is guidance for integration planning only.

| UI store action | Future backend concern | Mapping note |
| --- | --- | --- |
| `reload()` | bootstrap/list aggregation | May be one aggregate endpoint or several paged endpoints composed by store |
| `upsertProvider()` | technical provider write | Send only technical provider fields |
| `runProviderWizard()` | multi-record technical workflow | Backend may expose command or store may call multiple APIs |
| `upsertModelRule()` | model param rule write | Preserve unified `model_param_rules` storage; Models is the UI owner for exact/pattern/default |
| `upsertMetadataVariant()` / `upsertMetadataVersionRule()` | metadata record write | Keep variants/version rules independent records |
| `upsertProviderOps()` / `upsertModelOps()` / `upsertResolverOps()` | ops overlay write | Send only target key/type, disabled, and allowed ops patch fields |
| `applyBulkOperation()` | server-side bulk preview/apply | Large datasets should use preview token or server-side hit set |
| `importPlan()` / draft actions | import parser and staging | Parser returns actions, hit counts, samples, errors; no direct publish |
| `runPublishPreview()` | validation and materialized JSON preview | Return diagnostics, impact, and `models/patterns/defaults` split |
| `completePublish()` | publish command | Requires base revision, release note, risk confirmations, stale confirmation |

Open integration points:

- Whether list data is delivered as one aggregate workspace or multiple paged resources.
- Whether bulk preview produces a server-side token for apply.
- Exact KRPC service names and RBAC checks.
- Exact warning code taxonomy.
- Revision conflict payload format.

## 15. Verification Expectations

Round 8 is complete when:

- This document matches `types.ts`, `schemas.ts`, `selectors.ts`, store, and current routes.
- Pages access data through `useProviderMetadataStore()`.
- `ProviderMetadataStoreProvider` is the only UI layer importing `mock/api.ts`.
- Build/typecheck passes.
- Configured lint passes if a lint script exists.
- Playwright coverage matches current authoring rules. Tests that create provider, nick, variant, or version rule records must treat auto-assigned keys as read-only instead of filling key inputs.
