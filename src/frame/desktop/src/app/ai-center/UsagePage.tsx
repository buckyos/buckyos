import { UsageDashboard } from './components/home/UsageDashboard'
import type { PolicyIntent } from './PoliciesPage'

/** `fallbackOnly` opens the usage detail filtered to calls served by an unavailable policy (from Home). */
export function UsagePage({ onSetPolicy, fallbackOnly = false, onIntentConsumed }: { onSetPolicy?: (intent: PolicyIntent) => void; fallbackOnly?: boolean; onIntentConsumed?: () => void }) {
  return <UsageDashboard mode="usage" onSetPolicy={onSetPolicy} initialFallbackOnly={fallbackOnly} onIntentConsumed={onIntentConsumed} />
}
