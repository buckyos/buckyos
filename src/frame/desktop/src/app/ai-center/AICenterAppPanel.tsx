import { useEffect, useState } from 'react'
import { AICCStoreContext } from './hooks/use-aicc-store'
import { createAICCMgr } from '../../api/aicc_mgr'
import { AICenterShell } from './components/layout/AICenterShell'
import type { AICenterPage } from './components/layout/Sidebar'
import { HomePage } from './HomePage'
import { UsagePage } from './UsagePage'
import { ProvidersPage } from './ProvidersPage'
import { AddProviderPage } from './AddProviderPage'
import { ModelsPage } from './ModelsPage'
import { RoutingPage } from './RoutingPage'
import { UsecasesPage } from './UsecasesPage'
import { RoutingGroupsPage } from './RoutingGroupsPage'
import { PlaygroundPage } from './PlaygroundPage'
import { PoliciesPage, type PolicyIntent } from './PoliciesPage'
import { PricesPage } from './PricesPage'
import { CreditPage } from './CreditPage'
import { aiCenterLaunchSchema } from './launch'
import type { AppContentLoaderProps } from '../types'

interface PolicyIntentState {
  intent: PolicyIntent | null
  setIntent: (intent: PolicyIntent | null) => void
}

interface UsageIntentState {
  fallbackOnly: boolean
  setFallbackOnly: (value: boolean) => void
}

function PageRouter({ page, navigate, policy, usage }: { page: AICenterPage; navigate: (p: AICenterPage) => void; policy: PolicyIntentState; usage: UsageIntentState }) {
  const setPolicy = (intent: PolicyIntent) => {
    policy.setIntent(intent)
    navigate('policies')
  }
  const openSwitches = () => {
    usage.setFallbackOnly(true)
    navigate('usage')
  }
  switch (page) {
    case 'home':
      return <HomePage navigate={navigate} onSetPolicy={setPolicy} onOpenSwitches={openSwitches} />
    case 'usage':
      return <UsagePage onSetPolicy={setPolicy} fallbackOnly={usage.fallbackOnly} onIntentConsumed={() => usage.setFallbackOnly(false)} />
    case 'policies':
      return <PoliciesPage intent={policy.intent} onIntentConsumed={() => policy.setIntent(null)} />
    case 'prices':
      return <PricesPage />
    case 'credit':
      return <CreditPage />
    case 'providers':
      return <ProvidersPage navigate={navigate} />
    case 'providers/add':
      return <AddProviderPage navigate={navigate} />
    case 'models':
      return <ModelsPage navigate={navigate} />
    case 'playground':
      return null
    case 'usecases':
      return <UsecasesPage navigate={navigate} />
    case 'routing-groups':
      return <RoutingGroupsPage />
    case 'routing':
      return <RoutingPage />
    default:
      return <HomePage navigate={navigate} onSetPolicy={setPolicy} onOpenSwitches={openSwitches} />
  }
}

export function AICenterAppPanel({ launch }: AppContentLoaderProps) {
  const parsed = aiCenterLaunchSchema.safeParse(launch?.payload)
  const [store] = useState(() => createAICCMgr())
  const [policyIntent, setPolicyIntent] = useState<PolicyIntent | null>(null)
  const [fallbackOnly, setFallbackOnly] = useState(false)

  useEffect(() => {
    void store.refresh()
  }, [store])

  return (
    <AICCStoreContext.Provider value={store}>
      <AICenterShell key={launch?.requestId ?? 'default'} initialPage={parsed.success ? parsed.data.page : undefined}>
        {(page, navigate) => <>
          <PageRouter page={page} navigate={navigate} policy={{ intent: policyIntent, setIntent: setPolicyIntent }} usage={{ fallbackOnly, setFallbackOnly }} />
          <div hidden={page !== 'playground'}><PlaygroundPage active={page === 'playground'} /></div>
        </>}
      </AICenterShell>
    </AICCStoreContext.Provider>
  )
}
