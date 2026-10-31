/* A session's page (第四期规划 §1, §3, §4): `https://aiws-<source>.<zone>/<slug>?session=<id>#k=<link>` — the same
 * host as the source Workspace. The link names how this terminal is in the room: an audience link joins with the
 * visitor's own identity (an account, an External DID or a guest), a display link shows the public picture, a prompter
 * link drives the stage; without a link the presenter, the owner or a joined participant comes back. The page opens
 * the run Workspace and the session channel, then shows the room for the role — or, once the session has ended, its
 * replay and what people say about it. */

import { useEffect, useMemo, useState, useSyncExternalStore } from 'react'
import '../aiworkspace.css'
import '../presentation/presentation.css'
import './session.css'
import { AiwsClient } from '../api/client'
import { describeError, OnlineWorkspaceSession } from '../api/session'
import { LinkTransport, resolveTransport, type Transport } from '../api/transport'
import type { KeyedContent, ShowPathPayload } from '../api/types'
import { loadCore } from '../api/wasm'
import { loginUrl } from '../links'
import { randomId } from '../api/ids'
import { StoreContext, useOutlineVersion, WorkspaceUiContext, type WorkspaceUi } from '../state/hooks'
import { WorkspaceStore } from '../state/store'
import { BlockHost } from '../ui/blocks/BlockHost'
import { registerDefaultBlocks } from '../ui/blocks/registerAll'
import { registerClassroomBlocks } from '../ui/classroom/register'
import { useAiwsText } from '../useAiwsText'
import { linkOfHash, SessionApi } from './api'
import { SessionChannel, type Purpose, type Welcome } from './channel'
import { SessionContext, type SessionUi } from './context'
import { RoomModel } from './room'
import { AudienceRoom, PresenterRoom, PrompterRoom } from './Rooms'
import { ReplayRoom } from './ReplayRoom'
import type { SessionInfo } from './types'

const TERMINAL_KEY = 'aiworkspace.session-terminal'
const NAME_KEY = 'aiworkspace.session-name'

/** This tab's terminal id (Q05: control belongs to one terminal; a reload is the same terminal). */
function terminalId(): string {
  try {
    const kept = window.sessionStorage.getItem(TERMINAL_KEY)
    if (kept) return kept
    const id = randomId('tm')
    window.sessionStorage.setItem(TERMINAL_KEY, id)
    return id
  } catch {
    return randomId('tm')
  }
}

function keptName(): string {
  try { return window.localStorage.getItem(NAME_KEY) ?? '' } catch { return '' }
}

type Phase =
  | { kind: 'starting' }
  | { kind: 'gate'; info: SessionInfo | null }
  | { kind: 'joining' }
  | { kind: 'ready'; welcome: Welcome; store: WorkspaceStore; path: ShowPathPayload }
  | { kind: 'failed'; detail: string }

export function SessionPage({ sessionId }: { sessionId: string }) {
  const { t } = useAiwsText()
  const [link] = useState(() => linkOfHash(window.location.hash))
  const transport = useMemo<Transport | null>(() => {
    if (link && (link.startsWith('dt_') || link.startsWith('pt_'))) return new LinkTransport(link)
    const a = resolveTransport()
    return a.ok ? a.transport : null
  }, [link])
  const api = useMemo(() => (transport ? new SessionApi(transport, sessionId, link) : null), [transport, sessionId, link])
  const [phase, setPhase] = useState<Phase>(() => !api ? { kind: 'failed', detail: t('aiworkspace.session.page.noTransport') } : link?.startsWith('dt_') || link?.startsWith('pt_') ? { kind: 'joining' } : { kind: 'starting' })
  const [channel, setChannel] = useState<SessionChannel | null>(null)

  // who is this: a known member answers session.get; a link visitor passes the gate first (the gesture unlocks sound)
  useEffect(() => {
    if (!api || phase.kind !== 'starting') return
    let live = true
    api.get().then(
      (info) => { if (live) setPhase(info.role === 'audience' && info.state !== 'ended' && info.state !== 'archived' ? { kind: 'gate', info } : { kind: 'joining' }) },
      () => {
        if (!live) return
        if (link) { setPhase({ kind: 'gate', info: null }); return }
        // without a link, a member who has not signed into this host yet signs in and comes back
        void signedOut(transport).then((out) => {
          if (!live) return
          if (out) window.location.replace(loginUrl())
          else setPhase({ kind: 'failed', detail: t('aiworkspace.session.page.notInvited') })
        })
      },
    )
    return () => { live = false }
  }, [api, transport, link, t, phase.kind])

  // joining: the channel, then the run Workspace
  useEffect(() => {
    if (phase.kind !== 'joining' || !api || !transport) return
    let live = true
    const purpose: Purpose = link?.startsWith('dt_') ? 'display' : link?.startsWith('pt_') ? 'prompter' : 'audience'
    void (async () => {
      let opened: WorkspaceStore | null = null
      try {
        const known = purpose === 'audience' ? await api.get().catch(() => null) : null
        const ch = new SessionChannel(api, known?.role === 'presenter' || known?.role === 'owner' ? 'presenter' : purpose, terminalId(), keptName() || undefined)
        const welcome = await ch.open()
        registerDefaultBlocks()
        registerClassroomBlocks()
        const client = new AiwsClient(transport)
        const [core, session] = await Promise.all([loadCore(), OnlineWorkspaceSession.open(client, welcome.session.workspace_id)])
        opened = new WorkspaceStore(session, core)
        await opened.outline.reload()
        const path = (await opened.session.read<KeyedContent<ShowPathPayload>>(welcome.session.path_id)).content.payload
        if (!live) { ch.close(); void opened.dispose(); return }
        setChannel(ch)
        setPhase({ kind: 'ready', welcome, store: opened, path })
      } catch (error) {
        if (opened) void opened.dispose()
        if (live) setPhase({ kind: 'failed', detail: describeError(error) })
      }
    })()
    return () => { live = false }
  }, [phase.kind, api, transport, link])
  useEffect(() => () => { channel?.close() }, [channel])
  useEffect(() => () => { if (phase.kind === 'ready') void phase.store.dispose() }, [phase])

  return (
    <div className="aiws-root aiws-session-page" data-testid="aiws-session-page" data-phase={phase.kind}>
      {phase.kind === 'starting' && <div className="aiws-session-center aiws-muted">{t('aiworkspace.session.page.opening')}</div>}
      {phase.kind === 'joining' && <div className="aiws-session-center aiws-muted">{t('aiworkspace.session.page.joining')}</div>}
      {phase.kind === 'failed' && <div className="aiws-session-center"><div className="aiws-error" role="alert" data-testid="aiws-session-failed">{phase.detail}</div></div>}
      {phase.kind === 'gate' && transport && <JoinGate info={phase.info} transport={transport} onJoin={(name) => {
        try { if (name) window.localStorage.setItem(NAME_KEY, name) } catch { /* not kept */ }
        setPhase({ kind: 'joining' })
      }} />}
      {phase.kind === 'ready' && channel && <Room welcome={phase.welcome} store={phase.store} path={phase.path} channel={channel} />}
    </div>
  )
}

/** A visitor of the Zone host without an account session on it (a link token is no sign-in). */
async function signedOut(transport: Transport | null): Promise<boolean> {
  return !!transport && transport.mode === 'zone' && !(transport instanceof LinkTransport) && await transport.isGuest()
}

/** Before joining: the room's name and, for a visitor, the name to show; the button is the gesture browsers want
 * before they play sound (§7.3.1). A visitor who is not signed in may sign in first and join with the account (Q03). */
function JoinGate({ info, transport, onJoin }: { info: SessionInfo | null; transport: Transport; onJoin: (name: string) => void }) {
  const { t } = useAiwsText()
  const [name, setName] = useState(keptName)
  const [guest, setGuest] = useState(false)
  useEffect(() => {
    let live = true
    void signedOut(transport).then((out) => { if (live) setGuest(out) })
    return () => { live = false }
  }, [transport])
  return (
    <div className="aiws-session-center">
      <form className="aiws-panel aiws-session-gate" data-testid="aiws-session-gate" onSubmit={(event) => { event.preventDefault(); onJoin(name.trim()) }}>
        <b className="aiws-session-gate-title">{info?.title ?? t('aiworkspace.session.page.publicClass')}</b>
        {info?.state === 'prepared' && <span className="aiws-muted">{t('aiworkspace.session.page.notStarted')}</span>}
        <label className="aiws-inline-form">{t('aiworkspace.session.page.yourName')}
          <input data-testid="aiws-session-name" value={name} maxLength={64} onChange={(event) => setName(event.target.value)} placeholder={t('aiworkspace.session.page.namePlaceholder')} />
        </label>
        <button type="submit" className="is-primary" data-testid="aiws-session-join">{t('aiworkspace.session.page.join')}</button>
        {guest && (
          <div className="aiws-session-gate-signin">
            <span className="aiws-muted">{t('aiworkspace.session.page.guestHint')}</span>
            <button type="button" data-testid="aiws-session-signin" onClick={() => { window.location.assign(loginUrl()) }}>{t('aiworkspace.session.page.signIn')}</button>
          </div>
        )}
      </form>
    </div>
  )
}

function Room({ welcome, store, path, channel }: { welcome: Welcome; store: WorkspaceStore; path: ShowPathPayload; channel: SessionChannel }) {
  const [model] = useState(() => new RoomModel(welcome.session, welcome.controller))
  useEffect(() => model.attach(channel), [model, channel])
  const snap = useSyncExternalStore(model.subscribe, model.snapshot)
  const [following, setFollowing] = useState(true)
  const ui = useMemo<SessionUi>(() => ({
    sessionId: welcome.session.session_id,
    role: welcome.role,
    principal: welcome.principal,
    level: welcome.level,
    info: welcome.session,
    channel,
    myFeeds: welcome.session.my_feeds ?? {},
    mayInteract: welcome.role !== 'display' && (welcome.level !== 'guest' || welcome.session.guest_interact),
    replay: false,
    room: model,
    following: () => following,
    tRel: () => channel.tRel(),
    media: { get: (id) => model.media(id), subscribe: model.subscribe },
  }), [welcome, channel, model, following])
  const ended = snap.state === 'ended' || snap.state === 'archived' || welcome.session.state === 'expired'
  return (
    <SessionContext.Provider value={ui}>
      <StoreContext.Provider value={store}>
        <StageUi store={store}>
          {ended ? <ReplayRoom store={store} path={path} channel={channel} welcome={welcome} roomState={welcome.session.state === 'expired' ? 'expired' : snap.state} />
            : welcome.role === 'presenter' || welcome.role === 'owner' ? <PresenterRoom store={store} path={path} channel={channel} welcome={welcome} model={model} snap={snap} />
            : welcome.role === 'prompter' ? <PrompterRoom store={store} path={path} channel={channel} welcome={welcome} snap={snap} />
            : <AudienceRoom store={store} path={path} channel={channel} welcome={welcome} snap={snap} following={following} setFollowing={(on) => { setFollowing(on); channel.follow(on) }} />}
        </StageUi>
      </StoreContext.Provider>
    </SessionContext.Provider>
  )
}

/** The view context Blocks on stage need (no annotations, no navigation away from the stage). */
function StageUi({ store, children }: { store: WorkspaceStore; children: React.ReactNode }) {
  const outlineVersion = useOutlineVersion()
  const ui = useMemo<WorkspaceUi>(() => {
    const entities = store.outline.all()
    return {
      entities, byId: new Map(entities.map((e) => [e.entity_id, e])), annotations: [], openEntity: () => undefined, annotate: null,
      activeAnnotation: null, setActiveAnnotation: () => undefined, annotationFilter: null, showAnnotations: () => undefined,
      renderCell: (cellId, depth) => <div className="aiws-embedded"><BlockHost cellId={cellId} mode="view" view="source" depth={depth} /></div>,
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- outlineVersion is the invalidation signal
  }, [store, outlineVersion])
  return <WorkspaceUiContext.Provider value={ui}>{children}</WorkspaceUiContext.Provider>
}
