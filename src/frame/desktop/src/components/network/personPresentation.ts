/* ── Names and pictures of people in the network components, always through presentation/ ──
 *
 * msg-center's own text comes first (the owner's name for a contact, the profile hint of a
 * lookup); without one, the DID document (presentation store) supplies it; the readable part of
 * the DID is the last resort.
 */

import { useMemo } from 'react'
import { didDisplayName, presentAgent, presentUser, type ObjectPresentation } from '../../presentation/model'
import { useDidPresentation } from '../../presentation/useDidPresentation'
import type { LookupCandidate, LookupKind } from './types'
import type { PickedPerson } from './schemas'

export interface PersonRef {
  did: string
  kind?: LookupKind
  name?: string
  avatar?: string
}

const DOCUMENT_KINDS = new Set<LookupKind | undefined>(['user', 'agent', 'unknown', undefined])

export function usePersonPresentation(person: PersonRef): ObjectPresentation {
  const kind = person.kind === 'agent' ? 'agent' : 'user'
  const fromDocument = useDidPresentation(!person.name && DOCUMENT_KINDS.has(person.kind) ? person.did : undefined, kind)
  return useMemo(() => {
    const name = person.name || fromDocument?.title
    const avatar = person.avatar || fromDocument?.icons[0]
    return kind === 'agent'
      ? presentAgent({ did: person.did, name, profile: { avatar } })
      : presentUser({ did: person.did, profile: { display_name: name, avatar } })
  }, [kind, person.did, person.name, person.avatar, fromDocument])
}

export function personName(person: PersonRef): string {
  return person.name || didDisplayName(person.did)
}

export function pickedPerson(candidate: LookupCandidate): PickedPerson {
  return {
    did: candidate.did,
    kind: candidate.kind,
    name: personName(candidate),
    ...(candidate.avatar ? { avatar: candidate.avatar } : {}),
    ...(candidate.reachability.zone ? { zone: candidate.reachability.zone } : {}),
    isContact: candidate.relation.is_contact,
    ...(candidate.relation.friend_state ? { friendState: candidate.relation.friend_state } : {}),
  }
}

/** Short form of a long DID for list rows; the full DID stays in the title tooltip and detail. */
export function shortDid(did: string): string {
  return did.length > 34 ? `${did.slice(0, 20)}…${did.slice(-8)}` : did
}
