import type { GroupDisplayInfo, GroupHookInfo, GroupPanelRef } from './extensions/model'
import type { GroupInvitation, GroupInvitationView, GroupRole, PinnedMessage, ReadReceipt, RuntimeState, SessionAccess, SessionPanelInfo } from '../../components/conversation/model/types'

import type { Session as SharedSession } from '../../shared/messagehub/types.ts'

export type { GroupInvitation, GroupInvitationView, GroupRole, PinnedMessage, ReadReceipt, RuntimeState, SessionAccess, SessionPanelInfo }
export type {
  AccountBinding,
  CreationPolicy,
  Entity,
  EntityChildrenMode,
  EntityChildrenSection,
  EntityDetail,
  EntityType,
  MessagePreview,
  SessionBinding,
  SessionType,
} from '../../shared/messagehub/types.ts'

export interface Session extends SharedSession {
  /** Custom content of the floating session panel (`UI_DATAMODEL.md` §3.3.6); wins over a pinned message. */
  panel?: SessionPanelInfo
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

export interface SessionPreferences {
  title: string
  /** The owner's own description of the session (`ui.description`). */
  description: string
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

/* ── Self-host Group v2 ── */

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
  /** The session's rules let it invite guests (`SessionRules.allow_guests`). */
  allowGuests: boolean
  receipts: 'hidden' | 'count' | 'readers'
  /** Embedded application panels in effect for the session (EXT-01/02). */
  panels: GroupPanelRef[]
  /** Display configuration, the session's over the group's (EXT-04). */
  display: GroupDisplayInfo
  /** Business Hooks bound to the session (EXT-06). */
  hooks: GroupHookInfo[]
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

/** What the group editor saves; `historyBeforeJoin` is left out when the group shows the whole history. */
export interface GroupProfileInput {
  name: string
  description: string
  historyBeforeJoin?: number
}

/** The viewer's view of one group: hosted by this zone, or joined on a remote host. */
export interface GroupInfo {
  did: string
  name: string
  description: string
  ownerDid: string
  /** The Zone hosting the group (group Doc `host`); panels must come from its Apps (D-2). */
  hostDid: string
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
  /**
   * How many messages from before joining a new member sees in the default
   * session; undefined when it shows the whole history or the configuration is unknown.
   */
  historyBeforeJoin?: number
  /** An owner transfer the owner started and the target has not accepted yet. */
  pendingTransfer?: { memberDid: string; transferId: string; expiresAt: number }
}
