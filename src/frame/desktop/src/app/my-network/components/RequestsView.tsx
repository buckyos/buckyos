/* ── Requests (PRD §6.4, TODO §3.6): friend requests, message requests, group invitations, join requests ── */

import { Button } from '@mui/material'
import { Ban, Check, MessageSquare, RefreshCw, Send, ShieldCheck, UserPlus, X } from 'lucide-react'
import { useState, type ReactNode } from 'react'
import { useI18n } from '../../../i18n/provider'
import { networkActions } from '../../../components/network/cache'
import { useNetworkDialogs } from '../../../components/network/dialogs'
import { useLookup, useRequests } from '../../../components/network/hooks'
import { ListSkeleton, Pill, PersonAvatar, PersonName, SectionTitle, StateMessage } from '../../../components/network/Person'
import { networkErrorText } from '../../../components/network/relation'
import type { RequestItem } from '../../../components/network/types'
import { openMessageHub } from '../datamodel/links'
import { dangerOutlinedSx } from '../../../components/network/buttonStyles'
import { useContactEditPermission } from '../../../components/network/permissions'

type Run = (key: string, action: () => Promise<unknown>) => Promise<void>

function when(ms: number, locale: string) {
  return new Date(ms).toLocaleString(locale, { dateStyle: 'medium', timeStyle: 'short' })
}

function GroupName({ did, fallback }: { did: string; fallback?: string }) {
  const lookup = useLookup(fallback ? '' : did, { kinds: ['group'], enabled: !fallback, debounceMs: 0 })
  const name = fallback ?? lookup.result?.candidates.find(item => item.did === did)?.name
  return <span className="font-medium">{name ?? did}</span>
}

function RequestCard({ avatar, title, meta, body, actions, testId, kind }: { avatar: ReactNode; title: ReactNode; meta?: ReactNode; body?: ReactNode; actions: ReactNode; testId: string; kind: RequestItem['type'] }) {
  return (
    <li className="rounded-2xl border border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-4 py-3" data-testid={testId} data-kind={kind}>
      <div className="flex items-start gap-3">
        {avatar}
        <div className="min-w-0 flex-1 space-y-1">
          <div className="text-sm text-[color:var(--cp-text)]">{title}</div>
          {meta ? <div className="text-[11px] text-[color:var(--cp-muted)]">{meta}</div> : null}
          {body ? <div className="text-[13px] text-[color:var(--cp-text)]">{body}</div> : null}
          <div className="flex flex-wrap gap-1.5 pt-1">{actions}</div>
        </div>
      </div>
    </li>
  )
}

/** Request answers that change the address book; hidden for accounts that may not edit it. */
const CONTACT_WRITES = new Set(['accept', 'ignore', 'reject', 'block', 'add_friend', 'allow', 'cancel', 'resend'])

function Item({ item, run, busy }: { item: RequestItem; run: Run; busy: string | null }) {
  const { t, locale } = useI18n()
  const dialogs = useNetworkDialogs()
  const canEditContacts = useContactEditPermission()
  const disabled = busy !== null
  const contactWrite = item.type === 'friend_request' || item.type === 'message_request'
  const action = (key: string, label: string, icon: ReactNode, task: () => Promise<unknown>, variant: 'contained' | 'outlined' | 'text' = 'outlined', danger = false) => (
    contactWrite && !canEditContacts && CONTACT_WRITES.has(key) ? null : (
      <Button key={key} size="small" variant={variant} sx={danger ? dangerOutlinedSx : undefined} startIcon={icon} disabled={disabled} onClick={() => void run(`${item.key}:${key}`, task)} data-action={key}>
        {label}
      </Button>
    )
  )
  const confirmBlock = async (name: string) => {
    const confirmed = await dialogs.confirm({
      strength: 'explicit',
      danger: true,
      title: t('network.confirm.block.title', undefined, { name }),
      body: t('network.confirm.block.body', undefined, { name }),
      consequences: [t('network.confirm.block.c1'), t('network.confirm.block.c2')],
      confirmLabel: t('network.action.block'),
    })
    return Boolean(confirmed)
  }

  if (item.type === 'friend_request') {
    const request = item.request
    const person = { did: request.peer_did, name: request.peer_name, avatar: request.peer_avatar }
    const name = request.peer_name || request.peer_did
    if (request.direction === 'outgoing') {
      const delivery = request.delivery?.state
      return (
        <RequestCard
          kind={item.type}
          testId="request-friend-outgoing"
          avatar={<PersonAvatar person={person} size={36} />}
          title={<><PersonName person={person} className="font-medium" /> <Pill tone={delivery === 'failed' ? 'var(--cp-danger)' : delivery === 'pending' ? 'var(--cp-warning)' : 'var(--cp-accent)'}>{t(`myNetwork.requests.delivery.${delivery ?? 'unknown'}`)}</Pill></>}
          meta={t('myNetwork.requests.sentAt', undefined, { time: when(request.created_at_ms, locale) })}
          body={request.message}
          actions={<>
            {delivery === 'failed' ? action('resend', t('myNetwork.requests.resend'), <RefreshCw size={13} />, () => networkActions.sendFriendRequest(request.peer_did), 'contained') : null}
            {action('cancel', t('network.action.cancel_request'), <X size={13} />, () => networkActions.cancelFriendRequest(request.request_id))}
          </>}
        />
      )
    }
    return (
      <RequestCard
        kind={item.type}
        testId="request-friend-incoming"
        avatar={<PersonAvatar person={person} size={36} />}
        title={<span><PersonName person={person} className="font-medium" /> {t('myNetwork.requests.wantsFriend')}</span>}
        meta={t('myNetwork.requests.receivedAt', undefined, { time: when(request.created_at_ms, locale) })}
        body={request.message}
        actions={<>
          {action('accept', t('network.action.accept_request'), <Check size={13} />, () => networkActions.respondFriendRequest(request.request_id, 'accept'), 'contained')}
          {action('ignore', t('myNetwork.requests.ignore'), <X size={13} />, () => networkActions.respondFriendRequest(request.request_id, 'ignore'), 'text')}
          {action('reject', t('myNetwork.requests.rejectAndTell'), <Send size={13} />, () => networkActions.respondFriendRequest(request.request_id, 'reject'))}
          {action('block', t('network.action.block'), <Ban size={13} />, async () => {
            if (!await confirmBlock(name)) return
            await networkActions.respondFriendRequest(request.request_id, 'ignore')
            await networkActions.blockContact(request.peer_did)
          }, 'outlined', true)}
        </>}
      />
    )
  }

  if (item.type === 'message_request') {
    const request = item.request
    const person = { did: request.senderDid, name: request.senderName }
    const name = request.senderName || request.senderDid
    return (
      <RequestCard
        kind={item.type}
        testId="request-message"
        avatar={<PersonAvatar person={person} size={36} />}
        title={<span><PersonName person={person} className="font-medium" /> {t('myNetwork.requests.messages', undefined, { count: request.count })}</span>}
        meta={when(request.lastAtMs, locale)}
        body={request.preview ? <span className="line-clamp-2 text-[color:var(--cp-muted)]">“{request.preview}”</span> : undefined}
        actions={<>
          {action('add_friend', t('network.action.add_friend'), <UserPlus size={13} />, () => dialogs.addFriend({ did: request.senderDid }), 'contained')}
          {action('allow', t('network.action.allow_messages'), <ShieldCheck size={13} />, () => networkActions.allowMessages(request.senderDid, request.senderName))}
          {action('open', t('myNetwork.requests.read'), <MessageSquare size={13} />, () => openMessageHub(request.senderDid, request.sessionId), 'text')}
          {action('block', t('network.action.block'), <Ban size={13} />, async () => {
            if (await confirmBlock(name)) await networkActions.blockContact(request.senderDid)
          }, 'outlined', true)}
        </>}
      />
    )
  }

  if (item.type === 'group_invitation') {
    const invitation = item.invitation
    const inviter = { did: invitation.inviterDid }
    return (
      <RequestCard
        kind={item.type}
        testId="request-group-invitation"
        avatar={<PersonAvatar person={{ did: invitation.groupDid, kind: 'group' }} size={36} />}
        title={<span>{t('myNetwork.requests.invitedBy')} <PersonName person={inviter} className="font-medium" /> · <GroupName did={invitation.groupDid} fallback={invitation.groupName} /></span>}
        meta={invitation.expiresAtMs ? t('myNetwork.requests.expiresAt', undefined, { time: when(invitation.expiresAtMs, locale) }) : undefined}
        actions={<>
          {action('accept', t('myNetwork.requests.acceptInvite'), <Check size={13} />, () => networkActions.acceptGroupInvitation(invitation), 'contained')}
          {action('ignore', t('myNetwork.requests.ignore'), <X size={13} />, () => networkActions.dismissGroupInvitation(invitation), 'text')}
        </>}
      />
    )
  }

  const request = item.request
  const member = { did: request.memberDid, kind: request.entityKind === 'agent' ? 'agent' as const : 'user' as const }
  return (
    <RequestCard
      kind={item.type}
      testId="request-join"
      avatar={<PersonAvatar person={member} size={36} />}
      title={<span><PersonName person={member} className="font-medium" /> {t('myNetwork.requests.wantsToJoin')} <GroupName did={request.groupDid} fallback={request.groupName} /></span>}
      actions={<>
        {action('approve', t('myNetwork.requests.approve'), <Check size={13} />, () => networkActions.approveJoinRequest(request), 'contained')}
        {action('reject', t('myNetwork.requests.reject'), <X size={13} />, () => networkActions.rejectJoinRequest(request))}
      </>}
    />
  )
}

const GROUPS: Array<{ key: string; match: (item: RequestItem) => boolean }> = [
  { key: 'friendIncoming', match: item => item.type === 'friend_request' && item.request.direction === 'incoming' },
  { key: 'message', match: item => item.type === 'message_request' },
  { key: 'invitation', match: item => item.type === 'group_invitation' },
  { key: 'join', match: item => item.type === 'join_request' },
  { key: 'friendOutgoing', match: item => item.type === 'friend_request' && item.request.direction === 'outgoing' },
]

export function RequestsView() {
  const { t } = useI18n()
  const requests = useRequests()
  const [busy, setBusy] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const run: Run = async (key, action) => {
    setBusy(key)
    setError(null)
    try { await action() } catch (failure) { setError(networkErrorText(t, failure)) } finally { setBusy(null) }
  }
  return (
    <div className="desktop-scrollbar h-full min-w-0 flex-1 overflow-y-auto" data-testid="my-network-requests">
      <div className="mx-auto max-w-3xl space-y-3 px-5 py-5">
        <div className="flex items-center justify-between gap-2">
          <div>
            <h2 className="font-display text-lg font-semibold text-[color:var(--cp-text)]">{t('myNetwork.view.requests')}</h2>
            <p className="text-[12px] text-[color:var(--cp-muted)]">{t('myNetwork.view.requests.hint')}</p>
          </div>
          <Button variant="text" size="small" startIcon={<RefreshCw size={13} />} onClick={requests.retry}>{t('myNetwork.refresh')}</Button>
        </div>
        {requests.status === 'loading' ? <ListSkeleton rows={4} /> : null}
        {requests.status === 'error' ? (
          <StateMessage tone="danger" testId="requests-error" action={<Button variant="text" size="small" onClick={requests.retry}>{t('network.retry')}</Button>}>{networkErrorText(t, requests.errors[0])}</StateMessage>
        ) : null}
        {requests.status === 'partial' ? (
          <StateMessage tone="warning" testId="requests-partial" action={<Button variant="text" size="small" onClick={requests.retry}>{t('network.retry')}</Button>}>{t('myNetwork.requests.partial')}</StateMessage>
        ) : null}
        {error ? <StateMessage tone="danger" testId="requests-action-error">{error}</StateMessage> : null}
        {requests.status !== 'loading' && requests.status !== 'error' && requests.items.length === 0 ? (
          <StateMessage testId="requests-empty">{t('myNetwork.requests.empty')}</StateMessage>
        ) : null}
        {GROUPS.map(({ key, match }) => {
          const items = requests.items.filter(match)
          if (items.length === 0) return null
          return (
            <section key={key} data-testid={`requests-${key}`}>
              <SectionTitle>{t(`myNetwork.requests.section.${key}`)} · {items.length}</SectionTitle>
              <ul className="space-y-2">{items.map(item => <Item key={item.key} item={item} run={run} busy={busy} />)}</ul>
            </section>
          )
        })}
      </div>
    </div>
  )
}
