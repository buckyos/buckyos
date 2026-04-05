/* ── AppService – app panel entry point ── */

import { useState } from 'react'
import type { AppContentLoaderProps } from '../types'
import { AppServiceStoreContext } from './hooks/use-app-service-store'
import { AppServiceMockStore } from './mock/store'
import { AppServiceShell } from './components/layout/AppServiceShell'
import type { AppServiceNav } from './components/layout/navigation'
import { HomePage } from './pages/HomePage'
import { DetailPage } from './pages/DetailPage'
import { InstallWizard } from './pages/InstallWizard'

function PageRouter({
  nav,
  onNavigate,
}: {
  nav: AppServiceNav
  onNavigate: (nav: AppServiceNav) => void
}) {
  switch (nav.page) {
    case 'detail':
      return nav.serviceId ? (
        <DetailPage serviceId={nav.serviceId} onNavigate={onNavigate} />
      ) : (
        <HomePage onNavigate={onNavigate} />
      )
    case 'install':
      return <InstallWizard onNavigate={onNavigate} />
    case 'home':
    default:
      return <HomePage onNavigate={onNavigate} />
  }
}

export function AppServiceAppPanel(_props: AppContentLoaderProps) {
  const [store] = useState(() => new AppServiceMockStore())

  return (
    <AppServiceStoreContext.Provider value={store}>
      <AppServiceShell>
        {(nav, navigate) => <PageRouter nav={nav} onNavigate={navigate} />}
      </AppServiceShell>
    </AppServiceStoreContext.Provider>
  )
}
