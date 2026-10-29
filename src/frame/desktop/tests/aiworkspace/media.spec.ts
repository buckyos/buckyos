/* Media assets (doc/workspace/BuckyOS AI Workspace 多媒体对象.md): a video, a sound, a PDF and another file dropped on
 * a free canvas get their poster or card, play in place or open the preview. The media are made by the test (the video
 * by the browser's MediaRecorder). Screenshots go to test-results/aiworkspace-media/. */

import type { Page } from '@playwright/test'
import { openCanvas, setCanvasMode, test, expect, type Api } from './fixtures'

const ALICE = 'tok-alice'
const SHOTS = 'test-results/aiworkspace-media'

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

async function canvasWorkspace(api: Api, title: string) {
  const ws = await api.rpc(ALICE, 'ws.create', { title })
  await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'sf-m-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'i', payload: { kind: 'folder', title: '画布', system: 'surface_content', surface_id: 'sf-m' } },
    { op: 'entity.create', entity_id: 'sf-m', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'i', payload: { kind: 'surface', layout: { mode: 'free' }, title: '画布', content_folder_id: 'sf-m-content' } },
  ])
  return ws as { workspace_id: string; epoch: string }
}

/** A 16-bit mono WAV of a tone. */
function wav(seconds: number, rate = 8000): Buffer {
  const n = Math.round(seconds * rate)
  const b = Buffer.alloc(44 + n * 2)
  b.write('RIFF', 0); b.writeUInt32LE(36 + n * 2, 4); b.write('WAVEfmt ', 8); b.writeUInt32LE(16, 16); b.writeUInt16LE(1, 20); b.writeUInt16LE(1, 22)
  b.writeUInt32LE(rate, 24); b.writeUInt32LE(rate * 2, 28); b.writeUInt16LE(2, 32); b.writeUInt16LE(16, 34); b.write('data', 36); b.writeUInt32LE(n * 2, 40)
  for (let i = 0; i < n; i++) b.writeInt16LE(Math.round(Math.sin((i / rate) * 2 * Math.PI * 440) * 6000), 44 + i * 2)
  return b
}

/** A one-page PDF (300 × 400 pt): a blue box and a line of text. */
function pdf(): Buffer {
  const content = '0.2 0.4 0.8 rg 40 60 220 180 re f BT /F1 28 Tf 40 320 Td (Hello PDF) Tj ET'
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>',
    `<< /Length ${content.length} >>\nstream\n${content}\nendstream`,
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
  ]
  let text = '%PDF-1.4\n'
  const offsets: number[] = []
  objects.forEach((body, i) => { offsets.push(text.length); text += `${i + 1} 0 obj\n${body}\nendobj\n` })
  const xref = text.length
  text += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${offsets.map((o) => `${String(o).padStart(10, '0')} 00000 n \n`).join('')}`
  text += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`
  return Buffer.from(text, 'latin1')
}

/** A short WebM recorded from a canvas animation in the page (320 × 180). */
async function webm(page: Page): Promise<Buffer> {
  const bytes = await page.evaluate(async () => {
    const canvas = document.createElement('canvas')
    canvas.width = 320
    canvas.height = 180
    const ctx = canvas.getContext('2d') as CanvasRenderingContext2D
    let frame = 0
    const draw = () => { ctx.fillStyle = `hsl(${(frame * 9) % 360} 70% 45%)`; ctx.fillRect(0, 0, 320, 180); ctx.fillStyle = '#fff'; ctx.fillRect((frame * 6) % 280, 70, 40, 40); frame += 1 }
    draw()
    const recorder = new MediaRecorder(canvas.captureStream(25), { mimeType: 'video/webm;codecs=vp8' })
    const chunks: Blob[] = []
    recorder.ondataavailable = (event) => chunks.push(event.data)
    const timer = window.setInterval(draw, 40)
    recorder.start(100)
    await new Promise((resolve) => window.setTimeout(resolve, 1500))
    const stopped = new Promise((resolve) => { recorder.onstop = resolve })
    recorder.stop()
    await stopped
    window.clearInterval(timer)
    return Array.from(new Uint8Array(await new Blob(chunks).arrayBuffer()))
  })
  return Buffer.from(bytes)
}

/** Drop a file on the canvas at a screen point (the browser's own drag and drop). */
async function drop(page: Page, name: string, mime: string, bytes: Buffer, at: { x: number; y: number }) {
  const dt = await page.evaluateHandle(({ b64, name, mime }) => {
    const bin = atob(b64)
    const data = new Uint8Array(bin.length)
    for (let i = 0; i < bin.length; i++) data[i] = bin.charCodeAt(i)
    const transfer = new DataTransfer()
    transfer.items.add(new File([data], name, { type: mime }))
    return transfer
  }, { b64: bytes.toString('base64'), name, mime })
  const main = page.locator('.aiws-canvas-main')
  await main.dispatchEvent('dragover', { dataTransfer: dt, clientX: at.x, clientY: at.y })
  await main.dispatchEvent('drop', { dataTransfer: dt, clientX: at.x, clientY: at.y })
}

/** The asset Block showing `fileName`, once the insertion is committed: its Cell, its AssetRef payload. */
async function assetBlock(api: Api, ws: { workspace_id: string }, fileName: string): Promise<{ cell: Json; asset: Json }> {
  let found: { cell: Json; asset: Json } | null = null
  await expect.poll(async () => {
    const outline = await api.outline(ALICE, ws.workspace_id)
    const asset = outline.find((e: Json) => e.type_id === 'buckyos.asset-ref' && e.name === fileName && !e.deleted)
    const cell = asset && outline.find((e: Json) => e.type_id === 'buckyos.cell' && e.source_id === asset.entity_id && !e.deleted)
    if (!cell) return false
    found = { cell, asset: (await api.read(ALICE, ws.workspace_id, asset.entity_id)).content.payload }
    return true
  }, { timeout: 30_000 }).toBe(true)
  return found as unknown as { cell: Json; asset: Json }
}

const block = (page: Page, cellId: string) => page.getByTestId(`aiws-canvas-block-${cellId}`)

test('MD-01 a dropped video gets its frame as the poster, plays in place, and stops', async ({ page, api }) => {
  const ws = await canvasWorkspace(api, `md-01 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await drop(page, 'clip.webm', 'video/webm', await webm(page), { x: 600, y: 400 })
  const { cell, asset } = await assetBlock(api, ws, 'clip.webm')
  // the backend says what the bytes are; the poster is a stored picture of the frame, the Block has its proportions
  expect(asset.media_type).toBe('video/webm')
  expect(asset.poster).toMatch(/^cyfile:/)
  expect(asset.image).toEqual({ width: 320, height: 180 })
  expect(cell.placement.w / cell.placement.h).toBeCloseTo(16 / 9, 1)
  const view = block(page, cell.entity_id)
  await expect(view.getByTestId('aiws-media-poster')).toBeVisible()
  await expect.poll(() => view.getByTestId('aiws-media-poster').evaluate((img: HTMLImageElement) => img.naturalWidth)).toBe(320)
  await page.screenshot({ path: `${SHOTS}/md01-poster.png` })

  // play: the system Preview component plays it inside the Block
  await view.getByTestId('aiws-media-play').click()
  const inline = view.getByTestId('aiws-media-inline')
  await expect(inline).toHaveAttribute('data-renderer', 'video')
  await expect(inline).toHaveAttribute('data-status', 'ready')
  await expect.poll(() => inline.locator('video').evaluate((v: HTMLVideoElement) => v.currentTime > 0 || !v.paused)).toBe(true)
  await page.screenshot({ path: `${SHOTS}/md01-playing.png` })
  // a click elsewhere on the canvas ends the activation: the poster is back
  await page.mouse.click(600, 750)
  await expect(view.getByTestId('aiws-media-player')).toHaveCount(0)
  await expect(view.getByTestId('aiws-media-poster')).toBeVisible()
})

test('MD-02 a sound without a cover is a card that plays in place; a PDF shows its first page and opens the preview', async ({ page, api }) => {
  const ws = await canvasWorkspace(api, `md-02 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await drop(page, 'tone.wav', 'audio/wav', wav(2.5), { x: 500, y: 300 })
  const sound = await assetBlock(api, ws, 'tone.wav')
  expect(sound.asset.media_type).toBe('audio/wav')
  expect(sound.asset.poster).toBeUndefined()
  expect(sound.asset.duration_ms).toBeGreaterThan(2000)
  const card = block(page, sound.cell.entity_id)
  await expect(card.getByTestId('aiws-media-card')).toContainText('tone.wav')
  await expect(card.getByTestId('aiws-media-card')).toContainText('0:03')
  await card.getByTestId('aiws-media-play').click()
  await expect(card.getByTestId('aiws-media-inline')).toHaveAttribute('data-renderer', 'audio')
  await expect(card.getByTestId('content-preview-audio')).toBeVisible()

  await drop(page, 'paper.pdf', 'application/pdf', pdf(), { x: 1000, y: 450 })
  // a single PDF asks first: a file, or editable pages (PDF 导入方案 Q7)
  await page.getByTestId('aiws-pdf-ask-file').click()
  const paper = await assetBlock(api, ws, 'paper.pdf')
  expect(paper.asset.media_type).toBe('application/pdf')
  expect(paper.asset.page_count).toBe(1)
  expect(paper.asset.poster).toMatch(/^cyfile:/)
  // the page's proportions (300 × 400)
  expect(paper.asset.image.height / paper.asset.image.width).toBeCloseTo(4 / 3, 1)
  const page1 = block(page, paper.cell.entity_id)
  await expect(page1.getByTestId('aiws-media-poster')).toBeVisible()
  await expect(page1.getByTestId('aiws-media-badge')).toHaveText('1 页')
  await expect(page1.getByTestId('aiws-media-play')).toHaveCount(0)
  await page.screenshot({ path: `${SHOTS}/md02-sound-and-pdf.png` })
  // a PDF does not play on the canvas: a double-click opens the preview over the workspace, Escape closes it
  await page1.dblclick()
  const dialog = page.getByTestId('aiws-media-preview')
  await expect(dialog).toBeVisible()
  await expect(dialog.getByTestId('content-preview')).toHaveAttribute('data-renderer', 'pdf')
  await page.screenshot({ path: `${SHOTS}/md02-pdf-preview.png` })
  await page.keyboard.press('Escape')
  await expect(dialog).toHaveCount(0)
})

test('MD-03 any other file is an icon card; its preview pages through the media of the canvas; view mode plays on double-click', async ({ page, api }) => {
  const ws = await canvasWorkspace(api, `md-03 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await drop(page, 'notes.txt', 'text/plain', Buffer.from('第一行笔记\nsecond line\n'), { x: 500, y: 300 })
  await drop(page, 'tone.wav', 'audio/wav', wav(1.2), { x: 1000, y: 300 })
  const notes = await assetBlock(api, ws, 'notes.txt')
  const tone = await assetBlock(api, ws, 'tone.wav')
  const card = block(page, notes.cell.entity_id)
  await expect(card.getByTestId('aiws-media-card')).toContainText('notes.txt')
  await expect(card.getByTestId('aiws-media-card')).toContainText('TXT')
  await card.dblclick()
  const dialog = page.getByTestId('aiws-media-preview')
  const preview = dialog.getByTestId('content-preview')
  await expect(preview).toHaveAttribute('data-renderer', 'text')
  await expect(preview).toContainText('第一行笔记')
  // previous / next: the media of this canvas
  await expect(preview).toHaveAttribute('data-item-count', '2')
  await page.screenshot({ path: `${SHOTS}/md03-text-preview.png` })
  await page.keyboard.press('Escape')
  await expect(dialog).toHaveCount(0)

  // reading: a double-click plays a sound in place (no write, no editor)
  await setCanvasMode(page, 'view')
  const sound = block(page, tone.cell.entity_id)
  await sound.dblclick()
  await expect(sound.getByTestId('aiws-media-inline')).toHaveAttribute('data-renderer', 'audio')
  await sound.getByTestId('aiws-media-stop').click()
  await expect(sound.getByTestId('aiws-media-player')).toHaveCount(0)
})
