export interface ProviderModelSelectionDraft {
  model_driver_overrides_json?: string
  server_default_routes_json?: string
  policy_region?: string
  discovery_behavior_id?: string
  selectable_inventory_models?: string[]
  selected_inventory_models?: string[]
}

/**
 * Static catalogs use `enabled_inventory_models`; live discovery uses the
 * complementary `exclude_models`. Models outside the profile-owned selectable
 * list (for example Qwen first-party base models) remain published automatically.
 */
export function toProviderInstanceRules(
  draft: ProviderModelSelectionDraft,
): Record<string, unknown> | undefined {
  const policyRegion = draft.policy_region?.trim()
  const selection = draft.selected_inventory_models
  const selectable = draft.selectable_inventory_models
  const hasSelection = Array.isArray(selection)
  const advanced = parseProviderAdvancedRules(draft)
  if (!policyRegion && !hasSelection && !Object.keys(advanced.model_driver_overrides).length && !Object.keys(advanced.server_default_routes).length) return undefined
  const selected = new Set(selection ?? [])
  const catalogOnly = draft.discovery_behavior_id === 'catalog-only'
  return {
    policy_region: policyRegion || undefined,
    exclude_models: !catalogOnly && Array.isArray(selectable)
      ? selectable.filter((model) => !selected.has(model)).sort()
      : [],
    ...advanced,
    enabled_inventory_models: catalogOnly && hasSelection ? [...selection].sort() : undefined,
  }
}

export function parseProviderAdvancedRules(draft: ProviderModelSelectionDraft): {
  model_driver_overrides: Record<string, string>
  server_default_routes: Record<string, string[]>
} {
  const mapping: unknown = JSON.parse(draft.model_driver_overrides_json?.trim() || '{}')
  const routes: unknown = JSON.parse(draft.server_default_routes_json?.trim() || '{}')
  const object = (value: unknown): value is Record<string, unknown> => value !== null && typeof value === 'object' && !Array.isArray(value)
  if (!object(mapping) || Object.keys(mapping).some((key) => !key.trim()) || Object.values(mapping).some((value) => typeof value !== 'string' || !value.includes('/'))) {
    throw new Error('Model aliases must map endpoint model IDs to manufacturer/model IDs')
  }
  if (!object(routes) || Object.keys(routes).some((key) => !key.trim()) || Object.values(routes).some((value) => !Array.isArray(value) || value.length === 0 || value.some((path) => typeof path !== 'string' || !/^llm\.[a-z0-9_.-]+$/.test(path)))) {
    throw new Error('Server default routes must map model IDs to explicit LLM task lists')
  }
  return { model_driver_overrides: mapping as Record<string, string>, server_default_routes: routes as Record<string, string[]> }
}
