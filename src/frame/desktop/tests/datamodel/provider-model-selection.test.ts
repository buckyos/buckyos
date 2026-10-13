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
