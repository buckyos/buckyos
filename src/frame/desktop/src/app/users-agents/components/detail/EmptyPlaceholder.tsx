/* ── Empty state placeholder ── */

import { Users } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'

export function EmptyPlaceholder() {
  const { t } = useI18n()
  return (
    <div className="flex flex-col items-center justify-center h-full gap-3 px-8">
      <div
        className="flex items-center justify-center rounded-full"
        style={{
          width: 56,
          height: 56,
          background: 'color-mix(in srgb, var(--cp-accent-soft) 14%, var(--cp-surface))',
          color: 'var(--cp-accent)',
        }}
      >
        <Users size={24} />
      </div>
      <p
        className="font-display text-sm font-medium text-center"
        style={{ color: 'var(--cp-muted)' }}
      >
        {t('usersAgents.selectEntity')}
      </p>
    </div>
  )
}
