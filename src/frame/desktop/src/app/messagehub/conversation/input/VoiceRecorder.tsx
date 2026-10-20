import { useCallback, useEffect, useRef, useState } from 'react'
import { AlertTriangle, ChevronLeft, ChevronUp, Lock, Mic, Send, Trash2 } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import type { VoiceNote } from '../../protocol/msgobj'
import { compressWaveform, formatRecordingTime, pickRecorderType, VOICE_MAX_MS, voiceFileName } from '../media/voice'
import { voicePlayer } from '../media/voicePlayer'

/** A press released sooner than this is a tap: it records hands-free instead of sending. */
const TAP_MS = 350
const CANCEL_DX = 110
const LOCK_DY = 70
const MIN_VOICE_MS = 500
const METER_MS = 50
const LIVE_BARS = 40
const ERROR_MS = 8000
/**
 * Below half a 16-bit step: a microphone that delivers nothing at all (muted,
 * a closed laptop lid, a virtual or wrong input) records exact zeros, while a
 * working one always picks up some noise.
 */
const SILENCE_PEAK = 1 / 65536
const SILENCE_HINT_MS = 1500

type RecorderError = 'denied' | 'unavailable' | 'failed' | 'silent'

interface Recording {
  stream: MediaStream
  recorder: MediaRecorder
  chunks: Blob[]
  extension: string
  context: AudioContext | null
  levels: number[]
  /** Loudest sample heard, and how long the meter was really listening (a suspended AudioContext hears nothing). */
  peak: number
  measuredMs: number
  startedAt: number
  timer: number
}

function useVoiceRecorder(onLimit: () => void) {
  const [phase, setPhase] = useState<'idle' | 'starting' | 'recording'>('idle')
  const [elapsed, setElapsed] = useState(0)
  const [live, setLive] = useState<number[]>([])
  const [error, setError] = useState<RecorderError | null>(null)
  const [noSignal, setNoSignal] = useState(false)
  const [device, setDevice] = useState('')
  const recording = useRef<Recording | null>(null)
  // Bumped by every start / stop, so a start still waiting for the
  // microphone is dropped once it was cancelled.
  const generation = useRef(0)
  const limitRef = useRef(onLimit)
  useEffect(() => { limitRef.current = onLimit }, [onLimit])

  const release = useCallback((current: Recording) => {
    window.clearInterval(current.timer)
    current.stream.getTracks().forEach(track => track.stop())
    void current.context?.close().catch(() => undefined)
  }, [])

  const cancel = useCallback(() => {
    generation.current++
    const current = recording.current
    recording.current = null
    if (current) {
      current.recorder.ondataavailable = null
      if (current.recorder.state !== 'inactive') current.recorder.stop()
      release(current)
    }
    setPhase('idle')
    setElapsed(0)
    setLive([])
    setNoSignal(false)
  }, [release])

  const start = useCallback(async (): Promise<boolean> => {
    if (recording.current) return true
    const attempt = ++generation.current
    setError(null)
    setPhase('starting')
    let stream: MediaStream
    try {
      stream = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true } })
    } catch (failure) {
      if (attempt === generation.current) {
        const name = failure instanceof DOMException ? failure.name : ''
        setError(name === 'NotAllowedError' || name === 'SecurityError' ? 'denied' : name === 'NotFoundError' || name === 'OverconstrainedError' ? 'unavailable' : 'failed')
        setPhase('idle')
      }
      return false
    }
    if (attempt !== generation.current) {
      stream.getTracks().forEach(track => track.stop())
      return false
    }
    const type = pickRecorderType(value => MediaRecorder.isTypeSupported(value))
    let recorder: MediaRecorder
    try {
      recorder = new MediaRecorder(stream, { ...(type ? { mimeType: type.mimeType } : {}), audioBitsPerSecond: 32_000 })
    } catch {
      stream.getTracks().forEach(track => track.stop())
      setError('failed')
      setPhase('idle')
      return false
    }
    let context: AudioContext | null = null
    let analyser: AnalyserNode | null = null
    try {
      context = new AudioContext()
      void context.resume().catch(() => undefined)
      analyser = context.createAnalyser()
      analyser.fftSize = 1024
      context.createMediaStreamSource(stream).connect(analyser)
    } catch {
      analyser = null
    }
    const track = stream.getAudioTracks()[0]
    setDevice(track?.label ?? '')
    const samples = new Float32Array(analyser?.fftSize ?? 0)
    const current: Recording = { stream, recorder, chunks: [], extension: type?.extension ?? 'webm', context, levels: [], peak: 0, measuredMs: 0, startedAt: performance.now(), timer: 0 }
    recorder.ondataavailable = event => { if (event.data.size > 0) current.chunks.push(event.data) }
    recorder.start(250)
    current.timer = window.setInterval(() => {
      let level = 0
      if (analyser && context?.state === 'running') {
        analyser.getFloatTimeDomainData(samples)
        let sum = 0
        for (const value of samples) {
          sum += value * value
          current.peak = Math.max(current.peak, Math.abs(value))
        }
        level = Math.sqrt(sum / samples.length)
        current.measuredMs += METER_MS
      }
      current.levels.push(level)
      const ms = performance.now() - current.startedAt
      setElapsed(ms)
      setLive(previous => [...previous.slice(1 - LIVE_BARS), level])
      setNoSignal(track?.muted === true || (current.measuredMs >= SILENCE_HINT_MS && current.peak < SILENCE_PEAK))
      if (ms >= VOICE_MAX_MS) limitRef.current()
    }, METER_MS)
    recording.current = current
    setPhase('recording')
    return true
  }, [])

  /**
   * Stops recording; resolves to the clip, or `null` when nothing usable was
   * recorded. A clip that is nothing but digital silence is not sent: the
   * microphone delivered no sound, and the error says so.
   */
  const finish = useCallback(async (): Promise<{ file: File; voice: VoiceNote } | null> => {
    const current = recording.current
    if (!current) {
      cancel()
      return null
    }
    recording.current = null
    generation.current++
    const durationMs = Math.round(performance.now() - current.startedAt)
    window.clearInterval(current.timer)
    if (current.recorder.state !== 'inactive') {
      await new Promise<void>(resolve => {
        current.recorder.addEventListener('stop', () => resolve(), { once: true })
        current.recorder.stop()
      })
    }
    release(current)
    setPhase('idle')
    setElapsed(0)
    setLive([])
    setNoSignal(false)
    const type = (current.recorder.mimeType || current.chunks[0]?.type || `audio/${current.extension}`).split(';')[0]
    const blob = new Blob(current.chunks, { type })
    if (blob.size === 0 || durationMs < MIN_VOICE_MS) return null
    if (current.measuredMs >= Math.min(1000, durationMs * 0.8) && current.peak < SILENCE_PEAK) {
      setError('silent')
      return null
    }
    const now = new Date()
    return {
      file: new File([blob], voiceFileName(now, current.extension), { type, lastModified: now.getTime() }),
      voice: { duration_ms: durationMs, waveform: compressWaveform(current.levels) },
    }
  }, [cancel, release])

  useEffect(() => cancel, [cancel])
  useEffect(() => {
    if (!error) return
    const timer = window.setTimeout(() => setError(null), ERROR_MS)
    return () => window.clearTimeout(timer)
  }, [error])

  return { phase, elapsed, live, error, noSignal, device, start, finish, cancel }
}

const levelOf = (value: number | undefined) => Math.min(1, (value ?? 0) * 5)

/**
 * The microphone button of an empty composer and, while recording, the bar
 * over the input (Telegram's voice notes). Hold to record and release to
 * send; slide left to discard, slide up to lock. A tap (or Enter / Space)
 * records hands-free: the button then sends and the bin discards. Escape
 * discards in either mode.
 */
export function VoiceRecorder({ onSend, onActiveChange }: { onSend: (file: File, voice: VoiceNote) => void; onActiveChange: (active: boolean) => void }) {
  const { t } = useI18n()
  const [locked, setLocked] = useState(false)
  const [drag, setDrag] = useState({ dx: 0, dy: 0 })
  const gesture = useRef<{ pointerId: number; x: number; y: number; at: number } | null>(null)
  const pressed = useRef<number | null>(null)
  const ignoreClickUntil = useRef(0)
  const limitSend = useRef<() => void>(() => undefined)
  const { phase, elapsed, live, error, noSignal, device, start, finish, cancel } = useVoiceRecorder(useCallback(() => limitSend.current(), []))
  const active = phase !== 'idle'

  const send = useCallback(async () => {
    gesture.current = null
    setLocked(false)
    setDrag({ dx: 0, dy: 0 })
    const clip = await finish()
    if (clip) onSend(clip.file, clip.voice)
  }, [finish, onSend])
  useEffect(() => { limitSend.current = () => { void send() } }, [send])

  const discard = useCallback(() => {
    gesture.current = null
    setLocked(false)
    setDrag({ dx: 0, dy: 0 })
    cancel()
  }, [cancel])

  const begin = useCallback((hold: boolean) => {
    voicePlayer.pause()
    setLocked(!hold)
    setDrag({ dx: 0, dy: 0 })
    void start().then(ok => {
      if (ok) return
      gesture.current = null
      setLocked(false)
    })
  }, [start])

  useEffect(() => { onActiveChange(active) }, [active, onActiveChange])
  useEffect(() => {
    if (!active) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      event.preventDefault()
      discard()
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [active, discard])

  const handlePointerDown = (event: React.PointerEvent<HTMLButtonElement>) => {
    if (active || event.button !== 0) return
    event.preventDefault()
    event.currentTarget.setPointerCapture(event.pointerId)
    pressed.current = event.pointerId
    gesture.current = { pointerId: event.pointerId, x: event.clientX, y: event.clientY, at: performance.now() }
    begin(true)
  }

  const handlePointerMove = (event: React.PointerEvent<HTMLButtonElement>) => {
    const current = gesture.current
    if (!current || current.pointerId !== event.pointerId) return
    const dx = Math.max(0, current.x - event.clientX)
    const dy = Math.max(0, current.y - event.clientY)
    if (dx > CANCEL_DX) {
      discard()
      return
    }
    if (dy > LOCK_DY) {
      gesture.current = null
      setLocked(true)
      setDrag({ dx: 0, dy: 0 })
      return
    }
    setDrag({ dx, dy })
  }

  const handlePointerEnd = (event: React.PointerEvent<HTMLButtonElement>) => {
    if (pressed.current !== event.pointerId) return
    pressed.current = null
    ignoreClickUntil.current = performance.now() + 500
    const current = gesture.current
    if (!current) return
    gesture.current = null
    setDrag({ dx: 0, dy: 0 })
    // Released before recording started (a permission prompt) or right away: keep recording hands-free.
    if (event.type === 'pointercancel' || phase !== 'recording' || performance.now() - current.at < TAP_MS) {
      setLocked(true)
      return
    }
    void send()
  }

  const handleClick = (event: React.MouseEvent<HTMLButtonElement>) => {
    // The click that follows a press is the press itself; Enter / Space clicks have no detail.
    if (event.detail > 0 && performance.now() < ignoreClickUntil.current) return
    event.preventDefault()
    if (!active) begin(false)
    else if (locked && phase === 'recording') void send()
  }

  const level = levelOf(live.at(-1))
  const label = !active ? t('messagehub.voice.record') : locked ? t('messagehub.voice.send') : t('messagehub.voice.release')
  const deviceName = device || t('messagehub.voice.defaultDevice')
  const errorText = error === 'denied' ? t('messagehub.voice.denied') : error === 'unavailable' ? t('messagehub.voice.noMic') : error === 'silent' ? t('messagehub.voice.silent', undefined, { device: deviceName }) : error ? t('messagehub.voice.failed') : ''
  const noSignalText = t('messagehub.voice.noSignal', undefined, { device: deviceName })
  return (
    <>
      {active ? (
        <div className="mh-voice-recording" data-testid="voice-recording" data-mode={locked ? 'locked' : 'hold'} data-phase={phase}>
          {locked ? (
            <button type="button" className="mh-voice-discard" onClick={discard} aria-label={t('messagehub.voice.discard')} title={t('messagehub.voice.discard')} data-testid="voice-discard">
              <Trash2 size={18} />
            </button>
          ) : null}
          <span className="mh-voice-rec-dot" data-live={phase === 'recording' || undefined} aria-hidden />
          <span className="text-[15px] tabular-nums" role="timer" aria-label={t('messagehub.voice.recording')} data-testid="voice-timer">{formatRecordingTime(elapsed)}</span>
          {noSignal ? (
            <span className="mh-voice-warning" role="status" title={noSignalText} data-testid="voice-no-signal">
              <AlertTriangle size={15} aria-hidden />
              <span className="truncate">{noSignalText}</span>
            </span>
          ) : locked ? (
            <span className="mh-voice-live" aria-hidden>
              {live.map((value, index) => <span key={index} style={{ height: `${3 + levelOf(value) * 19}px` }} />)}
            </span>
          ) : (
            <span className="mh-voice-slide" style={{ transform: `translateX(${-drag.dx * 0.6}px)`, opacity: Math.max(0.15, 1 - drag.dx / CANCEL_DX) }}>
              <ChevronLeft size={16} aria-hidden />
              {t('messagehub.voice.slideToCancel')}
            </span>
          )}
        </div>
      ) : null}
      <div className="relative flex-shrink-0">
        {active && !locked ? (
          <span className="mh-voice-lock" style={{ transform: `translateY(${-Math.min(drag.dy, LOCK_DY)}px)`, opacity: 0.55 + Math.min(1, drag.dy / LOCK_DY) * 0.45 }} title={t('messagehub.voice.lockHint')} data-testid="voice-lock">
            <Lock size={14} aria-hidden />
            <ChevronUp size={14} aria-hidden />
          </span>
        ) : null}
        {errorText ? <p role="alert" className="mh-voice-error" data-testid="voice-error">{errorText}</p> : null}
        <button
          type="button"
          className="flex min-h-11 min-w-11 select-none items-center justify-center"
          style={{ touchAction: 'none', WebkitTouchCallout: 'none' }}
          onPointerDown={handlePointerDown}
          onPointerMove={handlePointerMove}
          onPointerUp={handlePointerEnd}
          onPointerCancel={handlePointerEnd}
          onClick={handleClick}
          onContextMenu={(event) => event.preventDefault()}
          aria-label={label}
          title={active ? label : t('messagehub.voice.holdHint')}
          data-testid="voice-record"
          data-state={!active ? 'idle' : locked ? 'locked' : 'hold'}
        >
          <span className="mh-voice-button" data-state={!active ? 'idle' : locked ? 'locked' : 'hold'} style={active && !locked ? { transform: `scale(${1.3 + level * 0.25})` } : undefined}>
            {active && locked ? <Send size={16} /> : <Mic size={18} />}
          </span>
        </button>
      </div>
    </>
  )
}
