import { useState } from 'react'
import type { AppContentLoaderProps } from '../types'
import { MockStoreContext } from './hooks/use-mock-store'
import { MockDataStore } from './mock/store'
import { AICenterShell } from './components/layout/AICenterShell'
import type { AICenterPage } from './components/layout/Sidebar'
import { HomePage } from './HomePage'
import { ProvidersPage } from './ProvidersPage'
import { AddProviderPage } from './AddProviderPage'
import { ModelsPage } from './ModelsPage'
import { RoutingPage } from './RoutingPage'

function PageRouter({ page, navigate }: { page: AICenterPage; navigate: (p: AICenterPage) => void }) {
  switch (page) {
    case 'home':
      return <HomePage navigate={navigate} />
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

export function AICenterAppPanel(_props: AppContentLoaderProps) {
  const [store] = useState(() => new MockDataStore())

  return (
    <MockStoreContext.Provider value={store}>
      <AICenterShell>
        {(page, navigate) => <PageRouter page={page} navigate={navigate} />}
      </AICenterShell>
    </MockStoreContext.Provider>
  )
}
