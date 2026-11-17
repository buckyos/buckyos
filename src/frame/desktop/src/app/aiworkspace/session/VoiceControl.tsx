/* The voice control of a room's bar (多人发言 TODO §4.8): the three modes — auto, push to talk, mute — with the
 * microphone's level and whether it is sending, "talk", the request to speak, the listening switch. On a narrow screen
 * it folds into one button (C10): phones are listeners first (§2 裁决 4). */

import { useCallback, useState, useSyncExternalStore } from 'react'
import { Hand, Mic, MicOff, Radio, Volume2, VolumeX } from 'lucide-react'
import { useAiwsText } from '../useAiwsText'
import type { SpeakMode } from './types'
import type { VoiceUi } from './useVoice'

function useNarrow(): boolean {
  const query = '(max-width: 640px)'
  const subscribe = useCallback((listener: () => void) => {
    if (typeof window.matchMedia !== 'function') return () => undefined
    const m = window.matchMedia(query)
    m.addEventListener('change', listener)
    return () => m.removeEventListener('change', listener)
  }, [])
  return useSyncExternalStore(subscribe, () => typeof window.matchMedia === 'function' && window.matchMedia(query).matches)
}

const MODES: SpeakMode[] = ['auto', 'manual', 'mute']

/** The three modes, the level and "talk", the request — or, folded on a narrow screen, one button that opens them. */
export function VoiceControl({ voice, interact, toggle }: { voice: VoiceUi; interact: boolean; toggle?: boolean }) {
  const { t } = useAiwsText()
  const narrow = useNarrow()
  const [open, setOpen] = useState(false)
  const { v } = voice
  if (!voice.available) return null
  const body = (
    <>
      {toggle && (
        <button type="button" className="aiws-tool" aria-label={voice.connected ? t('aiworkspace.session.voice.stop') : t('aiworkspace.session.voice.start')} aria-pressed={voice.connected}
          data-testid="aiws-session-voice" disabled={v.state === 'connecting'} onClick={() => { void (voice.connected ? voice.disconnect() : voice.connect()) }}>
          {voice.connected ? <Radio size={16} /> : <MicOff size={16} />}
        </button>
      )}
      {voice.connected && v.sending && (
        <span className="aiws-session-voice-modes" role="radiogroup" aria-label={t('aiworkspace.session.voice.controls')} data-testid="aiws-session-voice-modes" data-mode={v.mode}>
          {MODES.map((m) => (
            <button key={m} type="button" role="radio" className={`aiws-tool aiws-tool-text${v.mode === m ? ' is-on' : ''}`} aria-checked={v.mode === m} title={t(`aiworkspace.session.voice.modeTitle.${m}`)}
              data-testid={`aiws-session-voice-mode-${m}`} onClick={() => voice.setMode(m)}>
              {m === 'auto' ? <Mic size={14} /> : m === 'manual' ? <Hand size={14} /> : <MicOff size={14} />}<span>{t(`aiworkspace.session.voice.mode.${m}`)}</span>
            </button>
          ))}
        </span>
      )}
      {voice.connected && v.sending && v.mic && (
        <span className={`aiws-session-voice-level${v.transmitting && v.open ? ' is-sending' : ''}`} role="meter" aria-label={t('aiworkspace.session.voice.level')} aria-valuemin={0} aria-valuemax={1} aria-valuenow={v.level}
          title={v.transmitting && v.open ? t('aiworkspace.session.voice.sending') : undefined} data-testid="aiws-session-voice-level" data-open={v.open ? 'true' : 'false'} data-sending={v.transmitting ? 'true' : 'false'}>
          <span style={{ width: `${Math.round(v.level * 100)}%` }} />
        </span>
      )}
      {voice.connected && v.sending && v.mode === 'manual' && v.mic && (
        <button type="button" className={`aiws-tool aiws-tool-text aiws-session-voice-talk${v.open ? ' is-on' : ''}`} data-testid="aiws-session-voice-talk"
          onPointerDown={(e) => { e.currentTarget.setPointerCapture?.(e.pointerId); voice.talk(true) }} onPointerUp={() => voice.talk(false)} onPointerCancel={() => voice.talk(false)}
          onContextMenu={(e) => e.preventDefault()}>
          <Mic size={14} /><span>{t('aiworkspace.session.voice.talk')}</span>
        </button>
      )}
      {voice.connected && !voice.allowed && interact && (
        <button type="button" className={`aiws-tool aiws-tool-text${voice.requested ? ' is-on' : ''}`} aria-pressed={voice.requested} data-testid="aiws-session-voice-request"
          title={voice.requested ? t('aiworkspace.session.voice.requested') : undefined} onClick={() => voice.request(!voice.requested)}>
          <Hand size={14} /><span>{t('aiworkspace.session.voice.request')}</span>
        </button>
      )}
      {voice.connected && (
        <button type="button" className="aiws-tool" aria-label={voice.listenMuted ? t('aiworkspace.session.voice.unmute') : t('aiworkspace.session.voice.mute')} aria-pressed={voice.listenMuted}
          data-testid="aiws-session-listen" onClick={() => voice.setListenMuted(!voice.listenMuted)}>{voice.listenMuted ? <VolumeX size={16} /> : <Volume2 size={16} />}</button>
      )}
    </>
  )
  if (!narrow) return <span className="aiws-session-voice" data-testid="aiws-session-voice-control">{body}</span>
  const icon = !voice.connected ? <MicOff size={16} /> : v.sending && v.mode !== 'mute' ? <Mic size={16} /> : voice.listenMuted ? <VolumeX size={16} /> : <Volume2 size={16} />
  return (
    <span className="aiws-session-voice is-folded" data-testid="aiws-session-voice-control">
      <button type="button" className={`aiws-tool${v.transmitting && v.open ? ' is-on' : ''}`} aria-label={t('aiworkspace.session.voice.controls')} aria-expanded={open} data-testid="aiws-session-voice-menu"
        onClick={() => setOpen(!open)}>{icon}</button>
      {open && <span className="aiws-session-voice-pop aiws-panel" data-testid="aiws-session-voice-pop">{body}</span>}
    </span>
  )
}

/** What the voice has to say (a right given or taken, a refused microphone, a full mix) and the click that lets held
 * audio run. */
export function VoiceNotes({ voice }: { voice: VoiceUi }) {
  const { t } = useAiwsText()
  const lines = [voice.notice, voice.problem, voice.crowded ? t('aiworkspace.session.voice.crowded') : null].filter((x): x is string => !!x)
  if (lines.length === 0 && !voice.v.suspended) return null
  return (
    <div className="aiws-panel aiws-stage-ui aiws-session-voice-note" role="status" data-testid="aiws-session-voice-note">
      {lines.map((l, i) => <span key={i}>{l}</span>)}
      <button type="button" data-testid="aiws-session-voice-ok" onClick={voice.dismiss}>{voice.v.suspended ? t('aiworkspace.session.voice.resume') : t('aiworkspace.session.voice.ok')}</button>
    </div>
  )
}
