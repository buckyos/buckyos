import { useState } from 'react'
import { useMediaQuery } from '@mui/material'
import { Sidebar } from './Sidebar'
import { MobileTabBar } from './MobileTabBar'
import type { AICenterPage } from './Sidebar'

interface AICenterShellProps {
  children: (page: AICenterPage, navigate: (page: AICenterPage) => void) => React.ReactNode
}

export function AICenterShell({ children }: AICenterShellProps) {
  const [currentPage, setCurrentPage] = useState<AICenterPage>('home')
  const isMobile = useMediaQuery('(max-width: 767px)')

  return (
    <div className="flex flex-col h-full w-full" style={{ background: 'var(--cp-bg)' }}>
      {isMobile && (
        <MobileTabBar currentPage={currentPage} onNavigate={setCurrentPage} />
      )}
      <div className="flex flex-1 min-h-0">
        {!isMobile && (
          <Sidebar currentPage={currentPage} onNavigate={setCurrentPage} />
        )}
        <main
          className="flex-1 overflow-y-auto"
          style={{ background: 'var(--cp-bg)' }}
        >
          <div className={isMobile ? 'px-4 py-4' : 'px-8 py-6 max-w-5xl mx-auto'}>
            {children(currentPage, setCurrentPage)}
          </div>
        </main>
      </div>
    </div>
  )
}
