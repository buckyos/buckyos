import type { WizardDraft } from '../../../../../api/aicc_mgr'

export function isConnectionValid(draft: WizardDraft): boolean {
  if (!draft.provider_type) return false
  if (draft.provider_type === 'sn_router') return true
  if (!draft.api_key.trim()) return false
  if (draft.provider_type === 'custom') {
    if (!draft.endpoint.trim()) return false
    if (!draft.protocol_type) return false
  }
  return true
}
