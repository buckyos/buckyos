/* The page the mock Agent Service "renders": a form with one of everything the App handles outside the page
 * (a select, the three JS dialogs, a file input, a download, a permission prompt, a link that opens a tab), a text
 * field that shows what was typed, and a clock, so frames keep coming. Drawn on a canvas and encoded as JPEG. */

import type { SelectOption, Viewport } from '../api/types'

export interface Rect { x: number; y: number; width: number; height: number }

export const MOCK_PAGE_HEIGHT = 2400
export const MOCK_OPTIONS = ['Apple', 'Banana', 'Cherry', 'Durian']

export type MockElement = 'input' | 'select' | 'alert' | 'confirm' | 'prompt' | 'upload' | 'download' | 'location' | 'newTab' | 'signIn'

export interface MockPage {
  url: string
  title: string
  text: string
  composition: string
  focused: boolean
  selected: number[]
  multiple: boolean
  dialog: string
  files: string[]
  scrollY: number
  pointer: { x: number; y: number } | null
}

export function newMockPage(url: string): MockPage {
  let title = url
  try { title = new URL(url).hostname || url } catch { /* not a URL: shown as it is */ }
  return { url, title: url === 'about:blank' ? '' : title, text: '', composition: '', focused: false, selected: [0], multiple: false, dialog: '', files: [], scrollY: 0, pointer: null }
}

/** Where the elements are, in page coordinates (the page scrolls under the viewport). */
export function mockPageRects(width: number): Record<MockElement, Rect> {
  const pad = 16
  const inner = width - pad * 2
  const third = (inner - pad) / 3
  const row = (index: number, y: number): Rect => ({ x: pad + index * (third + pad / 2), y, width: third, height: 44 })
  return {
    input: { x: pad, y: 96, width: inner, height: 44 },
    select: { x: pad, y: 156, width: inner, height: 44 },
    alert: row(0, 216),
    confirm: row(1, 216),
    prompt: row(2, 216),
    upload: row(0, 276),
    download: row(1, 276),
    location: row(2, 276),
    newTab: { x: pad, y: 336, width: inner / 2 - pad / 2, height: 44 },
    signIn: { x: pad + inner / 2 + pad / 2, y: 336, width: inner / 2 - pad / 2, height: 44 },
  }
}

export function mockElementAt(page: MockPage, width: number, x: number, y: number): MockElement | null {
  const pageY = y + page.scrollY
  for (const [name, rect] of Object.entries(mockPageRects(width)) as Array<[MockElement, Rect]>) {
    if (x >= rect.x && x <= rect.x + rect.width && pageY >= rect.y && pageY <= rect.y + rect.height) return name
  }
  return null
}

export function mockSelectOptions(page: MockPage): SelectOption[] {
  return MOCK_OPTIONS.map((label, index) => ({ index, label, selected: page.selected.includes(index), disabled: index === 3 }))
}

function hue(text: string): number {
  let value = 0
  for (let i = 0; i < text.length; i++) value = (value * 31 + text.charCodeAt(i)) % 360
  return value
}

const LABELS: Record<Exclude<MockElement, 'input' | 'select'>, string> = {
  alert: 'Alert', confirm: 'Confirm', prompt: 'Prompt', upload: 'Upload', download: 'Download', location: 'Location', newTab: 'Open in new tab', signIn: 'Sign in',
}

export interface MockRenderInfo { viewport: Viewport; signedIn: boolean; frame: number }

function draw(ctx: OffscreenCanvasRenderingContext2D | CanvasRenderingContext2D, page: MockPage, info: MockRenderInfo) {
  const { width, height } = info.viewport
  const rects = mockPageRects(width)
  ctx.fillStyle = '#f6f7f9'
  ctx.fillRect(0, 0, width, height)
  ctx.save()
  ctx.translate(0, -page.scrollY)
  ctx.fillStyle = `hsl(${hue(page.url)} 55% 42%)`
  ctx.fillRect(0, 0, width, 72)
  ctx.fillStyle = '#ffffff'
  ctx.textBaseline = 'middle'
  ctx.font = '600 20px sans-serif'
  ctx.fillText(page.title || 'New tab', 16, 28, width - 32)
  ctx.font = '13px sans-serif'
  ctx.fillText(page.url, 16, 54, width - 32)

  const box = (rect: Rect, fill: string, stroke: string) => {
    ctx.fillStyle = fill
    ctx.strokeStyle = stroke
    ctx.lineWidth = 1.5
    ctx.beginPath()
    ctx.roundRect(rect.x, rect.y, rect.width, rect.height, 8)
    ctx.fill()
    ctx.stroke()
  }
  ctx.font = '15px sans-serif'
  box(rects.input, '#ffffff', page.focused ? '#2a78d6' : '#b8bec8')
  ctx.fillStyle = page.text || page.composition ? '#1c2230' : '#8a93a3'
  const shown = page.text || page.composition ? page.text : 'Type here'
  ctx.fillText(shown, rects.input.x + 12, rects.input.y + 22, rects.input.width - 24)
  if (page.composition) {
    const offset = Math.min(ctx.measureText(page.text).width, rects.input.width - 60)
    ctx.fillStyle = '#2a78d6'
    ctx.fillText(page.composition, rects.input.x + 12 + offset, rects.input.y + 22)
    const w = ctx.measureText(page.composition).width
    ctx.fillRect(rects.input.x + 12 + offset, rects.input.y + 34, w, 1.5)
  }
  box(rects.select, '#ffffff', '#b8bec8')
  ctx.fillStyle = '#1c2230'
  ctx.fillText(page.selected.map((index) => MOCK_OPTIONS[index]).join(', ') || '—', rects.select.x + 12, rects.select.y + 22, rects.select.width - 44)
  ctx.fillText('▾', rects.select.x + rects.select.width - 26, rects.select.y + 22)
  ctx.textAlign = 'center'
  for (const [name, label] of Object.entries(LABELS) as Array<[keyof typeof LABELS, string]>) {
    const rect = rects[name]
    box(rect, '#e7ebf3', '#b8bec8')
    ctx.fillStyle = '#1c2230'
    ctx.fillText(name === 'signIn' && info.signedIn ? 'Signed in' : label, rect.x + rect.width / 2, rect.y + 22, rect.width - 12)
  }
  ctx.textAlign = 'start'
  ctx.fillStyle = '#4a5466'
  ctx.font = '13px sans-serif'
  const lines = [
    `typed: ${page.text}`,
    `dialog: ${page.dialog}`,
    `files: ${page.files.join(', ')}`,
    `viewport: ${width}×${height} ${info.viewport.mobile ? 'mobile' : 'desktop'}`,
    `scroll: ${Math.round(page.scrollY)}`,
    `frame: ${info.frame}`,
  ]
  lines.forEach((line, index) => ctx.fillText(line, 16, 412 + index * 22, width - 32))
  for (let y = 600; y < MOCK_PAGE_HEIGHT; y += 200) {
    ctx.fillStyle = '#dfe3ea'
    ctx.fillRect(16, y, width - 32, 120)
    ctx.fillStyle = '#4a5466'
    ctx.fillText(`section at ${y}px`, 28, y + 24)
  }
  ctx.restore()
  if (page.pointer) {
    ctx.fillStyle = 'rgba(42, 120, 214, 0.35)'
    ctx.beginPath()
    ctx.arc(page.pointer.x, page.pointer.y, 10, 0, Math.PI * 2)
    ctx.fill()
  }
}

/** One frame of the page as JPEG bytes. */
export async function renderMockPage(page: MockPage, info: MockRenderInfo): Promise<ArrayBuffer> {
  const { width, height } = info.viewport
  if (typeof OffscreenCanvas !== 'undefined') {
    const canvas = new OffscreenCanvas(width, height)
    const ctx = canvas.getContext('2d')
    if (!ctx) throw Error('no 2d context')
    draw(ctx, page, info)
    return (await canvas.convertToBlob({ type: 'image/jpeg', quality: 0.6 })).arrayBuffer()
  }
  const canvas = document.createElement('canvas')
  canvas.width = width
  canvas.height = height
  const ctx = canvas.getContext('2d')
  if (!ctx) throw Error('no 2d context')
  draw(ctx, page, info)
  const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, 'image/jpeg', 0.6))
  if (!blob) throw Error('frame encoding failed')
  return blob.arrayBuffer()
}
