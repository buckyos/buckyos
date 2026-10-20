import { useCallback, useSyncExternalStore } from 'react'
import { getObjectAccess } from '../history/objectAccess'
import { VOICE_RATES } from './voice'

export interface VoicePlayback {
  playing: boolean
  loading: boolean
  /** Where playback stands; kept while paused. */
  positionMs: number
  error: boolean
}

const IDLE: VoicePlayback = { playing: false, loading: false, positionMs: 0, error: false }
const STORAGE_KEY = 'buckyos.messagehub.voice.v1'
const PLAYED_LIMIT = 500

interface Stored { rate: number; played: string[] }

function readStored(): Stored {
  try {
    const value: unknown = JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? 'null')
    const { rate, played } = value && typeof value === 'object' ? value as { rate?: unknown; played?: unknown } : {}
    return {
      rate: VOICE_RATES.includes(rate as never) ? rate as number : 1,
      played: Array.isArray(played) ? played.filter((item): item is string => typeof item === 'string').slice(-PLAYED_LIMIT) : [],
    }
  } catch {
    return { rate: 1, played: [] }
  }
}

/**
 * The one audio element that plays voice notes: starting one stops the
 * other, as in Telegram. The playback speed and which received notes were
 * played are remembered on this device.
 */
class VoicePlayer {
  private audio: HTMLAudioElement | null = null
  private key: string | null = null
  private state: VoicePlayback = IDLE
  private listeners = new Set<() => void>()
  private frame = 0
  /** Bumped by every play / pause / stop, so a play still loading never overrides a later one. */
  private attempt = 0
  private stored: Stored | null = null
  private playedIds: Set<string> | null = null

  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }

  snapshot(key: string): VoicePlayback {
    return key === this.key ? this.state : IDLE
  }

  rate(): number {
    return this.load().rate
  }

  played(id: string): boolean {
    this.load()
    return this.playedIds!.has(id)
  }

  /** Plays the note from `fromMs` (or where it was paused), loading its audio first. */
  async play(key: string, objId: string, fromMs?: number) {
    if (this.key !== key) {
      this.stop()
      this.key = key
    }
    const attempt = ++this.attempt
    this.set({ ...this.state, loading: true, error: false, ...(fromMs !== undefined ? { positionMs: fromMs } : {}) })
    try {
      const access = getObjectAccess()
      if (!access) throw new Error('unavailable')
      const url = await access.contentUrl(objId)
      if (attempt !== this.attempt) return
      const audio = this.element()
      if (audio.dataset.key !== key) {
        audio.dataset.key = key
        audio.src = url
      }
      audio.playbackRate = this.rate()
      const target = (fromMs ?? this.state.positionMs) / 1000
      if (Math.abs(audio.currentTime - target) > 0.05) audio.currentTime = target
      await audio.play()
      if (attempt !== this.attempt) {
        if (!this.state.playing && !this.state.loading) audio.pause()
        return
      }
      this.set({ ...this.state, playing: true, loading: false })
      this.markPlayed(key)
      this.tick()
    } catch {
      if (attempt === this.attempt) this.set({ ...this.state, playing: false, loading: false, error: true })
    }
  }

  pause() {
    if (!this.key) return
    this.attempt++
    this.audio?.pause()
    cancelAnimationFrame(this.frame)
    this.set({ ...this.state, playing: false, loading: false, positionMs: this.audio ? this.audio.currentTime * 1000 : this.state.positionMs })
  }

  toggle(key: string, objId: string) {
    if (this.key === key && (this.state.playing || this.state.loading)) this.pause()
    else void this.play(key, objId)
  }

  /** Moves to `ms`; a note that is not playing starts from there. */
  seek(key: string, objId: string, ms: number) {
    if (this.key === key && this.state.playing && this.audio) {
      this.audio.currentTime = ms / 1000
      this.set({ ...this.state, positionMs: ms })
      return
    }
    void this.play(key, objId, ms)
  }

  cycleRate() {
    const current = this.rate()
    const next = VOICE_RATES[(VOICE_RATES.indexOf(current as typeof VOICE_RATES[number]) + 1) % VOICE_RATES.length]
    this.load().rate = next
    this.save()
    if (this.audio) this.audio.playbackRate = next
    this.set({ ...this.state })
  }

  stop() {
    this.attempt++
    cancelAnimationFrame(this.frame)
    this.audio?.pause()
    this.key = null
    this.set(IDLE)
  }

  private element(): HTMLAudioElement {
    if (this.audio) return this.audio
    const audio = new Audio()
    audio.preload = 'auto'
    audio.addEventListener('ended', () => {
      cancelAnimationFrame(this.frame)
      this.set({ ...this.state, playing: false, positionMs: 0 })
    })
    this.audio = audio
    return audio
  }

  private tick = () => {
    cancelAnimationFrame(this.frame)
    if (!this.state.playing || !this.audio) return
    const positionMs = this.audio.currentTime * 1000
    if (Math.abs(positionMs - this.state.positionMs) >= 30) this.set({ ...this.state, positionMs })
    this.frame = requestAnimationFrame(this.tick)
  }

  private set(next: VoicePlayback) {
    this.state = next
    this.listeners.forEach(listener => listener())
  }

  private load(): Stored {
    if (!this.stored) {
      this.stored = readStored()
      this.playedIds = new Set(this.stored.played)
    }
    return this.stored
  }

  private markPlayed(id: string) {
    const stored = this.load()
    if (this.playedIds!.has(id)) return
    this.playedIds!.add(id)
    stored.played = [...stored.played, id].slice(-PLAYED_LIMIT)
    this.save()
  }

  private save() {
    try {
      window.localStorage.setItem(STORAGE_KEY, JSON.stringify(this.stored))
    } catch {
      // Remembered for this page only.
    }
  }
}

export const voicePlayer = new VoicePlayer()

export function useVoicePlayback(key: string): VoicePlayback {
  return useSyncExternalStore(voicePlayer.subscribe, useCallback(() => voicePlayer.snapshot(key), [key]))
}

export function useVoiceRate(): number {
  return useSyncExternalStore(voicePlayer.subscribe, () => voicePlayer.rate())
}

export function useVoicePlayed(key: string): boolean {
  return useSyncExternalStore(voicePlayer.subscribe, useCallback(() => voicePlayer.played(key), [key]))
}
