/* ── The user's desktop picture: preferences and what the desktop draws from them ──
 *
 * Pure helpers shared by the store, the desktop background and the Settings preview.
 */

import type { DesktopWallpaper } from '../../models/ui'

/** How the picture covers the screen; each is a `DesktopWallpaperMode`. */
export type WallpaperFit = 'fill' | 'fit' | 'stretch' | 'tile' | 'center' | 'panorama'

export const wallpaperFits: readonly WallpaperFit[] = ['fill', 'fit', 'stretch', 'tile', 'center', 'panorama']

export interface WallpaperPreferences {
  fit: WallpaperFit
  /** `tile`: percent of the image's own size. */
  tileScale: number
  /** `fill` / `panorama`: the point kept in view when the image is cropped, 0–100. */
  focusX: number
  focusY: number
}

export const TILE_SCALE_MIN = 25
export const TILE_SCALE_MAX = 400

export const defaultWallpaperPreferences: WallpaperPreferences = {
  fit: 'fill',
  tileScale: 100,
  focusX: 50,
  focusY: 50,
}

/** The picture as the account names it: `obj://bucket/user/control-panel/{user}/wallpapers/{sha256[..32]}.{ext}`. */
export interface AccountWallpaperImage {
  ref: string
  width: number
  height: number
  name: string
}

/** The account's wallpaper: its picture (none = the built-in background) and how it is shown. */
export interface AccountWallpaper extends WallpaperPreferences {
  image: AccountWallpaperImage | null
}

export const defaultAccountWallpaper: AccountWallpaper = { ...defaultWallpaperPreferences, image: null }

/** The picture as the page draws it. */
export interface WallpaperImageRef {
  url: string
  width: number
  height: number
  name: string
}

const PICTURE_REF = /^obj:\/\/bucket\/user\/control-panel\/[A-Za-z0-9._-]{1,128}\/wallpapers\/[0-9a-f]{32}\.(jpg|png|gif|webp|avif)$/

function normalizeImage(raw: unknown): AccountWallpaperImage | null {
  if (!raw || typeof raw !== 'object') return null
  const value = raw as Record<string, unknown>
  const { ref, width, height, name } = value
  if (typeof ref !== 'string' || !PICTURE_REF.test(ref)) return null
  if (typeof width !== 'number' || typeof height !== 'number' || !(width > 0) || !(height > 0)) return null
  return { ref, width: Math.round(width), height: Math.round(height), name: typeof name === 'string' ? name.slice(0, 255) : '' }
}

/** A stored or remote wallpaper from any JSON; an unusable picture reference becomes none. */
export function normalizeAccountWallpaper(raw: unknown): AccountWallpaper {
  const value = raw && typeof raw === 'object' ? (raw as Record<string, unknown>) : {}
  return { ...normalizePreferences(value), image: normalizeImage(value.image) }
}

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value))
}

function numberOr(value: unknown, fallback: number, min: number, max: number) {
  return typeof value === 'number' && Number.isFinite(value) ? clamp(value, min, max) : fallback
}

/** Stored preferences from any JSON, with defaults for what is missing or invalid. */
export function normalizePreferences(raw: unknown): WallpaperPreferences {
  const value = raw && typeof raw === 'object' ? (raw as Record<string, unknown>) : {}
  return {
    fit: wallpaperFits.includes(value.fit as WallpaperFit) ? (value.fit as WallpaperFit) : defaultWallpaperPreferences.fit,
    tileScale: Math.round(numberOr(value.tileScale, defaultWallpaperPreferences.tileScale, TILE_SCALE_MIN, TILE_SCALE_MAX)),
    focusX: numberOr(value.focusX, defaultWallpaperPreferences.focusX, 0, 100),
    focusY: numberOr(value.focusY, defaultWallpaperPreferences.focusY, 0, 100),
  }
}

/** What the desktop draws; null keeps the built-in background. */
export function toDesktopWallpaper(image: WallpaperImageRef | null, preferences: WallpaperPreferences): DesktopWallpaper | null {
  if (!image) return null
  return {
    mode: preferences.fit,
    imageUrl: image.url,
    imageWidth: image.width,
    imageHeight: image.height,
    tileScale: preferences.tileScale,
    focusX: preferences.focusX,
    focusY: preferences.focusY,
  }
}

/** Fits that crop the image, so the user can choose which part stays in view. */
export function fitCrops(fit: WallpaperFit) {
  return fit === 'fill' || fit === 'panorama'
}

/** How far a covering (`background-size: cover`) image overhangs a frame, in frame px on each axis. */
export function coverOverhang(frameWidth: number, frameHeight: number, imageWidth: number, imageHeight: number) {
  if (frameWidth <= 0 || frameHeight <= 0 || imageWidth <= 0 || imageHeight <= 0) return { x: 0, y: 0 }
  const scale = Math.max(frameWidth / imageWidth, frameHeight / imageHeight)
  return { x: imageWidth * scale - frameWidth, y: imageHeight * scale - frameHeight }
}

/**
 * The focus after dragging a covering image by (dx, dy) frame px: the image follows the pointer, so
 * dragging it right brings its left part into view. An axis without overhang does not move.
 */
export function panFocus(
  start: { x: number; y: number },
  delta: { dx: number; dy: number },
  overhang: { x: number; y: number },
) {
  return {
    x: overhang.x > 0.5 ? clamp(start.x - (delta.dx / overhang.x) * 100, 0, 100) : start.x,
    y: overhang.y > 0.5 ? clamp(start.y - (delta.dy / overhang.y) * 100, 0, 100) : start.y,
  }
}
