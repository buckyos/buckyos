/* ── Users & Agents – type definitions ── */

// ── Entity types ──

export type EntityKind = 'self' | 'agent' | 'local-user' | 'contact' | 'entity-group'

export interface MessageTunnelBinding {
  id: string
  platform: string          // e.g. 'telegram', 'email', 'wechat'
  accountId: string
  displayId: string
  status: 'active' | 'pending' | 'error'
  lastSyncAt?: string
}

export interface EntityBase {
  id: string
  kind: EntityKind
  displayName: string
  avatarUrl?: string
  did?: string
  bindings: MessageTunnelBinding[]
  createdAt: string
}

// ── Self ──

export interface SelfEntity extends EntityBase {
  kind: 'self'
  bio?: string
  email?: string
  phone?: string
  info: Record<string, string>          // lightweight public profile
  didDocument?: Record<string, unknown> // serious identity data
  twoFactorEnabled: boolean
  lastLogin: string
}

// ── Agent ──

export interface AgentEntity extends EntityBase {
  kind: 'agent'
  agentType: string
  version: string
  status: 'running' | 'stopped' | 'error'
  capabilities: string[]
  info: Record<string, string>
  didDocument?: Record<string, unknown>
  runtime: {
    uptime: string
    memoryUsage: string
    cpuUsage: string
    lastActive: string
    uiSessions: number
    workSessions: number
    workspaces: number
  }
}

// ── Local space user ──

export interface LocalUserEntity extends EntityBase {
  kind: 'local-user'
  role: 'admin' | 'member' | 'guest'
  storageUsed: string
  storageQuota: string
  lastActive: string
  isOnline: boolean
  availableApps: string[]
  defaultGroup: string
}

// ── Contact ──

export interface ContactEntity extends EntityBase {
  kind: 'contact'
  source: 'manual' | 'imported' | 'telegram' | 'email' | 'discovered'
  sourceLabel?: string     // e.g. 'Bob.telegram'
  isVerified: boolean      // bidirectional relationship
  tags: string[]
  notes?: string
  lastInteraction?: string
}

// ── Entity group ──

export interface EntityGroupEntity extends EntityBase {
  kind: 'entity-group'
  description?: string
  memberCount: number
  memberIds: string[]
  ownerName?: string
  isHostedBySelf: boolean
  canMessage: boolean
}

// ── Union type ──

export type AnyEntity =
  | SelfEntity
  | AgentEntity
  | LocalUserEntity
  | ContactEntity
  | EntityGroupEntity

// ── Collections ──

export type CollectionType = 'friends' | 'groups' | 'custom'

export interface Collection {
  id: string
  name: string
  type: CollectionType
  isBuiltIn: boolean
  entityIds: string[]
  createdAt: string
}

// ── View state ──

export type ViewMode = 'entity-detail' | 'collection-browse'

export type SidebarSelection =
  | { kind: 'entity'; entityId: string }
  | { kind: 'collection'; collectionId: string }

// ── Store snapshot ──

export interface UsersAgentsSnapshot {
  self: SelfEntity
  agent: AgentEntity
  localUsers: LocalUserEntity[]
  contacts: ContactEntity[]
  entityGroups: EntityGroupEntity[]
  collections: Collection[]
}

// ── New user wizard ──

export interface NewUserDraft {
  step: number
  displayName: string
  role: 'admin' | 'member' | 'guest'
  initialPassword: string
  storageQuota: string
}
