import { buildDriverMetadataTables } from './driverMetadataSeed'
import type {
  ChangeLogRecord,
  DictionaryItem,
  LogicalDirectoryRecord,
  ModelNickRecord,
  OpsOverlayRecord,
  PendingChangeRecord,
  ProviderCloudSeed,
  ProviderModelRuleRecord,
  ProviderRecord,
  WarningRecord,
} from '../datamodel/types'

const now = Date.UTC(2026, 6, 10, 9, 30, 0)
const driverTables = buildDriverMetadataTables()

const openRouterProvider: ProviderRecord = {
  provider_key: 'openrouter',
  provider_driver: 'openrouter',
  name: 'OpenRouter',
  base_url: 'https://openrouter.ai/api/v1',
  provider_kind: 'aggregator',
  protocol_family: 'openai-compatible',
  enabled: true,
  owner_service: 'tech',
  revision: 'draft-seed',
  created_at: now,
  updated_at: now,
}

const providerModelRules: ProviderModelRuleRecord[] = [
  {
    rule_key: 'openrouter-include-openai',
    provider_key: 'openrouter',
    rule_type: 'include_origin',
    selector: 'openai',
    priority: 1,
    enabled: true,
    created_at: now,
    updated_at: now,
  },
  {
    rule_key: 'openrouter-include-claude',
    provider_key: 'openrouter',
    rule_type: 'include_origin',
    selector: 'claude',
    priority: 2,
    enabled: true,
    created_at: now,
    updated_at: now,
  },
]

const modelNicks: ModelNickRecord[] = [
  {
    nick_key: 'openrouter-openai-prefix',
    provider_key: 'openrouter',
    original_provider: 'openai',
    model_id: 'gpt-*',
    nick: 'openai/{model}',
    selector_type: 'pattern',
    priority: 1,
    created_at: now,
    updated_at: now,
  },
  {
    nick_key: 'openrouter-claude-prefix',
    provider_key: 'openrouter',
    original_provider: 'claude',
    model_id: 'claude-*',
    nick: 'anthropic/{model}',
    selector_type: 'pattern',
    priority: 2,
    created_at: now,
    updated_at: now,
  },
]

const logicalDirectories: LogicalDirectoryRecord[] = [
  {
    directory_key: 'llm',
    path: '/llm',
    title: 'LLM',
    parent_key: null,
    model_rule_keys: ['openai-pattern-8', 'claude-pattern-7'],
    created_at: now,
    updated_at: now,
  },
  {
    directory_key: 'image',
    path: '/image',
    title: 'Image',
    parent_key: null,
    model_rule_keys: ['openai-exact-1', 'openai-pattern-6'],
    created_at: now,
    updated_at: now,
  },
  {
    directory_key: 'audio',
    path: '/audio',
    title: 'Audio',
    parent_key: null,
    model_rule_keys: ['openai-exact-6', 'openai-pattern-1'],
    created_at: now,
    updated_at: now,
  },
]

const dictionaries: DictionaryItem[] = [
  { key: 'llm', label: 'LLM', kind: 'api_type', value_type: 'boolean', referenced_by: 38 },
  { key: 'image.txt2img', label: 'Text to image', kind: 'api_type', value_type: 'boolean', referenced_by: 5 },
  { key: 'embedding.text', label: 'Text embedding', kind: 'api_type', value_type: 'boolean', referenced_by: 3 },
  { key: 'audio.asr', label: 'Speech to text', kind: 'api_type', value_type: 'boolean', referenced_by: 4 },
  { key: 'streaming', label: 'Streaming', kind: 'capability', value_type: 'boolean', referenced_by: 30 },
  { key: 'tool_call', label: 'Tool call', kind: 'capability', value_type: 'boolean', referenced_by: 24 },
  { key: 'json_schema', label: 'JSON schema', kind: 'capability', value_type: 'boolean', referenced_by: 22 },
  { key: 'vision', label: 'Vision', kind: 'capability', value_type: 'boolean', referenced_by: 16 },
  { key: 'max_context_tokens', label: 'Max context tokens', kind: 'capability', value_type: 'number', referenced_by: 26 },
]

const opsOverlays: OpsOverlayRecord[] = [
  {
    overlay_key: 'ops-expensive-models',
    target_type: 'model_param_rule',
    target_key: 'openai-pattern-8',
    disabled: false,
    ops_patch: {
      routing_weight: 80,
      cost_class: 'medium',
    },
    created_at: now,
    updated_at: now,
  },
]

const pendingChanges: PendingChangeRecord[] = [
  {
    change_key: 'pending-openrouter-provider',
    target_type: 'provider',
    target_key: 'openrouter',
    action: 'create',
    summary: 'Add OpenRouter aggregate provider',
    risk: 'warning',
  },
  {
    change_key: 'pending-openrouter-selection',
    target_type: 'model_param_rule',
    target_key: 'openai-pattern-8',
    action: 'update',
    summary: 'Include GPT model family for aggregate provider',
    risk: 'info',
  },
]

const warnings: WarningRecord[] = [
  {
    warning_key: 'warning-openrouter-key',
    severity: 'warning',
    target_type: 'provider',
    target_key: 'openrouter',
    message_key: 'warning.keyField',
    created_at: now,
  },
  {
    warning_key: 'warning-selector-coverage',
    severity: 'info',
    target_type: 'model_param_rule',
    target_key: 'claude-pattern-1',
    message_key: 'warning.selector',
    created_at: now,
  },
  {
    warning_key: 'warning-overlay-pollution',
    severity: 'warning',
    target_type: 'provider',
    target_key: 'openrouter',
    message_key: 'warning.overlay',
    created_at: now,
  },
]

const changeLogs: ChangeLogRecord[] = [
  {
    change_id: 'chg-20260709-0003',
    service_role: 'tech',
    operator_id: 'alice',
    from_revision: 'tech-20260708.000002',
    to_revision: 'tech-20260709.000003',
    source_revision: null,
    source_stale: false,
    summary: 'Refresh GPT and Claude resolver patterns',
    created_at: now - 86_400_000,
  },
  {
    change_id: 'chg-20260709-ops-0004',
    service_role: 'ops',
    operator_id: 'ops-admin',
    from_revision: 'ops-20260708.000004',
    to_revision: 'ops-20260709.000005',
    source_revision: 'tech-20260709.000003',
    source_stale: false,
    summary: 'Adjust recommended routing weights',
    created_at: now - 42_000_000,
  },
]

export const providerCloudSeed: ProviderCloudSeed = {
  published_revision: 'tech-20260710.000001',
  source_revision: 'tech-20260710.000001',
  ops_revision: 'ops-20260710.000006',
  tech_source: {
    service_url: 'https://metadata.tech-source.mock/kapi/provider-metadata-tech-service',
    source_revision: 'tech-20260710.000001',
    ops_revision: 'ops-20260710.000006',
    last_sync_at: now - 3_600_000,
    last_success_at: now - 3_600_000,
    last_error: null,
    stale: false,
    cache_revision: 'tech-20260710.000001',
  },
  providers: [...driverTables.providers, openRouterProvider],
  model_param_rules: driverTables.modelParamRules,
  metadata_variants: driverTables.variants,
  metadata_version_rules: driverTables.versionRules,
  provider_model_rules: providerModelRules,
  model_nicks: modelNicks,
  logical_directories: logicalDirectories,
  dictionaries,
  ops_overlays: opsOverlays,
  edit_session: {
    session_id: 'edit-tech-20260710-openrouter',
    service_role: 'tech',
    operator_id: 'alice',
    base_revision: 'tech-20260710.000001',
    status: 'editing',
    pending_change_count: pendingChanges.length,
    created_at: now,
    updated_at: now,
  },
  pending_changes: pendingChanges,
  change_logs: changeLogs,
  warnings,
  import_plan_result: null,
  import_plan_draft: null,
  revision_conflict: false,
}
