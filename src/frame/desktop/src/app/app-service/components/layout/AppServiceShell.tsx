/* ── App Service shell – responsive layout ── */

import { useCallback, useState } from 'react'
import { useMediaQuery } from '@mui/material'
import { useMobileBackHandler } from '../../../../desktop/windows/MobileNavContext'
import { useAppPath } from '../../../pathContext'
import { appPath } from '../../../standalone/address'
import type { AppServiceNav } from './navigation'

interface AppServiceShellProps {
  initialNav?: AppServiceNav
  children: (nav: AppServiceNav, navigate: (nav: AppServiceNav) => void) => React.ReactNode
}

export function AppServiceShell({ initialNav = { page: 'home' }, children }: AppServiceShellProps) {
  const [currentNav, setCurrentNav] = useState<AppServiceNav>(initialNav)
  const isMobile = useMediaQuery('(max-width: 767px)')

  useAppPath(currentNav.page === 'install' ? appPath('install') : appPath('', { service: currentNav.page === 'detail' ? currentNav.serviceId : undefined }))

  const handleBack = useCallback(() => setCurrentNav({ page: 'home' }), [])

  useMobileBackHandler(
    isMobile && currentNav.page !== 'home' ? handleBack : null,
  )

  return (
    <div
      className="relative flex h-full w-full min-w-0 flex-col"
      data-testid="app-service-root"
      style={{ background: 'var(--cp-bg)' }}
    >
      <main className="flex-1 overflow-y-auto desktop-scrollbar">
        <div className={isMobile ? 'px-4 pb-8 pt-3' : 'mx-auto max-w-6xl px-7 py-7'}>
          {children(currentNav, setCurrentNav)}
        </div>
      </main>
    </div>
  )
}
