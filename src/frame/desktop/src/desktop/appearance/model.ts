/* ── Appearance settings that follow the account ──
 *
 * Saved as the `appearance` desktop state of the account (control-panel `/api/desktop`, session `default`),
 * one JSON field per setting so devices changing different settings never overwrite each other. Settings that
 * belong to a screen rather than a person (the status-bar message preview, safe-area insets, the taskbar and
 * icon layout) stay on the device.
 */

import { densityRowHeight, type GridDensity } from '../../models/layout'
import {
  supportedLocales,
  windowAppearancePreferencesSchema,
  type SupportedLocale,
  type ThemeMode,
  type WindowAppearancePreferences,
} from '../../models/ui'
import { defaultAccountWallpaper, normalizeAccountWallpaper, type AccountWallpaper } from '../wallpaper/model'

export const APPEARANCE_STATE_KEY = 'appearance'

export interface AccountAppearance {
  theme: ThemeMode
  locale: SupportedLocale
  iconSize: GridDensity
  window: WindowAppearancePreferences
  wallpaper: AccountWallpaper
}

export type AppearanceField = keyof AccountAppearance

export const appearanceFields: readonly AppearanceField[] = ['theme', 'locale', 'iconSize', 'window', 'wallpaper']

/** The fields of a stored appearance that are usable; anything missing or invalid is left out. */
export function readAppearance(raw: unknown): Partial<AccountAppearance> {
  if (!raw || typeof raw !== 'object') return {}
  const value = raw as Record<string, unknown>
  const result: Partial<AccountAppearance> = {}
  if (value.theme === 'light' || value.theme === 'dark') result.theme = value.theme
  if (supportedLocales.includes(value.locale as SupportedLocale)) result.locale = value.locale as SupportedLocale
  if (typeof value.iconSize === 'string' && value.iconSize in densityRowHeight) result.iconSize = value.iconSize as GridDensity
  const windowPrefs = windowAppearancePreferencesSchema.safeParse(value.window)
  if (windowPrefs.success) result.window = windowPrefs.data
  if (value.wallpaper && typeof value.wallpaper === 'object') result.wallpaper = normalizeAccountWallpaper(value.wallpaper)
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

export interface SyncInput {
  /** What the account holds (only the usable fields). */
  remote: Partial<AccountAppearance>
  /** What this page shows. */
  local: AccountAppearance
  /** Fields changed here that the account has not taken yet; they win over the account's. */
  dirty: ReadonlySet<AppearanceField>
  /** This browser's wallpaper was another account's; it is not handed to this one. */
  foreignWallpaper: boolean
}

export interface SyncPlan {
  /** Account values to show here. */
  apply: Partial<AccountAppearance>
  /** Values to save to the account: changed here, or settings the account has none of yet. */
  push: Partial<AccountAppearance>
  /** What the account holds once `push` is saved. */
  synced: AccountAppearance
}

function setField<K extends AppearanceField>(target: Partial<AccountAppearance>, field: K, value: AccountAppearance[K]) {
  target[field] = value
}

/**
 * Account first: a setting the account has is shown here, unless it was just changed here. A setting the
 * account lacks (an account that never synced, or a setting added later) is seeded from this device, so the
 * choices made before settings followed the account carry over.
 */
export function planSync({ remote, local, dirty, foreignWallpaper }: SyncInput): SyncPlan {
  const apply: Partial<AccountAppearance> = {}
  const push: Partial<AccountAppearance> = {}
  const synced: AccountAppearance = { ...local }
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
