/* ── Read an image file into a document-safe data: URL (downscaled / re-encoded when large) ── */

import { MAX_IMAGE_EDGE, MAX_IMPORT_BYTES } from '../domain/types'

export interface ImageReadResult {
  src: string
  width: number
  height: number
  bytes: number
  mime: string
}

const KEEP_ORIGINAL_BYTES = 1.5 * 1024 * 1024

export function isImageFile(file: File): boolean {
  return file.type.startsWith('image/') || /\.(png|jpe?g|gif|webp|bmp|svg|avif)$/i.test(file.name)
}

function fileToDataUrl(file: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const r = new FileReader()
    r.onload = () => resolve(String(r.result))
    r.onerror = () => reject(r.error ?? new Error('读取文件失败'))
    r.readAsDataURL(file)
  })
}

function loadImage(src: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image()
    img.onload = () => resolve(img)
    img.onerror = () => reject(new Error('无法解码图片'))
    img.src = src
  })
}

export async function readImageFile(file: File): Promise<ImageReadResult> {
  if (!isImageFile(file)) throw new Error(`${file.name} 不是图片文件`)
  if (file.size > MAX_IMPORT_BYTES) throw new Error(`图片超过 ${MAX_IMPORT_BYTES / 1024 / 1024}MB 限制`)
  if (file.type === 'image/svg+xml' || /\.svg$/i.test(file.name)) {
    const text = await file.text()
    const src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(text)}`
    const img = await loadImage(src)
    return { src, width: img.naturalWidth || 512, height: img.naturalHeight || 512, bytes: src.length, mime: 'image/svg+xml' }
  }
  const url = URL.createObjectURL(file)
  try {
    const img = await loadImage(url)
    const w = img.naturalWidth
    const h = img.naturalHeight
    const scale = Math.min(1, MAX_IMAGE_EDGE / Math.max(w, h))
    if (scale === 1 && file.size <= KEEP_ORIGINAL_BYTES) {
      const src = await fileToDataUrl(file)
      return { src, width: w, height: h, bytes: file.size, mime: file.type || 'image/png' }
    }
    const canvas = document.createElement('canvas')
    canvas.width = Math.max(1, Math.round(w * scale))
    canvas.height = Math.max(1, Math.round(h * scale))
    const ctx = canvas.getContext('2d')
    if (!ctx) throw new Error('当前环境不支持图片处理')
    ctx.drawImage(img, 0, 0, canvas.width, canvas.height)
    const keepAlpha = file.type === 'image/png' || file.type === 'image/webp' || file.type === 'image/gif'
    const mime = keepAlpha ? 'image/png' : 'image/jpeg'
    let src = canvas.toDataURL(mime, 0.86)
    if (keepAlpha && src.length > KEEP_ORIGINAL_BYTES * 1.4) src = canvas.toDataURL('image/jpeg', 0.86)
    return { src, width: canvas.width, height: canvas.height, bytes: Math.round((src.length * 3) / 4), mime: src.startsWith('data:image/png') ? 'image/png' : 'image/jpeg' }
  } finally {
    URL.revokeObjectURL(url)
  }
}

export function formatBytes(n?: number): string {
  if (!n) return ''
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`
  return `${(n / 1024 / 1024).toFixed(1)} MB`
}
