import type { z } from 'zod'
import type { ComposerAttachmentInput } from '../conversation/input/attachmentDraft'
import type { ConversationMessageReader } from '../conversation/history/types'
import type { createSessionSchema } from '../sessionModel'
import type { CreationPolicy, Entity, EntityDetail, MessageHubContext, RuntimeState, Session, SessionAccess, SessionBinding, SessionPreferences } from '../types'

/** What the composer hands to the store: text plus the raw browser files. */
export interface OutgoingPayload {
  content: string
  attachments: ComposerAttachmentInput[]
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
  runtimeFor(context: MessageHubContext, sessionId: string): RuntimeState[]
  clearTransient(ownerDid: string): void
  title(context: MessageHubContext, session: Session): string
}
