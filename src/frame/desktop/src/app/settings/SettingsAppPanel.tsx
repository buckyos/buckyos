import type { AppContentLoaderProps } from '../types'
import { SettingsStoreContext } from './hooks/use-settings-store'
import { globalSettingsStore } from './data/store'
import { SettingsShell } from './components/layout/SettingsShell'
import type { SettingsPage } from './components/layout/navigation'
import { GeneralPage } from './pages/GeneralPage'
import { AppearancePage } from './pages/AppearancePage'
import { ClusterManagerPage } from './pages/ClusterManagerPage'
import { PrivacyPage } from './pages/PrivacyPage'
import { DeveloperModePage } from './pages/DeveloperModePage'
import { ZonePage } from './pages/ZonePage'

function PageRouter({ page, appProps }: { page: SettingsPage; appProps: AppContentLoaderProps }) {
  switch (page) {
    case 'general':
      return <GeneralPage />
    case 'appearance':
      return <AppearancePage appProps={appProps} />
    case 'zone':
      return <ZonePage />
    case 'cluster':
      return <ClusterManagerPage />
    case 'privacy':
      return <PrivacyPage />
    case 'developer':
      return <DeveloperModePage />
    default:
      return <GeneralPage />
  }
}

export function SettingsAppPanel(props: AppContentLoaderProps) {
  return (
    <SettingsStoreContext.Provider value={globalSettingsStore}>
      <SettingsShell>
        {(page) => <PageRouter page={page} appProps={props} />}
      </SettingsShell>
    </SettingsStoreContext.Provider>
  )
}
