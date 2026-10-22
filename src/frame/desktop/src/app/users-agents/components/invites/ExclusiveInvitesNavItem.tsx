/* ── Sidebar entry of the exclusive invites page (admins only) ── */

import { ChevronRight, KeyRound } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { useViewerIsAdmin } from '../../hooks/use-users-agents-store'

export function ExclusiveInvitesNavItem({ isActive, onClick }: { isActive: boolean; onClick: () => void }) {
  const { t } = useI18n()
  const isAdmin = useViewerIsAdmin()
  if (!isAdmin) return null

  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={isActive ? 'page' : undefined}
      data-testid="users-agents-exclusive-invites"
      className="mt-1 w-full flex items-center gap-3 px-3 py-2.5 rounded-[16px] text-left transition-all duration-150"
      style={{
        background: isActive ? 'color-mix(in srgb, var(--cp-accent-soft) 18%, var(--cp-surface))' : 'transparent',
        border: isActive ? '1px solid color-mix(in srgb, var(--cp-accent) 24%, var(--cp-border))' : '1px solid transparent',
      }}
    >
      <span
        className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full"
        style={{ background: 'color-mix(in srgb, var(--cp-accent) 12%, var(--cp-surface))', color: 'var(--cp-accent)' }}
      >
        <KeyRound size={16} />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium" style={{ color: 'var(--cp-text)' }}>
          {t('sso.invites.nav', 'Exclusive invites')}
        </span>
        <span className="block truncate text-[11px]" style={{ color: 'var(--cp-muted)' }}>
          {t('sso.invites.navHint', 'Password-free links for customers')}
        </span>
      </span>
      <ChevronRight size={14} style={{ color: 'var(--cp-muted)' }} />
    </button>
  )
}
