import type { DesktopWallpaper } from '../../models/ui'
import { fitBackdropStyle, imageLayerStyle, type WallpaperView } from './style'

/**
 * The picture of a wallpaper, filling its positioned, clipping parent. A preview of `panorama` passes the page
 * count, and shows the first page of the strip the picture spans.
 */
export function WallpaperImageLayers({
  wallpaper,
  view,
  pages = 1,
}: {
  wallpaper: DesktopWallpaper
  view: WallpaperView
  pages?: number
}) {
  const strip = wallpaper.mode === 'panorama' && pages > 1
  return (
    <>
      {wallpaper.mode === 'fit' ? (
        <div className="absolute inset-0" style={fitBackdropStyle(wallpaper, view)} />
      ) : null}
      <div
        key={wallpaper.imageUrl}
        className={strip ? 'desktop-wallpaper-image absolute inset-y-0 left-0' : 'desktop-wallpaper-image absolute inset-0'}
        style={strip ? { ...imageLayerStyle(wallpaper, view), width: `${pages * 100}%` } : imageLayerStyle(wallpaper, view)}
      />
    </>
  )
}
