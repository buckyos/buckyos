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
