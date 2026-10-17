/* ── CSS for a picture wallpaper, on the desktop itself or scaled down in the Settings preview ── */

import type { CSSProperties } from 'react'
import type { DesktopWallpaper } from '../../models/ui'

export interface WallpaperView {
  /** Drawn px per CSS px of the screen shown: 1 on the desktop, smaller in a preview. */
  scale: number
  /** Device pixels per CSS px of that screen; `center` and `tile` draw the image pixel for pixel. */
  pixelRatio: number
}

export function cssImageUrl(url: string) {
  return `url("${url.replace(/["\\\n]/g, (char) => encodeURIComponent(char))}")`
}

export function focusPosition(wallpaper: DesktopWallpaper) {
  const x = Math.min(100, Math.max(0, wallpaper.focusX ?? 50))
  const y = Math.min(100, Math.max(0, wallpaper.focusY ?? 50))
  return `${x}% ${y}%`
}

/** `center` and `tile`: the image at its own pixel size (times the tile scale), in px of the view. */
function pixelSize(wallpaper: DesktopWallpaper, view: WallpaperView): string | null {
  const { imageWidth, imageHeight } = wallpaper
  if (!imageWidth || !imageHeight) return null
  const factor =
    (view.scale / Math.max(view.pixelRatio, 0.25)) *
    (wallpaper.mode === 'tile' ? (wallpaper.tileScale ?? 100) / 100 : 1)
  return `${Math.max(1, imageWidth * factor)}px ${Math.max(1, imageHeight * factor)}px`
}

/**
 * The picture layer for every mode but the desktop's `panorama`, which pans across pages and is drawn by
 * DesktopBackground; a preview shows `panorama` as one page of it, like `fill`.
 */
export function imageLayerStyle(wallpaper: DesktopWallpaper, view: WallpaperView): CSSProperties {
  const backgroundImage = wallpaper.imageUrl ? cssImageUrl(wallpaper.imageUrl) : undefined
  switch (wallpaper.mode) {
    case 'fit':
      return { backgroundImage, backgroundSize: 'contain', backgroundPosition: 'center', backgroundRepeat: 'no-repeat' }
    case 'stretch':
      return { backgroundImage, backgroundSize: '100% 100%', backgroundRepeat: 'no-repeat' }
    case 'center':
      return {
        backgroundImage,
        backgroundSize: pixelSize(wallpaper, view) ?? 'auto',
        backgroundPosition: 'center',
        backgroundRepeat: 'no-repeat',
      }
    case 'tile': {
      const fallback = wallpaper.tileSize ? `${wallpaper.tileSize * view.scale}px auto` : 'auto'
      return {
        backgroundImage,
        backgroundSize: pixelSize(wallpaper, view) ?? fallback,
        // Centred, so a phone and a wide screen both show the pattern evenly.
        backgroundPosition: 'center',
        backgroundRepeat: 'repeat',
      }
    }
    default:
      return { backgroundImage, backgroundSize: 'cover', backgroundPosition: focusPosition(wallpaper), backgroundRepeat: 'no-repeat' }
  }
}

/** `fit` leaves bars on a screen of another shape; a blurred copy of the picture fills them. */
export function fitBackdropStyle(wallpaper: DesktopWallpaper, view: WallpaperView): CSSProperties {
  return {
    backgroundImage: wallpaper.imageUrl ? cssImageUrl(wallpaper.imageUrl) : undefined,
    backgroundSize: 'cover',
    backgroundPosition: 'center',
    backgroundRepeat: 'no-repeat',
    filter: `blur(${Math.max(3, Math.round(40 * view.scale))}px) brightness(0.9)`,
    // Blur fades the edges toward transparent; overscan hides that.
    transform: 'scale(1.15)',
  }
}
