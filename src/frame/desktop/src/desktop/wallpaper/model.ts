/* ── The user's desktop picture: preferences and what the desktop draws from them ──
 *
 * Pure helpers shared by the store, the desktop background and the Settings preview. What the account holds is
 * `shared/appearance/wallpaper.ts`.
 */

import type { DesktopWallpaper } from '../../models/ui'
import type { WallpaperFit, WallpaperPreferences } from '../../shared/appearance/wallpaper.ts'

export {
  defaultAccountWallpaper,
  defaultWallpaperPreferences,
  normalizeAccountWallpaper,
  normalizePreferences,
  TILE_SCALE_MAX,
  TILE_SCALE_MIN,
  wallpaperFits,
  type AccountWallpaper,
  type AccountWallpaperImage,
  type WallpaperFit,
  type WallpaperPreferences,
} from '../../shared/appearance/wallpaper.ts'

/** The picture as the page draws it. */
export interface WallpaperImageRef {
  url: string
  width: number
  height: number
  name: string
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

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value))
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
