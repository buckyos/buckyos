import { VOICE_WAVEFORM_MAX, VOICE_WAVEFORM_SAMPLES } from '../../protocol/msgobj'

/** Longest voice note; recording stops and sends when it is reached. */
export const VOICE_MAX_MS = 5 * 60_000

/**
 * Levels sampled while recording, folded into `samples` buckets (the loudest
 * of each) and scaled to 0–`VOICE_WAVEFORM_MAX` against the loudest bucket,
 * so a quiet clip still draws a readable waveform.
 */
export function compressWaveform(levels: readonly number[], samples = VOICE_WAVEFORM_SAMPLES): number[] {
  if (levels.length === 0) return []
  const count = Math.min(samples, levels.length)
  const buckets = Array.from({ length: count }, (_, index) => {
    const start = Math.floor((index * levels.length) / count)
    const end = Math.max(start + 1, Math.floor(((index + 1) * levels.length) / count))
    return Math.max(...levels.slice(start, end).map(level => (Number.isFinite(level) ? Math.max(0, level) : 0)))
  })
  const peak = Math.max(...buckets)
  return buckets.map(level => (peak > 0 ? Math.round((level / peak) * VOICE_WAVEFORM_MAX) : 0))
}

/** `waveform` stretched or squeezed to `bars` levels for drawing; a missing waveform is a flat line. */
export function resampleWaveform(waveform: readonly number[], bars: number): number[] {
  if (waveform.length === 0) return Array.from({ length: bars }, () => 0)
  return Array.from({ length: bars }, (_, index) => {
    const start = Math.floor((index * waveform.length) / bars)
    const end = Math.max(start + 1, Math.floor(((index + 1) * waveform.length) / bars))
    return Math.max(...waveform.slice(start, end))
  })
}

/** Bars a voice bubble draws: longer clips get a wider waveform, as in Telegram. */
export function voiceBarCount(durationMs: number): number {
  return Math.round(Math.min(48, Math.max(24, 24 + (durationMs / 1000 - 2) * 0.9)))
}

export function formatVoiceDuration(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000))
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, '0')}`
}

/** The recording timer: `m:ss,t` with tenths of a second. */
export function formatRecordingTime(ms: number): string {
  return `${formatVoiceDuration(ms)},${Math.floor((Math.max(0, ms) % 1000) / 100)}`
}

const RECORDER_TYPES: ReadonlyArray<readonly [mimeType: string, extension: string]> = [
  ['audio/webm;codecs=opus', 'webm'],
  ['audio/ogg;codecs=opus', 'ogg'],
  ['audio/mp4;codecs=mp4a.40.2', 'm4a'],
  ['audio/mp4', 'm4a'],
  ['audio/webm', 'webm'],
]

/** The first recording format the browser supports (Opus where it can), with its file extension. */
export function pickRecorderType(isTypeSupported: (type: string) => boolean): { mimeType: string; extension: string } | null {
  const match = RECORDER_TYPES.find(([type]) => isTypeSupported(type))
  return match ? { mimeType: match[0], extension: match[1] } : null
}

/** File name of a voice note recorded at `at`, e.g. `voice-20261019-153012.webm`. */
export function voiceFileName(at: Date, extension: string): string {
  const pad = (value: number) => String(value).padStart(2, '0')
  return `voice-${at.getFullYear()}${pad(at.getMonth() + 1)}${pad(at.getDate())}-${pad(at.getHours())}${pad(at.getMinutes())}${pad(at.getSeconds())}.${extension}`
}

export const VOICE_RATES = [1, 1.5, 2] as const

/** Recording needs MediaRecorder and microphone access (secure contexts only). */
export function voiceRecordingSupported(): boolean {
  return typeof window !== 'undefined' && window.isSecureContext && typeof window.MediaRecorder === 'function' && typeof navigator.mediaDevices?.getUserMedia === 'function'
}
