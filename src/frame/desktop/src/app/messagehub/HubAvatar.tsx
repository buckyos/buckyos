/* ── MessageHub avatars: the identity's own picture, the entity type's glyph as the default ── */

import { Bot, SlidersHorizontal, User, Users } from 'lucide-react'
import { ObjectAvatar } from '../../presentation/ObjectIcon'
import { presentUnresolvedDid } from '../../presentation/model'
import { useDidPresentation } from '../../presentation/useDidPresentation'
import type { MessageHubStore } from './store/types'
import type { EntityType, MessageHubContext } from './types'

const typeColors: Record<EntityType, string> = {
  person: 'var(--cp-accent)',
  agent: 'var(--cp-success)',
  group: 'var(--cp-warning)',
  service: 'var(--cp-danger)',
}

function TypeGlyph({ type, size }: { type: EntityType; size: number }) {
  switch (type) {
    case 'agent':
      return <Bot size={size} />
    case 'group':
      return <Users size={size} />
    case 'service':
      return <SlidersHorizontal size={size} />
    default:
      return <User size={size} />
  }
}

export function HubAvatar({
  type,
  name,
  avatar,
  size,
  isOnline,
  decorative = true,
}: {
  type: EntityType
  name: string
  avatar?: string
  size: number
  isOnline?: boolean
  decorative?: boolean
}) {
  const glyph = size >= 64 ? 32 : size >= 46 ? 20 : size >= 38 ? 17 : size >= 28 ? 15 : Math.max(10, Math.round(size * 0.6))
  return (
    <span className="relative inline-flex shrink-0" style={{ width: size, height: size }} data-testid="hub-avatar" data-avatar={avatar ? 'image' : 'default'}>
      <ObjectAvatar
        title={name}
        icons={[avatar]}
        size={size}
        tone={typeColors[type]}
        fallbackIcon={<TypeGlyph type={type} size={glyph} />}
        decorative={decorative}
      />
      {isOnline ? (
        <span
          className="absolute bottom-0 right-0 rounded-full border-2"
          style={{
            width: Math.max(10, Math.round(size * 0.24)),
            height: Math.max(10, Math.round(size * 0.24)),
            background: 'var(--cp-success)',
            borderColor: 'var(--cp-surface)',
          }}
        />
      ) : null}
    </span>
  )
}

/**
 * A member who is not among the owner's contacts: their own DID document (the Zone resolver)
 * supplies the picture; until then, or when it cannot be read, the type glyph shows.
 */
export function UnknownMemberAvatar({ did, type, name, size }: { did: string; type: EntityType; name: string; size: number }) {
  const presentation = useDidPresentation(did, type === 'agent' ? 'agent' : 'user') ?? presentUnresolvedDid(did, 'user', 'loading')
  return <HubAvatar type={type} name={name} avatar={presentation.icons[0]} size={size} />
}

/** A member row's picture: the owner's own, a contact's, or the member's DID document's. */
export function MemberAvatar({
  store,
  context,
  did,
  name,
  size = 32,
}: {
  store: Pick<MessageHubStore, 'findEntity' | 'avatarOf'>
  context: MessageHubContext
  did: string
  name: string
  size?: number
}) {
  const entity = store.findEntity(context, did)
  const type = entity?.type ?? 'person'
  return entity || did === context.ownerDid
    ? <HubAvatar type={type} name={name} avatar={store.avatarOf(context, did)} size={size} />
    : <UnknownMemberAvatar did={did} type={type} name={name} size={size} />
}
