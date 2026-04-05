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
  title: string
  type: SessionType
  /** Protocol/tunnel source label */
  source?: string
  isActive: boolean
  lastActiveAt: number
  unreadCount: number
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
}

export interface AccountBinding {
  platform: string
  accountId: string
  displayId: string
}

/* ── Filter / Search ── */

export type EntityFilter = 'all' | 'unread' | 'pinned' | 'agents' | 'groups' | 'people'

/* ── View State ── */

export type MobileView = 'entity-list' | 'conversation' | 'details'

export interface MessageHubState {
  selectedEntityId: string | null
  selectedSessionId: string | null
  activeFilter: EntityFilter
  searchQuery: string
  mobileView: MobileView
  showSessionSidebar: boolean
  showDetails: boolean
}
