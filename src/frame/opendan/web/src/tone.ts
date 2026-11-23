export type Tone = 'accent' | 'ok' | 'warn' | 'bad' | 'mute'

const TONES: Record<string, Tone> = {
  running: 'accent',
  ready: 'accent',
  waiting: 'warn',
  pending: 'warn',
  finished: 'mute',
  created: 'mute',
  succeeded: 'ok',
  accepted: 'ok',
  sent: 'ok',
  ok: 'ok',
  failed: 'bad',
  discarded: 'bad',
  stopped: 'bad',
  bad: 'bad',
}

const NAMES = new Set<string>(['accent', 'ok', 'warn', 'bad', 'mute'])

/** A state name (`running`, `failed`, …) or a tone as the dot shows it; nothing for a plain label. */
export const toneOf = (key?: string): Tone | undefined => (key ? TONES[key] ?? (NAMES.has(key) ? (key as Tone) : undefined) : undefined)
