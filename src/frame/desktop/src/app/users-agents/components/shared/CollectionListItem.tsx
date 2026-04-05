/* ── Single-line dense list item for collection elements ── */

import { EntityAvatar } from './EntityAvatar'
import type { AnyEntity } from '../../mock/types'

interface CollectionListItemProps {
  entity: AnyEntity
  isActive: boolean
  onClick: () => void
}

function getSubtitle(entity: AnyEntity): string {
  switch (entity.kind) {
    case 'contact':
      return entity.sourceLabel ?? entity.source
    case 'entity-group':
      return `${entity.memberCount} members`
    case 'local-user':
      return entity.role
    case 'agent':
      return entity.agentType
    case 'self':
      return 'Self'
    default:
      return ''
  }
}

export function CollectionListItem({ entity, isActive, onClick }: CollectionListItemProps) {
  const subtitle = getSubtitle(entity)
  const isOnline =
    entity.kind === 'local-user' ? entity.isOnline :
    entity.kind === 'agent' ? entity.status === 'running' :
    undefined

  return (
    <button
      type="button"
      onClick={onClick}
      className="w-full flex items-center gap-2.5 px-3 py-2 text-left transition-colors duration-100"
      style={{
        background: isActive
          ? 'color-mix(in srgb, var(--cp-accent) 10%, var(--cp-surface))'
          : 'transparent',
        borderLeft: isActive
          ? '2px solid var(--cp-accent)'
          : '2px solid transparent',
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
        <span
          className="truncate text-sm block"
          style={{ color: 'var(--cp-text)' }}
        >
          {entity.displayName}
        </span>
      </div>

      {subtitle && (
        <span
          className="shrink-0 text-[11px] truncate max-w-[80px]"
          style={{ color: 'var(--cp-muted)' }}
        >
          {subtitle}
        </span>
      )}

      {entity.kind === 'contact' && entity.isVerified && (
        <span
          className="shrink-0 rounded-full"
          style={{
            width: 6,
            height: 6,
            background: 'var(--cp-success)',
          }}
        />
      )}
    </button>
  )
}
