import { Home, Plug, HardDrive, GitFork } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import type { AICenterPage } from './Sidebar'

const tabs = [
  { key: 'home' as const, icon: Home, labelKey: 'aiCenter.nav.home', label: 'Home' },
  { key: 'providers' as const, icon: Plug, labelKey: 'aiCenter.nav.providers', label: 'Providers' },
  { key: 'models' as const, icon: HardDrive, labelKey: 'aiCenter.nav.models', label: 'Models' },
  { key: 'routing' as const, icon: GitFork, labelKey: 'aiCenter.nav.routing', label: 'Routing' },
]

interface MobileTabBarProps {
  currentPage: AICenterPage
  onNavigate: (page: AICenterPage) => void
}

export function MobileTabBar({ currentPage, onNavigate }: MobileTabBarProps) {
  const { t } = useI18n()
  const activePage = currentPage === 'providers/add' ? 'providers' : currentPage

  return (
    <div
      className="flex overflow-x-auto shrink-0"
      style={{ borderBottom: '1px solid var(--cp-border)' }}
    >
      {tabs.map((tab) => {
        const active = activePage === tab.key
        return (
          <button
            key={tab.key}
            type="button"
            onClick={() => onNavigate(tab.key)}
            className="flex flex-1 items-center justify-center gap-1.5 px-2 py-2.5 text-sm whitespace-nowrap transition-colors"
            style={{
              color: active ? 'var(--cp-accent)' : 'var(--cp-muted)',
              borderBottom: active ? '2px solid var(--cp-accent)' : '2px solid transparent',
            }}
          >
            <tab.icon size={16} />
            <span>{t(tab.labelKey, tab.label)}</span>
          </button>
        )
      })}
    </div>
  )
}
