/* ── Network UI data model: the views of msg-center's relationship surface (see UI_DATAMODEL.md) ── */

import type {
  AccessGroupLevel,
  Contact,
  FriendRequest,
  FriendRequestDecision,
  FriendRequestResult,
  FriendState,
  LookupCandidate,
  LookupKind,
  LookupResult,
  Reachability,
} from 'buckyos'

export type {
  AccessGroupLevel,
  Contact,
  FriendRequest,
  FriendRequestDecision,
  FriendRequestResult,
  FriendState,
  LookupCandidate,
  LookupKind,
  LookupResult,
  Reachability,
}

export type GroupRole = 'owner' | 'admin' | 'member'
export type GroupMemberState = 'active' | 'invited' | 'pending_admin_approval'
export type JoinOutcome = 'active' | 'pending_admin_approval'

/** A group the owner belongs to: `group.list_by_member` plus the owner's own member record and post access. */
export interface GroupRef {
  did: string
  name?: string
  description?: string
  avatar?: string
  owner?: string
  host?: string
  joinPolicy?: string
  memberCount?: number
  role?: GroupRole
  state: GroupMemberState
  /** Hosted by this Zone (`items`); false for a group joined on another host (`joined`). */
  hostedHere: boolean
  /** `group.check_access(session.post)`; null when it could not be checked. */
  canPost: boolean | null
  postBlockedReason?: string
}

/** What `group.get_doc` adds to a lookup candidate before joining. */
export interface GroupDocInfo {
  did: string
  name?: string
  description?: string
  owner?: string
  host?: string
  joinPolicy?: string
}

/** A conversation (`dm:<peer>`) with messages waiting in the owner's REQUEST_BOX (`msg.list_sessions`, `request_count > 0`). */
export interface RequestSession {
  sessionId: string
  peerDid: string
  peerName?: string
  /** `request_count` of the session. */
  count: number
  lastAtMs: number
  /** `content.machine.intent` of the latest record, when it carries one. */
  intent?: string
  preview?: string
}

/** Ordinary messages of one sender that wait in REQUEST_BOX. */
export interface MessageRequest {
  senderDid: string
  senderName?: string
  count: number
  lastAtMs: number
  preview?: string
  sessionId: string
}

/** A pending invitation of the owner (`group.list_by_member` `invitations`, state `invited`). */
export interface GroupInvitationItem {
  groupDid: string
  groupName?: string
  /** The `buckyos.group_invitation` notice in `dm:<inviter>`, when found; ignoring archives it. */
  recordId?: string
  inviteId: string
  inviterDid: string
  role: GroupRole
  receivedAtMs?: number
  expiresAtMs?: number
}

/** A member waiting for the owner's approval in a group the owner administers. */
export interface JoinRequestItem {
  groupDid: string
  groupName?: string
  memberDid: string
  entityKind?: string
}

export type RequestItem =
  | { type: 'friend_request'; key: string; request: FriendRequest }
  | { type: 'message_request'; key: string; request: MessageRequest }
  | { type: 'group_invitation'; key: string; invitation: GroupInvitationItem }
  | { type: 'join_request'; key: string; request: JoinRequestItem }

export type NetworkChange = 'contacts' | 'requests' | 'mailbox' | 'groups'

/** The msg-center calls the network components make; one mock and one real implementation. */
export interface NetworkBackend {
  readonly isMock: boolean
  ownerDid(): Promise<string>
  /** What another person pastes into their Add Friend to find the owner. */
  shareText(): Promise<string>
  lookup(input: string, options?: { kinds?: LookupKind[]; limit?: number }): Promise<LookupResult>
  listContacts(): Promise<Contact[]>
  /** Creates the contact record (Stranger level) when it does not exist. */
  saveContact(did: string, name?: string): Promise<Contact>
  updateTags(did: string, change: { add?: string[]; remove?: string[] }): Promise<Contact>
  removeContact(did: string): Promise<void>
  blockContact(did: string): Promise<void>
  unblockContact(did: string): Promise<Contact>
  allowMessages(did: string, name?: string): Promise<Contact>
  sendFriendRequest(did: string, message?: string): Promise<FriendRequestResult>
  respondFriendRequest(requestId: string, decision: FriendRequestDecision): Promise<FriendRequestResult>
  cancelFriendRequest(requestId: string): Promise<FriendRequestResult>
  removeFriend(did: string, notify: boolean): Promise<FriendRequestResult>
  /** Recent requests in both directions and every state (pending ones drive Requests). */
  listFriendRequests(): Promise<FriendRequest[]>
  listRequestSessions(): Promise<RequestSession[]>
  listGroups(): Promise<GroupRef[]>
  getGroupDoc(groupDid: string): Promise<GroupDocInfo>
  requestJoin(groupDid: string, invite?: string): Promise<JoinOutcome>
  leaveGroup(groupDid: string): Promise<void>
  listGroupInvitations(): Promise<GroupInvitationItem[]>
  acceptGroupInvitation(invitation: GroupInvitationItem): Promise<JoinOutcome>
  dismissGroupInvitation(invitation: GroupInvitationItem): Promise<void>
  listJoinRequests(): Promise<JoinRequestItem[]>
  approveJoinRequest(request: JoinRequestItem): Promise<void>
  rejectJoinRequest(request: JoinRequestItem): Promise<void>
  /** Change notifications (contact kevent, mailbox events); returns the unsubscribe. */
  subscribe(listener: (change: NetworkChange) => void): () => void
}
