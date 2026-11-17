/* ── MessageHub avatars: the identity's own picture; without one a person's initial in their colour, other kinds their type's glyph ── */

import { Bell, Bot, SlidersHorizontal, User, Users } from 'lucide-react'
import { ObjectAvatar } from '../../presentation/ObjectIcon'
import { presentUnresolvedDid } from '../../presentation/model'
import { useDidPresentation } from '../../presentation/useDidPresentation'
import type { MessageHubStore } from './store/types'
import type { EntityType, MessageHubContext } from './types'
import { senderTone } from '../../presentation/senderTone'

/** Kinds without a face keep one tone each; a person gets their own (`senderTone`). */
const typeColors: Record<EntityType, string> = {
  person: 'var(--cp-app-blue)',
  agent: 'var(--cp-app-green)',
  group: 'var(--cp-app-amber)',
  service: 'var(--cp-app-slate)',
}

/** Below this size an initial is too small to read, so a person shows the glyph in their colour. */
const INITIAL_MIN_SIZE = 28

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
  did,
  avatar,
  size,
  isOnline,
  decorative = true,
  systemNotify = false,
}: {
  type: EntityType
  name: string
  /** A person's DID: their colour (the same one beside their bubbles in a group) and their initial. */
  did?: string
  avatar?: string
  size: number
  isOnline?: boolean
  decorative?: boolean
  /** The System notifications entity: a bell instead of the service glyph. */
  systemNotify?: boolean
}) {
  const glyph = size >= 64 ? 32 : size >= 46 ? 20 : size >= 38 ? 17 : size >= 28 ? 15 : Math.max(10, Math.round(size * 0.6))
  const person = type === 'person' && !systemNotify
  const tone = systemNotify ? 'var(--cp-accent)' : person ? senderTone(did ?? name) : typeColors[type]
  const fallbackIcon = systemNotify ? <Bell size={glyph} /> : person && size >= INITIAL_MIN_SIZE ? undefined : <TypeGlyph type={type} size={glyph} />
  return (
    <span className="relative inline-flex shrink-0" style={{ width: size, height: size }} data-testid="hub-avatar" data-avatar={avatar ? 'image' : 'default'}>
      <ObjectAvatar
        title={name}
        icons={[avatar]}
        size={size}
        tone={tone}
        fallbackIcon={fallbackIcon}
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
  return <HubAvatar type={type} name={name} did={did} avatar={presentation.icons[0]} size={size} />
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
    ? <HubAvatar type={type} name={name} did={did} avatar={store.avatarOf(context, did)} size={size} />
    : <UnknownMemberAvatar did={did} type={type} name={name} size={size} />
}

/** A sender's own picture, else the initial of their name (an Agent: its glyph) on their colour. */
export function SenderPicture({ did, type = 'person', name, avatar, size }: { did: string; type?: EntityType; name: string; avatar?: string; size: number }) {
  return (
    <span className="relative inline-flex shrink-0" style={{ width: size, height: size }} data-testid="hub-avatar" data-avatar={avatar ? 'image' : 'default'}>
      <ObjectAvatar title={name} icons={[avatar]} size={size} tone={senderTone(did)} fallbackIcon={type === 'agent' ? <Bot size={Math.round(size * 0.5)} /> : undefined} decorative />
    </span>
  )
}

function UnknownSenderPicture({ did, name, size }: { did: string; name: string; size: number }) {
  const presentation = useDidPresentation(did, 'user') ?? presentUnresolvedDid(did, 'user', 'loading')
  return <SenderPicture did={did} name={name} avatar={presentation.icons[0]} size={size} />
}

/** The picture beside a sender's bubbles in a group conversation: a contact's, or the member's DID document's. */
export function SenderAvatar({
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
  return entity || did === context.ownerDid
    ? <SenderPicture did={did} type={entity?.type} name={name} avatar={store.avatarOf(context, did)} size={size} />
    : <UnknownSenderPicture did={did} name={name} size={size} />
}
