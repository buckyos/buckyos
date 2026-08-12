import type {
  EntityGroupEntity,
  SocialAccount,
} from '../../users-agents/datamodel/types'

export interface ContactEntity {
  id: string
  kind: 'contact'
  displayName: string
  avatarUrl?: string
  did?: string
  socialAccounts: SocialAccount[]
  createdAt: string
  source: 'manual' | 'imported' | 'telegram' | 'email' | 'discovered'
  sourceLabel?: string
  isVerified: boolean
  relation: 'one-way' | 'mutual'
  lastSyncedAt?: string
  importBatch?: string
  isMergeCandidate?: boolean
  tags: string[]
  notes?: string
  lastInteraction?: string
}

export type MyNetworkEntity = ContactEntity | EntityGroupEntity

export type CollectionType = 'contacts' | 'friends' | 'groups' | 'custom' | 'dynamic-view'
export type CollectionMode = 'static' | 'view'

export interface Collection {
  id: string
  name: string
  type: CollectionType
  mode: CollectionMode
  description?: string
  sourceType: 'built-in' | 'manual' | 'import' | 'sync'
  isBuiltIn: boolean
  isReadOnly: boolean
  entityIds: string[]
  createdAt: string
  updatedAt: string
}

export type MyNetworkSelection =
  | { kind: 'entity'; entityId: string }
  | { kind: 'collection'; collectionId: string }

export interface MyNetworkSnapshot {
  contacts: ContactEntity[]
  entityGroups: EntityGroupEntity[]
  collections: Collection[]
}
