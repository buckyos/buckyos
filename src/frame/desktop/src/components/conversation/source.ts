import type { ReactNode } from 'react'
import type { DID } from 'buckyos'
import type { AgentSessionControls } from './agentControls'
import type { ConversationClock } from './clock'
import type { ConversationMessageActions } from './history/actions'
import type { ConversationMessageReader } from './history/types'
import type { ComposerAttachmentInput } from './input/attachmentDraft'
import type { MentionCandidate } from './input/mentions'
import type { MessageObject } from './model/msgobj'
import type { OutgoingPayload } from './model/outgoing'
import type { PinnedMessage, ReadReceipt, RuntimeState, SessionAccess, SessionPanelInfo } from './model/types'

/** The rules of a group conversation. */
export interface GroupConversationRules {
  /** Offered while typing `@` and by the @ button. */
  mentionCandidates: MentionCandidate[]
  canMentionAll: boolean
  /** How long an own message may be edited / recalled; undefined: without limit, 0: never. */
  editWindowMs?: number
  recallWindowMs?: number
  /** The viewer may ask everyone to delete any message. */
  redactAny: boolean
  /** The sender's picture beside a peer's bubbles. */
  senderAvatar?: (did: DID, name: string) => ReactNode
}

/**
 * One conversation as the conversation component sees it: a snapshot the host builds again whenever its data
 * changes (MessageHub's: `app/messagehub/conversationSource.tsx`). Optional members are capabilities; what is left
 * out is not offered.
 */
export interface ConversationSource {
  /** Empty while the conversation has no session yet. */
  sessionId: string
  /** Whose messages are the viewer's own: the owner of the mailbox shown. */
  selfDid: DID
  reader: ConversationMessageReader
  historyStatus: 'idle' | 'loading' | 'ready' | 'error'
  hasOlder: boolean
  loadOlder(): Promise<boolean>
  /** Loads older pages until the message with this ObjId is in the history; resolves to whether it is. */
  locate?(messageId: string): Promise<boolean>
  markRead(recordIds: string[]): void
  /** null: nothing can be sent (there is no session). */
  access: SessionAccess | null
  /** Why nothing can be sent when `access` gives no reason, as a `messagehub.reason.*` key. */
  unavailableReason?: string
  /** The viewer acts in this conversation (replies, reactions, pins, forwards); false for an observer. */
  canAct: boolean
  /** false: text only, without files, folders or voice notes. */
  attachments?: boolean
  send(payload: OutgoingPayload): Promise<void>
  /** Bubbles of what the composer submitted and has not sent yet; replaces the previous set. */
  stageOutgoing?(queued: OutgoingPayload[]): void
  resend?(message: MessageObject): Promise<void>
  /** Removes a message from the viewer's own view; other participants keep it. */
  remove?(message: MessageObject): Promise<void>
  /** Attachments left the draft: what was prepared for their upload can go. */
  releaseAttachments?(attachments: ComposerAttachmentInput[]): void
  draft?: {
    text: string
    attachments: ComposerAttachmentInput[]
    saveText(value: string): Promise<void> | undefined
    saveAttachments(attachments: ComposerAttachmentInput[]): Promise<void> | undefined
  }
  /** Name of a participant; the component itself names `selfDid`. */
  displayName(did: DID): string
  runtime: RuntimeState[]
  preferences: { showActions: boolean; pinned: PinnedMessage | null; setPinned?(pinned: PinnedMessage | null): Promise<void> }
  /** A host-defined panel over the history (the state of a ticket); shown instead of the pinned message. */
  panel?: SessionPanelInfo | null
  readReceipt?(message: MessageObject): ReadReceipt | null
  /** The peer is an Agent: its text renders as Markdown. */
  peerMarkdown?: boolean
  /** The peer is one of the viewer's own Agents: the viewer controls its Turns and answers its questions. */
  agent?: { did: DID; name: string }
  /** Pause / resume / stop of the Agent sessions the conversation shows. */
  agentControls?: AgentSessionControls
  group?: GroupConversationRules
  /** The time relative labels follow; a local clock without it. */
  clock?: ConversationClock
  /** The current time for edit and recall windows; `Date.now` without it. */
  now?(): number
  /** What a preview opened from the conversation belongs to; the session without it. */
  hostContext?: string
  /** Actions the host's own message renderers use (`registerMessageRenderer`). */
  messageActions?: Partial<ConversationMessageActions>
}
