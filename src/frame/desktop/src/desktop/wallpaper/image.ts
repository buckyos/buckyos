/* ── Turning a picked file into the stored wallpaper image ──
 *
 * The picture stays in this browser (IndexedDB), so unlike avatars it is kept large. A file is stored as it is
 * unless it is too big, which keeps GIF animation, transparency and the original quality; a bigger one is
 * scaled down to MAX_WALLPAPER_EDGE on its longest side.
 */

export const MAX_WALLPAPER_INPUT_BYTES = 40 * 1024 * 1024
/** Longest side kept; 4096 × 4096 is also the largest canvas iOS Safari draws. */
export const MAX_WALLPAPER_EDGE = 4096
export const MAX_KEPT_BYTES = 8 * 1024 * 1024

const KEPT_TYPES = ['image/jpeg', 'image/png', 'image/webp', 'image/gif', 'image/avif']
const ALPHA_TYPES = ['image/png', 'image/webp', 'image/gif', 'image/avif']

export type WallpaperImageErrorReason = 'type' | 'too_large' | 'unreadable'

export class WallpaperImageError extends Error {
  readonly reason: WallpaperImageErrorReason

  constructor(reason: WallpaperImageErrorReason) {
    super(`wallpaper image rejected: ${reason}`)
    this.reason = reason
  }
}

export interface PreparedWallpaperImage {
  blob: Blob
  width: number
  height: number
  name: string
}

/** Rejects what is plainly not a picture; an empty type (some mobile pickers) is left to the decoder. */
export function checkWallpaperFile(file: Pick<File, 'type' | 'size'>): void {
  if (file.type && (!file.type.startsWith('image/') || file.type === 'image/svg+xml')) {
    throw new WallpaperImageError('type')
  }
  if (file.size > MAX_WALLPAPER_INPUT_BYTES) throw new WallpaperImageError('too_large')
}

export function keepsOriginal(file: Pick<File, 'type' | 'size'>, width: number, height: number): boolean {
  return KEPT_TYPES.includes(file.type) && file.size <= MAX_KEPT_BYTES && Math.max(width, height) <= MAX_WALLPAPER_EDGE
}

export function scaledSize(width: number, height: number, maxEdge = MAX_WALLPAPER_EDGE) {
  const scale = Math.min(1, maxEdge / Math.max(width, height))
  return { width: Math.max(1, Math.round(width * scale)), height: Math.max(1, Math.round(height * scale)) }
}

function canvasBlob(canvas: HTMLCanvasElement, type: string, quality: number) {
  return new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, type, quality))
}

export async function prepareWallpaperImage(file: File): Promise<PreparedWallpaperImage> {
  checkWallpaperFile(file)
  let bitmap: ImageBitmap
  try {
    bitmap = await createImageBitmap(file)
  } catch {
    throw new WallpaperImageError('unreadable')
  }
  try {
    const { width, height } = bitmap
    if (keepsOriginal(file, width, height)) return { blob: file, width, height, name: file.name }

    const target = scaledSize(width, height)
    const canvas = document.createElement('canvas')
    canvas.width = target.width
    canvas.height = target.height
    const context = canvas.getContext('2d')
    if (!context) throw new WallpaperImageError('unreadable')
    context.imageSmoothingQuality = 'high'
    context.drawImage(bitmap, 0, 0, target.width, target.height)
    // WebP keeps transparency; a browser that cannot encode it hands back PNG.
    const blob = ALPHA_TYPES.includes(file.type)
      ? await canvasBlob(canvas, 'image/webp', 0.92)
      : await canvasBlob(canvas, 'image/jpeg', 0.9)
    if (!blob) throw new WallpaperImageError('unreadable')
    return { blob, width: target.width, height: target.height, name: file.name }
  } finally {
    bitmap.close()
  }
}
