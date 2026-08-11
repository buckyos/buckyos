import { Activity, CalendarClock, Home, ListTodo } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import type { TaskCenterNav, TaskCenterPage } from './navigation'

const tabs = [
  { key: 'home' as const, icon: Home, labelKey: 'taskCenter.nav.home', label: 'Home' },
  { key: 'tasks' as const, icon: ListTodo, labelKey: 'taskCenter.nav.tasks', label: 'Tasks' },
  { key: 'schedules' as const, icon: CalendarClock, labelKey: 'taskCenter.nav.schedules', label: 'Scheduled' },
  { key: 'events' as const, icon: Activity, labelKey: 'taskCenter.nav.events', label: 'System Events' },
]

interface MobileTabBarProps {
  currentNav: TaskCenterNav
  onNavigate: (page: TaskCenterPage) => void
}

export function MobileTabBar({ currentNav, onNavigate }: MobileTabBarProps) {
  const { t } = useI18n()

  return (
    <nav
      className="flex shrink-0 overflow-x-auto"
      style={{ borderBottom: '1px solid var(--cp-border)' }}
    >
      {tabs.map((tab) => {
        const active = currentNav.page === tab.key
        return (
          <button
            key={tab.key}
            type="button"
            onClick={() => onNavigate(tab.key)}
            className="flex min-w-[112px] flex-1 items-center justify-center gap-1.5 whitespace-nowrap px-2 py-2.5 text-sm transition-colors"
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
    </nav>
  )
}
