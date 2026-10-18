/* ── Mock runtime only: other apps' mocks (MessageHub) add their people and groups to the one mock social graph ──
 *
 * In a real Zone every app reads the same msg-center; in the mock runtime each app seeds its own
 * world. A host registers what its world knows so FindPeople, PeoplePicker and JoinGroupDialog see
 * the same people and groups that app shows, and joining one of its groups goes through it.
 */

import type { ReachabilityState } from 'buckyos'
import type { GroupRole, JoinOutcome, LookupKind } from '../types'

export interface NetworkMockPerson {
  did: string
  name: string
  kind: LookupKind
  reachability: ReachabilityState
  zone?: string
  avatar?: string
  /** Seeded as a friend (otherwise as a plain contact). */
  friend?: boolean
}

export interface NetworkMockGroup {
  did: string
  name: string
  description?: string
  owner?: string
  joinPolicy?: string
  memberCount?: number
  /** The owner's member state and role in the group, when any. */
  myState?: string
  myRole?: GroupRole
}

export interface NetworkMockHost {
  people(): NetworkMockPerson[]
  group(did: string): NetworkMockGroup | null
  /** Groups the owner is an active member of. */
  groups(): NetworkMockGroup[]
  requestJoin(did: string, invite?: string): Promise<JoinOutcome>
}

const hosts = new Set<NetworkMockHost>()
const listeners = new Set<() => void>()

export function registerNetworkMockHost(host: NetworkMockHost): () => void {
  hosts.add(host)
  listeners.forEach(listener => listener())
  return () => {
    hosts.delete(host)
    listeners.forEach(listener => listener())
  }
}

export function networkMockHosts(): NetworkMockHost[] {
  return [...hosts]
}

export function onNetworkMockHosts(listener: () => void): () => void {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}
