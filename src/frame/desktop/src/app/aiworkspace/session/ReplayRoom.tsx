/* After the class (第四期规划 §3.2, §7, §8): the replay at the session's own address — play, pause, speed, seek, the
 * steps as marks — with the recorded voice on the same time line, the owner's save / access / delete, and what people
 * say: comments of the session, moments at a point of the replay, ratings for the presenter. The replay renders the
 * recorded state only (§7.2): its Blocks read the rebuilt Workspace, nothing is written or run. */

import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react'
import { Pause, Play, Star, Trash2 } from 'lucide-react'
import { AiwsClient } from '../api/client'
import { describeError, OnlineWorkspaceSession } from '../api/session'
import type { ShowPathPayload } from '../api/types'
import { loadCore } from '../api/wasm'
import { StoreContext } from '../state/hooks'
import { WorkspaceStore } from '../state/store'
import { useAiwsText } from '../useAiwsText'
import type { SessionChannel, Welcome } from './channel'
import { SessionContext, useSession, type SessionUi } from './context'
import { LiveStage } from './LiveStage'
import { loadReplay, ReplayEngine, replaySession } from './replay'
import { useSteps } from './steps'
import type { SessionInfo, SessionPost } from './types'
import { STATE_KEY } from './labels'

const SPEEDS = [0.5, 1, 1.5, 2]

function clock(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000))
  const pad = (n: number) => String(n).padStart(2, '0')
  const h = Math.floor(s / 3600)
  return h ? `${h}:${pad(Math.floor((s % 3600) / 60))}:${pad(s % 60)}` : `${pad(Math.floor(s / 60))}:${pad(s % 60)}`
}

/** The playhead: a clock that runs at a speed while playing. */
class Playhead {
  private at = 0
  private startedAt: number | null = null
  speed = 1
  readonly duration: number
  private readonly listeners = new Set<() => void>()
  private version = 0
  constructor(duration: number) { this.duration = duration }
  subscribe = (l: () => void) => { this.listeners.add(l); return () => { this.listeners.delete(l) } }
  snapshot = () => this.version
  private changed() { this.version += 1; for (const l of [...this.listeners]) l() }
  now(): number {
    if (this.startedAt === null) return this.at
    return Math.min(this.duration, this.at + (performance.now() - this.startedAt) * this.speed)
  }
  get playing(): boolean { return this.startedAt !== null }
  play() { if (this.startedAt === null) { if (this.at >= this.duration) this.at = 0; this.startedAt = performance.now(); this.changed() } }
  pause() { if (this.startedAt !== null) { this.at = this.now(); this.startedAt = null; this.changed() } }
  seek(t: number) { this.at = Math.max(0, Math.min(this.duration, t)); if (this.startedAt !== null) this.startedAt = performance.now(); this.changed() }
  setSpeed(s: number) { this.at = this.now(); if (this.startedAt !== null) this.startedAt = performance.now(); this.speed = s; this.changed() }
  tick() { if (this.startedAt !== null) { if (this.now() >= this.duration) this.pause(); else this.changed() } }
}

export function ReplayRoom({ path, channel, welcome, roomState }: { store: WorkspaceStore; path: ShowPathPayload; channel: SessionChannel; welcome: Welcome; roomState: SessionInfo['state'] }) {
  const { t } = useAiwsText()
  const live = useSession()!
  const timeRef = useRef(0)
  const [replay, setReplay] = useState<{ engine: ReplayEngine; store: WorkspaceStore } | null>(null)
  const [problem, setProblem] = useState<string | null>(null)
  // what the room knows now (the welcome may be from before the end)
  const [saved, setSaved] = useState(false)
  const state: SessionInfo['state'] = saved ? 'archived' : roomState === 'live' || roomState === 'prepared' ? 'ended' : roomState
  useEffect(() => {
    let alive = true
    let made: { engine: ReplayEngine; store: WorkspaceStore } | null = null
    void (async () => {
      try {
        const client = new AiwsClient(channel.api.transport)
        const [core, data, direct] = await Promise.all([loadCore(), loadReplay(channel.api, client), OnlineWorkspaceSession.open(client, welcome.session.workspace_id)])
        const engine = new ReplayEngine(core, welcome.principal, data)
        const store = new WorkspaceStore(replaySession(direct, engine), core)
        await store.outline.reload()
        engine.onReset(() => { void store.outline.reload().then(() => store.versions.bump(['any', 'outline'])) })
        made = { engine, store }
        if (alive) setReplay(made)
        else { void store.dispose(); engine.dispose() }
      } catch (error) {
        if (alive) setProblem(describeError(error))
      }
    })()
    return () => { alive = false; if (made) { void made.store.dispose(); made.engine.dispose() } }
  }, [channel, welcome])

  const owner = welcome.session.owner_actions === true || welcome.principal === welcome.session.owner
  const save = async () => {
    try { await channel.call('session.save', {}); setSaved(true) } catch (e) { setProblem(describeError(e)) }
  }
  const remove = async () => {
    if (!window.confirm(t('aiworkspace.session.replay.deleteConfirm'))) return
    try { await channel.call('session.delete', {}); window.location.reload() } catch (e) { setProblem(describeError(e)) }
  }
  return (
    <div className="aiws-session-room is-replay" data-testid="aiws-session-replay" data-state={state}>
      <div className="aiws-session-replay-head aiws-panel">
        <b>{welcome.session.title}</b>
        <span className="aiws-muted" data-testid="aiws-session-replay-state">{t(STATE_KEY[state])}</span>
        {state === 'ended' && <span className="aiws-warning">{t('aiworkspace.session.replay.unsaved', { at: welcome.session.expires_at ?? '' })}</span>}
        {welcome.session.archive && !welcome.session.archive.complete && <span className="aiws-warning" data-testid="aiws-session-incomplete">{t('aiworkspace.session.replay.incomplete')}</span>}
        <span className="aiws-grow" />
        {owner && state === 'ended' && <button type="button" className="is-primary" data-testid="aiws-session-save" onClick={() => { void save() }}>{t('aiworkspace.session.replay.save')}</button>}
        {owner && <button type="button" className="aiws-tool" aria-label={t('aiworkspace.session.replay.delete')} data-testid="aiws-session-delete" onClick={() => { void remove() }}><Trash2 size={16} /></button>}
      </div>
      {problem && <div className="aiws-error" role="alert" data-testid="aiws-session-replay-problem">{problem}</div>}
      {replay ? <ReplayPlayer engine={replay.engine} store={replay.store} path={path} live={live} channel={channel} timeRef={timeRef} />
        : !problem && <div className="aiws-session-center aiws-muted">{t('aiworkspace.session.replay.loading')}</div>}
      <PostsPanel channel={channel} owner={owner} timeRef={timeRef} />
    </div>
  )
}

function ReplayPlayer({ engine, store, path, live, channel, timeRef }: { engine: ReplayEngine; store: WorkspaceStore; path: ShowPathPayload; live: SessionUi; channel: SessionChannel; timeRef: { current: number } }) {
  const { t } = useAiwsText()
  const [head] = useState(() => new Playhead(engine.duration))
  useSyncExternalStore(head.subscribe, head.snapshot)
  const steps = useSteps(store, path)
  const marks = useMemo(() => engine.marks(), [engine])
  const voice = useMemo(() => engine.voiceSegments(), [engine])
  const audio = useRef<HTMLAudioElement | null>(null)
  const [audioUrl, setAudioUrl] = useState<{ objectId: string; url: string } | null>(null)
  const now = head.now()
  // the content follows the playhead
  useEffect(() => { timeRef.current = now; engine.seek(engine.seqAt(now)) }, [engine, now, timeRef])
  useEffect(() => {
    let raf = 0
    const loop = () => { head.tick(); raf = requestAnimationFrame(loop) }
    raf = requestAnimationFrame(loop)
    return () => cancelAnimationFrame(raf)
  }, [head])
  // the recorded voice: the segment under the playhead, at its offset (§7.3.3)
  const segment = voice.find((s) => now >= s.start && now < s.end) ?? null
  useEffect(() => {
    if (!segment || audioUrl?.objectId === segment.objectId) return
    let alive = true
    store.session.fetchAsset(segment.objectId).then((blob) => { if (alive) setAudioUrl({ objectId: segment.objectId, url: URL.createObjectURL(blob) }) }, () => undefined)
    return () => { alive = false }
  }, [segment, audioUrl, store])
  useEffect(() => () => { if (audioUrl) URL.revokeObjectURL(audioUrl.url) }, [audioUrl])
  useEffect(() => {
    const el = audio.current
    if (!el || !segment || audioUrl?.objectId !== segment.objectId) { el?.pause(); return }
    const want = (now - segment.start) / 1000
    if (Math.abs(el.currentTime - want) > 0.3) el.currentTime = want
    el.playbackRate = head.speed
    if (head.playing && el.paused) void el.play().catch(() => undefined)
    if (!head.playing && !el.paused) el.pause()
  })
  const snap = engine.roomAt(now, 'archived')
  const ui = useMemo<SessionUi>(() => ({
    ...live, replay: true, mayInteract: false, room: null, following: () => true, tRel: () => head.now(),
    media: { get: (id) => engine.mediaAt(head.now(), id), subscribe: head.subscribe },
  }), [live, engine, head])
  return (
    <SessionContext.Provider value={ui}>
      <StoreContext.Provider value={store}>
        <div className="aiws-session-replay-stage">
          <LiveStage store={store} path={path} steps={steps} snap={snap} channel={null} presenter={false} />
        </div>
        <audio ref={audio} src={audioUrl?.url} preload="auto" data-testid="aiws-session-replay-audio" />
        <div className="aiws-panel aiws-session-timeline" data-testid="aiws-session-timeline">
          <button type="button" className="aiws-tool" aria-label={head.playing ? t('aiworkspace.session.replay.pause') : t('aiworkspace.session.replay.play')} data-testid="aiws-session-replay-play"
            onClick={() => (head.playing ? head.pause() : head.play())}>{head.playing ? <Pause size={16} /> : <Play size={16} />}</button>
          <span className="aiws-session-clock" data-testid="aiws-session-replay-clock">{clock(now)} / {clock(engine.duration)}</span>
          <div className="aiws-session-scrub">
            <input type="range" min={0} max={Math.max(1, engine.duration)} step={1} value={Math.round(now)} aria-label={t('aiworkspace.session.replay.position')} data-testid="aiws-session-replay-scrub"
              onChange={(event) => head.seek(Number(event.target.value))} />
            <div className="aiws-session-marks">
              {marks.map((m, i) => <button key={i} type="button" className={`aiws-session-mark is-${m.kind}`} title={m.label} style={{ left: `${(m.t / Math.max(1, engine.duration)) * 100}%` }} onClick={() => head.seek(m.t)} />)}
            </div>
          </div>
          <select aria-label={t('aiworkspace.session.replay.speed')} value={head.speed} data-testid="aiws-session-replay-speed" onChange={(event) => head.setSpeed(Number(event.target.value))}>
            {SPEEDS.map((s) => <option key={s} value={s}>{s}×</option>)}
          </select>
          {voice.length === 0 && <span className="aiws-muted">{t('aiworkspace.session.replay.noVoice')}</span>}
          <span className="aiws-muted" data-testid="aiws-session-replay-step">{steps.length ? `${Math.min(steps.length, (snap.stage.index ?? 0) + 1)} / ${steps.length}` : ''}</span>
          {channel.status === 'failed' && <span className="aiws-warning">{t('aiworkspace.session.channel.failed')}</span>}
        </div>
      </StoreContext.Provider>
    </SessionContext.Provider>
  )
}

/** Comments, moments and ratings (§8): new room data, never inserted into the recorded time line. */
function PostsPanel({ channel, owner, timeRef }: { channel: SessionChannel; owner: boolean; timeRef: { current: number } }) {
  const { t, dateTime } = useAiwsText()
  const [posts, setPosts] = useState<SessionPost[]>([])
  const [open, setOpen] = useState(true)
  const [text, setText] = useState('')
  const [moment, setMoment] = useState(false)
  const [problem, setProblem] = useState<string | null>(null)
  const load = useCallback(() => channel.call<{ posts: SessionPost[]; open: boolean }>('session.list_posts', {}).then((r) => { setPosts(r.posts); setOpen(r.open) }, (e: unknown) => setProblem(describeError(e))), [channel])
  useEffect(() => { void load() }, [load])
  const post = async (params: Record<string, unknown>) => {
    try { await channel.call('session.post', params); setProblem(null); await load() } catch (e) { setProblem(describeError(e)) }
  }
  const mine = channel.welcome?.principal
  const rated = posts.find((p) => p.kind === 'rating' && p.author === mine)
  return (
    <div className="aiws-panel aiws-session-posts" data-testid="aiws-session-posts">
      <div className="aiws-dialog-head">
        <b>{t('aiworkspace.session.posts.title')}</b><span className="aiws-grow" />
        {owner && <button type="button" data-testid="aiws-session-posts-toggle" onClick={() => { void channel.call('session.set_access', { posts: open ? 'closed' : 'open' }).then(load, (e: unknown) => setProblem(describeError(e))) }}>{open ? t('aiworkspace.session.posts.close') : t('aiworkspace.session.posts.open')}</button>}
      </div>
      {problem && <div className="aiws-error" role="alert">{problem}</div>}
      <ul className="aiws-session-list">
        {posts.map((p) => (
          <li key={p.post_id} data-testid={`aiws-session-post-${p.kind}`}>
            <b>{p.author_name ?? p.author}</b>
            {p.kind === 'rating' ? <span>{'★'.repeat(p.score ?? 0)}</span> : <span className="aiws-grow">{p.kind === 'moment' && p.t_rel_ms !== null ? <span className="aiws-muted">[{clock(p.t_rel_ms)}] </span> : null}{p.body}</span>}
            <span className="aiws-muted">{dateTime(p.created_at)}</span>
            {(p.author === mine || owner) && <button type="button" className="aiws-tool" aria-label={t('aiworkspace.session.posts.delete')} onClick={() => { void channel.call('session.delete_post', { post_id: p.post_id }).then(load) }}><Trash2 size={14} /></button>}
          </li>
        ))}
      </ul>
      {open ? (
        <form className="aiws-session-post-form" onSubmit={(event) => { event.preventDefault(); const body = text.trim(); if (!body) return; setText(''); void post(moment ? { kind: 'moment', body, t_rel_ms: Math.round(timeRef.current) } : { kind: 'comment', body }) }}>
          <input data-testid="aiws-session-post-input" value={text} maxLength={4000} placeholder={t('aiworkspace.session.posts.placeholder')} onChange={(event) => setText(event.target.value)} />
          <label className="aiws-check"><input type="checkbox" checked={moment} onChange={(event) => setMoment(event.target.checked)} />{t('aiworkspace.session.posts.atMoment')}</label>
          <button type="submit" data-testid="aiws-session-post-send">{t('aiworkspace.session.posts.send')}</button>
          {!owner && !rated && <span className="aiws-session-rating">{[1, 2, 3, 4, 5].map((n) => <button key={n} type="button" className="aiws-tool" aria-label={t('aiworkspace.session.posts.rate', { n })} data-testid={`aiws-session-rate-${n}`} onClick={() => { void post({ kind: 'rating', score: n }) }}><Star size={14} /></button>)}</span>}
        </form>
      ) : <div className="aiws-muted">{t('aiworkspace.session.posts.closed')}</div>}
    </div>
  )
}
