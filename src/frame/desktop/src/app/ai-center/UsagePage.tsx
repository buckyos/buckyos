import { UsageDashboard } from './components/home/UsageDashboard'
import type { PolicyIntent } from './PoliciesPage'

export function UsagePage({ onSetPolicy }: { onSetPolicy?: (intent: PolicyIntent) => void }) {
  return <UsageDashboard mode="usage" onSetPolicy={onSetPolicy} />
}
