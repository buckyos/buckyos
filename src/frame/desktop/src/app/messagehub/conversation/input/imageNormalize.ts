const HEIF_TYPES = new Set(['image/heic', 'image/heif', 'image/heic-sequence', 'image/heif-sequence'])
const HEIF_BRANDS = new Set(['heic', 'heix', 'hevc', 'hevx', 'heim', 'heis', 'hevm', 'hevs', 'mif1', 'msf1'])
const MAX_CANVAS_PIXELS = 16_000_000
const JPEG_QUALITY = 0.9

async function isHeif(file: Blob & { name?: string }): Promise<boolean> {
  const type = file.type.toLowerCase()
  if (HEIF_TYPES.has(type) || /\.(heic|heif)$/i.test(file.name ?? '')) return true
  if (type && type !== 'application/octet-stream') return false
  const head = new Uint8Array(await file.slice(0, 12).arrayBuffer())
  if (head.length < 12) return false
  const text = (from: number) => String.fromCharCode(...head.slice(from, from + 4))
  return text(4) === 'ftyp' && HEIF_BRANDS.has(text(8))
}

export function jpegName(name: string): string {
  const slash = name.lastIndexOf('/')
  const dot = name.lastIndexOf('.')
  return `${dot > slash + 1 ? name.slice(0, dot) : name}.jpg`
}

export async function normalizeImageFile(file: File): Promise<File> {
  if (!(await isHeif(file).catch(() => false))) return file
  const url = URL.createObjectURL(file)
  try {
    const image = new Image()
    await new Promise((resolve, reject) => {
      image.onload = resolve
      image.onerror = reject
      image.src = url
    })
    const pixels = image.naturalWidth * image.naturalHeight
    if (!pixels) return file
    const scale = Math.min(1, Math.sqrt(MAX_CANVAS_PIXELS / pixels))
    const canvas = document.createElement('canvas')
    canvas.width = Math.max(1, Math.round(image.naturalWidth * scale))
    canvas.height = Math.max(1, Math.round(image.naturalHeight * scale))
    const context = canvas.getContext('2d')
    if (!context) return file
    context.drawImage(image, 0, 0, canvas.width, canvas.height)
    const blob = await new Promise<Blob | null>(resolve => canvas.toBlob(resolve, 'image/jpeg', JPEG_QUALITY))
    canvas.width = 0
    canvas.height = 0
    if (!blob) return file
    return new File([blob], jpegName(file.name), { type: 'image/jpeg', lastModified: file.lastModified })
  } catch {
    return file
  } finally {
    URL.revokeObjectURL(url)
  }
}
