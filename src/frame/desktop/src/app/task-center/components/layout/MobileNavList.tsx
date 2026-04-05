/* ── TaskCenter mobile navigation list ── */

import { Home, ListTodo, Activity } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import type { TaskCenterPage } from './navigation'

const navItems: { key: TaskCenterPage; labelKey: string; fallback: string; icon: typeof Home }[] = [
  { key: 'home', labelKey: 'taskCenter.nav.home', fallback: 'Home', icon: Home },
  { key: 'tasks', labelKey: 'taskCenter.nav.tasks', fallback: 'Tasks', icon: ListTodo },
  { key: 'events', labelKey: 'taskCenter.nav.events', fallback: 'System Events', icon: Activity },
]

interface MobileNavListProps {
  onNavigate: (page: TaskCenterPage) => void
}

export function MobileNavList({ onNavigate }: MobileNavListProps) {
  const { t } = useI18n()

  return (
    <div className="px-4 py-4 space-y-1">
      {navItems.map((item) => (
        <button
          key={item.key}
          type="button"
          onClick={() => onNavigate(item.key)}
          className="flex w-full items-center gap-3 rounded-[18px] px-4 py-3 text-left text-sm transition-colors"
          style={{
            background: 'transparent',
            color: 'var(--cp-muted)',
            border: '1px solid transparent',
          }}
        >
          <div
            className="flex h-9 w-9 items-center justify-center rounded-[14px]"
            style={{
              background: 'color-mix(in srgb, var(--cp-surface) 84%, transparent)',
              color: 'var(--cp-muted)',
            }}
          >
            <item.icon size={16} />
          </div>
          <span className="font-medium">{t(item.labelKey, item.fallback)}</span>
        </button>
      ))}
    </div>
  )
}
