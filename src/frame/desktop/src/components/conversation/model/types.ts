/** Host-defined content of the floating session panel, e.g. the state of a ticket. */
export interface SessionPanelInfo {
  title: string
  /** Free text, shown in at most three lines until expanded. */
  text?: string
  fields?: Array<{ label: string; value: string; tone?: 'success' | 'warning' | 'danger' }>
}

export type GroupRole = 'owner' | 'admin' | 'member'

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
  state: 'pending' | 'joined' | 'approval' | 'expired' | 'syncing'
}

/** Read receipt of one own group message (`group.get_read_markers`). */
export interface ReadReceipt {
  count: number
  readers?: string[]
}

/** Snapshot of the message a viewer pinned to the top of a session. */
export interface PinnedMessage {
  /** `messageObjId` of the pinned message. */
  id: string
  text: string
  senderDid: string
  createdAt: number
}

export interface SessionAccess {
  mode: 'read_only' | 'read_write'
  canManage: boolean
  canEnableWrite: boolean
  canEditPresentation: boolean
  canEditSharedState: boolean
  canEditOwnMemberState: boolean
  readOnlyReason?: string
}

export interface RuntimeState {
  memberDid: string
  status: 'typing' | 'processing' | 'active'
  statusLine?: string
  expiresAt: number
}
