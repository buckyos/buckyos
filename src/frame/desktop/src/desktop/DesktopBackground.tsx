import { memo, type CSSProperties } from 'react'
import type { DesktopWallpaper } from '../models/ui'
import { DESKTOP_VIEWPORT_PROGRESS_VAR } from './viewportProgress'

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
      [
        'radial-gradient(circle at 12% 24%, color-mix(in srgb, var(--cp-accent-soft) 36%, transparent) 0, transparent 18%)',
        'radial-gradient(circle at 34% 68%, color-mix(in srgb, var(--cp-wallpaper-c) 38%, transparent) 0, transparent 22%)',
        'radial-gradient(circle at 62% 22%, color-mix(in srgb, var(--cp-wallpaper-b) 34%, transparent) 0, transparent 20%)',
        'radial-gradient(circle at 82% 62%, color-mix(in srgb, var(--cp-accent) 18%, transparent) 0, transparent 24%)',
        'linear-gradient(122deg, var(--cp-wallpaper-a), color-mix(in srgb, var(--cp-bg) 80%, white) 52%, color-mix(in srgb, var(--cp-wallpaper-b) 72%, white))',
      ].join(','),
    ),
    backgroundPosition: 'center',
    backgroundRepeat: 'no-repeat',
    backgroundSize: wallpaper.imageUrl ? 'cover' : '100% 100%',
  }
}

function buildTileStyle(wallpaper: DesktopWallpaper): CSSProperties {
  const tileSize = wallpaper.tileSize ?? 160

  return {
    backgroundImage: resolveBackgroundImage(
      wallpaper.imageUrl,
      [
        'linear-gradient(135deg, color-mix(in srgb, var(--cp-wallpaper-b) 18%, transparent) 25%, transparent 25%)',
        'linear-gradient(225deg, color-mix(in srgb, var(--cp-wallpaper-c) 16%, transparent) 25%, transparent 25%)',
        'linear-gradient(45deg, color-mix(in srgb, var(--cp-accent-soft) 12%, transparent) 25%, transparent 25%)',
        'linear-gradient(315deg, color-mix(in srgb, var(--cp-wallpaper-a) 74%, white), color-mix(in srgb, var(--cp-bg) 90%, white))',
      ].join(','),
    ),
    backgroundPosition: wallpaper.imageUrl ? 'top left' : '0 0, 0 0, 50% 50%, 0 0',
    backgroundRepeat: 'repeat',
    backgroundSize: wallpaper.imageUrl
      ? `${tileSize}px ${tileSize}px`
      : `${tileSize}px ${tileSize}px, ${tileSize}px ${tileSize}px, ${tileSize}px ${tileSize}px, auto`,
  }
}

function buildInfiniteStyle(wallpaper: DesktopWallpaper): CSSProperties {
  if (wallpaper.imageUrl) {
    return {
      backgroundImage: resolveBackgroundImage(wallpaper.imageUrl, ''),
      backgroundPosition: 'center',
      backgroundRepeat: 'no-repeat',
      backgroundSize: 'cover',
    }
  }

  return {
    backgroundImage: [
      'radial-gradient(circle at 14% 20%, color-mix(in srgb, var(--cp-wallpaper-c) 36%, transparent) 0, transparent 28%)',
      'radial-gradient(circle at 82% 18%, color-mix(in srgb, var(--cp-wallpaper-b) 34%, transparent) 0, transparent 24%)',
      'radial-gradient(circle at 50% 100%, color-mix(in srgb, var(--cp-accent-soft) 12%, transparent) 0, transparent 30%)',
      'linear-gradient(145deg, var(--cp-wallpaper-a), color-mix(in srgb, var(--cp-bg) 84%, white))',
    ].join(','),
    backgroundPosition: 'center',
    backgroundRepeat: 'repeat',
    backgroundSize: 'auto',
  }
}

// The two former overlay layers (corner glows + vertical sheen) merged into
// one element: background layers paint top-to-bottom in list order, so this
// is pixel-identical while costing one full-screen layer instead of two.
const overlayStyle: CSSProperties = {
  backgroundImage: [
    'linear-gradient(180deg, color-mix(in srgb, white 9%, transparent), transparent 22%, transparent 78%, color-mix(in srgb, var(--cp-surface) 12%, transparent))',
    'radial-gradient(circle at top left, color-mix(in srgb, white 28%, transparent), transparent 28%)',
    'radial-gradient(circle at bottom right, color-mix(in srgb, var(--cp-surface) 22%, transparent), transparent 28%)',
  ].join(','),
}

export const DesktopBackground = memo(function DesktopBackground({
  wallpaper = defaultWallpaper,
  pageCount,
}: DesktopBackgroundProps) {
  const resolvedWallpaper = wallpaper ?? defaultWallpaper

  return (
    <div
      aria-hidden="true"
      className="pointer-events-none fixed inset-0 z-0 select-none overflow-hidden"
    >
      <div className="absolute inset-0 bg-[color:var(--cp-bg)]" />

      {resolvedWallpaper.mode === 'panorama' ? (
        <div
          className="absolute inset-y-0 left-0"
          style={buildPanoramaStyle(resolvedWallpaper, pageCount)}
        />
      ) : (
        <div
          className="absolute inset-0"
          style={
            resolvedWallpaper.mode === 'tile'
              ? buildTileStyle(resolvedWallpaper)
              : buildInfiniteStyle(resolvedWallpaper)
          }
        />
      )}

      <div className="absolute inset-0" style={overlayStyle} />
    </div>
  )
})
