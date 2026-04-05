import { useCallback, useState } from 'react'
import { useMediaQuery } from '@mui/material'
import { Sidebar } from './Sidebar'
import { MobileSettingsList } from './MobileSettingsList'
import { useMobileBackHandler } from '../../../../desktop/windows/MobileNavContext'
import type { SettingsPage } from './navigation'

interface SettingsShellProps {
  children: (page: SettingsPage, navigate: (page: SettingsPage) => void) => React.ReactNode
}

export function SettingsShell({ children }: SettingsShellProps) {
  const [currentPage, setCurrentPage] = useState<SettingsPage | null>(null)
  const isMobile = useMediaQuery('(max-width: 767px)')

  // Mobile: null = show list, non-null = show detail page
  // Desktop: always show a page (default to 'general')
  const activePage = isMobile ? currentPage : (currentPage ?? 'general')

  const handleNavigate = (page: SettingsPage) => setCurrentPage(page)
  const handleBack = useCallback(() => setCurrentPage(null), [])

  // Register back handler with the shell status bar when on a detail page
  useMobileBackHandler(isMobile && activePage !== null ? handleBack : null)

  return (
    <div className="flex flex-col h-full w-full" style={{ background: 'var(--cp-bg)' }}>
      {isMobile ? (
        activePage === null ? (
          <div className="flex-1 overflow-y-auto">
            <MobileSettingsList onNavigate={handleNavigate} />
          </div>
        ) : (
          <main className="flex-1 overflow-y-auto">
            <div className="px-4 pb-5 pt-2">
              {children(activePage, handleNavigate)}
            </div>
          </main>
        )
      ) : (
        <div className="flex flex-1 min-h-0">
          <Sidebar currentPage={activePage!} onNavigate={handleNavigate} />
          <main
            className="flex-1 overflow-y-auto"
            style={{ background: 'var(--cp-bg)' }}
          >
            <div className="px-6 py-5 max-w-4xl">
              {children(activePage!, handleNavigate)}
            </div>
          </main>
        </div>
      )}
    </div>
  )
}
