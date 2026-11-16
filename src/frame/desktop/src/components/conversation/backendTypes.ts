import type { ReactNode } from 'react'
import type { DID } from 'buckyos'
import type { OutgoingPayload } from './model/outgoing'
import type { ConversationSource } from './source'

/** A conversation of the viewer as the bound parts list it. */
export interface ConversationSessionInfo {
  id: string
  /** The entity the conversation is with. */
  entityId: string
  title: string
  lastActiveAt: number
  lifecycle: 'active' | 'archived'
  /** Runs over the native message protocol, not a tunnel to another platform. */
  native: boolean
}

/** Where `SessionConversation`'s conversation stands. */
export type BoundConversation =
  | { status: 'loading' }
  | { status: 'error'; retry: () => void }
  | { status: 'missing' }
  | { status: 'ready'; source: ConversationSource }

/** A Zone content reference attached to a message (an object the recipient can read). */
export interface ConversationAttachmentRef { objId: string; label?: string }

/**
 * MessageHub's data layer as the conversation component's bound parts use it, always as the signed-in viewer
 * looking at their own messages (`app/messagehub/conversationBackend.tsx`, loaded through `backend.ts`).
 */
export interface ConversationBackend {
  readonly isMock: boolean
  /** Renders `children` with the state of the viewer's session `sessionId`; keeps it in sync while mounted. */
  SessionSource(props: { sessionId: string; children: (state: BoundConversation) => ReactNode }): ReactNode
  /** The viewer's conversations are loaded (idempotent). */
  ensureReady(): Promise<void>
  /** The signed-in viewer; empty before `ensureReady`. */
  viewerDid(): DID
  /** The viewer's own default Agent, when there is one. */
  defaultAgent(): Promise<{ did: DID; name: string } | null>
  /** The viewer's active conversations with the entity, newest first. */
  sessions(entityId: string): ConversationSessionInfo[]
  session(sessionId: string): ConversationSessionInfo | null
  /** Lists an Agent as someone the viewer talks to, under `name` until the directory names it. */
  addAgent(agentDid: DID, name?: string): void
  /** The viewer's default conversation with the entity, created when there is none. */
  ensureDefaultSession(entityId: string): Promise<ConversationSessionInfo | null>
  createSession(entityId: string, title: string): Promise<ConversationSessionInfo>
  deleteSession(sessionId: string): Promise<void>
  /** Sends a message in the viewer's name. */
  send(sessionId: string, message: { text: string; attachments?: ConversationAttachmentRef[] }): Promise<void>
  /** Sends what a composer submitted (files included), as the conversation component does. */
  post(sessionId: string, payload: OutgoingPayload): Promise<void>
  /** Entries of the viewer's own UI state of a session (`ui_session.list_state`). */
  listUiState(sessionId: string): Promise<Array<{ key: string; value: unknown }>>
  updateUiState(sessionId: string, key: string, value: string): Promise<void>
}
