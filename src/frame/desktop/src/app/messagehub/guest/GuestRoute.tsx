import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore, type ReactNode } from 'react'
import { useNavigate, useSearchParams } from 'react-router-dom'
import { useMediaQuery } from '@mui/material'
import { ChevronRight, Lock, LogOut, MessageSquarePlus, ShieldAlert, Users } from 'lucide-react'
import type { AccessDenial, CurrentIdentity } from 'buckyos'
import '../messagehub.css'
import { useI18n } from '../../../i18n/provider'
import { useDocumentTitle } from '../../../presentation/documentHead'
import { WindowDialogProvider } from '../../../desktop/windows/dialogs'
import { denialAdjustText, denialText } from '../../../sso/denial'
import { FirstPasswordPrompt } from '../../../sso/FirstPasswordPrompt'
import { IdentityCard } from '../../../sso/IdentityCard'
import { identityDisplayName } from '../../../sso/identity'
import { HubAvatar } from '../HubAvatar'
import { hubButtonClass, hubPrimaryButtonClass } from '../SessionDialogs'
import { InMemoryConversationMessageReader } from '../conversation/history/data-source'
import { ConversationHistoryPane, type ConversationHistoryPaneHandle } from '../conversation/history/ConversationHistoryPane'
import { ConversationMessageActionsContext, type ConversationMessageActions } from '../conversation/history/actions'
import { registerObjectAccess } from '../conversation/history/objectAccess'
import { messageObjId, ownReactionId } from '../conversation/history/relations'
import { ConversationComposer, type ComposerAnswer, type ComposerRelation, type ConversationComposerSubmitPayload } from '../conversation/input/ConversationComposer'
import { conversationInputState } from '../conversation/input/inputState'
import { ConversationMediaScopeContext } from '../conversation/media/context'
import { MessageMediaHost } from '../conversation/media/MessageMediaHost'
import type { MessageObject } from '../protocol/msgobj'
import { loadGuestApi, type GuestApi } from './api'
import { GuestHub, type GuestSnapshot } from './hub'
import { guestEntryPath, guestPagePath, isGroupDid, type GuestSession } from './model'
import { useGuestText, type GuestText } from './text'
import { isMessageHubMock } from '../store'
import { GroupExtensionPanels } from '../extensions/GroupExtensionPanels'
import { GroupDisplaySection } from '../extensions/GroupDisplaySection'
import { mockPanelOrigins } from '../extensions/model'

const LOADING: GuestSnapshot = { step: { kind: 'loading' }, identity: null, groupName: '', hostDid: '', reader: InMemoryConversationMessageReader.empty(), historyLoaded: false, members: [], offline: false }
const noSubscribe = () => () => {}
const loadingSnapshot = () => LOADING

/**
 * `/messagehub/guest?group=<group_did>[&session=<session_id>]` (SSO-12): the
 * page a Session Only visitor (a Limited account or an External DID) uses.
 * It shows only the Sessions of this group the visitor is in, never a
 * personal mailbox, contacts, notifications or group management. Without a
 * browser session the visitor goes to the group's entry to sign in.
 */
export function MessageHubGuestRoute() {
  const [searchParams] = useSearchParams()
  const navigate = useNavigate()
  const groupDid = searchParams.get('group')
  const sessionId = searchParams.get('session')
  const openSession = useCallback((id: string) => { if (isGroupDid(groupDid)) navigate(guestPagePath(groupDid, id)) }, [groupDid, navigate])
  return (
    <main className="min-h-dvh bg-[color:var(--cp-bg)] p-0 md:p-5">
      <div
        className="mx-auto flex h-dvh w-full max-w-[1100px] flex-col overflow-hidden text-[color:var(--cp-text)] md:h-[calc(100dvh-2.5rem)] md:rounded-[28px] md:border md:shadow-[var(--cp-window-shadow)]"
        style={{ borderColor: 'var(--cp-border)', background: 'linear-gradient(180deg, color-mix(in srgb, var(--cp-surface) 96%, transparent), color-mix(in srgb, var(--cp-surface-2) 94%, transparent))', backdropFilter: 'blur(20px)' }}
      >
        {isGroupDid(groupDid) ? <GuestPage key={`${groupDid}\n${sessionId ?? ''}`} groupDid={groupDid} sessionId={sessionId || null} onOpenSession={openSession} /> : <InvalidLink />}
      </div>
    </main>
  )
}

function InvalidLink() {
  const text = useGuestText()
  return <Centered><p role="alert" data-testid="guest-invalid-link">{text('invalidLink')}</p></Centered>
}

function Centered({ children, testId }: { children: ReactNode; testId?: string }) {
  return <div className="flex flex-1 flex-col items-center justify-center gap-4 p-6 text-center" data-testid={testId}>{children}</div>
}

function useGuestHub(groupDid: string, sessionId: string | null, attempt: number) {
  const { t } = useI18n()
  const you = t('messagehub.you')
  const [current, setCurrent] = useState<{ api: GuestApi; hub: GuestHub } | null>(null)
  // Built in the effect, not during render: a stopped hub never restarts (StrictMode runs effects twice).
  useEffect(() => {
    let live = true, hub: GuestHub | null = null
    void loadGuestApi().then(api => {
      if (!live) return
      hub = new GuestHub(api, groupDid, sessionId, you)
      registerObjectAccess(api.objects)
      setCurrent({ api, hub })
      void hub.start()
    })
    return () => { live = false; hub?.stop() }
  }, [groupDid, sessionId, attempt, you])
  const hub = current?.hub ?? null
  const snapshot = useSyncExternalStore(hub?.subscribe ?? noSubscribe, hub?.getSnapshot ?? loadingSnapshot)
  return { api: current?.api ?? null, hub, snapshot }
}

function GuestPage({ groupDid, sessionId, onOpenSession }: { groupDid: string; sessionId: string | null; onOpenSession: (id: string) => void }) {
  const text = useGuestText()
  const { t } = useI18n()
  const [attempt, setAttempt] = useState(0)
  const { api, hub, snapshot } = useGuestHub(groupDid, sessionId, attempt)
  const { step, identity, groupName } = snapshot
  const session = step.kind === 'session' ? step.session : null
  useDocumentTitle(groupName ? `${session?.title || groupName} · ${t('apps.messagehub')}` : t('apps.messagehub'))
  useEffect(() => {
    if (step.kind === 'anonymous') window.location.replace(guestEntryPath(groupDid))
    if (step.kind === 'identity_changed') window.location.reload()
  }, [step.kind, groupDid])
  const startConversation = async () => {
    const id = await hub?.startConversation()
    if (id) onOpenSession(id)
  }
  let body: ReactNode
  switch (step.kind) {
    case 'loading':
    case 'anonymous':
    case 'identity_changed':
      body = <Centered><p role="status">{text('loading')}</p></Centered>
      break
    case 'error':
      body = <Centered testId="guest-error"><p role="alert">{text('error.title')}</p><button type="button" className={hubButtonClass} onClick={() => setAttempt(value => value + 1)}>{text('retry')}</button></Centered>
      break
    case 'denied':
      body = <DeniedView denial={step.denial} text={text} showAdjust={identity?.kind === 'account' && !identity.restricted} />
      break
    case 'invited':
      body = <Centered testId="guest-invitation">
        <HubAvatar type="group" name={groupName} size={56} />
        <h1 className="text-lg font-semibold">{text('invited.title', { group: groupName || text('conversation') })}</h1>
        <p className="max-w-md text-sm text-[color:var(--cp-muted)]">{text('invited.body')}</p>
        <button type="button" className={hubPrimaryButtonClass} disabled={step.joining} onClick={() => void hub?.accept()} data-testid="guest-accept">{text(step.joining ? 'invited.joining' : 'invited.join')}</button>
        {step.failed && <p role="alert" className="max-w-md text-sm text-[color:var(--cp-danger)]" data-testid="guest-accept-failed">{step.denial ? denialText(t, step.denial) : text('invited.failed')}</p>}
      </Centered>
      break
    case 'lobby':
      body = <div className="flex-1 overflow-y-auto p-4 md:p-6" data-testid="guest-lobby">
        <div className="mx-auto max-w-lg space-y-4">
          <h1 className="text-lg font-semibold">{text('lobby.title', { group: groupName || text('conversation') })}</h1>
          {step.sessions.length === 0 ? <p className="text-sm text-[color:var(--cp-muted)]">{text('lobby.empty')}</p> : <ul className="space-y-2">{step.sessions.map(item => <li key={item.sessionId}>
            <button type="button" className="flex min-h-12 w-full items-center gap-3 rounded-xl border border-[color:var(--cp-border)] px-3 text-left hover:bg-[color:color-mix(in_srgb,var(--cp-text)_5%,transparent)]" onClick={() => onOpenSession(item.sessionId)} data-testid="guest-session-link">
              <span className="min-w-0 flex-1 truncate">{item.title || text('conversation')}</span><ChevronRight size={16} className="shrink-0 text-[color:var(--cp-muted)]" />
            </button>
          </li>)}</ul>}
          {step.entryOpen && <button type="button" className={`${hubPrimaryButtonClass} inline-flex items-center gap-2`} disabled={step.starting} onClick={() => void startConversation()} data-testid="guest-start"><MessageSquarePlus size={16} />{text(step.starting ? 'lobby.starting' : 'lobby.start')}</button>}
          {step.failed && <p role="alert" className="text-sm text-[color:var(--cp-danger)]">{step.denial ? denialText(t, step.denial) : text('lobby.startFailed')}</p>}
        </div>
      </div>
      break
    case 'session':
      body = hub && api ? <GuestConversation hub={hub} snapshot={snapshot} session={step.session} /> : null
      break
  }
  return <>
    <GuestHeader groupName={groupName} session={session} identity={identity} api={api} groupDid={groupDid} offline={snapshot.offline} members={snapshot.members.length} />
    {identity ? <div className="shrink-0 px-3 py-2 empty:hidden md:px-4"><FirstPasswordPrompt identity={identity} /></div> : null}
    <div className="flex min-h-0 flex-1 flex-col">{body}</div>
  </>
}

/** Visitors see the category and whom to contact; a non-restricted viewer also sees where it is adjusted (D-4). */
function DeniedView({ denial, text, showAdjust }: { denial: AccessDenial | null; text: GuestText; showAdjust: boolean }) {
  const { t } = useI18n()
  const adjust = denial && showAdjust ? denialAdjustText(t, denial) : null
  return <Centered testId="guest-denied">
    <ShieldAlert size={36} className="text-[color:var(--cp-muted)]" aria-hidden />
    <h1 className="text-lg font-semibold">{text('denied.title')}</h1>
    <p role="alert" className="max-w-md text-sm text-[color:var(--cp-muted)]" data-reason={denial?.reason}>{denial ? denialText(t, denial) : text('denied.body')}</p>
    {adjust && <p className="max-w-md text-xs text-[color:var(--cp-muted)]" data-testid="guest-denied-adjust">{adjust}</p>}
  </Centered>
}

function GuestHeader({ groupName, session, identity, api, groupDid, offline, members }: { groupName: string; session: GuestSession | null; identity: CurrentIdentity | null; api: GuestApi | null; groupDid: string; offline: boolean; members: number }) {
  const text = useGuestText()
  const [open, setOpen] = useState(false)
  const [signingOut, setSigningOut] = useState(false)
  const [failed, setFailed] = useState(false)
  const panelRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!open) return
    const close = (event: PointerEvent | KeyboardEvent) => {
      if (event instanceof KeyboardEvent ? event.key === 'Escape' : !panelRef.current?.contains(event.target as Node)) setOpen(false)
    }
    window.addEventListener('pointerdown', close)
    window.addEventListener('keydown', close)
    return () => { window.removeEventListener('pointerdown', close); window.removeEventListener('keydown', close) }
  }, [open])
  const signOut = async () => {
    if (!api || signingOut) return
    setSigningOut(true); setFailed(false)
    try {
      await api.signOut()
      window.location.replace(guestEntryPath(groupDid))
    } catch {
      setFailed(true); setSigningOut(false)
    }
  }
  const signedIn = identity && identity.kind !== 'anonymous' ? identity : null
  return <header className="flex shrink-0 items-center gap-2 border-b border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-3 py-2 md:px-4" data-testid="guest-header">
    <HubAvatar type="group" name={groupName} size={32} />
    <div className="min-w-0 flex-1">
      <p className="truncate text-[15px] font-semibold" data-testid="guest-group-name">{groupName || ' '}</p>
      <p className="flex items-center gap-1.5 truncate text-xs text-[color:var(--cp-muted)]">
        {session ? <span className="truncate" data-testid="guest-session-title">{session.title || text('conversation')}</span> : null}
        {session && members > 0 ? <span className="inline-flex shrink-0 items-center gap-0.5"><Users size={12} aria-hidden />{members}</span> : null}
        {offline ? <span role="status" className="shrink-0 text-[color:var(--cp-warning)]">{text('reconnecting')}</span> : null}
      </p>
    </div>
    {signedIn ? <div className="relative" ref={panelRef}>
      <button type="button" className="flex min-h-11 max-w-[14rem] items-center gap-2 rounded-xl px-2 hover:bg-[color:color-mix(in_srgb,var(--cp-text)_6%,transparent)]" aria-haspopup="dialog" aria-expanded={open} onClick={() => setOpen(value => !value)} data-testid="guest-identity">
        <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-[color:color-mix(in_srgb,var(--cp-accent)_14%,transparent)] text-sm font-semibold text-[color:var(--cp-accent)]" aria-hidden>{identityDisplayName(signedIn).slice(0, 1).toUpperCase()}</span>
        <span className="hidden min-w-0 flex-col text-left sm:flex">
          <span className="truncate text-sm font-medium">{identityDisplayName(signedIn)}</span>
          <span className="truncate text-[11px] text-[color:var(--cp-muted)]">{signedIn.zone_host}</span>
        </span>
      </button>
      {open && <div role="dialog" aria-label={text('account')} className="absolute right-0 top-full z-50 mt-2 w-[min(22rem,calc(100vw-1.5rem))] space-y-3 rounded-2xl border border-[color:var(--cp-border)] p-3 shadow-lg" style={{ background: 'var(--cp-surface-opaque, var(--cp-surface))' }} data-testid="guest-identity-panel">
        <IdentityCard identity={signedIn} />
        <button type="button" className={`${hubButtonClass} inline-flex w-full items-center justify-center gap-2`} disabled={signingOut} onClick={() => void signOut()} data-testid="guest-sign-out"><LogOut size={15} />{text(signingOut ? 'signingOut' : 'signOut')}</button>
        {failed && <p role="alert" className="text-xs text-[color:var(--cp-danger)]">{text('signOutFailed')}</p>}
      </div>}
    </div> : null}
  </header>
}

/** The open Session: its timeline from the host group store and a text-only composer (no uploads in v1). */
function GuestConversation({ hub, snapshot, session }: { hub: GuestHub; snapshot: GuestSnapshot; session: GuestSession }) {
  const { t, locale } = useI18n()
  const text = useGuestText()
  const isDesktop = useMediaQuery('(min-width: 769px)')
  // The Session's panels and display, as any member sees them (EXT-02/04); the guest page passes no viewer identity either.
  const extensions = (session.panels.length > 0 || !!session.display.announcement || session.display.links.length > 0 || session.display.files.length > 0 || session.display.featuredMembers.length > 0) && snapshot.hostDid
    ? <div className="space-y-3" data-testid="guest-extensions">
      <GroupDisplaySection display={session.display} nameOf={hub.nameOf} testId="session-display" />
      <GroupExtensionPanels panels={session.panels} context={{ groupDid: hub.groupDid, sessionId: session.sessionId, session: session.address, locale }} hostDid={snapshot.hostDid} allowOrigins={isMessageHubMock() ? mockPanelOrigins() : undefined} defaultOpen />
    </div>
    : null
  const historyRef = useRef<ConversationHistoryPaneHandle>(null)
  const { reader, members } = snapshot
  const selfDid = hub.selfDid
  useEffect(() => {
    const sync = () => { if (document.visibilityState === 'visible') void hub.sync() }
    window.addEventListener('online', sync)
    document.addEventListener('visibilitychange', sync)
    return () => { window.removeEventListener('online', sync); document.removeEventListener('visibilitychange', sync) }
  }, [hub])
  const canSend = session.lifecycle === 'active'
  const [relation, setRelation] = useState<ComposerRelation | null>(null)
  const inputState = useMemo(() => conversationInputState(reader.timeline ?? [], selfDid, { group: true }), [reader, selfDid])
  const answerState = inputState.mode === 'answer' ? inputState : null
  if (answerState && relation) setRelation(null)
  const answer = useMemo<ComposerAnswer | null>(() => answerState ? { ...answerState, askerName: hub.nameOf(answerState.question.from) } : null, [answerState, hub])
  const mediaScope = useMemo(() => ({ reader, hostContext: session.address }), [reader, session.address])
  const mentionCandidates = useMemo(() => members.filter(member => member.did !== selfDid).map(member => ({ did: member.did, name: member.name })), [members, selfDid])
  const send = useCallback(async (payload: ConversationComposerSubmitPayload) => {
    historyRef.current?.scrollToBottom()
    await hub.send({ content: payload.content, attachments: [], ...(payload.relatesTo ? { relatesTo: payload.relatesTo } : {}), ...(payload.mentions ? { mentions: payload.mentions } : {}), ...(payload.machine ? { machine: payload.machine } : {}), ...(payload.localId ? { localId: payload.localId, submittedAt: payload.submittedAt } : {}) })
    historyRef.current?.scrollToBottom()
  }, [hub])
  const actions = useMemo<ConversationMessageActions>(() => {
    const relate = (target: string | undefined, rel: 'redact' | 'reaction', key?: string) => target
      ? hub.send({ content: key ?? '', attachments: [], relatesTo: { rel, target, ...(key ? { key } : {}) } })
      : Promise.reject(Error('not-found'))
    return {
      displayName: hub.nameOf,
      relations: canSend ? {
        capabilities: () => ({ reply: !answerState, react: true, edit: false, redact: false, recall: false }),
        reply: (message: MessageObject) => setRelation({ kind: 'reply', message }),
        edit: () => undefined,
        redact: () => Promise.reject(Error('permission_denied')),
        react: (message: MessageObject, key: string) => relate(messageObjId(message), 'reaction', key),
        unreact: (message: MessageObject, key: string) => relate(ownReactionId(message, selfDid, key), 'redact'),
      } : undefined,
    }
  }, [hub, canSend, answerState, selfDid])
  return <WindowDialogProvider surface={isDesktop ? 'desktop' : 'mobile'} permissions={{ fullscreen: false }}>
    <MessageMediaHost>
      <div className="flex min-h-0 flex-1">
      <div className="relative flex min-h-0 flex-1 flex-col" style={{ background: 'var(--cp-message-canvas)' }} data-testid="guest-conversation">
        {extensions && !isDesktop ? <div className="shrink-0 border-b border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-3 py-2 text-sm">{extensions}</div> : null}
        <div className="relative flex min-h-0 flex-1 flex-col">
          <ConversationMediaScopeContext.Provider value={mediaScope}>
            <ConversationMessageActionsContext.Provider value={actions}>
              <ConversationHistoryPane ref={historyRef} reader={reader} selfDid={selfDid} isGroup emptyLabel={t(snapshot.historyLoaded ? 'messagehub.startConversation' : 'messagehub.loadingHistory')} />
            </ConversationMessageActionsContext.Provider>
          </ConversationMediaScopeContext.Provider>
        </div>
        {canSend ? <ConversationComposer
          placeholder={t('messagehub.inputPlaceholder', 'Message...')}
          onSendMessage={send}
          relation={answer ? null : relation}
          onCancelRelation={() => setRelation(null)}
          answer={answer}
          mentionCandidates={mentionCandidates}
          allowFiles={false}
        /> : <div className="flex shrink-0 items-center justify-center gap-2 border-t border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-4 py-3 text-[13px] text-[color:var(--cp-muted)]" data-testid="composer-readonly"><Lock size={14} className="shrink-0" aria-hidden /><span>{text('archived')}</span></div>}
      </div>
      {extensions && isDesktop ? <aside className="shell-scrollbar w-[320px] shrink-0 overflow-y-auto border-l border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] p-4 text-sm" data-testid="guest-side-panel">{extensions}</aside> : null}
      </div>
    </MessageMediaHost>
  </WindowDialogProvider>
}
