import { useAIStatus } from './hooks/use-aicc-store'
import { EnableAIGuide } from './components/home/EnableAIGuide'
import { UsageDashboard } from './components/home/UsageDashboard'
import { AiccEventsCard } from './components/home/AiccEventsCard'
import { CreditHomeCard } from './components/home/CreditHomeCard'
import { UsecaseHealthCard } from './components/home/UsecaseHealthCard'
import type { AICenterPage } from './components/layout/Sidebar'
import type { PolicyIntent } from './PoliciesPage'

interface HomePageProps {
  navigate: (page: AICenterPage) => void
  /** Usage breakdown rows (Provider, model, App) open the policy of that row. */
  onSetPolicy?: (intent: PolicyIntent) => void
  /** Open Usage filtered to calls served by an unavailable policy. */
  onOpenSwitches?: () => void
}

export function HomePage({ navigate, onSetPolicy, onOpenSwitches }: HomePageProps) {
  const status = useAIStatus()

  if (status.state === 'disabled') {
    return <EnableAIGuide onGetStarted={() => navigate('providers/add')} />
  }

  return (
    <div className="flex flex-col gap-6">
      <CreditHomeCard onOpenCredit={() => navigate('credit')} onOpenPolicies={() => navigate('policies')} />
      <UsecaseHealthCard onOpenUsecases={() => navigate('usecases')} onOpenProviders={() => navigate('providers')} onOpenSwitches={onOpenSwitches ?? (() => navigate('usage'))} />
      <AiccEventsCard onOpenRouting={() => navigate('routing')} />
      <UsageDashboard onSetPolicy={onSetPolicy} />
    </div>
  )
}
