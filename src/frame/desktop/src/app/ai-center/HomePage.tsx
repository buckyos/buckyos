import { useAIStatus } from './hooks/use-aicc-store'
import { EnableAIGuide } from './components/home/EnableAIGuide'
import { UsageDashboard } from './components/home/UsageDashboard'
import { AiccEventsCard } from './components/home/AiccEventsCard'
import { QuotaHomeCard } from './components/home/QuotaHomeCard'
import type { AICenterPage } from './components/layout/Sidebar'
import type { QuotaRuleDraft } from './datamodel/quota'

interface HomePageProps {
  navigate: (page: AICenterPage) => void
  /** Usage breakdown rows (Provider, model, App) open the quota editor for that row. */
  onSetQuota?: (draft: Partial<QuotaRuleDraft>) => void
}

export function HomePage({ navigate, onSetQuota }: HomePageProps) {
  const status = useAIStatus()

  if (status.state === 'disabled') {
    return <EnableAIGuide onGetStarted={() => navigate('providers/add')} />
  }

  return (
    <div className="flex flex-col gap-6">
      <QuotaHomeCard onOpen={() => navigate('quota')} />
      <AiccEventsCard onOpenRouting={() => navigate('routing')} />
      <UsageDashboard onSetQuota={onSetQuota} />
    </div>
  )
}
