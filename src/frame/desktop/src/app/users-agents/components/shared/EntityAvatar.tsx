/* ── Entity avatar with type indicator ── */

import { Bot, CircleUser, Contact, Users2 } from 'lucide-react'
import type { EntityKind } from '../../datamodel/types'
import { ObjectAvatar } from '../../../../presentation/ObjectIcon'

export type EntityAvatarKind = EntityKind | 'contact'

const kindIcon: Record<EntityAvatarKind, typeof Bot> = {
  self: CircleUser,
  agent: Bot,
  'local-user': CircleUser,
  contact: Contact,
  'entity-group': Users2,
}

const kindColor: Record<EntityAvatarKind, string> = {
  self: 'var(--cp-accent)',
  agent: 'var(--cp-success)',
  'local-user': 'var(--cp-accent)',
  contact: 'var(--cp-muted)',
  'entity-group': 'var(--cp-warning)',
}

interface EntityAvatarProps {
  name: string
  kind: EntityAvatarKind
  avatarUrl?: string
  size?: 'sm' | 'md' | 'lg'
  isOnline?: boolean
}

const sizeMap = { sm: 32, md: 40, lg: 56 }
const iconSizeMap = { sm: 16, md: 20, lg: 28 }

export function EntityAvatar({ name, kind, avatarUrl, size = 'md', isOnline }: EntityAvatarProps) {
  const px = sizeMap[size]
  const Icon = kindIcon[kind]

  return (
    <div className="relative shrink-0" style={{ width: px, height: px }}>
      <ObjectAvatar
        title={name}
        icons={[avatarUrl]}
        size={px}
        tone={kindColor[kind]}
        fallbackIcon={kind === 'agent' || kind === 'entity-group' ? <Icon size={iconSizeMap[size]} /> : undefined}
      />

      {/* online indicator */}
      {isOnline !== undefined && (
        <span
          className="absolute bottom-0 right-0 block rounded-full border-2"
          style={{
            width: size === 'sm' ? 8 : 10,
            height: size === 'sm' ? 8 : 10,
            background: isOnline ? 'var(--cp-success)' : 'var(--cp-muted)',
            borderColor: 'var(--cp-surface)',
          }}
        />
      )}
    </div>
  )
}
