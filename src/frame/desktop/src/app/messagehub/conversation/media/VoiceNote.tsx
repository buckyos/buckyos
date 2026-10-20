import { useRef } from 'react'
import { AlertCircle, Loader2, Pause, Play } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import type { VoiceNote } from '../../protocol/msgobj'
import { formatVoiceDuration, resampleWaveform, voiceBarCount } from './voice'
import { useVoicePlayback, useVoicePlayed, useVoiceRate, voicePlayer } from './voicePlayer'

const SEEK_STEP_MS = 5000

/**
 * A voice note as Telegram draws it: a round play button, the waveform that
 * fills as it plays (click or drag to seek) and the time below it. A received
 * note shows a dot until it was first played on this device.
 */
export function VoiceNoteView({ id, objId, voice, isSelf }: { id: string; objId: string; voice: VoiceNote; isSelf: boolean }) {
  const { t } = useI18n()
  const playback = useVoicePlayback(id)
  const played = useVoicePlayed(id)
  const rate = useVoiceRate()
  const waveRef = useRef<HTMLDivElement>(null)
  const dragging = useRef(false)
  const duration = voice.duration_ms
  const bars = resampleWaveform(voice.waveform, voiceBarCount(duration))
  const active = playback.playing || playback.loading || playback.positionMs > 0
  const ratio = duration > 0 ? Math.min(1, playback.positionMs / duration) : 0
  const shown = active ? playback.positionMs : duration

  const seekAt = (clientX: number) => {
    const rect = waveRef.current?.getBoundingClientRect()
    if (!rect || rect.width <= 0) return
    voicePlayer.seek(id, objId, Math.min(1, Math.max(0, (clientX - rect.left) / rect.width)) * duration)
  }

  const playLabel = playback.error ? `${t('messagehub.voice.unavailable')} · ${t('messagehub.retry')}` : t(playback.playing ? 'messagehub.voice.pause' : 'messagehub.voice.play')
  return (
    <div className="mh-voice" data-self={isSelf || undefined} data-testid="voice-note" data-state={playback.playing ? 'playing' : playback.loading ? 'loading' : active ? 'paused' : 'idle'} data-no-details>
      <button type="button" className="mh-voice-play" onClick={() => voicePlayer.toggle(id, objId)} aria-label={playLabel} title={playLabel} data-testid="voice-play">
        {playback.error ? <AlertCircle size={20} /> : playback.loading ? <Loader2 size={20} className="animate-spin" /> : playback.playing ? <Pause size={18} fill="currentColor" /> : <Play size={18} fill="currentColor" className="translate-x-px" />}
      </button>
      <div className="flex min-w-0 flex-col gap-1">
        <div
          ref={waveRef}
          role="slider"
          tabIndex={0}
          aria-label={t('messagehub.voice.position')}
          aria-valuemin={0}
          aria-valuemax={Math.round(duration / 1000)}
          aria-valuenow={Math.round(playback.positionMs / 1000)}
          aria-valuetext={`${formatVoiceDuration(playback.positionMs)} / ${formatVoiceDuration(duration)}`}
          className="mh-voice-wave"
          data-testid="voice-waveform"
          onPointerDown={(event) => {
            if (event.button !== 0) return
            dragging.current = true
            event.currentTarget.setPointerCapture(event.pointerId)
            seekAt(event.clientX)
          }}
          onPointerMove={(event) => { if (dragging.current) seekAt(event.clientX) }}
          onPointerUp={() => { dragging.current = false }}
          onPointerCancel={() => { dragging.current = false }}
          onKeyDown={(event) => {
            const step = event.key === 'ArrowRight' ? SEEK_STEP_MS : event.key === 'ArrowLeft' ? -SEEK_STEP_MS : 0
            const target = event.key === 'Home' ? 0 : event.key === 'End' ? duration - 1 : step ? playback.positionMs + step : null
            if (target === null) return
            event.preventDefault()
            voicePlayer.seek(id, objId, Math.min(duration - 1, Math.max(0, target)))
          }}
        >
          {bars.map((level, index) => (
            <span key={index} data-played={(index + 0.5) / bars.length <= ratio || undefined} style={{ height: `${2 + (level / 31) * 18}px` }} />
          ))}
        </div>
        <div className="flex items-center gap-1.5 text-[12px] leading-4 tabular-nums">
          <span className="mh-voice-time" data-testid="voice-time">{formatVoiceDuration(shown)}</span>
          {!isSelf && !played ? <span className="mh-voice-unplayed" role="img" aria-label={t('messagehub.voice.unplayed')} title={t('messagehub.voice.unplayed')} data-testid="voice-unplayed" /> : null}
          {active ? (
            <button type="button" className="mh-voice-rate" onClick={() => voicePlayer.cycleRate()} aria-label={t('messagehub.voice.speed', undefined, { rate: String(rate) })} title={t('messagehub.voice.speed', undefined, { rate: String(rate) })} data-testid="voice-rate">{rate}×</button>
          ) : null}
        </div>
      </div>
    </div>
  )
}
