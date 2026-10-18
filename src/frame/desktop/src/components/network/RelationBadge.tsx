/* ── RelationBadge: the §3.3 relation display of one person ── */

import { useI18n } from '../../i18n/provider'
import { Pill } from './Person'
import { relationBadges, type RelationBadgeKind, type RelationFacts } from './relation'

const TONE: Record<RelationBadgeKind, string> = {
  blocked: 'var(--cp-danger)',
  friend: 'var(--cp-success)',
  outgoing_pending: 'var(--cp-accent)',
  incoming_pending: 'var(--cp-accent)',
  following: 'var(--cp-accent)',
  contact: 'var(--cp-muted)',
  message_request: 'var(--cp-warning)',
  stranger: 'var(--cp-muted)',
  temporary: 'var(--cp-warning)',
}

/** `compact` shows only the leading state (lists); the full set is shown in cards and details. */
export function RelationBadge({ relation, compact = false }: { relation: RelationFacts; compact?: boolean }) {
  const { t } = useI18n()
  const badges = relationBadges(relation)
  const shown = compact ? badges.filter((badge, index) => index === 0 || badge === 'following') : badges
  return (
    <span className="inline-flex flex-wrap items-center gap-1" data-testid="relation-badge">
      {shown.map(badge => (
        <Pill key={badge} tone={TONE[badge]} testId={`relation-${badge}`}>
          {badge === 'temporary' && relation.temporaryExpiresAtMs
            ? t('network.relation.temporaryUntil', undefined, { date: new Date(relation.temporaryExpiresAtMs).toLocaleString() })
            : t(`network.relation.${badge}`)}
        </Pill>
      ))}
    </span>
  )
}
