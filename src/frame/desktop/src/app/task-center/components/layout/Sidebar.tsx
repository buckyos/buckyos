/* ── TaskCenter sidebar ── */

import { Home, ListTodo, Activity } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import type { TaskCenterPage } from './navigation'

const navItems: { key: TaskCenterPage; labelKey: string; fallback: string; icon: typeof Home }[] = [
  { key: 'home', labelKey: 'taskCenter.nav.home', fallback: 'Home', icon: Home },
  { key: 'tasks', labelKey: 'taskCenter.nav.tasks', fallback: 'Tasks', icon: ListTodo },
  { key: 'events', labelKey: 'taskCenter.nav.events', fallback: 'System Events', icon: Activity },
]

interface SidebarProps {
  currentPage: TaskCenterPage
  onNavigate: (page: TaskCenterPage) => void
}

export function Sidebar({ currentPage, onNavigate }: SidebarProps) {
  const { t } = useI18n()

  return (
    <nav
      className="flex h-full w-52 shrink-0 flex-col overflow-y-auto px-2.5 py-4"
      style={{ borderRight: '1px solid var(--cp-border)' }}
    >
      <div className="space-y-1">
        {navItems.map((item) => {
          const active = currentPage === item.key
          return (
            <button
              key={item.key}
              type="button"
              onClick={() => onNavigate(item.key)}
              className="flex w-full items-center gap-3 rounded-[18px] px-4 py-3 text-left text-sm transition-colors"
              style={{
                background: active
                  ? 'color-mix(in srgb, var(--cp-accent-soft) 14%, var(--cp-surface-2))'
                  : 'transparent',
                color: active ? 'var(--cp-text)' : 'var(--cp-muted)',
                border: active
                  ? '1px solid color-mix(in srgb, var(--cp-accent) 22%, var(--cp-border))'
                  : '1px solid transparent',
              }}
            >
              <div
                className="flex h-9 w-9 items-center justify-center rounded-[14px]"
                style={{
                  background: active
                    ? 'color-mix(in srgb, var(--cp-accent) 16%, transparent)'
                    : 'color-mix(in srgb, var(--cp-surface) 84%, transparent)',
                  color: active ? 'var(--cp-accent)' : 'var(--cp-muted)',
                }}
              >
                <item.icon size={16} />
              </div>
              <span className="font-medium">{t(item.labelKey, item.fallback)}</span>
            </button>
          )
        })}
      </div>
    </nav>
  )
}
