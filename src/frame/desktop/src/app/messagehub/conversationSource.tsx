/* C5: the conversation component's view of one MessageHub session. The store stays the single owner of sync,
 * drafts and pending bubbles; this only narrows store + context + session to `ConversationSource`. Importing it
 * also joins MessageHub's own message cards to the component's renderer chain. */

import { useEffect } from 'react'
import type { DID } from 'buckyos'
import { InMemoryConversationMessageReader } from '../../components/conversation/history/data-source'
import { registerMessageRenderer } from '../../components/conversation/history/messageRenderers'
import { parseFriendNotice } from '../../components/conversation/model/friendNotice'
import { parseGroupNotice, type GroupNotice } from '../../components/conversation/model/groupNotice'
import { messageNotify, type MessageObject } from '../../components/conversation/model/msgobj'
import type { ConversationClock } from '../../components/conversation/clock'
import type { ConversationSource } from '../../components/conversation/source'
import { getAgentPanelSource } from './agentpanel/source'
import { friendlyDidName } from './api/projection'
import { releaseAttachmentUploads } from './api/upload'
import { FriendRequestMessage, GroupNoticeMessage, NotifyMessage } from './conversationCards'
import { SenderAvatar } from './HubAvatar'
import { channelAccess, isChannelSession, readingContext } from './sessionModel'
import { getMessageHubStore, useMessageHubStore, type MessageHubStore } from './store'
import type { MessageHubContext, Session } from './types'

registerMessageRenderer('notice', (message, context) => parseFriendNotice(message) ? <FriendRequestMessage message={message} context={context} /> : null)
registerMessageRenderer('notice', (message, context) => parseGroupNotice(message) ? <GroupNoticeMessage message={message} context={context} /> : null)
registerMessageRenderer('card', (message, context) => messageNotify(message) ? <NotifyMessage message={message} context={context} /> : null)

const EMPTY_READER = InMemoryConversationMessageReader.empty()
const clocks = new WeakMap<MessageHubStore, ConversationClock>()
/** The store's clock, for the parts of MessageHub that show conversation pieces outside a conversation. */
export const messageHubClock = (store: MessageHubStore) => {
  let clock = clocks.get(store)
  if (!clock) { clock = { subscribe: store.subscribeTime, now: store.getTime, tick: store.tick }; clocks.set(store, clock) }
  return clock
}

/** A session of the viewer's own messages, active or archived. */
export function findSession(context: MessageHubContext, sessionId: string): Session | null {
  const store = getMessageHubStore()
  return (['active', 'archived'] as const).flatMap(lifecycle => store.sessions(context, undefined, lifecycle)).find(session => session.id === sessionId) ?? null
}

export interface MessageHubSourceOptions {
  context: MessageHubContext
  /** The entity the conversation is with. */
  entityId: string
  /** null while the entity has no session yet: nothing can be sent. */
  session: Session | null
  /** The write confirmation of a tunnel session (MessageHub's session details). */
  confirmation?: string
  confirmed?: boolean
  /** Why no session can be created for the entity, shown when there is none. */
  creationReason?: string
}

/** The source of a MessageHub session; re-renders the caller with the store. */
export function useMessageHubConversationSource({ context, entityId, session, confirmation, confirmed = false, creationReason }: MessageHubSourceOptions): ConversationSource {
  const store = useMessageHubStore()
  const sessionId = session?.id ?? ''
  const entity = store.findEntity(context, entityId)
  const isGroup = entity?.type === 'group'
  // A conversation on a channel is read from the Agent's mailbox, shown from the viewer's side of it.
  const channel = !!session && isChannelSession(context, session)
  const reading = session ? readingContext(context, session) : context
  const isOwner = !channel && context.mode === 'self' && context.ownerDid === context.viewerDid
  const group = isGroup ? store.group(context, entityId) : null
  const groupMembers = group?.members
  const groupKnown = group !== null
  useEffect(() => { if (groupKnown && sessionId) void store.ensureGroupSessionMembers(context, entityId, sessionId) }, [store, groupKnown, entityId, sessionId, context.ownerDid, context.viewerDid, context.mode]) // eslint-disable-line react-hooks/exhaustive-deps
  // Mentions go to the session's participants (guests included); the group's members stand in until those are known.
  const sessionMembers = isGroup && sessionId ? store.groupSessionMembers(context, entityId, sessionId) : null
  const mentionDids = sessionMembers ? sessionMembers.items.filter(item => item.state === 'included').map(item => item.did) : (groupMembers ?? []).filter(member => member.state === 'active').map(member => member.did)
  const mentionCandidates = mentionDids.filter(did => did !== context.ownerDid).map(did => ({ did, name: store.findEntity(context, did)?.name ?? friendlyDidName(did, false) }))

  const nameOf = (did: DID) => store.findEntity(context, did)?.name ?? friendlyDidName(did, false)
  const notice = (message: MessageObject, operation: (notice: GroupNotice) => Promise<void>) => {
    const parsed = parseGroupNotice(message)
    return parsed ? operation(parsed) : Promise.reject(Error('not-found'))
  }
  const preferences = session ? store.preferences(reading, sessionId) : null
  const ownAgent = isOwner && store.isOwnAgent(context, entityId)
  const missing = () => Promise.reject(Error('session_missing'))
  return {
    sessionId,
    selfDid: channel && session.binding.kind === 'tunnel' ? session.binding.endpointDid : context.ownerDid,
    reader: session ? store.reader(reading, sessionId) : EMPTY_READER,
    historyStatus: session ? store.historyStatus(reading, sessionId) : 'ready',
    hasOlder: session ? store.hasOlder(reading, sessionId) : false,
    loadOlder: () => session ? store.loadOlder(reading, sessionId) : Promise.resolve(false),
    locate: (messageId) => store.locateMessage(reading, sessionId, messageId),
    markRead: (recordIds) => { if (session) void store.markRead(reading, sessionId, recordIds) },
    access: !session ? null : channel ? channelAccess : store.access(context, session, confirmed),
    unavailableReason: creationReason,
    canAct: isOwner,
    send: (payload) => session ? store.send(context, sessionId, payload, confirmation) : missing(),
    stageOutgoing: (queued) => { if (session) store.stageOutgoing(context, sessionId, queued) },
    resend: (message) => session ? store.resend(context, sessionId, message, confirmation) : missing(),
    remove: isOwner ? (message) => store.deleteMessage(context, sessionId, message) : undefined,
    releaseAttachments: releaseAttachmentUploads,
    draft: {
      text: session ? store.draft(context, sessionId) : '',
      attachments: session ? store.attachments(context, sessionId) : [],
      saveText: (value) => session ? store.saveDraft(context, sessionId, value) : undefined,
      saveAttachments: (attachments) => session ? store.saveAttachments(context, sessionId, attachments) : undefined,
    },
    displayName: nameOf,
    runtime: session ? store.runtimeFor(reading, sessionId) : [],
    preferences: {
      showActions: preferences?.showActions ?? true,
      pinned: preferences?.pinnedMessage ?? null,
      setPinned: isOwner && session ? (pinned) => store.updatePreferences(context, sessionId, { pinnedMessage: pinned }) : undefined,
    },
    panel: session?.panel ?? null,
    readReceipt: (message) => store.readReceipt(reading, sessionId, message),
    peerMarkdown: entity?.type === 'agent',
    agent: ownAgent ? { did: entityId, name: nameOf(entityId) } : undefined,
    agentControls: getAgentPanelSource(),
    group: isGroup ? {
      mentionCandidates,
      canMentionAll: group?.can.mentionAll ?? false,
      editWindowMs: group?.messageRules.editWindowMs,
      recallWindowMs: group?.messageRules.recallWindowMs,
      redactAny: group?.can.redactAny ?? false,
      senderAvatar: (did, name) => <SenderAvatar store={store} context={context} did={did} name={name} />,
    } : undefined,
    clock: messageHubClock(store),
    now: store.now,
    messageActions: {
      groupNotice: (message) => {
        const parsed = parseGroupNotice(message)
        if (!parsed) return null
        return parsed.invitation ? store.groupInvitation(context, parsed.invitation) : { groupName: store.findEntity(context, parsed.groupDid)?.name ?? friendlyDidName(parsed.groupDid, false) }
      },
      joinGroup: isOwner ? (message) => notice(message, parsed => parsed.invitation ? store.acceptGroupInvitation(context, parsed.invitation) : Promise.reject(Error('not-found'))) : undefined,
      approveMember: isOwner ? (message) => notice(message, parsed => parsed.memberDid ? store.approveGroupMember(context, parsed.groupDid, parsed.memberDid) : Promise.reject(Error('not-found'))) : undefined,
      rejectMember: isOwner ? (message) => notice(message, parsed => parsed.memberDid ? store.rejectGroupMember(context, parsed.groupDid, parsed.memberDid) : Promise.reject(Error('not-found'))) : undefined,
      acceptOwnerTransfer: isOwner ? (message) => notice(message, parsed => parsed.transferId ? store.acceptGroupOwnerTransfer(context, parsed.groupDid, parsed.transferId) : Promise.reject(Error('transfer-mismatch'))) : undefined,
      acceptSessionInvitation: isOwner ? (message) => notice(message, parsed => parsed.sessionId ? store.acceptGroupSessionInvitation(context, parsed.groupDid, parsed.sessionId) : Promise.reject(Error('not-found'))) : undefined,
      handleNotify: isOwner,
      answerFriendRequests: isOwner,
    },
  }
}
