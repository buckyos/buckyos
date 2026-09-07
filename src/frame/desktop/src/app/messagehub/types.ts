/* ── Entity ── */

export type EntityType = 'person' | 'agent' | 'group' | 'service'
export type EntityChildrenMode = 'inline' | 'drilldown'

export interface EntityChildrenSection {
  id: string
  title: string
  description?: string
  childIds: string[]
}

export interface Entity {
  id: string
  type: EntityType
  sessionCreation?: { policy: CreationPolicy; canCreate: boolean; unavailableReason?: string }
  name: string
  avatar?: string
  /** Short status line, e.g. "online", "last seen 2h ago" */
  statusText?: string
  isOnline?: boolean
  isPinned?: boolean
  isMuted?: boolean
  unreadCount: number
  /** Tags for filtering */
  tags: string[]
  /** Last message preview */
  lastMessage?: MessagePreview
  /** Timestamp of last activity (ms) */
  lastActiveAt: number
  /** Sub-entities (e.g. topics under a group) */
  children?: Entity[]
  /** Whether children stay inline or take over the entity panel */
  childrenMode?: EntityChildrenMode
  /** Custom grouped content for drill-down entity panels */
  childrenSections?: EntityChildrenSection[]
  /** Summary shown in the drill-down overview */
  drilldownDescription?: string
  /** Platform/protocol source */
  source?: string
  /** All protocol sources of the entity (`UI_DATAMODEL.md` §3.2). */
  sources?: string[]
  /** Managed (zone user / agent / hosted group) vs external identity. */
  domain?: 'managed' | 'external'
  /** Number of visible sessions under the entity. */
  sessionCount?: number
  /** Visible REQUEST_BOX records across the entity's sessions. */
  requestCount?: number
}

export interface MessagePreview {
  senderName?: string
  text: string
  timestamp: number
}

/* ── Session ── */

export type SessionType = 'chat' | 'task' | 'workspace'

export interface Session {
  id: string
  entityId: string
  ownerDid: string
  binding: SessionBinding
  origin: 'manual' | 'connection' | 'remote_context' | 'unknown'
  lifecycle: 'active' | 'archived'
  createdAt: number
  shared: { title: string; description: string; updatedAt: number }
  members: Record<string, { nickname: string; updatedAt: number }>
  lastMessage?: MessagePreview
  title: string
  type: SessionType
  /** Protocol/tunnel source label */
  source?: string
  lastActiveAt: number
  unreadCount: number
  /** Visible REQUEST_BOX records in this session (real backend only). */
  requestCount?: number
  /** Aggregated delivery state of the latest outbound message. */
  lastDelivery?: 'sending' | 'delivered' | 'partial_failed' | 'failed'
  /** How the entity attribution was derived (real backend only). */
  attributionEvidence?: 'registered' | 'group_tag' | 'group_message' | 'direct' | 'message' | 'record' | 'none'
}

/* ── Entity Details ── */

export interface EntityDetail extends Entity {
  bio?: string
  /** Account bindings / protocol sources */
  bindings?: AccountBinding[]
  /** Group members count */
  memberCount?: number
  /** Notes added by user */
  note?: string
  createdAt?: number
  /** Contact admission level of the current owner towards this entity. */
  accessLevel?: 'block' | 'stranger' | 'temporary' | 'friend'
  isVerified?: boolean
  contactSource?: string
}

export interface AccountBinding {
  platform: string
  accountId: string
  displayId: string
}

/* ── Filter / Search ── */

export type EntityFilter = 'all' | 'unread' | 'pinned' | 'agents' | 'groups' | 'people' | 'requests'

/* ── View State ── */

export type MobileView = 'entity-list' | 'conversation' | 'details'

export interface MessageHubState {
  selectedEntityId: string | null
  selectedSessionId: string | null
  activeFilter: EntityFilter
  searchQuery: string
  mobileView: MobileView
  showSessionSidebar: boolean
  detailsTarget: 'entity' | 'session' | null
}

export type CreationPolicy = 'default' | 'allow' | 'deny'
export type SessionBinding =
  | { kind: 'native'; targetDid: string }
  | { kind: 'tunnel'; tunnelInstanceId: string; endpointDid: string; connectionName: string; remoteContextId?: string; supportsMultipleSessions: boolean; canCreateRemoteSession: boolean; canSend: boolean; connected: boolean; revision?: number }
  | { kind: 'unknown' }

export interface SessionAccess {
  mode: 'read_only' | 'read_write'
  canManage: boolean
  canEnableWrite: boolean
  canEditPresentation: boolean
  canEditSharedState: boolean
  canEditOwnMemberState: boolean
  readOnlyReason?: string
}

export interface SessionPreferences {
  title: string
  pinned: boolean
  muted: boolean
  showActions: boolean
}

export interface MessageHubContext {
  viewerDid: string
  ownerDid: string
  mode: 'self' | 'observe'
}

export interface RuntimeState {
  memberDid: string
  status: 'typing' | 'processing' | 'active'
  statusLine?: string
  expiresAt: number
}
