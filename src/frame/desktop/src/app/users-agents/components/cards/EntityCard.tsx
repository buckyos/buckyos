/* ── Entity card for sidebar ── */

import { EntityAvatar } from '../shared/EntityAvatar'
import type { AnyEntity } from '../../mock/types'

interface EntityCardProps {
  entity: AnyEntity
  isActive: boolean
  onClick: () => void
}

const kindLabel: Record<string, string> = {
  self: 'Self',
  agent: 'Agent',
  'local-user': 'User',
  'entity-group': 'Group',
}

export function EntityCard({ entity, isActive, onClick }: EntityCardProps) {
  const isOnline =
    entity.kind === 'local-user' ? entity.isOnline :
    entity.kind === 'agent' ? entity.status === 'running' :
    entity.kind === 'self' ? true :
    undefined

  return (
    <button
      type="button"
      onClick={onClick}
      className="w-full flex items-center gap-3 px-3 py-2.5 rounded-[16px] text-left transition-all duration-150"
      style={{
        background: isActive
          ? 'color-mix(in srgb, var(--cp-accent) 14%, var(--cp-surface))'
          : 'transparent',
        border: isActive
          ? '1px solid color-mix(in srgb, var(--cp-accent) 30%, transparent)'
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
          {kindLabel[entity.kind] ?? entity.kind}
          {entity.did ? ` · ${entity.did}` : ''}
        </div>
      </div>

      {entity.kind === 'agent' && (
        <span
          className="shrink-0 rounded-full px-1.5 py-0.5 text-[10px] font-semibold"
          style={{
            background: entity.status === 'running'
              ? 'color-mix(in srgb, var(--cp-success) 18%, transparent)'
              : 'color-mix(in srgb, var(--cp-muted) 18%, transparent)',
            color: entity.status === 'running' ? 'var(--cp-success)' : 'var(--cp-muted)',
          }}
        >
          {entity.status}
        </span>
      )}
    </button>
  )
}
