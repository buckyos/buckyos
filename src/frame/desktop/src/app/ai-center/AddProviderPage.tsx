import { WizardShell } from './components/providers/wizard/WizardShell'
import type { AICenterPage } from './components/layout/Sidebar'

interface AddProviderPageProps {
  navigate: (page: AICenterPage) => void
}

export function AddProviderPage({ navigate }: AddProviderPageProps) {
  return (
    <WizardShell
      onBack={() => navigate('providers')}
      onCreated={() => navigate('home')}
    />
  )
}
