/* ── Sidebar entry of the system user groups page (admins only) ── */

import { ChevronRight, UsersRound } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { useUserGroups, useViewerIsAdmin } from '../../hooks/use-users-agents-store'

export function UserGroupsNavItem({ isActive, onClick }: { isActive: boolean; onClick: () => void }) {
  const { t } = useI18n()
  const isAdmin = useViewerIsAdmin()
  const userGroups = useUserGroups()
  if (!isAdmin) return null

  return (
    <div className="mt-3 pt-2" style={{ borderTop: '1px solid color-mix(in srgb, var(--cp-border) 50%, transparent)' }}>
      <button
        type="button"
        onClick={onClick}
        aria-current={isActive ? 'page' : undefined}
        data-testid="users-agents-user-groups"
        className="w-full flex items-center gap-3 px-3 py-2.5 rounded-[16px] text-left transition-all duration-150"
        style={{
          background: isActive ? 'color-mix(in srgb, var(--cp-accent-soft) 18%, var(--cp-surface))' : 'transparent',
          border: isActive ? '1px solid color-mix(in srgb, var(--cp-accent) 24%, var(--cp-border))' : '1px solid transparent',
        }}
      >
        <span
          className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full"
          style={{ background: 'color-mix(in srgb, var(--cp-accent) 12%, var(--cp-surface))', color: 'var(--cp-accent)' }}
        >
          <UsersRound size={16} />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate text-sm font-medium" style={{ color: 'var(--cp-text)' }}>
            {t('usersAgents.userGroups.nav')}
          </span>
          <span className="block truncate text-[11px]" style={{ color: 'var(--cp-muted)' }}>
            {userGroups ? t('usersAgents.userGroups.navHint', undefined, { count: userGroups.groups.length }) : t('usersAgents.userGroups.navUnavailable')}
          </span>
        </span>
        <ChevronRight size={14} style={{ color: 'var(--cp-muted)' }} />
      </button>
    </div>
  )
}
