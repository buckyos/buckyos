import type { SettingsPage } from './navigation'
import { SettingsNavigationPanel } from './SettingsNavigationPanel'

interface SidebarProps {
  currentPage: SettingsPage
  onNavigate: (page: SettingsPage) => void
}

export function Sidebar({ currentPage, onNavigate }: SidebarProps) {
  return (
    <nav
      className="flex h-full w-52 shrink-0 flex-col overflow-y-auto px-2.5 py-4"
      style={{ borderRight: '1px solid var(--cp-border)' }}
    >
      <SettingsNavigationPanel currentPage={currentPage} onNavigate={onNavigate} />
    </nav>
  )
}
