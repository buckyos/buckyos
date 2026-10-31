/* The rooms of a running session (第四期规划 §4, §5): what the presenter, an audience member, a public display and a
 * prompter see and do. Every control is a `session.*` call the service confirms; the stage follows the confirmation. */

import { useCallback, useEffect, useRef, useState } from 'react'
import { ChevronLeft, ChevronRight, Copy, Eye, EyeOff, Hand, Link2, ListOrdered, LogOut, MessageSquare, Mic, MicOff, MousePointer2, NotebookText, Pencil, Play, RotateCcw, Square, Users, Volume2, VolumeX } from 'lucide-react'
import { describeError } from '../api/session'
import { randomId } from '../api/ids'
import type { ShowPathPayload } from '../api/types'
import type { MarkTool } from '../presentation/overlays'
import type { WorkspaceStore } from '../state/store'
import { TableImportHost } from '../ui/import/TableImportDialog'
import { useAiwsText } from '../useAiwsText'
import type { SessionChannel, Welcome } from './channel'
import { useChannelStatus } from './context'
import { LiveStage } from './LiveStage'
import type { RoomModel, RoomSnapshot } from './room'
import { stepOf, useSteps } from './steps'
import type { Invite, SessionParticipant, SessionStage } from './types'
import { VoiceListener, VoicePublisher } from './voice'
import { CHANNEL_KEY, LINK_KEY, STATE_KEY } from './labels'

const INK_COLORS = ['#ef4444', '#2563eb', '#16a34a', '#111827', '#f59e0b']
const SYMBOLS = ['👍', '❓', '👏', '💡']

function sessionLink(welcome: Welcome, token: string | null): string {
  const url = new URL(window.location.href)
  url.search = `?session=${encodeURIComponent(welcome.session.session_id)}`
  url.hash = token ? `k=${encodeURIComponent(token)}` : ''
  return url.toString()
}

async function command(channel: SessionChannel, model: RoomModel | null, kind: string, args?: Record<string, unknown>): Promise<string | null> {
  try {
    const r = await channel.call<{ stage: SessionStage }>('session.command', { command: { command_id: randomId('cmd'), kind, ...(args ? { args } : {}) } })
    model?.confirm(r.stage)
    return null
  } catch (error) {
    return describeError(error)
  }
}

// ---------------------------------------------------------------------------
// presenter
// ---------------------------------------------------------------------------

export function PresenterRoom({ store, path, channel, welcome, model, snap }: { store: WorkspaceStore; path: ShowPathPayload; channel: SessionChannel; welcome: Welcome; model: RoomModel; snap: RoomSnapshot }) {
  const { t } = useAiwsText()
  const steps = useSteps(store, path)
  const [tool, setTool] = useState<MarkTool>('none')
  const [inkColor, setInkColor] = useState(INK_COLORS[0])
  const [panel, setPanel] = useState<'none' | 'links' | 'people' | 'notes' | 'steps'>('none')
  const [problem, setProblem] = useState<string | null>(null)
  const [voice, setVoice] = useState<VoicePublisher | null>(null)
  const [muted, setMuted] = useState(false)
  const [editing, setEditing] = useState<string | null>(null)
  const [notes, setNotes] = useState<Record<string, string>>({})
  const live = snap.state === 'live'
  const presenter = welcome.role === 'presenter'
  const controls = presenter && (snap.controller === null || snap.controller === channel.terminalId)
  const index = stepOf(steps, snap.stage)
  const current = steps[index] ?? null
  const voiceService = welcome.session.voice_service
  const channelStatus = useChannelStatus(channel)
  const run = useCallback((kind: string, args?: Record<string, unknown>) => { void command(channel, model, kind, args).then((e) => setProblem(e)) }, [channel, model])

  useEffect(() => {
    let alive = true
    channel.call<{ notes: Record<string, string> }>('session.notes', {}).then((n) => { if (alive) setNotes(n.notes) }, () => undefined)
    return () => { alive = false }
  }, [channel])

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const el = event.target instanceof Element ? event.target : null
      if (editing || event.isComposing || el?.closest('input, textarea, select, [contenteditable="true"], [data-role="editor"]')) return
      if (event.ctrlKey || event.metaKey || event.altKey || !live || !controls) return
      const go = (kind: string) => { event.preventDefault(); run(kind) }
      switch (event.key) {
        case 'ArrowRight': case 'ArrowDown': case 'PageDown': case ' ': go('next'); break
        case 'ArrowLeft': case 'ArrowUp': case 'PageUp': go('prev'); break
        case 'Home': go('first'); break
        case 'End': go('last'); break
        case 'b': case 'B': case '.': go('black'); break
        case 'Escape': if (panel !== 'none') setPanel('none'); else if (tool !== 'none') setTool('none'); else if (snap.stage.free) go('back'); break
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [run, editing, live, controls, panel, tool, snap.stage.free])

  useEffect(() => { if (tool !== 'pointer' && live) void channel.call('session.sample', { kind: 'pointer', data: null }).catch(() => undefined) }, [tool, live, channel])
  useEffect(() => () => { void voice?.stop() }, [voice])

  const start = async () => {
    setProblem(null)
    try {
      const r = await channel.call<{ stage: SessionStage }>('session.start', {})
      model.setState('live')
      model.confirm(r.stage)
    } catch (error) { setProblem(describeError(error)) }
  }
  const end = async () => {
    if (!window.confirm(t('aiworkspace.session.presenter.endConfirm'))) return
    try { await channel.call('session.end', {}); model.setState('ended') } catch (error) { setProblem(describeError(error)) }
  }
  const toggleVoice = async () => {
    if (voice) { await voice.stop(); setVoice(null); return }
    const v = new VoicePublisher(channel)
    try { await v.start(); setVoice(v); setMuted(false) } catch (error) { setProblem(t('aiworkspace.session.voice.failed', { detail: describeError(error) })) }
  }
  const takeover = async () => {
    try { await channel.call('session.takeover', {}) } catch (error) { setProblem(describeError(error)) }
  }
  const onEditingChange = (id: string | null) => {
    if (!id) { setEditing(null); return }
    if (store.outline.get(id)?.live) setEditing(id)
  }

  return (
    <div className="aiws-session-room is-presenter" data-testid="aiws-session-presenter" data-state={snap.state} data-channel={channelStatus} data-controls={controls ? 'true' : undefined}>
      <LiveStage store={store} path={path} steps={steps} snap={snap} channel={channel} presenter tool={controls ? tool : 'none'} inkColor={inkColor}
        onFree={controls && live ? () => { if (!snap.stage.free) run('free', { on: true }) } : undefined} editing={editing} onEditingChange={onEditingChange} />
      {snap.stage.free && controls && <button type="button" className="aiws-show-back aiws-stage-ui" data-testid="aiws-session-back" onClick={() => run('back')}><RotateCcw size={16} aria-hidden="true" />{t('aiworkspace.session.presenter.back')}</button>}
      {snap.state === 'prepared' && (
        <div className="aiws-session-overlay">
          <div className="aiws-panel aiws-session-start" data-testid="aiws-session-prestart">
            <b>{welcome.session.title}</b>
            <span className="aiws-muted">{t('aiworkspace.session.presenter.notStarted', { count: steps.length })}</span>
            {welcome.session.voice && !voiceService?.available && <div className="aiws-warning" data-testid="aiws-session-voice-unavailable">{t('aiworkspace.session.voice.unavailable', { reason: voiceService?.reason ?? '' })}</div>}
            {presenter && <button type="button" className="is-primary" data-testid="aiws-session-start" onClick={() => { void start() }}><Play size={16} aria-hidden="true" />{t('aiworkspace.session.presenter.start')}</button>}
            <button type="button" data-testid="aiws-session-open-links" onClick={() => setPanel('links')}><Link2 size={16} aria-hidden="true" />{t('aiworkspace.session.links.title')}</button>
          </div>
        </div>
      )}
      <div className="aiws-show-bar aiws-panel aiws-stage-ui aiws-session-bar" role="toolbar" aria-label={t('aiworkspace.session.presenter.controls')} data-testid="aiws-session-bar">
        <button type="button" className="aiws-tool" aria-label={t('aiworkspace.session.presenter.prev')} data-testid="aiws-session-prev" disabled={!live || !controls} onClick={() => run('prev')}><ChevronLeft size={18} /></button>
        <button type="button" className="aiws-tool aiws-show-counter" aria-label={t('aiworkspace.session.presenter.steps')} aria-pressed={panel === 'steps'} data-testid="aiws-session-counter" onClick={() => setPanel(panel === 'steps' ? 'none' : 'steps')}>
          <ListOrdered size={16} aria-hidden="true" /><span>{steps.length ? index + 1 : 0} / {steps.length}</span>
        </button>
        <button type="button" className="aiws-tool" aria-label={t('aiworkspace.session.presenter.next')} data-testid="aiws-session-next" disabled={!live || !controls} onClick={() => run('next')}><ChevronRight size={18} /></button>
        <span className="aiws-show-sep" />
        <button type="button" className="aiws-tool" aria-label={t('aiworkspace.session.presenter.black')} aria-pressed={snap.stage.black === true} data-testid="aiws-session-black" disabled={!live || !controls} onClick={() => run('black')}><Square size={16} fill={snap.stage.black ? 'currentColor' : 'none'} /></button>
        <button type="button" className="aiws-tool" aria-label={t('aiworkspace.session.presenter.pointer')} aria-pressed={tool === 'pointer'} data-testid="aiws-session-pointer" disabled={!live || !controls} onClick={() => setTool(tool === 'pointer' ? 'none' : 'pointer')}><MousePointer2 size={16} /></button>
        <button type="button" className="aiws-tool" aria-label={t('aiworkspace.session.presenter.pen')} aria-pressed={tool === 'pen'} data-testid="aiws-session-pen" disabled={!live || !controls} onClick={() => setTool(tool === 'pen' ? 'none' : 'pen')}><Pencil size={16} /></button>
        {tool === 'pen' && INK_COLORS.map((color) => <button key={color} type="button" className={`aiws-show-swatch${inkColor === color ? ' is-on' : ''}`} style={{ background: color }} aria-label={t('aiworkspace.session.presenter.penColor', { color })} onClick={() => setInkColor(color)} />)}
        <span className="aiws-show-sep" />
        <button type="button" className="aiws-tool" aria-label={t('aiworkspace.session.presenter.notes')} aria-pressed={panel === 'notes'} data-testid="aiws-session-notes" onClick={() => setPanel(panel === 'notes' ? 'none' : 'notes')}><NotebookText size={16} /></button>
        <button type="button" className="aiws-tool" aria-label={t('aiworkspace.session.links.title')} aria-pressed={panel === 'links'} data-testid="aiws-session-links" onClick={() => setPanel(panel === 'links' ? 'none' : 'links')}><Link2 size={16} /></button>
        <button type="button" className="aiws-tool" aria-label={t('aiworkspace.session.people.title')} aria-pressed={panel === 'people'} data-testid="aiws-session-people" onClick={() => setPanel(panel === 'people' ? 'none' : 'people')}><Users size={16} /></button>
        {welcome.session.voice && presenter && (
          <>
            <button type="button" className="aiws-tool" aria-label={voice ? t('aiworkspace.session.voice.stop') : t('aiworkspace.session.voice.start')} aria-pressed={voice !== null} data-testid="aiws-session-voice"
              disabled={!voiceService?.available} title={voiceService?.available ? undefined : voiceService?.reason} onClick={() => { void toggleVoice() }}>{voice ? <Mic size={16} /> : <MicOff size={16} />}</button>
            {voice && <button type="button" className="aiws-tool" aria-label={muted ? t('aiworkspace.session.voice.unmute') : t('aiworkspace.session.voice.mute')} aria-pressed={muted} onClick={() => { voice.setMuted(!muted); setMuted(!muted) }}>{muted ? <VolumeX size={16} /> : <Volume2 size={16} />}</button>}
          </>
        )}
        <span className="aiws-show-sep" />
        {presenter && !controls && live && <button type="button" className="aiws-tool aiws-tool-text" data-testid="aiws-session-takeover" onClick={() => { void takeover() }}><Hand size={16} /><span>{t('aiworkspace.session.presenter.takeover')}</span></button>}
        <span className="aiws-show-status" data-testid="aiws-session-status">{t(STATE_KEY[snap.state])}{channelStatus !== 'open' ? ` · ${t(CHANNEL_KEY[channelStatus])}` : ''}</span>
        {live && <button type="button" className="aiws-tool aiws-tool-text" data-testid="aiws-session-end" onClick={() => { void end() }}><LogOut size={16} /><span>{t('aiworkspace.session.presenter.end')}</span></button>}
      </div>
      {problem && <div className="aiws-show-note aiws-stage-ui aiws-warning" role="status" data-testid="aiws-session-problem">{problem}</div>}
      {panel === 'steps' && (
        <div className="aiws-show-panel aiws-panel aiws-stage-ui" data-testid="aiws-session-steps" role="listbox">
          {steps.map((s, i) => (
            <button key={s.step.id} type="button" role="option" aria-selected={i === index} className={`aiws-show-step${i === index ? ' is-current' : ''}`} disabled={!live || !controls}
              onClick={() => { run('goto', { step_id: s.step.id }); setPanel('none') }}>
              <span className="aiws-show-step-n">{i + 1}</span><span className="aiws-show-step-title">{s.title}</span>
            </button>
          ))}
        </div>
      )}
      {panel === 'notes' && (
        <div className="aiws-show-panel aiws-show-notes aiws-panel aiws-stage-ui" data-testid="aiws-session-notes-panel">
          <div className="aiws-show-notes-title">{current?.title}</div>
          <div className="aiws-show-notes-body">{(current && notes[current.targetId ?? '']) || <span className="aiws-muted">{t('aiworkspace.session.presenter.noNotes')}</span>}</div>
        </div>
      )}
      {panel === 'links' && <LinksPanel channel={channel} welcome={welcome} onClose={() => setPanel('none')} />}
      {panel === 'people' && <PeoplePanel channel={channel} snap={snap} onClose={() => setPanel('none')} />}
      <TableImportHost />
    </div>
  )
}

/** The room's links (Q14): their rights are the session's, not the Workspace's sharing. */
function LinksPanel({ channel, welcome, onClose }: { channel: SessionChannel; welcome: Welcome; onClose: () => void }) {
  const { t } = useAiwsText()
  const [invites, setInvites] = useState<Invite[] | null>(null)
  const [problem, setProblem] = useState<string | null>(null)
  const [copied, setCopied] = useState<string | null>(null)
  const load = useCallback(() => channel.call<{ invites: Invite[] }>('session.list_invites', {}).then((r) => setInvites(r.invites), (e: unknown) => setProblem(describeError(e))), [channel])
  useEffect(() => { void load() }, [load])
  const make = async (kind: 'at' | 'dt' | 'pt', config?: Record<string, unknown>) => {
    try { await channel.call('session.invite', { kind, ...(config ? { config } : {}) }); await load() } catch (e) { setProblem(describeError(e)) }
  }
  const revoke = async (id: string) => {
    try { await channel.call('session.revoke_invite', { invite_id: id }); await load() } catch (e) { setProblem(describeError(e)) }
  }
  const copy = (invite: Invite) => {
    const link = sessionLink(welcome, invite.token)
    void navigator.clipboard?.writeText(link).then(() => { setCopied(invite.invite_id); window.setTimeout(() => setCopied(null), 1500) }, () => undefined)
  }
  const label = (i: Invite) => t(LINK_KEY[i.kind]) + (i.kind === 'dt' && i.config.voice === true ? ` · ${t('aiworkspace.session.links.withVoice')}` : '')
  return (
    <div className="aiws-show-panel aiws-panel aiws-stage-ui aiws-session-links" data-testid="aiws-session-links-panel">
      <div className="aiws-dialog-head"><b>{t('aiworkspace.session.links.title')}</b><span className="aiws-grow" /><button type="button" onClick={onClose}>{t('aiworkspace.common.close')}</button></div>
      <div className="aiws-muted">{t('aiworkspace.session.links.help')}</div>
      <div className="aiws-dialog-actions">
        <button type="button" data-testid="aiws-session-new-at" onClick={() => { void make('at') }}>{t('aiworkspace.session.links.newAudience')}</button>
        <button type="button" data-testid="aiws-session-new-dt" onClick={() => { void make('dt', { voice: false }) }}>{t('aiworkspace.session.links.newDisplay')}</button>
        <button type="button" onClick={() => { void make('dt', { voice: true }) }}>{t('aiworkspace.session.links.newRemoteDisplay')}</button>
        <button type="button" data-testid="aiws-session-new-pt" onClick={() => { void make('pt') }}>{t('aiworkspace.session.links.newPrompter')}</button>
      </div>
      {problem && <div className="aiws-error" role="alert">{problem}</div>}
      <ul className="aiws-session-list">
        {(invites ?? []).filter((i) => !i.revoked).map((i) => (
          <li key={i.invite_id} data-testid={`aiws-session-invite-${i.kind}`} data-link={sessionLink(welcome, i.token)}>
            <span className="aiws-grow">{label(i)}</span>
            <button type="button" className="aiws-tool" aria-label={t('aiworkspace.session.links.copy')} onClick={() => copy(i)}><Copy size={14} /></button>
            {copied === i.invite_id && <span className="aiws-muted">{t('aiworkspace.session.links.copied')}</span>}
            <button type="button" onClick={() => { void revoke(i.invite_id) }}>{t('aiworkspace.session.links.revoke')}</button>
          </li>
        ))}
      </ul>
    </div>
  )
}

/** Who came in and who is here now (§5.4); the presenter's own view. */
function PeoplePanel({ channel, snap, onClose }: { channel: SessionChannel; snap: RoomSnapshot; onClose: () => void }) {
  const { t } = useAiwsText()
  const [people, setPeople] = useState<SessionParticipant[]>([])
  useEffect(() => {
    let alive = true
    const load = () => channel.call<{ participants?: SessionParticipant[] }>('session.get', {}).then((r) => { if (alive) setPeople(r.participants ?? []) }, () => undefined)
    void load()
    const timer = window.setInterval(() => { void load() }, 5000)
    return () => { alive = false; window.clearInterval(timer) }
  }, [channel])
  const online = (p: SessionParticipant) => snap.presence[p.principal]?.online ?? p.online
  const audience = people.filter((p) => p.role === 'audience')
  return (
    <div className="aiws-show-panel aiws-panel aiws-stage-ui" data-testid="aiws-session-people-panel">
      <div className="aiws-dialog-head"><b>{t('aiworkspace.session.people.title')}</b><span className="aiws-muted">{t('aiworkspace.session.people.count', { online: audience.filter(online).length, all: audience.length })}</span><span className="aiws-grow" /><button type="button" onClick={onClose}>{t('aiworkspace.common.close')}</button></div>
      <ul className="aiws-session-list">
        {audience.map((p) => (
          <li key={p.principal} data-online={online(p) ? 'true' : 'false'}>
            <span className={`aiws-session-dot${online(p) ? ' is-on' : ''}`} />
            <span className="aiws-grow">{p.name}</span>
            {p.level === 'guest' && <span className="aiws-muted">{t('aiworkspace.session.people.guest')}</span>}
          </li>
        ))}
      </ul>
    </div>
  )
}

// ---------------------------------------------------------------------------
// audience and display
// ---------------------------------------------------------------------------

export function AudienceRoom({ store, path, channel, welcome, snap, following, setFollowing }: { store: WorkspaceStore; path: ShowPathPayload; channel: SessionChannel; welcome: Welcome; snap: RoomSnapshot; following: boolean; setFollowing: (on: boolean) => void }) {
  const { t } = useAiwsText()
  const steps = useSteps(store, path)
  const display = welcome.role === 'display'
  const interact = !display && (welcome.level !== 'guest' || welcome.session.guest_interact)
  const [back, setBack] = useState(0)
  const [pure, setPure] = useState(false)
  const [picking, setPicking] = useState<string | null>(null)
  const [text, setText] = useState('')
  const [problem, setProblem] = useState<string | null>(null)
  const listener = useRef<VoiceListener | null>(null)
  const [voiceOn, setVoiceOn] = useState(false)
  const [muted, setMuted] = useState(false)
  const wantsVoice = welcome.session.voice && welcome.session.voice_service?.available === true && snap.state === 'live'
  const channelStatus = useChannelStatus(channel)

  // the voice: listened to from the start of the class (the join button was the gesture), muted on request
  useEffect(() => {
    if (!wantsVoice) return
    const l = new VoiceListener(channel)
    listener.current = l
    let alive = true
    l.start().then(() => { if (alive) setVoiceOn(true) }, (e: unknown) => { if (alive && !display) setProblem(t('aiworkspace.session.voice.listenFailed', { detail: describeError(e) })) })
    return () => { alive = false; if (listener.current === l) listener.current = null; void l.stop() }
  }, [wantsVoice, channel, display, t])

  const react = async (params: Record<string, unknown>) => {
    try { await channel.call('session.react', { command_id: randomId('re'), ...params }); setProblem(null) } catch (e) { setProblem(describeError(e)) }
  }
  const sendText = () => {
    const v = text.trim()
    if (!v) return
    setText('')
    void react({ kind: 'danmaku', text: v })
  }
  const live = snap.state === 'live'
  return (
    <div className={`aiws-session-room ${display ? 'is-display' : 'is-audience'}`} data-testid={display ? 'aiws-session-display' : 'aiws-session-audience'} data-state={snap.state} data-channel={channelStatus} data-following={following ? 'true' : 'false'}>
      <LiveStage store={store} path={path} steps={steps} snap={snap} channel={channel} presenter={false} free={!following} back={back} pure={pure}
        onFree={display ? undefined : () => setFollowing(false)}
        onPick={picking ? (p) => { void react({ kind: 'feedback', symbol: picking, x: p.x, y: p.y, surface_id: p.surfaceId }); setPicking(null) } : undefined} />
      {!live && <div className="aiws-session-overlay is-passive"><div className="aiws-panel aiws-session-start" data-testid="aiws-session-waiting"><b>{welcome.session.title}</b><span className="aiws-muted">{t('aiworkspace.session.audience.waiting')}</span></div></div>}
      {!display && (
        <div className="aiws-show-bar aiws-panel aiws-stage-ui aiws-session-bar" role="toolbar" aria-label={t('aiworkspace.session.audience.controls')} data-testid="aiws-session-audience-bar">
          {following
            ? <span className="aiws-muted" data-testid="aiws-session-following">{t('aiworkspace.session.audience.following')}</span>
            : <button type="button" className="aiws-tool aiws-tool-text" data-testid="aiws-session-follow" onClick={() => { setFollowing(true); setBack((n) => n + 1) }}><RotateCcw size={16} /><span>{t('aiworkspace.session.audience.backToPresenter')}</span></button>}
          {interact && live && (
            <>
              <span className="aiws-show-sep" />
              {SYMBOLS.map((s) => <button key={s} type="button" className={`aiws-tool${picking === s ? ' is-on' : ''}`} aria-pressed={picking === s} title={t('aiworkspace.session.audience.feedbackTitle')} data-testid={`aiws-session-symbol-${s}`} onClick={() => setPicking(picking === s ? null : s)}>{s}</button>)}
              <form className="aiws-session-danmaku-form" onSubmit={(event) => { event.preventDefault(); sendText() }}>
                <input data-testid="aiws-session-danmaku-input" value={text} maxLength={200} placeholder={t('aiworkspace.session.audience.say')} onChange={(event) => setText(event.target.value)} />
                <button type="submit" className="aiws-tool" aria-label={t('aiworkspace.session.audience.send')} data-testid="aiws-session-danmaku-send"><MessageSquare size={16} /></button>
              </form>
            </>
          )}
          <span className="aiws-show-sep" />
          <button type="button" className="aiws-tool" aria-label={pure ? t('aiworkspace.session.audience.showReactions') : t('aiworkspace.session.audience.pure')} aria-pressed={pure} data-testid="aiws-session-pure" onClick={() => setPure(!pure)}>{pure ? <EyeOff size={16} /> : <Eye size={16} />}</button>
          {voiceOn && <button type="button" className="aiws-tool" aria-label={muted ? t('aiworkspace.session.voice.unmute') : t('aiworkspace.session.voice.mute')} aria-pressed={muted} data-testid="aiws-session-listen" onClick={() => { listener.current?.setMuted(!muted); setMuted(!muted) }}>{muted ? <VolumeX size={16} /> : <Volume2 size={16} />}</button>}
          {!interact && !display && <span className="aiws-muted" data-testid="aiws-session-watch-only">{t('aiworkspace.session.audience.watchOnly')}</span>}
          <span className="aiws-show-status">{welcome.name}{channelStatus !== 'open' ? ` · ${t(CHANNEL_KEY[channelStatus])}` : ''}</span>
        </div>
      )}
      {picking && <div className="aiws-show-hint aiws-stage-ui" role="status">{t('aiworkspace.session.audience.pickHint')}</div>}
      {problem && <div className="aiws-show-note aiws-stage-ui aiws-warning" role="status" data-testid="aiws-session-problem">{problem}</div>}
    </div>
  )
}

// ---------------------------------------------------------------------------
// prompter
// ---------------------------------------------------------------------------

/** The presenter's remote (`pt_` link, Q14): the current step's notes large, the next step, the controls. */
export function PrompterRoom({ store, path, channel, welcome, snap }: { store: WorkspaceStore; path: ShowPathPayload; channel: SessionChannel; welcome: Welcome; snap: RoomSnapshot }) {
  const { t } = useAiwsText()
  const steps = useSteps(store, path)
  const [notes, setNotes] = useState<Record<string, string>>({})
  const [problem, setProblem] = useState<string | null>(null)
  useEffect(() => {
    channel.call<{ notes: Record<string, string> }>('session.notes', {}).then((n) => setNotes(n.notes), (e: unknown) => setProblem(describeError(e)))
  }, [channel])
  const index = stepOf(steps, snap.stage)
  const current = steps[index] ?? null
  const next = steps[index + 1] ?? null
  const send = (kind: string) => { void command(channel, null, kind).then(setProblem) }
  const channelStatus = useChannelStatus(channel)
  return (
    <div className="aiws-root aiws-prompter" data-testid="aiws-session-prompter" data-index={index} data-channel={channelStatus}>
      <div className="aiws-prompter-panel">
        <div className="aiws-prompter-head">
          <span className="aiws-prompter-title" data-testid="aiws-prompter-title">{current?.title ?? ''}</span>
          <span className="aiws-muted">{steps.length ? `${index + 1} / ${steps.length}` : ''}</span>
          <span className="aiws-muted">{welcome.session.title}</span>
        </div>
        {snap.state !== 'live' && <div className="aiws-warning">{t(STATE_KEY[snap.state])}</div>}
        {snap.stage.black && <div className="aiws-warning">{t('aiworkspace.session.prompter.black')}</div>}
        {problem && <div className="aiws-warning" role="status">{problem}</div>}
        <div className="aiws-prompter-notes" data-testid="aiws-prompter-notes">{(current && notes[current.targetId ?? '']) || <span className="aiws-muted">{t('aiworkspace.session.presenter.noNotes')}</span>}</div>
        <div className="aiws-prompter-next">{next ? t('aiworkspace.session.prompter.next', { title: next.title }) : ''}</div>
        <div className="aiws-prompter-controls">
          <button type="button" data-testid="aiws-prompter-prev" disabled={snap.state !== 'live'} onClick={() => send('prev')}>{t('aiworkspace.session.presenter.prev')}</button>
          <button type="button" className="is-primary" data-testid="aiws-prompter-next-button" disabled={snap.state !== 'live'} onClick={() => send('next')}>{t('aiworkspace.session.presenter.next')}</button>
        </div>
        <div className="aiws-prompter-row">
          <button type="button" aria-pressed={snap.stage.black === true} disabled={snap.state !== 'live'} onClick={() => send('black')}>{t('aiworkspace.session.presenter.black')}</button>
          <button type="button" disabled={!snap.stage.free} onClick={() => send('back')}>{t('aiworkspace.session.presenter.back')}</button>
        </div>
      </div>
    </div>
  )
}
