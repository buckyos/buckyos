import { useI18n } from '../../../../i18n/provider'
import { useNavItems, type AICenterPage } from './nav'

export type { AICenterPage }

interface SidebarProps {
  currentPage: AICenterPage
  onNavigate: (page: AICenterPage) => void
}

export function Sidebar({ currentPage, onNavigate }: SidebarProps) {
  const { t } = useI18n()
  const items = useNavItems()

  const activePage = currentPage === 'providers/add' ? 'providers' : currentPage

  return (
    <nav
      className="flex flex-col w-60 shrink-0 h-full py-3 overflow-y-auto"
      style={{ borderRight: '1px solid var(--cp-border)' }}
    >
      <div
        className="px-5 pb-3 mb-1 text-sm font-semibold"
        style={{ color: 'var(--cp-text)' }}
      >
        {t('aiCenter.title', 'AI Center')}
      </div>
      {items.map((item) => {
        const active = activePage === item.key
        return (
          <button
            key={item.key}
            type="button"
            onClick={() => onNavigate(item.key)}
            className="flex items-center gap-3 px-5 py-2 mx-2 rounded-lg text-sm transition-colors"
            style={{
              background: active ? 'var(--cp-surface-2)' : 'transparent',
              color: active ? 'var(--cp-text)' : 'var(--cp-muted)',
              borderLeft: active ? '2px solid var(--cp-accent)' : '2px solid transparent',
            }}
          >
            <item.icon size={18} />
            <span>{t(item.labelKey, item.label)}</span>
          </button>
        )
      })}
    </nav>
  )
}
