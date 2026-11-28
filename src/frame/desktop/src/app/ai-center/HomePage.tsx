import { useAIStatus } from './hooks/use-aicc-store'
import { EnableAIGuide } from './components/home/EnableAIGuide'
import { UsageDashboard } from './components/home/UsageDashboard'
import { AiccEventsCard } from './components/home/AiccEventsCard'
import { CreditHomeCard } from './components/home/CreditHomeCard'
import type { AICenterPage } from './components/layout/Sidebar'
import type { PolicyIntent } from './PoliciesPage'

interface HomePageProps {
  navigate: (page: AICenterPage) => void
  /** Usage breakdown rows (Provider, model, App) open the policy of that row. */
  onSetPolicy?: (intent: PolicyIntent) => void
}

export function HomePage({ navigate, onSetPolicy }: HomePageProps) {
  const status = useAIStatus()

  if (status.state === 'disabled') {
    return <EnableAIGuide onGetStarted={() => navigate('providers/add')} />
  }

  return (
    <div className="flex flex-col gap-6">
      <CreditHomeCard onOpenCredit={() => navigate('credit')} onOpenPolicies={() => navigate('policies')} />
      <AiccEventsCard onOpenRouting={() => navigate('routing')} />
      <UsageDashboard onSetPolicy={onSetPolicy} />
    </div>
  )
}
