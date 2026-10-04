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
  /** Custom content of the floating session panel (`UI_DATAMODEL.md` §3.3.6); wins over a pinned message. */
  panel?: SessionPanelInfo
}

/** Host-defined content of the floating session panel, e.g. the state of a ticket. */
export interface SessionPanelInfo {
  title: string
  /** Free text, shown in at most three lines until expanded. */
  text?: string
  fields?: Array<{ label: string; value: string; tone?: 'success' | 'warning' | 'danger' }>
}

/** Snapshot of the message a viewer pinned to the top of a session. */
export interface PinnedMessage {
  /** `messageObjId` of the pinned message. */
  id: string
  text: string
  senderDid: string
  createdAt: number
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
  /** The owner's pinned message of the session; null or absent when nothing is pinned. */
  pinnedMessage?: PinnedMessage | null
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

/* ── Self-host Group v2 ── */

export type GroupRole = 'owner' | 'admin' | 'member'
export type GroupMemberState = 'invited' | 'pending_admin_approval' | 'active' | 'left' | 'removed' | 'rejected' | 'expired' | 'revoked'

export interface GroupMember {
  did: string
  role: GroupRole
  state: GroupMemberState
  expiresAt?: number
  /** Who issued the current invitation (member-issued ones need approval). */
  invitedBy?: string
}

/** Capabilities of the viewer in a group, from `group.check_access` (never inferred from the role). */
export interface GroupCapabilities {
  invite: boolean
  remove: boolean
  createSession: boolean
  approve: boolean
  updateRole: boolean
  moderate: boolean
  updateConfig: boolean
  manageSession: boolean
  inviteGuest: boolean
  updateSharedState: boolean
  redactAny: boolean
  mentionAll: boolean
  /** Only the controller (the owner) may start an owner transfer. */
  transferOwner: boolean
}

export const noGroupCapabilities: GroupCapabilities = { invite: false, remove: false, createSession: false, approve: false, updateRole: false, moderate: false, updateConfig: false, manageSession: false, inviteGuest: false, updateSharedState: false, redactAny: false, mentionAll: false, transferOwner: false }

/** One Group Session as `group.list_sessions` reports it; `key` is the local session id (canonical MailboxAddress). */
export interface GroupSessionInfo {
  key: string
  /** null for the default session. */
  sessionId: string | null
  title: string
  description: string
  announcement: string
  /** Revision of the shared state (`group.update_shared_state` expects it). */
  sharedRevision: string
  lifecycle: string
  /** Session record revision (`group.archive_session` / `group.update_session` expect it). */
  revision: string
  hasGuests: boolean
  receipts: 'hidden' | 'count' | 'readers'
}

/** One participant of a Group Session (`group.list_session_members`). */
export interface GroupSessionParticipant {
  did: string
  kind: 'group_member' | 'guest'
  /** Group role of a member; absent for a guest. */
  role?: GroupRole
  /** `invited`: a guest invitation not accepted yet (listed only for those who may invite guests). */
  state: 'included' | 'invited'
}

/**
 * The participants of a Group Session the viewer may see. `complete` is false
 * for a guest, or a member the member list is hidden from: they see only the
 * explicit participants and those who posted (v2 §3.3).
 */
export interface GroupSessionParticipants {
  items: GroupSessionParticipant[]
  complete: boolean
}

/** The viewer's view of one group: hosted by this zone, or joined on a remote host. */
export interface GroupInfo {
  did: string
  name: string
  description: string
  ownerDid: string
  hosted: boolean
  lifecycle: 'active' | 'archived' | 'deleted'
  /** Configuration revision (`group.apply_config` expects it). */
  revision: string
  myRole?: GroupRole
  /** null when the member list is hidden from the viewer or managed by a remote host. */
  members: GroupMember[] | null
  sessions: GroupSessionInfo[]
  can: GroupCapabilities
  /** Edit / recall windows of the default session; undefined means unlimited. */
  messageRules: { editWindowMs?: number; recallWindowMs?: number }
  /** An owner transfer the owner started and the target has not accepted yet. */
  pendingTransfer?: { memberDid: string; transferId: string; expiresAt: number }
}

/** `buckyos.group_invitation` notification sent to an invited DID. */
export interface GroupInvitation {
  groupDid: string
  inviteId: string
  role: GroupRole
  expiresAt?: number
  inviterDid: string
  /** `active`: accepted automatically; `pending_admin_approval`: accepted, waiting for approval; `invited`: waiting for the member. */
  state?: 'invited' | 'active' | 'pending_admin_approval'
  /** Set when the invitation is for the viewer's agent and the viewer accepts on its behalf. */
  memberDid?: string
}

export interface GroupInvitationView {
  groupName: string
  state: 'pending' | 'joined' | 'approval' | 'expired'
}

/** Read receipt of one own group message (`group.get_read_markers`). */
export interface ReadReceipt {
  count: number
  readers?: string[]
}
