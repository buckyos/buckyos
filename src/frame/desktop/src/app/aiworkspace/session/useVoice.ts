/* The voice of a terminal in a room (多人发言 TODO §4.3, §4.8): the connection to the host's mix for everyone, and for
 * whoever may speak the three modes. The right to speak follows the room's `voice.allow` events (the host decides,
 * §4.2); a manager's "mute all" switches this terminal to mute; the mode one picked is kept per session on this
 * device; "too many speaking" is read off `voice.speaking` (Q2). */

import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { describeError } from '../api/session'
import { useAiwsText } from '../useAiwsText'
import type { SessionChannel, Welcome } from './channel'
import type { RoomSnapshot } from './room'
import type { SpeakMode } from './types'
import { VoiceClient, VoiceUnreachable, type VoiceSnapshot } from './voice'

const KEPT = 'aiworkspace.session-voice.'
/** Hold to talk (manual mode): not one of the stage's keys. */
const TALK_KEY = 'KeyT'
const CROWDED_MS = 1500

interface Kept { mode?: SpeakMode; noticed?: boolean }

function kept(sessionId: string): Kept {
  try { return JSON.parse(window.localStorage.getItem(KEPT + sessionId) ?? '{}') as Kept } catch { return {} }
}

function keep(sessionId: string, patch: Kept) {
  try { window.localStorage.setItem(KEPT + sessionId, JSON.stringify({ ...kept(sessionId), ...patch })) } catch { /* not kept */ }
}

function touchScreen(): boolean {
  return typeof window.matchMedia === 'function' && window.matchMedia('(pointer: coarse)').matches
}

/** The media addresses a browser elsewhere would use (the loopback one is for the host itself). */
export function mediaHosts(welcome: Welcome): string {
  const service = welcome.session.voice_service
  return (service?.hosts ?? []).filter((h) => !h.startsWith('127.') && !h.startsWith('[::1]')).join(', ') || `UDP ${service?.port ?? ''}`
}

export interface VoiceUi {
  /** The room has voice and the host's media port is up. */
  available: boolean
  v: VoiceSnapshot
  connected: boolean
  /** The person may speak (§4.2). */
  allowed: boolean
  /** Asked to speak (Q6). */
  requested: boolean
  /** Speaking now, but not heard: K others are (Q2). */
  crowded: boolean
  problem: string | null
  notice: string | null
  listenMuted: boolean
  connect: () => Promise<void>
  disconnect: () => Promise<void>
  setMode: (mode: SpeakMode) => void
  talk: (down: boolean) => void
  request: (on: boolean) => void
  setListenMuted: (muted: boolean) => void
  dismiss: () => void
}

/** The voice of this terminal. `auto`: connect as soon as the room is live (the audience joined with a click). */
export function useVoice({ channel, welcome, snap, auto }: { channel: SessionChannel; welcome: Welcome; snap: RoomSnapshot; auto: boolean }): VoiceUi {
  const { t } = useAiwsText()
  const [client] = useState(() => new VoiceClient(channel))
  const v = useSyncExternalStore(client.subscribe, client.snapshot)
  const sid = welcome.session.session_id
  const me = welcome.principal
  const presents = welcome.role === 'presenter' || welcome.role === 'prompter'
  const available = welcome.session.voice && welcome.session.voice_service?.available === true
  const mine = snap.voices[me]
  const allowed = presents || (mine?.allow ?? welcome.session.my_speak === true)
  const startMode: SpeakMode = presents ? 'auto' : mine?.startMode ?? welcome.session.speak_mode ?? 'manual'
  const [problem, setProblem] = useState<string | null>(null)
  const [notice, setNotice] = useState<string | null>(null)
  const [listenMuted, setListenMutedState] = useState(false)
  const [crowded, setCrowded] = useState(false)
  const [dismissed, setDismissed] = useState<string | null>(null)
  const connected = v.state === 'connected'
  const modeName = useCallback((m: SpeakMode) => t(`aiworkspace.session.voice.mode.${m}`), [t])

  const report = useCallback((mode: SpeakMode) => {
    keep(sid, { mode })
    void channel.call('session.voice', { op: 'mode', mode }).catch(() => undefined)
  }, [channel, sid])

  const failed = useCallback((error: unknown) => {
    setProblem(error instanceof VoiceUnreachable ? t('aiworkspace.session.voice.unreachable', { hosts: mediaHosts(welcome) }) : t('aiworkspace.session.voice.failed', { detail: describeError(error) }))
  }, [t, welcome])

  const connect = useCallback(async () => {
    if (!available) return
    setProblem(null)
    const mode = allowed ? kept(sid).mode ?? startMode : 'mute'
    try {
      await client.start(allowed, mode)
      if (client.snapshot().sending) report(client.snapshot().mode)
      if (allowed && !presents && !kept(sid).noticed) {
        keep(sid, { noticed: true })
        setNotice([t(welcome.session.speak === 'all' ? 'aiworkspace.session.voice.allSpeak' : 'aiworkspace.session.voice.granted', { mode: modeName(client.snapshot().mode) }),
                   client.snapshot().mode === 'auto' ? t('aiworkspace.session.voice.inRoom') : '', touchScreen() ? t('aiworkspace.session.voice.usePc') : ''].filter(Boolean).join(' '))
      }
    } catch (error) {
      failed(error)
    }
  }, [available, allowed, sid, startMode, client, report, presents, t, welcome.session.speak, modeName, failed])

  // the audience is in the voice from the start of the class (the join button was the gesture)
  const live = snap.state === 'live'
  const started = useRef(false)
  useEffect(() => {
    if (!auto || !available || !live || started.current) return
    started.current = true
    void connect()
  }, [auto, available, live, connect])
  // the room goes, the voice goes (and comes back if the room does: React's development remount)
  useEffect(() => () => { started.current = false; void client.stop() }, [client])

  // the right to speak came or went
  const was = useRef(allowed)
  useEffect(() => {
    if (was.current === allowed) return
    was.current = allowed
    if (!connected) return
    const mode = allowed ? kept(sid).mode ?? startMode : 'mute'
    void client.setAllowed(allowed, mode).then(() => {
      if (!allowed) { setNotice(t('aiworkspace.session.voice.revoked')); return }
      if (client.snapshot().sending) report(client.snapshot().mode)
      keep(sid, { noticed: true })
      setNotice([t('aiworkspace.session.voice.granted', { mode: modeName(client.snapshot().mode) }), client.snapshot().mode === 'auto' ? t('aiworkspace.session.voice.inRoom') : '',
                 touchScreen() ? t('aiworkspace.session.voice.usePc') : ''].filter(Boolean).join(' '))
    }, (e: unknown) => setProblem(describeError(e)))
  }, [allowed, connected, client, sid, startMode, report, t, modeName])

  // a manager muted everyone (Q7): only what happens while this terminal is in the room
  const head = welcome.session.head_event ?? 0
  const lastMute = useRef(snap.muteAll)
  useEffect(() => {
    if (snap.muteAll === lastMute.current) return
    lastMute.current = snap.muteAll
    if (presents || snap.muteAll <= head || !client.snapshot().sending) return
    void client.setMode('mute').then(() => { keep(sid, { mode: 'mute' }); setNotice(t('aiworkspace.session.voice.mutedAll')) })
  }, [snap.muteAll, presents, head, client, sid, t])

  // speaking but not heard: the mix is full (Q2)
  const max = welcome.session.voice_service?.mixing?.max_speakers ?? 4
  const speakingOthers = Object.keys(snap.speaking).filter((p) => p !== me).length
  const unheard = v.transmitting && v.open && !(me in snap.speaking) && speakingOthers >= max
  useEffect(() => {
    if (!unheard) return
    const timer = window.setTimeout(() => setCrowded(true), CROWDED_MS)
    return () => { window.clearTimeout(timer); setCrowded(false) }
  }, [unheard])

  // push to talk: hold T (never in a text field)
  const manual = connected && v.sending && v.mode === 'manual'
  useEffect(() => {
    if (!manual) return
    const typing = (e: KeyboardEvent) => e.target instanceof Element && !!e.target.closest('input, textarea, select, [contenteditable="true"]')
    const down = (e: KeyboardEvent) => { if (e.code === TALK_KEY && !e.repeat && !e.ctrlKey && !e.metaKey && !e.altKey && !typing(e)) { e.preventDefault(); client.talk(true) } }
    const up = (e: KeyboardEvent) => { if (e.code === TALK_KEY) client.talk(false) }
    const blur = () => client.talk(false)
    window.addEventListener('keydown', down)
    window.addEventListener('keyup', up)
    window.addEventListener('blur', blur)
    return () => { window.removeEventListener('keydown', down); window.removeEventListener('keyup', up); window.removeEventListener('blur', blur); client.talk(false) }
  }, [manual, client])

  // what the connection itself says: shown until dismissed
  const state = v.state === 'lost' ? 'aiworkspace.session.voice.lost' : v.micDenied ? 'aiworkspace.session.voice.micDenied' : null
  const shown = problem ?? (state && state !== dismissed ? t(state) : null)

  return {
    available, v, connected, allowed, requested: !allowed && mine?.request === true, crowded, problem: shown, notice, listenMuted,
    connect,
    disconnect: async () => { await client.stop() },
    setMode: (mode) => { setProblem(null); void client.setMode(mode).then(() => report(client.snapshot().mode)) },
    talk: (down) => client.talk(down),
    request: (on) => { void channel.call('session.voice', { op: 'request', on }).catch((e: unknown) => setProblem(describeError(e))) },
    setListenMuted: (muted) => { client.setListenMuted(muted); setListenMutedState(muted) },
    dismiss: () => { setNotice(null); setProblem(null); setDismissed(state); client.resume() },
  }
}

