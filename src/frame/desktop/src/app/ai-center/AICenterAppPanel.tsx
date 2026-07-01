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

function PageRouter({ page, navigate }: { page: AICenterPage; navigate: (p: AICenterPage) => void }) {
  switch (page) {
    case 'home':
      return <HomePage navigate={navigate} />
    case 'usage':
      return <UsagePage />
    case 'providers':
      return <ProvidersPage navigate={navigate} />
    case 'providers/add':
      return <AddProviderPage navigate={navigate} />
    case 'models':
      return <ModelsPage />
    case 'routing':
      return <RoutingPage />
    default:
      return <HomePage navigate={navigate} />
  }
}

export function AICenterAppPanel() {
  const [store] = useState(() => createAICCMgr())

  useEffect(() => {
    void store.refresh()
  }, [store])

  return (
    <AICCStoreContext.Provider value={store}>
      <AICenterShell>
        {(page, navigate) => <PageRouter page={page} navigate={navigate} />}
      </AICenterShell>
    </AICCStoreContext.Provider>
  )
}
