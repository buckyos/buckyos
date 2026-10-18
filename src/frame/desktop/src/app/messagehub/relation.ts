/* ── The signed-in owner's relation with an entity, as the network components (msg-center) know it ── */

import { useMemo } from 'react'
import { useNetworkResource } from '../../components/network/hooks'
import { messagesAllowed, NOT_RELATED, relationOfContact, withPendingRequest, type RelationFacts } from '../../components/network/relation'
import type { Entity, MessageHubContext } from './types'

/** People and Agents of the owner's own view have a relation; groups, services and observed owners do not. */
export function hasRelation(context: MessageHubContext, entity: Pick<Entity, 'id' | 'type' | 'systemNotify'> | null | undefined) {
  return Boolean(entity) && context.mode === 'self' && context.ownerDid === context.viewerDid && entity!.id !== context.ownerDid
    && (entity!.type === 'person' || entity!.type === 'agent') && !entity!.systemNotify && entity!.id.startsWith('did:')
}

export function useEntityRelation(entityId: string | null): RelationFacts | null {
  const contacts = useNetworkResource('contacts')
  const friendRequests = useNetworkResource('friendRequests')
  return useMemo(() => {
    if (!entityId || !contacts.data) return null
    const contact = contacts.data.find(item => item.did === entityId)
    return withPendingRequest(contact ? relationOfContact(contact) : NOT_RELATED, entityId, friendRequests.data)
  }, [entityId, contacts.data, friendRequests.data])
}

/** Messages of this sender wait as requests: neither a friend nor allowed (a blocked sender still shows, to unblock). */
export function needsAdmission(relation: RelationFacts | null) {
  return Boolean(relation) && relation!.friendState !== 'friend' && !messagesAllowed(relation!)
}
