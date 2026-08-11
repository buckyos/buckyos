/* ── Single-line dense list item for collection elements ── */

import { EntityAvatar } from './EntityAvatar'
import type { MyNetworkEntity } from '../../../my-network/datamodel/types'
import { Checkbox } from '@mui/material'

interface CollectionListItemProps {
  entity: MyNetworkEntity
  isActive: boolean
  isSelected?: boolean
  onClick: () => void
  onToggleSelected?: () => void
}

function getSubtitle(entity: MyNetworkEntity): string {
  switch (entity.kind) {
    case 'contact':
      return entity.sourceLabel ?? entity.source
    case 'entity-group':
      return `${entity.memberCount} members`
    default:
      return ''
  }
}

export function CollectionListItem({
  entity,
  isActive,
  isSelected = false,
  onClick,
  onToggleSelected,
}: CollectionListItemProps) {
  const subtitle = getSubtitle(entity)
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
      {onToggleSelected && (
        <span
          onClick={(event) => {
            event.stopPropagation()
            onToggleSelected()
          }}
        >
          <Checkbox size="small" checked={isSelected} tabIndex={-1} />
        </span>
      )}

      <EntityAvatar
        name={entity.displayName}
        kind={entity.kind}
        avatarUrl={entity.avatarUrl}
        size="sm"
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

      {entity.kind === 'contact' && entity.isMergeCandidate && (
        <span
          className="shrink-0 rounded-full px-1.5 py-0.5 text-[10px] font-semibold"
          style={{
            background: 'color-mix(in srgb, var(--cp-warning) 16%, transparent)',
            color: 'var(--cp-warning)',
          }}
        >
          merge
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
