import { useI18n } from '../../../../i18n/provider'
import { useNavItems, type AICenterPage } from './nav'

interface MobileTabBarProps {
  currentPage: AICenterPage
  onNavigate: (page: AICenterPage) => void
}

export function MobileTabBar({ currentPage, onNavigate }: MobileTabBarProps) {
  const { t } = useI18n()
  const tabs = useNavItems()
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
