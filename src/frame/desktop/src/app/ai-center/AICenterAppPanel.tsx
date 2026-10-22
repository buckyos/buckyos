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
import { PlaygroundPage } from './PlaygroundPage'
import { QuotaPage } from './QuotaPage'
import { VisibilityPage } from './VisibilityPage'
import type { QuotaRuleDraft } from './datamodel/quota'

interface QuotaIntent {
  intent: Partial<QuotaRuleDraft> | null
  setIntent: (intent: Partial<QuotaRuleDraft> | null) => void
}

function PageRouter({ page, navigate, quota }: { page: AICenterPage; navigate: (p: AICenterPage) => void; quota: QuotaIntent }) {
  const setQuota = (draft: Partial<QuotaRuleDraft>) => {
    quota.setIntent(draft)
    navigate('quota')
  }
  switch (page) {
    case 'home':
      return <HomePage navigate={navigate} onSetQuota={setQuota} />
    case 'usage':
      return <UsagePage />
    case 'quota':
      return <QuotaPage intent={quota.intent} onIntentConsumed={() => quota.setIntent(null)} />
    case 'visibility':
      return <VisibilityPage />
    case 'providers':
      return <ProvidersPage navigate={navigate} />
    case 'providers/add':
      return <AddProviderPage navigate={navigate} />
    case 'models':
      return <ModelsPage />
    case 'playground':
      return null
    case 'routing':
      return <RoutingPage />
    default:
      return <HomePage navigate={navigate} onSetQuota={setQuota} />
  }
}

export function AICenterAppPanel() {
  const [store] = useState(() => createAICCMgr())
  const [quotaIntent, setQuotaIntent] = useState<Partial<QuotaRuleDraft> | null>(null)

  useEffect(() => {
    void store.refresh()
  }, [store])

  return (
    <AICCStoreContext.Provider value={store}>
      <AICenterShell>
        {(page, navigate) => <>
          <PageRouter page={page} navigate={navigate} quota={{ intent: quotaIntent, setIntent: setQuotaIntent }} />
          <div hidden={page !== 'playground'}><PlaygroundPage active={page === 'playground'} /></div>
        </>}
      </AICenterShell>
    </AICCStoreContext.Provider>
  )
}
