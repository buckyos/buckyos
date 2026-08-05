/* ── AppService – app panel entry point ── */

import { AppServiceShell } from './components/layout/AppServiceShell'
import type { AppServiceNav } from './components/layout/navigation'
import { HomePage } from './pages/HomePage'
import { DetailPage } from './pages/DetailPage'
import { InstallWizard } from './pages/InstallWizard'
import {
  AppServiceStoreProvider,
  useAppServiceStore,
} from './hooks/use-app-service-store'
import {
  APP_INSTALLER_DIALOG_PATH,
  useSystemDialog,
} from '../../sysdlg'

function PageRouter({
  nav,
  onNavigate,
}: {
  nav: AppServiceNav
  onNavigate: (nav: AppServiceNav) => void
}) {
  const store = useAppServiceStore()
  const systemDialog = useSystemDialog()

  const openAppInstaller = async (taskId: string) => {
    const result = await systemDialog.open(APP_INSTALLER_DIALOG_PATH, {
      task_id: taskId,
    })
    if (!result) return

    switch (result.action) {
      case 'background':
        onNavigate({ page: 'home' })
        return
      case 'change-source':
        store.clearActiveTask()
        onNavigate({ page: 'install' })
        return
      case 'view-app':
        store.clearActiveTask()
        onNavigate({ page: 'detail', serviceId: result.serviceId })
        return
      case 'close':
        store.clearActiveTask()
        onNavigate({ page: 'home' })
    }
  }

  switch (nav.page) {
    case 'detail':
      return nav.serviceId ? (
        <DetailPage serviceId={nav.serviceId} onNavigate={onNavigate} />
      ) : (
        <HomePage onNavigate={onNavigate} onOpenInstaller={openAppInstaller} />
      )
    case 'install':
      return <InstallWizard onNavigate={onNavigate} onOpenInstaller={openAppInstaller} />
    case 'home':
    default:
      return <HomePage onNavigate={onNavigate} onOpenInstaller={openAppInstaller} />
  }
}

export function AppServiceAppPanel() {
  return (
    <AppServiceStoreProvider>
      <AppServiceShell>
        {(nav, navigate) => <PageRouter nav={nav} onNavigate={navigate} />}
      </AppServiceShell>
    </AppServiceStoreProvider>
  )
}
