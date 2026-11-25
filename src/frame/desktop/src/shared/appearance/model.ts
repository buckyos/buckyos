/* ── Appearance settings that follow the account ──
 *
 * Saved as the `appearance` desktop state of the account (`controlPanel/desktopState.ts`), one JSON field per setting
 * so devices changing different settings never overwrite each other: write one with
 * `writeDesktopState(channel, APPEARANCE_STATE_KEY, value, field)`. Settings that belong to a screen rather than a
 * person (the status-bar message preview, safe-area insets, the taskbar and icon layout) stay on the device.
 */

import { normalizeAccent, type AccentColor } from './accent.ts'
import { defaultAccountWallpaper, normalizeAccountWallpaper, type AccountWallpaper } from './wallpaper.ts'

export const APPEARANCE_STATE_KEY = 'appearance'

export type IconSize = 'small' | 'medium' | 'large'
/** What the mobile client's feed tab opens on. */
export type MobileFeed = 'homestation' | 'activity'

export const iconSizes: readonly IconSize[] = ['small', 'medium', 'large']
export const mobileFeeds: readonly MobileFeed[] = ['homestation', 'activity']
export const DEFAULT_MOBILE_FEED: MobileFeed = 'homestation'

export interface WindowOpacity {
  titleBarOpacity: number
  backgroundOpacity: number
}

/** `L` is the languages the reader offers. */
export interface AccountAppearance<L extends string = string> {
  theme: 'light' | 'dark'
  locale: L
  iconSize: IconSize
  window: WindowOpacity
  wallpaper: AccountWallpaper
  /** The Zone's colour; null is the default. */
  accent: AccentColor | null
  mobileFeed: MobileFeed
}

export type AppearanceField = keyof AccountAppearance

export const appearanceFields: readonly AppearanceField[] = ['theme', 'locale', 'iconSize', 'window', 'wallpaper', 'accent', 'mobileFeed']

function percent(value: unknown): number | undefined {
  const number = Number(value)
  return Number.isInteger(number) && number >= 0 && number <= 100 ? number : undefined
}

function windowOpacity(value: unknown): WindowOpacity | undefined {
  if (!value || typeof value !== 'object') return undefined
  const { titleBarOpacity, backgroundOpacity } = value as Record<string, unknown>
  const titleBar = percent(titleBarOpacity)
  const background = percent(backgroundOpacity)
  return titleBar === undefined || background === undefined ? undefined : { titleBarOpacity: titleBar, backgroundOpacity: background }
}

/** The fields of a stored appearance that are usable; anything missing or invalid is left out. */
export function readAppearance<L extends string>(raw: unknown, locales: readonly L[]): Partial<AccountAppearance<L>> {
  if (!raw || typeof raw !== 'object') return {}
  const value = raw as Record<string, unknown>
  const result: Partial<AccountAppearance<L>> = {}
  if (value.theme === 'light' || value.theme === 'dark') result.theme = value.theme
  if (locales.includes(value.locale as L)) result.locale = value.locale as L
  if (iconSizes.includes(value.iconSize as IconSize)) result.iconSize = value.iconSize as IconSize
  const opacity = windowOpacity(value.window)
  if (opacity) result.window = opacity
  if (value.wallpaper && typeof value.wallpaper === 'object') result.wallpaper = normalizeAccountWallpaper(value.wallpaper)
  if (value.accent === null) result.accent = null
  else if (normalizeAccent(value.accent)) result.accent = normalizeAccent(value.accent)
  if (mobileFeeds.includes(value.mobileFeed as MobileFeed)) result.mobileFeed = value.mobileFeed as MobileFeed
  return result
}

function stable(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(stable)
  if (value && typeof value === 'object') {
    return Object.fromEntries(
      Object.keys(value as Record<string, unknown>)
        .sort()
        .map((key) => [key, stable((value as Record<string, unknown>)[key])]),
    )
  }
  return value
}

export function sameValue(a: unknown, b: unknown) {
  return JSON.stringify(stable(a)) === JSON.stringify(stable(b))
}

export interface SyncInput<A extends AccountAppearance = AccountAppearance> {
  /** What the account holds (only the usable fields). */
  remote: Partial<A>
  /** What this page shows. */
  local: A
  /** Fields changed here that the account has not taken yet; they win over the account's. */
  dirty: ReadonlySet<AppearanceField>
  /** This browser's wallpaper was another account's; it is not handed to this one. */
  foreignWallpaper: boolean
}

export interface SyncPlan<A extends AccountAppearance = AccountAppearance> {
  /** Account values to show here. */
  apply: Partial<A>
  /** Values to save to the account: changed here, or settings the account has none of yet. */
  push: Partial<A>
  /** What the account holds once `push` is saved. */
  synced: A
}

function setField(target: object, field: AppearanceField, value: unknown) {
  Object.assign(target, { [field]: value })
}

/**
 * Account first: a setting the account has is shown here, unless it was just changed here. A setting the
 * account lacks (an account that never synced, or a setting added later) is seeded from this device, so the
 * choices made before settings followed the account carry over.
 */
export function planSync<A extends AccountAppearance>({ remote, local, dirty, foreignWallpaper }: SyncInput<A>): SyncPlan<A> {
  const apply: Partial<A> = {}
  const push: Partial<A> = {}
  const synced: A = { ...local }
  for (const field of appearanceFields) {
    const remoteValue = remote[field]
    if (dirty.has(field)) {
      setField(push, field, local[field])
    } else if (remoteValue !== undefined) {
      setField(synced, field, remoteValue)
      if (!sameValue(remoteValue, local[field])) setField(apply, field, remoteValue)
    } else if (field === 'wallpaper' && foreignWallpaper) {
      // Another account's picture is not handed on: this account starts from the built-in background.
      setField(synced, field, defaultAccountWallpaper)
      setField(apply, field, defaultAccountWallpaper)
      setField(push, field, defaultAccountWallpaper)
    } else {
      setField(push, field, local[field])
    }
  }
  return { apply, push, synced }
}
