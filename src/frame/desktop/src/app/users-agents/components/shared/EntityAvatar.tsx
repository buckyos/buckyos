/* ── Entity avatar with type indicator ── */

import { Bot, CircleUser, Contact, Users2 } from 'lucide-react'
import type { EntityKind } from '../../mock/types'

const kindIcon: Record<EntityKind, typeof Bot> = {
  self: CircleUser,
  agent: Bot,
  'local-user': CircleUser,
  contact: Contact,
  'entity-group': Users2,
}

const kindColor: Record<EntityKind, string> = {
  self: 'var(--cp-accent)',
  agent: 'var(--cp-success)',
  'local-user': 'var(--cp-accent)',
  contact: 'var(--cp-muted)',
  'entity-group': 'var(--cp-warning)',
}

interface EntityAvatarProps {
  name: string
  kind: EntityKind
  avatarUrl?: string
  size?: 'sm' | 'md' | 'lg'
  isOnline?: boolean
}

const sizeMap = { sm: 32, md: 40, lg: 56 }
const iconSizeMap = { sm: 16, md: 20, lg: 28 }
const textSizeMap = { sm: 'text-xs', md: 'text-sm', lg: 'text-xl' }

export function EntityAvatar({ name, kind, avatarUrl, size = 'md', isOnline }: EntityAvatarProps) {
  const px = sizeMap[size]
  const iconPx = iconSizeMap[size]
  const initial = name.charAt(0).toUpperCase()
  const color = kindColor[kind]
  const Icon = kindIcon[kind]

  return (
    <div className="relative shrink-0" style={{ width: px, height: px }}>
      {avatarUrl ? (
        <img
          src={avatarUrl}
          alt={name}
          className="rounded-full object-cover"
          style={{ width: px, height: px }}
        />
      ) : (
        <div
          className={`flex items-center justify-center rounded-full font-display font-semibold ${textSizeMap[size]}`}
          style={{
            width: px,
            height: px,
            background: `color-mix(in srgb, ${color} 16%, var(--cp-surface))`,
            color,
          }}
        >
          {kind === 'agent' || kind === 'entity-group' ? (
            <Icon size={iconPx} />
          ) : (
            initial
          )}
        </div>
      )}

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
