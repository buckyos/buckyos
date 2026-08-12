import { Settings, Palette, Network, Shield, Code } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import type { SettingsPage } from './navigation'

const tabs = [
  { key: 'general' as const, icon: Settings, labelKey: 'settings.nav.general', label: 'General' },
  { key: 'appearance' as const, icon: Palette, labelKey: 'settings.nav.appearance', label: 'Appearance' },
  { key: 'cluster' as const, icon: Network, labelKey: 'settings.nav.cluster', label: 'Cluster' },
  { key: 'privacy' as const, icon: Shield, labelKey: 'settings.nav.privacy', label: 'Privacy' },
  { key: 'developer' as const, icon: Code, labelKey: 'settings.nav.developer', label: 'Dev' },
]

interface MobileTabBarProps {
  currentPage: SettingsPage
  onNavigate: (page: SettingsPage) => void
}

export function MobileTabBar({ currentPage, onNavigate }: MobileTabBarProps) {
  const { t } = useI18n()

  return (
    <nav
      className="flex shrink-0 overflow-x-auto"
      style={{ borderBottom: '1px solid var(--cp-border)' }}
    >
      {tabs.map((tab) => {
        const active = currentPage === tab.key
        return (
          <button
            key={tab.key}
            type="button"
            onClick={() => onNavigate(tab.key)}
            className="flex flex-col items-center gap-1 px-3 py-2 min-w-[64px] text-xs transition-colors"
            style={{
              color: active ? 'var(--cp-accent)' : 'var(--cp-muted)',
              borderBottom: active ? '2px solid var(--cp-accent)' : '2px solid transparent',
            }}
          >
            <tab.icon size={18} />
            <span>{t(tab.labelKey, tab.label)}</span>
          </button>
        )
      })}
    </nav>
  )
}
