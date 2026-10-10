/* ── Entity card for sidebar ── */

import { EntityAvatar } from '../shared/EntityAvatar'
import type { AnyEntity } from '../../datamodel/types'
import { useI18n } from '../../../../i18n/provider'
import { agentStatusLabelKey, agentStatusTone } from '../shared/agentLabels'

interface EntityCardProps {
  entity: AnyEntity
  isActive: boolean
  onClick: () => void
}

function getSubLabel(entity: AnyEntity, t: ReturnType<typeof useI18n>['t']) {
  if (entity.kind === 'self') {
    return entity.bio ?? t('usersAgents.role.owner')
  }
  if (entity.kind === 'agent') {
    return `@${entity.name} · ${t('usersAgents.agent.ownerShort', undefined, { owner: entity.ownerUserId })}`
  }
  if (entity.kind === 'local-user') {
    return `${entity.source === 'primary-did' ? 'BNS / DID' : t('usersAgents.source.local')} · ${t(`usersAgents.userStatus.${entity.status}`)}`
  }
  if (entity.kind === 'entity-group') {
    return `${t('usersAgents.group.members', undefined, { count: entity.memberCount })} · ${entity.isHostedBySelf ? t('usersAgents.group.selfHosted') : t('usersAgents.group.joined')}`
  }
  return ''
}

const toneColor = {
  success: 'var(--cp-success)',
  warning: 'var(--cp-warning)',
  danger: 'var(--cp-danger)',
  muted: 'var(--cp-muted)',
}

export function EntityCard({ entity, isActive, onClick }: EntityCardProps) {
  const { t } = useI18n()
  const isOnline =
    entity.kind === 'local-user' ? entity.isOnline :
    entity.kind === 'agent' ? entity.status === 'running' :
    entity.kind === 'self' ? true :
    undefined

  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={isActive ? 'page' : undefined}
      data-entity-id={entity.id}
      className="w-full flex items-center gap-3 px-3 py-2.5 rounded-[16px] text-left transition-all duration-150"
      style={{
        background: isActive
          ? 'color-mix(in srgb, var(--cp-accent-soft) 18%, var(--cp-surface))'
          : 'transparent',
        border: isActive
          ? '1px solid color-mix(in srgb, var(--cp-accent) 24%, var(--cp-border))'
          : '1px solid transparent',
      }}
    >
      <EntityAvatar
        name={entity.displayName}
        kind={entity.kind}
        avatarUrl={entity.avatarUrl}
        size="sm"
        isOnline={isOnline}
      />

      <div className="flex-1 min-w-0">
        <div
          className="truncate text-sm font-medium"
          style={{ color: 'var(--cp-text)' }}
        >
          {entity.displayName}
        </div>
        <div
          className="truncate text-[11px]"
          style={{ color: 'var(--cp-muted)' }}
        >
          {getSubLabel(entity, t)}
        </div>
      </div>

      {entity.kind === 'agent' && (
        <span
          className="shrink-0 rounded-full px-1.5 py-0.5 text-[10px] font-semibold"
          style={{
            background: `color-mix(in srgb, ${toneColor[agentStatusTone(entity.status)]} 18%, transparent)`,
            color: `color-mix(in srgb, ${toneColor[agentStatusTone(entity.status)]} 75%, var(--cp-text))`,
          }}
        >
          {t(agentStatusLabelKey(entity.status))}
        </span>
      )}
    </button>
  )
}
