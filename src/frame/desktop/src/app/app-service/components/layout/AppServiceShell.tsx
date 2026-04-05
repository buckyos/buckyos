/* ── App Service shell – responsive layout ── */

import { useCallback, useState } from 'react'
import { useMediaQuery } from '@mui/material'
import { useMobileBackHandler } from '../../../../desktop/windows/MobileNavContext'
import type { AppServiceNav } from './navigation'

interface AppServiceShellProps {
  children: (nav: AppServiceNav, navigate: (nav: AppServiceNav) => void) => React.ReactNode
}

export function AppServiceShell({ children }: AppServiceShellProps) {
  const [currentNav, setCurrentNav] = useState<AppServiceNav>({ page: 'home' })
  const isMobile = useMediaQuery('(max-width: 767px)')

  const handleBack = useCallback(() => setCurrentNav({ page: 'home' }), [])

  useMobileBackHandler(
    isMobile && currentNav.page !== 'home' ? handleBack : null,
  )

  return (
    <div className="flex flex-col h-full w-full" style={{ background: 'var(--cp-bg)' }}>
      <main className="flex-1 overflow-y-auto desktop-scrollbar">
        <div className={isMobile ? 'px-4 pb-5 pt-2' : 'px-6 py-5 mx-auto max-w-5xl'}>
          {children(currentNav, setCurrentNav)}
        </div>
      </main>
    </div>
  )
}
