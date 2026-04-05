import { useAIStatus } from './hooks/use-mock-store'
import { EnableAIGuide } from './components/home/EnableAIGuide'
import { UsageDashboard } from './components/home/UsageDashboard'
import type { AICenterPage } from './components/layout/Sidebar'

interface HomePageProps {
  navigate: (page: AICenterPage) => void
}

export function HomePage({ navigate }: HomePageProps) {
  const status = useAIStatus()

  if (status.state === 'disabled') {
    return <EnableAIGuide onGetStarted={() => navigate('providers/add')} />
  }

  return <UsageDashboard />
}
