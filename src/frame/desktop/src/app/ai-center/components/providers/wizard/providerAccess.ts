import type { PolicyApi } from '../../../../../api/aicc_policy'
import { isAppSide, normalizeBody, policyOf, withProvider } from '../../../datamodel/policy'

/** Who may use a new Provider and whether it is free, written as policies once it exists. */
export interface ProviderAccess { openTo: string[]; free: boolean }

export const NO_PROVIDER_ACCESS: ProviderAccess = { openTo: [], free: false }

/** Open the Provider (all models, following its list) to the chosen user side layers and, for a free Provider,
 * set its coefficient to 0 on every layer that sees it or sets a coefficient — so no layer charges for it. */
export async function applyProviderAccess(api: PolicyApi, provider: string, access: ProviderAccess) {
  if (access.openTo.length === 0 && !access.free) return
  const policies = await api.listPolicies('manage')
  const subjects = new Set(access.openTo)
  if (access.free) {
    for (const view of policies) {
      const body = view.policy.body
      if (view.policy.source === 'admin' && !isAppSide(view.policy.subject) && (body.all_providers || body.price_coefficient !== undefined || body.providers?.[provider])) {
        subjects.add(view.policy.subject)
      }
    }
  }
  for (const subject of subjects) {
    const view = policyOf(policies, subject, 'admin')
    let body = view?.policy.body ?? {}
    if (access.openTo.includes(subject) && !body.all_providers) body = withProvider(body, provider, { visible: 'all_models' })
    if (access.free) body = withProvider(body, provider, { price_coefficient: 0 })
    await api.upsertPolicy({ source: 'admin', subject, enabled: view?.policy.enabled ?? true, body: normalizeBody(body) }, view?.policy.version)
  }
}
