/* ── TaskCenter sidebar ── */

import { useI18n } from '../../../../i18n/provider'
import { NAV_ITEMS, NAV_LABELS, type TaskCenterPage } from './navigation'

interface SidebarProps {
  currentPage: TaskCenterPage
  onNavigate: (page: TaskCenterPage) => void
}

export function Sidebar({ currentPage, onNavigate }: SidebarProps) {
  const { t } = useI18n()

  return (
    <nav
      className="flex h-full w-[200px] shrink-0 flex-col gap-1 overflow-y-auto px-2 py-3"
      style={{ borderRight: '1px solid var(--cp-border)' }}
    >
      {NAV_ITEMS.map((item) => (
        <button
          key={item.key}
          type="button"
          onClick={() => onNavigate(item.key)}
          className="tc-nav-item"
          aria-current={currentPage === item.key ? 'page' : undefined}
        >
          <item.icon size={16} strokeWidth={1.75} className="shrink-0" />
          <span>{t(NAV_LABELS[item.key].key, NAV_LABELS[item.key].fallback)}</span>
        </button>
      ))}
    </nav>
  )
}
