import { useI18n } from '../../../../i18n/provider'
import { NAV_ITEMS, NAV_LABELS, type TaskCenterPage } from './navigation'

interface MobileTabBarProps {
  currentPage: TaskCenterPage
  onNavigate: (page: TaskCenterPage) => void
}

export function MobileTabBar({ currentPage, onNavigate }: MobileTabBarProps) {
  const { t } = useI18n()

  return (
    <nav className="tc-tabs">
      {NAV_ITEMS.map((tab) => (
        <button
          key={tab.key}
          type="button"
          onClick={() => onNavigate(tab.key)}
          className="tc-tab"
          aria-current={currentPage === tab.key ? 'page' : undefined}
        >
          <tab.icon size={18} strokeWidth={1.75} />
          <span>{t(NAV_LABELS[tab.key].key, NAV_LABELS[tab.key].fallback)}</span>
        </button>
      ))}
    </nav>
  )
}
