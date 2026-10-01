import type { z } from 'zod'
import type { ComposerAttachmentInput } from '../conversation/input/attachmentDraft'
import type { ConversationMessageReader } from '../conversation/history/types'
import type { MessageObject, MsgMentions, MsgRelation } from '../protocol/msgobj'
import type { createSessionSchema } from '../sessionModel'
import type { createGroupSchema } from '../groupModel'
import type { CreationPolicy, Entity, EntityDetail, GroupInfo, GroupInvitation, GroupInvitationView, GroupSessionInfo, MessageHubContext, ReadReceipt, RuntimeState, Session, SessionAccess, SessionBinding, SessionPreferences } from '../types'

/**
 * What the composer hands to the store: text plus the raw browser files, and
 * for a relation message (edit / redact / reaction / reply) the MsgObject v2
 * `relates_to` and structured `mentions`, which travel in the message itself.
 */
export interface OutgoingPayload {
  content: string
  attachments: ComposerAttachmentInput[]
  relatesTo?: MsgRelation
  mentions?: MsgMentions
}

export interface ConnectionChoice {
  id: string
  binding: SessionBinding
  label: string
}

/** Load state of one owner's data (self or an observed agent). */
export type OwnerStatus =
  | { phase: 'idle' | 'loading' | 'ready' }
  | { phase: 'denied' }
  | { phase: 'error'; message: string }

export type ManageAction = 'archive' | 'restore' | 'delete'

/** Contact admission actions available for an entity in the current context. */
export interface EntityAdmission {
  accessLevel?: 'block' | 'stranger' | 'temporary' | 'friend'
  temporaryExpiresAt?: number
  canChange: boolean
}

/**
 * Data layer consumed by every MessageHub component. Two implementations:
 * the interactive mock (`mock/store.ts`) and the msg-center backed store
 * (`api/store.ts`). Components never import either directly.
 */
export interface MessageHubStore {
  readonly isMock: boolean
  subscribe(listener: () => void): () => void
  getSnapshot(): unknown
  subscribeTime(listener: () => void): () => void
  getTime(): number
  subscribeRuntime(listener: () => void): () => void
  getRuntimeVersion(): number
  tick(): void
  now(): number
  initialize(): Promise<void>
  /** Resolved after `initialize`: the logged-in viewer looking at itself. */
  defaultContext(): MessageHubContext
  canView(context: MessageHubContext): boolean
  ownerStatus(context: MessageHubContext): OwnerStatus
  /** Ensure the owner's data is loaded (idempotent); `refresh` forces a reload. */
  ensureOwner(context: MessageHubContext, refresh?: boolean): Promise<void>
  /** Start / stop background synchronisation for a context (events + polling). */
  startSync(context: MessageHubContext, activeSessionId: string | null): () => void
  findEntity(context: MessageHubContext, id: string): Entity | undefined
  entities(context: MessageHubContext): Entity[]
  hasMoreEntities(context: MessageHubContext): boolean
  loadMoreEntities(context: MessageHubContext): Promise<void>
  entityDetail(context: MessageHubContext, id: string): EntityDetail | null
  admission(context: MessageHubContext, entityId: string): EntityAdmission | null
  setAdmission(context: MessageHubContext, entityId: string, action: 'accept' | 'block'): Promise<void>
  sessions(context: MessageHubContext, entityId?: string, lifecycle?: Session['lifecycle']): Session[]
  defaultSession(context: MessageHubContext, entityId: string): Session | null
  ensureDefaultSession(context: MessageHubContext, entityId: string): Promise<Session | null>
  connections(context: MessageHubContext, entityId: string): ConnectionChoice[]
  reader(context: MessageHubContext, sessionId: string): ConversationMessageReader
  historyStatus(context: MessageHubContext, sessionId: string): 'idle' | 'loading' | 'ready' | 'error'
  hasOlder(context: MessageHubContext, sessionId: string): boolean
  loadOlder(context: MessageHubContext, sessionId: string): Promise<boolean>
  /** Mark displayed inbound records read (no-op for observers). */
  markRead(context: MessageHubContext, sessionId: string, recordIds: string[]): Promise<void>
  access(context: MessageHubContext, session: Session, confirmed: boolean): SessionAccess
  draft(context: MessageHubContext, sessionId: string): string
  attachments(context: MessageHubContext, sessionId: string): ComposerAttachmentInput[]
  saveDraft(context: MessageHubContext, sessionId: string, value: string): Promise<void>
  saveAttachments(context: MessageHubContext, sessionId: string, attachments: ComposerAttachmentInput[]): Promise<void>
  policy(context: MessageHubContext, entityId: string): CreationPolicy
  setPolicy(context: MessageHubContext, entityId: string, policy: CreationPolicy): Promise<void>
  create(context: MessageHubContext, input: z.infer<typeof createSessionSchema>): Promise<Session>
  manage(context: MessageHubContext, sessionId: string, action: ManageAction): Promise<void>
  preferences(context: MessageHubContext, sessionId: string): SessionPreferences
  updatePreferences(context: MessageHubContext, sessionId: string, patch: Partial<SessionPreferences>): Promise<void>
  updateState(context: MessageHubContext, sessionId: string, scope: 'shared' | 'member', input: unknown): Promise<void>
  send(context: MessageHubContext, sessionId: string, payload: OutgoingPayload, confirmation: string | undefined): Promise<void>
  /** Post a failed outgoing message again as a new message (same text and attachment refs). */
  resend(context: MessageHubContext, sessionId: string, message: MessageObject, confirmation: string | undefined): Promise<void>
  runtimeFor(context: MessageHubContext, sessionId: string): RuntimeState[]
  clearTransient(ownerDid: string): void
  title(context: MessageHubContext, session: Session): string
  /**
   * Self-host groups (`Self-Host-Groupv2.md`). Group operations reject with
   * the host's reason code (`capability-denied`, `invitation-mismatch`, …).
   */
  group(context: MessageHubContext, groupDid: string): GroupInfo | null
  groupStatus(context: MessageHubContext, groupDid: string): 'idle' | 'loading' | 'ready' | 'error'
  ensureGroup(context: MessageHubContext, groupDid: string, refresh?: boolean): Promise<void>
  /** Creates the group and invites the members; resolves to the group DID (its entity id). */
  createGroup(context: MessageHubContext, input: z.infer<typeof createGroupSchema>): Promise<string>
  /** `group.apply_config` on `profile` with the group's current revision. */
  updateGroupProfile(context: MessageHubContext, groupDid: string, profile: { name: string; description: string }): Promise<void>
  /** Invites each DID; resolves to the DIDs whose invitation failed, with the reason. */
  inviteGroupMembers(context: MessageHubContext, groupDid: string, memberDids: string[]): Promise<Array<{ did: string; reason: string }>>
  removeGroupMember(context: MessageHubContext, groupDid: string, memberDid: string): Promise<void>
  approveGroupMember(context: MessageHubContext, groupDid: string, memberDid: string): Promise<void>
  rejectGroupMember(context: MessageHubContext, groupDid: string, memberDid: string): Promise<void>
  updateGroupMemberRole(context: MessageHubContext, groupDid: string, memberDid: string, role: 'admin' | 'member'): Promise<void>
  /** `mutedUntil: null` lifts a mute; `blocked: true` also removes the member. */
  moderateGroupMember(context: MessageHubContext, groupDid: string, memberDid: string, patch: { blocked?: boolean; mutedUntil?: number | null }): Promise<void>
  /** Owner starts a transfer; the target accepts it from its `owner_transfer` notification. */
  transferGroupOwner(context: MessageHubContext, groupDid: string, memberDid: string): Promise<void>
  cancelGroupOwnerTransfer(context: MessageHubContext, groupDid: string): Promise<void>
  acceptGroupOwnerTransfer(context: MessageHubContext, groupDid: string, transferId: string): Promise<void>
  /** Resolves to the link text (`formatInviteLink`). */
  createGroupInviteLink(context: MessageHubContext, groupDid: string, options: { expiresAt?: number; maxUses?: number; requireApproval?: boolean }): Promise<string>
  revokeGroupInviteLink(context: MessageHubContext, groupDid: string, token: string): Promise<void>
  /** `group.request_join`; resolves to the resulting member state (`active` or `pending_admin_approval`). */
  requestGroupJoin(context: MessageHubContext, groupDid: string, invite?: string): Promise<string>
  leaveGroup(context: MessageHubContext, groupDid: string): Promise<void>
  deleteGroup(context: MessageHubContext, groupDid: string): Promise<void>
  groupInvitation(context: MessageHubContext, invitation: GroupInvitation): GroupInvitationView
  acceptGroupInvitation(context: MessageHubContext, invitation: GroupInvitation): Promise<void>
  /** The Group Session behind a local group session id, once the group is loaded. */
  groupSession(context: MessageHubContext, groupDid: string, sessionId: string): GroupSessionInfo | null
  manageGroupSession(context: MessageHubContext, groupDid: string, sessionId: string, action: 'archive' | 'delete'): Promise<void>
  addGroupSessionMembers(context: MessageHubContext, groupDid: string, sessionId: string, memberDids: string[]): Promise<void>
  removeGroupSessionMember(context: MessageHubContext, groupDid: string, sessionId: string, memberDid: string): Promise<void>
  leaveGroupSession(context: MessageHubContext, groupDid: string, sessionId: string): Promise<void>
  inviteGroupSessionGuest(context: MessageHubContext, groupDid: string, sessionId: string, memberDid: string): Promise<void>
  acceptGroupSessionInvitation(context: MessageHubContext, groupDid: string, sessionId: string): Promise<void>
  /** Read receipt of an own group message when the session's `receipts` rule shows one; null while unknown or hidden. */
  readReceipt(context: MessageHubContext, sessionId: string, message: MessageObject): ReadReceipt | null
}
