/* How clear the screen's frames are and what they cost (Agent Browser Protocol §3.2 `quality`, §3.5 client advice):
 * four tiers, from the device's own pixel density down to three frames a second. The user picks "auto" (the default),
 * "clear" or "data saver"; in "auto" the tier follows what the connection carries, measured on the frames themselves:
 * how long a JPEG takes to arrive after its `frame_meta`, how long it takes to paint until its `ack`, how busy that
 * keeps the connection. A tier goes down as soon as frames take too long; it goes up only once the next tier has been
 * seen to fit for a while, and later each time the tier fell soon after going up. */

export type QualityChoice = 'auto' | 'clear' | 'saver'
export type TierName = 'high' | 'standard' | 'saver' | 'minimal'
export interface Tier { name: TierName; pixel_ratio: number; jpeg_quality: number; max_fps: number }

export const QUALITY_CHOICES: readonly QualityChoice[] = ['auto', 'clear', 'saver']
const TIER_NAMES: readonly TierName[] = ['high', 'standard', 'saver', 'minimal']
const FIXED: Record<Exclude<QualityChoice, 'auto'>, TierName> = { clear: 'high', saver: 'saver' }
/** The densest frames the service paints. */
const PIXEL_RATIO_MAX = 2
/** Below this the screen's own density is not asked for: a tab beyond 1 is painted at 2 and made smaller, costing the
 * service about three times the CPU for a little sharpness. */
const DENSE_FROM = 1.5

/** Frames of the last few seconds say whether the tier is too much for the connection. */
const DOWN_WINDOW_MS = 3000
/** A tier is too much when a frame takes this long on average to arrive, or one of them takes `STALL_MS`… */
const SLOW_TRANSFER_MS = 250
const STALL_MS = 700
/** …when frames keep the connection busy most of the time, or the device is slow to paint them. */
const BUSY_DOWN = 0.7
const SLOW_PAINT_MS = 120
/** Frames of a longer stretch say whether the next tier up would fit. */
const UP_WINDOW_MS = 10_000
const UP_MIN_FRAMES = 2
const UP_MIN_BYTES = 16_000
const FIT_TRANSFER_MS = 100
const FIT_BUSY = 0.35
const FIT_PAINT_MS = 50
/** How long the next tier must keep fitting. */
const UP_HOLD_MS = 5000
/** No tier up this soon after one down; doubled each time a tier went down within `FLAP_MS` of going up. */
const UP_AFTER_DOWN_MS = 10_000
const UP_AFTER_DOWN_MAX_MS = 120_000
const FLAP_MS = 60_000

export function tierOf(name: TierName, devicePixelRatio: number): Tier {
  switch (name) {
    case 'high': return { name, pixel_ratio: devicePixelRatio >= DENSE_FROM ? Math.round(Math.min(PIXEL_RATIO_MAX, devicePixelRatio) * 100) / 100 : 1, jpeg_quality: 70, max_fps: 15 }
    case 'standard': return { name, pixel_ratio: 1, jpeg_quality: 60, max_fps: 15 }
    case 'saver': return { name, pixel_ratio: 1, jpeg_quality: 40, max_fps: 8 }
    case 'minimal': return { name, pixel_ratio: 1, jpeg_quality: 30, max_fps: 3 }
  }
}

/** The bytes of a frame of `tier` against one of the standard tier (measured on Chromium 159: twice the pixel ratio is
 * 2.7 times the bytes; quality 30 is 0.71 of quality 60, quality 70 is 1.12). */
function cost(tier: Tier): number {
  return tier.pixel_ratio ** 1.43 * (0.47 + 0.0088 * tier.jpeg_quality)
}

interface Sample { at: number; bytes: number; transfer: number; paint: number }

const sum = (samples: Sample[], of: (sample: Sample) => number) => samples.reduce((total, sample) => total + of(sample), 0)

export class QualityGovernor {
  private choice: QualityChoice
  private index = 0
  private samples: Sample[] = []
  private fitSince: number | null = null
  private lastDown = -Infinity
  private lastUp = -Infinity
  private upAfterDown = UP_AFTER_DOWN_MS
  private readonly devicePixelRatio: () => number

  constructor(choice: QualityChoice, devicePixelRatio: () => number) {
    this.choice = choice
    this.devicePixelRatio = devicePixelRatio
    this.index = TIER_NAMES.indexOf(choice === 'auto' ? 'high' : FIXED[choice])
  }

  tier(): Tier {
    return tierOf(TIER_NAMES[this.index], this.devicePixelRatio())
  }

  /** The user's choice: a fixed tier, or "auto" from the clearest one down. */
  choose(choice: QualityChoice) {
    this.choice = choice
    this.index = TIER_NAMES.indexOf(choice === 'auto' ? 'high' : FIXED[choice])
    this.forget()
  }

  /** A frame of the current tier: its bytes, how long it took to arrive after its description, and to paint. */
  sample(bytes: number, transfer: number, paint: number, at: number) {
    this.samples.push({ at, bytes, transfer: Math.max(0, transfer), paint: Math.max(0, paint) })
  }

  /** The tier from now on when it changes ("auto" only); called once a second and after every frame. */
  adjust(now: number): Tier | null {
    if (this.choice !== 'auto') return null
    this.samples = this.samples.filter((sample) => now - sample.at <= UP_WINDOW_MS)
    if (now - this.lastDown > UP_AFTER_DOWN_MAX_MS) this.upAfterDown = UP_AFTER_DOWN_MS
    const recent = this.samples.filter((sample) => now - sample.at <= DOWN_WINDOW_MS)
    if (recent.length > 0 && this.index < TIER_NAMES.length - 1) {
      const transfer = sum(recent, (sample) => sample.transfer) / recent.length
      const worst = Math.max(...recent.map((sample) => sample.transfer))
      const busy = sum(recent, (sample) => sample.transfer + sample.paint) / DOWN_WINDOW_MS
      const paint = sum(recent, (sample) => sample.paint) / recent.length
      if (transfer > SLOW_TRANSFER_MS || worst > STALL_MS || busy > BUSY_DOWN || paint > SLOW_PAINT_MS) {
        if (now - this.lastUp < FLAP_MS) this.upAfterDown = Math.min(this.upAfterDown * 2, UP_AFTER_DOWN_MAX_MS)
        this.lastDown = now
        return this.move(this.index + 1)
      }
    }
    if (this.index === 0) return null
    const fits = this.nextFits()
    if (fits === false) this.fitSince = null
    if (!fits) return null
    this.fitSince ??= now
    if (now - this.fitSince < UP_HOLD_MS || now - this.lastDown < this.upAfterDown) return null
    this.lastUp = now
    return this.move(this.index - 1)
  }

  /** Whether the next tier up would fit what the last frames took; null when they are too few to tell. */
  private nextFits(): boolean | null {
    const samples = this.samples
    if (samples.length < UP_MIN_FRAMES || sum(samples, (sample) => sample.bytes) < UP_MIN_BYTES) return null
    const current = this.tier()
    const next = tierOf(TIER_NAMES[this.index - 1], this.devicePixelRatio())
    const bytes = cost(next) / cost(current)
    const pixels = (next.pixel_ratio / current.pixel_ratio) ** 2
    const frames = Math.max(1, next.max_fps / current.max_fps)
    const transfer = (sum(samples, (sample) => sample.transfer) / samples.length) * bytes
    const paint = (sum(samples, (sample) => sample.paint) / samples.length) * pixels
    const busy = ((sum(samples, (sample) => sample.transfer) * bytes + sum(samples, (sample) => sample.paint) * pixels) * frames) / UP_WINDOW_MS
    return transfer < FIT_TRANSFER_MS && paint < FIT_PAINT_MS && busy < FIT_BUSY
  }

  private move(index: number): Tier {
    this.index = index
    this.forget()
    return this.tier()
  }

  /** Frames of another tier say nothing of this one. */
  private forget() {
    this.samples = []
    this.fitSince = null
  }
}

const CHOICE_KEY = 'buckyos.agent-browser.screen-quality'

export function rememberedQuality(): QualityChoice {
  try {
    const value = window.localStorage.getItem(CHOICE_KEY)
    return QUALITY_CHOICES.includes(value as QualityChoice) ? (value as QualityChoice) : 'auto'
  } catch {
    return 'auto'
  }
}

export function rememberQuality(choice: QualityChoice) {
  try {
    if (choice === 'auto') window.localStorage.removeItem(CHOICE_KEY)
    else window.localStorage.setItem(CHOICE_KEY, choice)
  } catch { /* no storage */ }
}
