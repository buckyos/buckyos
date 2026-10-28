import {
  toProviderInstanceRules,
  type ProviderModelSelectionDraft,
} from '../../src/api/provider_model_selection.ts'

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message)
}

function draft(discoveryBehavior: string): ProviderModelSelectionDraft {
  return {
    discovery_behavior_id: discoveryBehavior,
    selectable_inventory_models: ['third-party-a', 'third-party-b'],
    selected_inventory_models: ['third-party-a'],
  }
}

Deno.test('live discovery model selection excludes only unchecked selectable models', () => {
  const rules = toProviderInstanceRules(draft('openai-compatible-models'))
  assert(rules !== undefined, 'selection must produce instance rules')
  assert(rules.enabled_inventory_models === undefined, 'live discovery must not create a static inventory allowlist')
  assert(
    JSON.stringify(rules.exclude_models) === JSON.stringify(['third-party-b']),
    'live discovery must exclude only unchecked selectable models',
  )
})

Deno.test('catalog-only model selection keeps using the static inventory allowlist', () => {
  const rules = toProviderInstanceRules(draft('catalog-only'))
  assert(rules !== undefined, 'selection must produce instance rules')
  assert(JSON.stringify(rules.exclude_models) === JSON.stringify([]), 'catalog-only selection must not add exclusions')
  assert(
    JSON.stringify(rules.enabled_inventory_models) === JSON.stringify(['third-party-a']),
    'catalog-only selection must retain the selected static inventory allowlist',
  )
})

Deno.test('custom model identity and server defaults are independent from channel presets', () => {
  const rules = toProviderInstanceRules({
    model_driver_overrides_json: '{"local-qwen":"qwen/qwen3.7-plus"}',
    server_default_routes_json: '{"local-qwen":["llm.chat"]}',
  })
  assert(rules !== undefined, 'explicit advanced settings must survive serialization')
  assert(JSON.stringify(rules.model_driver_overrides) === '{"local-qwen":"qwen/qwen3.7-plus"}', 'alias identity must be retained')
  assert(JSON.stringify(rules.server_default_routes) === '{"local-qwen":["llm.chat"]}', 'server defaults need explicit task admission')
  assert(!('reasoning' in rules), 'server defaults must not disable or prescribe thinking')
})

Deno.test('invalid advanced settings cannot produce a provider payload', () => {
  for (const draft of [
    { model_driver_overrides_json: '[]' },
    { model_driver_overrides_json: '{"":"qwen/qwen3.7-plus"}' },
    { server_default_routes_json: '{"":["llm.chat"]}' },
    { model_driver_overrides_json: '{"local":"unknown"}' },
    { server_default_routes_json: '{"local":["model@instance"]}' },
    { server_default_routes_json: '{"local":[]}' },
  ]) {
    let rejected = false
    try { toProviderInstanceRules(draft) } catch { rejected = true }
    assert(rejected, 'invalid identity/routes must be rejected before submission')
  }
})
