/* eslint-disable react-refresh/only-export-components -- the player and its pure sync rules */
/* Media in a session (第四期规划 §7.3.2, Q24): the sound of a video or audio Block is not relayed. The presenter's
 * player reports play / pause / seek / rate (and a `sync` anchor every few seconds) as commands the service confirms
 * and timestamps; every follower plays the same asset from the run Workspace by Range and keeps to the presenter's
 * position: a deviation over 400 ms seeks, 80–400 ms is caught up through the playback rate (±5 %), less is left
 * alone. Following, the audience's controls are off; browsing on one's own, they affect this device only. */

import { useEffect, useRef, useState } from 'react'
import type { Transport } from '../api/transport'
import { useStore } from '../state/hooks'
import { useAiwsText } from '../useAiwsText'
import { streamUrl } from './api'
import { useSession, type SessionUi } from './context'
import type { MediaState } from './types'

const HARD_MS = 400
const SOFT_MS = 80
const SYNC_EVERY_MS = 5000
const CHECK_MS = 500

/** Where the presenter's media is now, by the demonstration clock. */
export function targetPosition(state: MediaState, tRel: number): number {
  if (!state.playing) return state.media_pos_ms
  return state.media_pos_ms + Math.max(0, tRel - state.t_rel_ms) * state.rate
}

/** One correction step of a follower: what to do about `currentMs` against `targetMs`. */
export function correction(currentMs: number, targetMs: number, rate: number): { seek?: number; playbackRate: number } {
  const diff = targetMs - currentMs
  if (Math.abs(diff) > HARD_MS) return { seek: targetMs, playbackRate: rate }
  if (Math.abs(diff) > SOFT_MS) return { playbackRate: rate * (diff > 0 ? 1.05 : 0.95) }
  return { playbackRate: rate }
}

function useStreamUrl(transport: Transport, workspaceId: string, objectId: string): string | null {
  const [url, setUrl] = useState<string | null>(null)
  useEffect(() => {
    let live = true
    streamUrl(transport, workspaceId, objectId).then((u) => { if (live) setUrl(u) }, () => { if (live) setUrl(null) })
    return () => { live = false }
  }, [transport, workspaceId, objectId])
  return url
}

/** A video or sound on a session's stage. */
export function SessionMediaPlayer({ entityId, objectId, kind, name }: { entityId: string; objectId: string; kind: 'video' | 'audio'; name: string }) {
  const session = useSession()!
  const store = useStore()
  const { t } = useAiwsText()
  const url = useStreamUrl(session.channel.api.transport, store.session.workspaceId, objectId)
  const ref = useRef<HTMLVideoElement & HTMLAudioElement>(null)
  const presenter = session.role === 'presenter' && !session.replay
  const [blocked, setBlocked] = useState(false)

  // the presenter's player: every action is a command, and an anchor while it plays
  useEffect(() => {
    const el = ref.current
    if (!el || !presenter) return
    const send = (action: MediaState['action']) => {
      void session.channel.call('session.command', { command: { command_id: `m${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`, kind: action === 'sync' ? 'sync' : 'media',
        args: { entity_id: entityId, action, media_pos_ms: Math.round(el.currentTime * 1000), rate: el.playbackRate } } }).catch(() => undefined)
    }
    const on: [string, MediaState['action']][] = [['play', 'play'], ['pause', 'pause'], ['seeked', 'seek'], ['ratechange', 'rate'], ['ended', 'ended']]
    const handlers = on.map(([name, action]) => { const h = () => send(action); el.addEventListener(name, h); return [name, h] as const })
    const anchor = window.setInterval(() => { if (!el.paused) send('sync') }, SYNC_EVERY_MS)
    return () => { for (const [name, h] of handlers) el.removeEventListener(name, h); window.clearInterval(anchor) }
  }, [presenter, session, entityId, url])

  // a follower keeps to the presenter
  useEffect(() => {
    const el = ref.current
    if (!el || presenter) return
    const apply = () => follow(session, el, entityId, setBlocked)
    apply()
    const off = session.media.subscribe(apply)
    const timer = window.setInterval(apply, CHECK_MS)
    return () => { off(); window.clearInterval(timer) }
  }, [presenter, session, entityId, url])

  if (!url) return <div className="aiws-muted aiws-media-loading">{t('aiworkspace.session.media.loading')}</div>
  const following = !presenter && session.following()
  const common = { ref, src: url, controls: presenter || !following, preload: 'auto' as const, 'data-testid': `aiws-session-media-${entityId}`, 'aria-label': name, onPointerDown: (e: React.PointerEvent) => e.stopPropagation() }
  return (
    <div className={`aiws-media-player aiws-session-media is-${kind}`} data-role="editor">
      {kind === 'video' ? <video {...common} playsInline /> : <audio {...common} />}
      {blocked && <button type="button" className="aiws-session-unmute" data-testid="aiws-session-media-unblock" onClick={() => { setBlocked(false); void ref.current?.play().catch(() => setBlocked(true)) }}>{t('aiworkspace.session.media.tapToPlay')}</button>}
    </div>
  )
}

function follow(session: SessionUi, el: HTMLMediaElement, entityId: string, blocked: (v: boolean) => void) {
  if (!session.following()) return
  const state = session.media.get(entityId)
  if (!state) return
  const target = targetPosition(state, session.tRel())
  if (!state.playing) {
    if (!el.paused) el.pause()
    if (Math.abs(el.currentTime * 1000 - target) > SOFT_MS) el.currentTime = target / 1000
    return
  }
  const c = correction(el.currentTime * 1000, target, state.rate)
  if (c.seek !== undefined) el.currentTime = c.seek / 1000
  if (Math.abs(el.playbackRate - c.playbackRate) > 1e-3) el.playbackRate = c.playbackRate
  if (el.paused) el.play().then(() => blocked(false), () => blocked(true))
}
