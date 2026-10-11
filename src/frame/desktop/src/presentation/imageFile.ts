/* ── Turning a picked image file into the stored data URL ──
 *
 * Avatars and icons are saved inline as small base64 data URLs (control_panel validates the
 * same limits), so every reader of the record can load them; a `blob:` URL is preview-only.
 */

export interface ImageProcessOptions {
  /** Longest side of the stored image. */
  maxPx: number
  /** JPEG for photos; PNG keeps transparency (Zone and app icons). */
  format: 'image/jpeg' | 'image/png'
  /** `square` centre-crops (avatars); `contain` keeps the whole image and its aspect ratio. */
  fit: 'square' | 'contain'
  quality?: number
}

export const AVATAR_IMAGE: ImageProcessOptions = { maxPx: 192, format: 'image/jpeg', fit: 'square', quality: 0.85 }
export const ZONE_ICON_IMAGE: ImageProcessOptions = { maxPx: 256, format: 'image/png', fit: 'contain' }

export const MAX_IMAGE_INPUT_BYTES = 20 * 1024 * 1024
export const MAX_IMAGE_DATA_URL_BYTES = 512 * 1024
export const ACCEPTED_IMAGE_TYPES = ['image/png', 'image/jpeg', 'image/webp', 'image/gif', 'image/bmp'] as const

export type ImageFileErrorReason = 'type' | 'too_large' | 'unreadable'

export class ImageFileError extends Error {
  readonly reason: ImageFileErrorReason

  constructor(reason: ImageFileErrorReason) {
    super(`image file rejected: ${reason}`)
    this.reason = reason
  }
}

export function checkImageFile(file: Pick<File, 'type' | 'size'>): void {
  if (!(ACCEPTED_IMAGE_TYPES as readonly string[]).includes(file.type)) throw new ImageFileError('type')
  if (file.size > MAX_IMAGE_INPUT_BYTES) throw new ImageFileError('too_large')
}

/** Target canvas size and the source rectangle drawn into it. */
export function imageDrawPlan(width: number, height: number, options: Pick<ImageProcessOptions, 'maxPx' | 'fit'>) {
  if (options.fit === 'square') {
    const side = Math.min(width, height)
    const target = Math.min(options.maxPx, side)
    return { width: target, height: target, sx: (width - side) / 2, sy: (height - side) / 2, sw: side, sh: side }
  }
  const scale = Math.min(1, options.maxPx / Math.max(width, height))
  return {
    width: Math.max(1, Math.round(width * scale)),
    height: Math.max(1, Math.round(height * scale)),
    sx: 0,
    sy: 0,
    sw: width,
    sh: height,
  }
}

export async function imageFileToDataUrl(file: File, options: ImageProcessOptions): Promise<string> {
  checkImageFile(file)
  let bitmap: ImageBitmap
  try {
    bitmap = await createImageBitmap(file)
  } catch {
    throw new ImageFileError('unreadable')
  }
  try {
    const plan = imageDrawPlan(bitmap.width, bitmap.height, options)
    const canvas = document.createElement('canvas')
    canvas.width = plan.width
    canvas.height = plan.height
    const context = canvas.getContext('2d')
    if (!context) throw new ImageFileError('unreadable')
    context.drawImage(bitmap, plan.sx, plan.sy, plan.sw, plan.sh, 0, 0, plan.width, plan.height)
    const url = canvas.toDataURL(options.format, options.quality)
    if (url.length > MAX_IMAGE_DATA_URL_BYTES * 4 / 3) throw new ImageFileError('too_large')
    return url
  } finally {
    bitmap.close()
  }
}
