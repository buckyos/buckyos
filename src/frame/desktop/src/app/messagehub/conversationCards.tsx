/* The message cards that act on MessageHub's own data (notifications, friend requests, group notices). They join the
 * conversation component's renderer chain from `conversationBackend.ts`; a conversation without them shows these
 * messages as ordinary ones. */

import { useContext, useState } from 'react'
import { TriangleAlert, UserPlus, Users } from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import { networkActions } from '../../components/network/cache'
import { useNetworkDialogs } from '../../components/network/dialogs'
import { useNetworkResource } from '../../components/network/hooks'
import { networkErrorText } from '../../components/network/relation'
import { ConversationMessageActionsContext } from '../../components/conversation/history/actions'
import { bubbleStyle, bubbleWidthClass, type MessageRenderContext } from '../../components/conversation/history/bubble'
import { FallbackMessage, MessageFooter, MessageRow } from '../../components/conversation/history/renderers'
import { effectiveContent, messageObjId } from '../../components/conversation/history/relations'
import { parseFriendNotice } from '../../components/conversation/model/friendNotice'
import { groupErrorCode, groupErrorText, isAgentGroupDisabled, parseGroupNotice } from '../../components/conversation/model/groupNotice'
import { messageNotify, type MessageObject } from '../../components/conversation/model/msgobj'
import { AgentGroupSettingsLink } from './AgentGroupSettingsLink'
import { NotifyCard } from './notify/NotifyCard'
import { useEntityRelation } from './relation'

/** A System Notify card: the record in a category session, or a copy in a conversation with its Agent. */
export function NotifyMessage({ message, context }: { message: MessageObject; context: MessageRenderContext }) {
  const data = messageNotify(message)
  if (!data) return null
  return (
    <MessageRow message={message} isSelf={false} context={context}>
      <NotifyCard message={message} data={data} className={`${bubbleWidthClass} w-[420px] min-w-[240px]`} style={{ ...bubbleStyle(false, context.continued), padding: 12 }} footer={<MessageFooter message={message} isSelf={false} mode="card" />} />
    </MessageRow>
  )
}

/** The Agent says a fallback model answered: shown apart from its replies, in the Agent's own words. */
export function RouteFallbackMessage({ message, context }: { message: MessageObject; context: MessageRenderContext }) {
  return (
    <MessageRow message={message} isSelf={false} context={context}>
      <div role="note" className={`${bubbleWidthClass} min-w-[160px]`} style={{ ...bubbleStyle(false, context.continued), background: 'color-mix(in srgb, var(--cp-warning) 12%, var(--cp-surface))', borderColor: 'color-mix(in srgb, var(--cp-warning) 40%, transparent)' }} data-testid="route-fallback-notice" data-message-id={messageObjId(message)}>
        <p className="flex items-start gap-1.5 whitespace-pre-wrap break-words text-[13px]"><TriangleAlert size={14} className="mt-0.5 shrink-0" style={{ color: 'var(--cp-warning)' }} aria-hidden />{effectiveContent(message).content}</p>
        <MessageFooter message={message} isSelf={false} selfDid={context.selfDid} mode="card" />
      </div>
    </MessageRow>
  )
}

/** A `buckyos.friend` request: the request's current state from msg-center, and its answers for the owner. */
export function FriendRequestMessage({ message, context }: { message: MessageObject; context: MessageRenderContext }) {
  const { t } = useI18n()
  const actions = useContext(ConversationMessageActionsContext)
  const dialogs = useNetworkDialogs()
  const requests = useNetworkResource('friendRequests')
  const notice = parseFriendNotice(message)
  const outgoing = message.from === context.selfDid
  const peer = outgoing ? message.to?.[0] ?? '' : message.from
  const relation = useEntityRelation(peer || null)
  const [pending, setPending] = useState(false)
  const [error, setError] = useState('')
  if (!notice || notice.action !== 'request') return <FallbackMessage message={message} context={context} />
  const request = requests.data?.find(item => item.request_id === notice.requestId)
  const state = request?.state ?? (relation?.friendState === 'friend' ? 'accepted' : undefined)
  const name = actions.displayName?.(peer) ?? notice.profileHint?.name ?? peer
  const canAnswer = actions.answerFriendRequests && !outgoing && state === 'pending'
  const run = async (operation: () => Promise<unknown>) => {
    setPending(true); setError('')
    try { await operation() } catch (failure) { setError(networkErrorText(t, failure)) } finally { setPending(false) }
  }
  const block = async () => {
    const confirmed = await dialogs.confirm({ strength: 'explicit', danger: true, title: t('network.confirm.block.title', undefined, { name }), body: t('network.confirm.block.body', undefined, { name }), consequences: [t('network.confirm.block.c1'), t('network.confirm.block.c2')], confirmLabel: t('network.action.block') })
    if (!confirmed) return
    await run(async () => {
      await networkActions.respondFriendRequest(notice.requestId, 'ignore')
      await networkActions.blockContact(peer)
    })
  }
  const primaryClass = 'min-h-11 rounded-lg px-3 text-sm font-medium text-[color:var(--cp-surface-opaque)] disabled:opacity-50 md:min-h-9'
  const secondaryClass = 'min-h-11 rounded-lg border px-3 text-sm disabled:opacity-50 md:min-h-9'
  return (
    <MessageRow message={message} isSelf={outgoing} context={context}>
      <div className={`${bubbleWidthClass} w-72 min-w-[220px]`} style={{ ...bubbleStyle(false, context.continued), padding: 12 }} data-testid="friend-notice" data-state={state ?? 'unknown'}>
        <div className="flex items-center gap-2.5">
          <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full" style={{ background: 'color-mix(in srgb, var(--cp-accent) 16%, transparent)', color: 'var(--cp-accent)' }} aria-hidden><UserPlus size={17} /></span>
          <div className="min-w-0 flex-1">
            <p className="truncate text-xs" style={{ color: 'var(--cp-muted)' }}>{t(outgoing ? 'messagehub.friend.sent' : 'messagehub.friend.received')}</p>
            <p className="truncate text-[15px] font-medium" title={peer}>{name}</p>
          </div>
        </div>
        {notice.message ? <p className="mt-2 whitespace-pre-wrap break-words text-sm">{notice.message}</p> : null}
        <div className="mt-3 flex flex-wrap items-center gap-2">
          {canAnswer ? <>
            <button type="button" className={primaryClass} style={{ background: 'var(--cp-accent)' }} disabled={pending} onClick={() => void run(() => networkActions.respondFriendRequest(notice.requestId, 'accept'))}>{t('network.action.accept_request')}</button>
            <button type="button" className={secondaryClass} style={{ borderColor: 'var(--cp-border)' }} disabled={pending} onClick={() => void run(() => networkActions.respondFriendRequest(notice.requestId, 'ignore'))}>{t('myNetwork.requests.ignore')}</button>
            <button type="button" className={secondaryClass} style={{ borderColor: 'var(--cp-border)', color: 'var(--cp-danger)' }} disabled={pending} onClick={() => void block()}>{t('network.action.block')}</button>
          </> : state ? <span className="text-xs font-medium" style={{ color: state === 'accepted' ? 'var(--cp-success)' : 'var(--cp-muted)' }} data-testid="friend-notice-state">{t(`messagehub.friend.state.${state}${outgoing && state === 'pending' ? 'Outgoing' : ''}`, undefined, { name })}</span> : null}
          {actions.answerFriendRequests && outgoing && state === 'pending' ? <button type="button" className={secondaryClass} style={{ borderColor: 'var(--cp-border)' }} disabled={pending} onClick={() => void run(() => networkActions.cancelFriendRequest(notice.requestId))}>{t('network.action.cancel_request')}</button> : null}
        </div>
        {error ? <p role="alert" className="mt-2 text-xs" style={{ color: 'var(--cp-danger)' }}>{error}</p> : null}
        <MessageFooter message={message} isSelf={outgoing} mode="card" />
      </div>
    </MessageRow>
  )
}

export function GroupNoticeMessage({ message, context }: { message: MessageObject; context: MessageRenderContext }) {
  const { t } = useI18n()
  const actions = useContext(ConversationMessageActionsContext)
  const [state, setState] = useState<'idle' | 'pending' | 'done'>('idle')
  const [outcome, setOutcome] = useState('')
  const [error, setError] = useState('')
  const [errorReason, setErrorReason] = useState('')
  const [agentGroupDisabled, setAgentGroupDisabled] = useState(false)
  const notice = parseGroupNotice(message)
  const view = notice ? actions.groupNotice?.(message) : null
  if (!notice || !view) return <FallbackMessage message={message} context={context} />
  const nameOf = (did: string) => actions.displayName?.(did) ?? did
  const inviter = nameOf(notice.by ?? message.from)
  const invitation = notice.invitation
  const run = (operation: ((message: MessageObject) => Promise<void>) | undefined, doneKey: string) => {
    if (!operation || state === 'pending') return
    setState('pending'); setError(''); setAgentGroupDisabled(false)
    void operation(message).then(() => { setState('done'); setOutcome(doneKey) }, failure => { setState('idle'); setError(groupErrorText(t, failure)); setErrorReason(groupErrorCode(failure)); setAgentGroupDisabled(isAgentGroupDisabled(failure)) })
  }
  const forAgent = invitation?.memberDid && invitation.memberDid !== context.selfDid ? nameOf(invitation.memberDid) : undefined
  const line = invitation ? (forAgent ? t('messagehub.group.invitedAgent', undefined, { name: inviter, agent: forAgent }) : t('messagehub.group.invitedBy', undefined, { name: inviter }))
    : notice.action === 'pending_approval' ? t('messagehub.group.notice.pending_approval', undefined, { name: notice.memberDid ? nameOf(notice.memberDid) : inviter })
    : notice.action === 'owner_transfer' ? t('messagehub.group.notice.owner_transfer', undefined, { name: inviter })
    : notice.action === 'session_invite' ? t('messagehub.group.notice.session_invite', undefined, { name: inviter, session: notice.sessionTitle || notice.sessionId || '' })
    : notice.action === 'rejected' || notice.action === 'removed' || notice.action === 'session_removed' ? t(`messagehub.group.notice.${notice.action}`, undefined, { name: inviter })
    : notice.action
  const primaryClass = 'min-h-11 rounded-lg px-3 text-sm font-medium text-[color:var(--cp-surface-opaque)] disabled:opacity-50 md:min-h-9'
  const secondaryClass = 'min-h-11 rounded-lg border px-3 text-sm disabled:opacity-50 md:min-h-9'
  const pendingLabel = (key: string) => t(state === 'pending' ? 'messagehub.saving' : key)
  return (
    <MessageRow message={message} isSelf={false} context={context}>
      <div className={`${bubbleWidthClass} w-72 min-w-[220px]`} style={{ ...bubbleStyle(false, context.continued), padding: 12 }} data-testid="group-notice" data-action={notice.action} data-state={view.state ?? (state === 'done' ? outcome : undefined)}>
        <div className="flex items-center gap-2.5">
          <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full" style={{ background: 'color-mix(in srgb, var(--cp-warning) 18%, transparent)', color: 'var(--cp-warning)' }} aria-hidden><Users size={17} /></span>
          <div className="min-w-0 flex-1">
            <p className="truncate text-xs" style={{ color: 'var(--cp-muted)' }}>{line}</p>
            <p className="truncate text-[15px] font-medium" title={notice.groupDid}>{view.groupName}</p>
          </div>
        </div>
        {invitation && invitation.role !== 'member' ? <p className="mt-2 text-xs" style={{ color: 'var(--cp-muted)' }}>{t('messagehub.group.invitedAs', undefined, { role: t(`messagehub.group.role.${invitation.role}`) })}</p> : null}
        {invitation ? (
          <div className="mt-3 flex flex-wrap items-center gap-2">
            {view.state === 'pending' && actions.joinGroup ? <button type="button" className={primaryClass} style={{ background: 'var(--cp-accent)' }} disabled={state === 'pending'} onClick={() => run(actions.joinGroup, 'joined')}>{t(state === 'pending' ? 'messagehub.group.joining' : forAgent ? 'messagehub.group.acceptForAgent' : 'messagehub.group.join')}</button> : null}
            {view.state === 'syncing' ? <>
              <span className="text-xs font-medium" style={{ color: 'var(--cp-muted)' }}>{t('messagehub.group.syncPending')}</span>
              {actions.joinGroup ? <button type="button" className={secondaryClass} style={{ borderColor: 'var(--cp-border)' }} disabled={state === 'pending'} onClick={() => run(actions.joinGroup, 'joined')}>{t(state === 'pending' ? 'messagehub.group.joining' : 'messagehub.retry')}</button> : null}
            </> : null}
            {view.state === 'joined' ? <>
              <span className="text-xs font-medium" style={{ color: 'var(--cp-success)' }}>{t(forAgent ? 'messagehub.group.agentJoined' : 'messagehub.group.joined')}</span>
              {actions.openEntity && !forAgent ? <button type="button" className={secondaryClass} style={{ borderColor: 'var(--cp-border)' }} onClick={() => actions.openEntity?.(notice.groupDid)}>{t('messagehub.group.open')}</button> : null}
            </> : null}
            {view.state === 'approval' ? <span className="text-xs font-medium" style={{ color: 'var(--cp-muted)' }}>{t('messagehub.group.acceptedPendingApproval')}</span> : null}
            {view.state === 'expired' ? <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('messagehub.group.invitationExpired')}</span> : null}
            {view.state === 'pending' && invitation.expiresAt ? <span className="text-[11px]" style={{ color: 'var(--cp-muted)' }}>{t('messagehub.group.expiresAt', undefined, { time: new Date(invitation.expiresAt).toLocaleDateString() })}</span> : null}
          </div>
        ) : notice.action === 'pending_approval' && (actions.approveMember || actions.rejectMember) ? (
          <div className="mt-3 flex flex-wrap items-center gap-2">
            {state === 'done' ? <span className="text-xs font-medium" style={{ color: outcome === 'approved' ? 'var(--cp-success)' : 'var(--cp-muted)' }}>{t(`messagehub.group.${outcome}`)}</span> : <>
              <button type="button" className={primaryClass} style={{ background: 'var(--cp-accent)' }} disabled={state === 'pending'} onClick={() => run(actions.approveMember, 'approved')}>{pendingLabel('messagehub.group.approve')}</button>
              <button type="button" className={secondaryClass} style={{ borderColor: 'var(--cp-border)' }} disabled={state === 'pending'} onClick={() => run(actions.rejectMember, 'rejected')}>{pendingLabel('messagehub.group.reject')}</button>
            </>}
          </div>
        ) : notice.action === 'owner_transfer' && actions.acceptOwnerTransfer ? (
          <div className="mt-3 flex flex-wrap items-center gap-2">
            {state === 'done' ? <span className="text-xs font-medium" style={{ color: 'var(--cp-success)' }}>{t('messagehub.group.transferAccepted')}</span>
              : <button type="button" className={primaryClass} style={{ background: 'var(--cp-accent)' }} disabled={state === 'pending'} onClick={() => run(actions.acceptOwnerTransfer, 'transferred')}>{pendingLabel('messagehub.group.acceptTransfer')}</button>}
          </div>
        ) : notice.action === 'session_invite' && actions.acceptSessionInvitation ? (
          <div className="mt-3 flex flex-wrap items-center gap-2">
            {state === 'done' ? <>
              <span className="text-xs font-medium" style={{ color: 'var(--cp-success)' }}>{t('messagehub.group.joined')}</span>
              {actions.openEntity ? <button type="button" className={secondaryClass} style={{ borderColor: 'var(--cp-border)' }} onClick={() => actions.openEntity?.(notice.groupDid)}>{t('messagehub.group.open')}</button> : null}
            </> : <button type="button" className={primaryClass} style={{ background: 'var(--cp-accent)' }} disabled={state === 'pending'} onClick={() => run(actions.acceptSessionInvitation, 'joined')}>{pendingLabel('messagehub.group.acceptSessionInvite')}</button>}
          </div>
        ) : null}
        {error && !(view.state === 'joined' && ['group-sync-pending', 'group-refresh-pending'].includes(errorReason)) ? <p role="alert" className="mt-2 text-xs" style={{ color: 'var(--cp-danger)' }}>{error}</p> : null}
        {agentGroupDisabled && invitation?.memberDid ? <AgentGroupSettingsLink agentDid={invitation.memberDid} name={nameOf(invitation.memberDid)} /> : null}
        <MessageFooter message={message} isSelf={false} mode="card" />
      </div>
    </MessageRow>
  )
}
