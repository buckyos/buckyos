import { memo, type CSSProperties } from 'react'
import type { DesktopWallpaper } from '../models/ui'
import { DESKTOP_VIEWPORT_PROGRESS_VAR } from './viewportProgress'
import { focusPosition } from './wallpaper/style'
import { WallpaperImageLayers } from './wallpaper/WallpaperImageLayers'

interface DesktopBackgroundProps {
  wallpaper: DesktopWallpaper
  pageCount: number
}

const defaultWallpaper: DesktopWallpaper = {
  mode: 'infinite',
}

function resolveBackgroundImage(imageUrl: string | undefined, fallback: string) {
  if (!imageUrl) {
    return fallback
  }

  return /^(url|linear-gradient|radial-gradient|conic-gradient|image-set)\(/.test(imageUrl)
    ? imageUrl
    : `url("${imageUrl}")`
}

function buildPanoramaStyle(
  wallpaper: DesktopWallpaper,
  pageCount: number,
): CSSProperties {
  const resolvedPageCount = Math.max(pageCount, 1)
  // Full travel (progress = 1) shifts the panorama by all but one page.
  const travelPercent =
    resolvedPageCount > 1 ? -((resolvedPageCount - 1) * 100) / resolvedPageCount : 0

  return {
    width: `${resolvedPageCount * 100}%`,
    // The paging progress lives in a CSS variable (see viewportProgress.ts),
    // so a swipe never re-renders this component: the compositor moves the
    // layer as the variable changes.
    transform: `translate3d(calc(var(${DESKTOP_VIEWPORT_PROGRESS_VAR}, 0) * ${travelPercent}%), 0, 0)`,
    backgroundImage: resolveBackgroundImage(
      wallpaper.imageUrl,
      'linear-gradient(90deg, var(--cp-wallpaper-a), var(--cp-wallpaper-c) 50%, var(--cp-wallpaper-b))',
    ),
    backgroundPosition: wallpaper.imageUrl ? focusPosition(wallpaper) : 'center',
    backgroundRepeat: 'no-repeat',
    backgroundSize: wallpaper.imageUrl ? 'cover' : '100% 100%',
  }
}

function buildTileStyle(wallpaper: DesktopWallpaper): CSSProperties {
  const tileSize = wallpaper.tileSize ?? 160

  return {
    backgroundImage: [
      'linear-gradient(135deg, color-mix(in srgb, var(--cp-wallpaper-b) 18%, transparent) 25%, transparent 25%)',
      'linear-gradient(225deg, color-mix(in srgb, var(--cp-wallpaper-c) 16%, transparent) 25%, transparent 25%)',
      'linear-gradient(45deg, color-mix(in srgb, var(--cp-wallpaper-b) 12%, transparent) 25%, transparent 25%)',
      'linear-gradient(315deg, var(--cp-wallpaper-a), var(--cp-bg))',
    ].join(','),
    backgroundPosition: '0 0, 0 0, 50% 50%, 0 0',
    backgroundRepeat: 'repeat',
    backgroundSize: `${tileSize}px ${tileSize}px, ${tileSize}px ${tileSize}px, ${tileSize}px ${tileSize}px, auto`,
  }
}

function buildInfiniteStyle(): CSSProperties {
  return {
    backgroundImage: 'linear-gradient(180deg, var(--cp-wallpaper-a), var(--cp-wallpaper-b))',
  }
}

export const DesktopBackground = memo(function DesktopBackground({
  wallpaper = defaultWallpaper,
  pageCount,
}: DesktopBackgroundProps) {
  const resolvedWallpaper = wallpaper ?? defaultWallpaper
  const hasImage = Boolean(resolvedWallpaper.imageUrl)

  return (
    <div
      aria-hidden="true"
      data-testid="desktop-background"
      data-wallpaper-mode={hasImage ? resolvedWallpaper.mode : undefined}
      className="pointer-events-none fixed inset-0 z-0 select-none overflow-hidden"
    >
      <div className="absolute inset-0 bg-[color:var(--cp-bg)]" />

      {resolvedWallpaper.mode === 'panorama' ? (
        <div
          className={hasImage ? 'desktop-wallpaper-image absolute inset-y-0 left-0' : 'absolute inset-y-0 left-0'}
          style={buildPanoramaStyle(resolvedWallpaper, pageCount)}
        />
      ) : hasImage ? (
        <WallpaperImageLayers
          wallpaper={resolvedWallpaper}
          view={{ scale: 1, pixelRatio: window.devicePixelRatio || 1 }}
        />
      ) : (
        <div
          className="absolute inset-0"
          style={
            resolvedWallpaper.mode === 'tile'
              ? buildTileStyle(resolvedWallpaper)
              : buildInfiniteStyle()
          }
        />
      )}
    </div>
  )
})
