export interface ProviderModelSelectionDraft {
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
  if (!policyRegion && !hasSelection) return undefined
  const selected = new Set(selection ?? [])
  const catalogOnly = draft.discovery_behavior_id === 'catalog-only'
  return {
    policy_region: policyRegion || undefined,
    exclude_models: !catalogOnly && Array.isArray(selectable)
      ? selectable.filter((model) => !selected.has(model)).sort()
      : [],
    model_driver_overrides: {},
    enabled_inventory_models: catalogOnly && hasSelection ? [...selection].sort() : undefined,
  }
}
