/* Importing a PDF (PDF 导入方案 §5): every page of the chosen range becomes a page group on one new free canvas —
 * text, pictures and shapes taken apart by the shared converter (`@pdf/*`), what cannot be taken apart as a raster
 * under them — laid out in a grid, plus a presentation path with one cut step per page on a black stage. This file
 * is the browser host: it reads pages with pdf.js, encodes pictures, renders the rasters through pdf.js's operator
 * filter, uploads (3 at a time, each content once), then commits in batches of 2 000 operations.
 *
 *   1 inspect   page count, first page size and picture, what the first page would become (the dialog's estimate)
 *   2 convert   page by page, with progress; cancelling before the first commit leaves the workspace untouched
 *   3 commit    the canvas, its content folder and the original file; the pages; the path
 * A page that fails to convert is kept whole as a picture; a refused batch stops the import and says how far it got. */

import { batches, buildOperations, ImportReport, type ImportOp, type UploadedAsset } from '@drawio/mod'
import { keysAfter } from '@drawio/keys'
import { convertPdfPage, defaultColumns, extractPage, gridOrigins, OPS, pageScale, readBookmarks, stageOf, type EncodedImage, type ImageRequest, type PdfConvertOptions, type PdfjsDocument, type PdfjsPage, type PdfPage, type PdfPagePlan, type RasterRequest, type RasterResult } from '@pdf/mod'
import { randomId } from '../../api/ids'
import type { Json, Operation, ShowStep } from '../../api/types'
import { aiwsT } from '../../i18n'
import { createPathOp } from '../../presentation/pathOps'
import type { WorkspaceStore } from '../../state/store'
import { openPdfDocument, renderPage, type PDFDocumentProxy, type PDFPageProxy } from '../media/pdfDoc'
import type { ImportOutcome } from './drawioImport'
import type { PdfImportSource } from './pdfImportRequest'
import { createSurfaceOps } from './surfaceOps'

/** Workspace assets are limited to 32 MiB (the original file cannot be kept above). */
export const ASSET_LIMIT = 32 * 1024 * 1024
/** More filled or stroked paths than this on a page: its drawing goes to the raster (§4.3, Q4). */
export const PATH_LIMIT = 3000
const RASTER_EDGE = 2560
const PICTURE_EDGE = 4096
const UPLOADS_AT_ONCE = 3
const MARGIN = 80

export interface PdfImportChoices {
  from: number
  to: number
  mode: 'editable' | 'raster'
  /** Pages with more paths than PATH_LIMIT: drawing in the raster, or the page whole as a picture. */
  complex: 'vectors' | 'whole'
  unit: 'paragraph' | 'line'
  keepOriginal: boolean
  createPath: boolean
  columns: number
  /** Long side of a whole-page raster in pixels (high-fidelity mode). */
  rasterEdge: 2560 | 3840
}

export interface PdfInspection {
  pageCount: number
  /** First page in points. */
  size: { w: number; h: number }
  /** First page, 480 px on its long side, as a JPEG (the dialog's picture and the original file's poster). */
  preview: Blob | null
  estimate: { text: number; images: number; shapes: number; raster: boolean } | null
  title: string | null
}

export function defaultChoices(inspection: PdfInspection, bytes: number): PdfImportChoices {
  return { from: 1, to: inspection.pageCount, mode: 'editable', complex: 'vectors', unit: 'paragraph', keepOriginal: bytes <= ASSET_LIMIT, createPath: true, columns: defaultColumns(inspection.pageCount), rasterEdge: 2560 }
}

const canvasBlob = (canvas: HTMLCanvasElement, type: string, quality?: number) =>
  new Promise<Blob>((resolve, reject) => canvas.toBlob((blob) => (blob ? resolve(blob) : reject(new Error('encode'))), type, quality))

/** Whether any pixel is not opaque (every pixel of up to 1 M pixels, else a sample). */
function hasAlpha(data: Uint8ClampedArray): boolean {
  const step = data.length / 4 > 1_000_000 ? 4 * 7 : 4
  for (let i = 3; i < data.length; i += step) if (data[i] < 255) return true
  return false
}

/** A pdf.js picture object (decoded, its soft mask applied) as something a canvas draws. */
function sourceOf(data: unknown): { draw: CanvasImageSource; width: number; height: number } | null {
  const d = data as { width?: number; height?: number; bitmap?: ImageBitmap; data?: Uint8Array | Uint8ClampedArray; kind?: number } | null
  if (!d || !d.width || !d.height) return null
  if (d.bitmap) return { draw: d.bitmap, width: d.width, height: d.height }
  if (!d.data) return null
  const rgba = new Uint8ClampedArray(d.width * d.height * 4)
  const src = d.data
  if (d.kind === 3) rgba.set(src.subarray(0, rgba.length))
  else if (d.kind === 2) for (let i = 0, j = 0; j < rgba.length; i += 3, j += 4) { rgba[j] = src[i]; rgba[j + 1] = src[i + 1]; rgba[j + 2] = src[i + 2]; rgba[j + 3] = 255 }
  else if (d.kind === 1) {
    // 1 bit per pixel, rows padded to a byte; a set bit is white
    const row = (d.width + 7) >> 3
    for (let y = 0; y < d.height; y++) for (let x = 0; x < d.width; x++) {
      const v = src[y * row + (x >> 3)] & (0x80 >> (x & 7)) ? 255 : 0
      const j = (y * d.width + x) * 4
      rgba[j] = rgba[j + 1] = rgba[j + 2] = v; rgba[j + 3] = 255
    }
  } else return null
  const canvas = document.createElement('canvas')
  canvas.width = d.width
  canvas.height = d.height
  canvas.getContext('2d')!.putImageData(new ImageData(rgba, d.width, d.height), 0, 0)
  return { draw: canvas, width: d.width, height: d.height }
}

/** A picture object of the page, waited for (pdf.js may still be decoding it). */
function pictureObject(page: PDFPageProxy, ref: string, inline: unknown[]): Promise<unknown> {
  if (ref.startsWith('op:')) return Promise.resolve((inline[Number(ref.slice(3))] as unknown[] | undefined)?.[0] ?? null)
  const objects = (ref.startsWith('g_') ? page.commonObjs : page.objs) as unknown as { get(id: string, callback: (data: unknown) => void): unknown }
  return new Promise((resolve) => {
    const timer = window.setTimeout(() => resolve(null), 15_000)
    try { objects.get(ref, (data) => { window.clearTimeout(timer); resolve(data) }) } catch { window.clearTimeout(timer); resolve(null) }
  })
}

async function encodePicture(page: PDFPageProxy, inline: unknown[], request: ImageRequest): Promise<EncodedImage | null> {
  const source = sourceOf(await pictureObject(page, request.ref, inline))
  if (!source) return null
  const crop = request.crop ?? { x: 0, y: 0, w: 1, h: 1 }
  const sx = Math.round(crop.x * source.width), sy = Math.round(crop.y * source.height)
  const sw = Math.max(1, Math.round(crop.w * source.width)), sh = Math.max(1, Math.round(crop.h * source.height))
  // the proportions it is drawn at (a stretched picture stays stretched), at the picture's own resolution
  const aspect = request.aspect > 0 ? request.aspect : sw / sh
  const ow = Math.max(sw, sh * aspect), oh = ow / aspect
  const k = Math.min(1, PICTURE_EDGE / Math.max(ow, oh))
  const canvas = document.createElement('canvas')
  canvas.width = Math.max(1, Math.round(ow * k))
  canvas.height = Math.max(1, Math.round(oh * k))
  const ctx = canvas.getContext('2d')!
  if (request.flipX) { ctx.translate(canvas.width, 0); ctx.scale(-1, 1) }
  ctx.drawImage(source.draw, sx, sy, sw, sh, 0, 0, canvas.width, canvas.height)
  const alpha = hasAlpha(ctx.getImageData(0, 0, canvas.width, canvas.height).data)
  // a photo as JPEG; transparency, small pictures and line art as PNG (§4.2)
  const jpeg = !alpha && canvas.width * canvas.height >= 256 * 256
  const blob = await canvasBlob(canvas, jpeg ? 'image/jpeg' : 'image/png', 0.9)
  return { bytes: new Uint8Array(await blob.arrayBuffer()), mediaType: blob.type || (jpeg ? 'image/jpeg' : 'image/png') }
}

/** The page drawn without the operators that became objects, cropped to what is left (§4.5). */
async function renderRaster(page: PDFPageProxy, opCount: number, world: { w: number; h: number }, request: RasterRequest, edge: number, report: ImportReport): Promise<RasterResult | null> {
  let mismatch = false
  const canvas = await renderPage(page, edge, {
    transparent: true,
    // the indices are those of the operator list the converter read: never filter another one
    filter: (index, list) => { if (list.fnArray.length !== opCount) { mismatch = true; return false } return !request.skip.has(index) },
  })
  if (mismatch) { report.add('pdf_raster_failed', 'Page rasters that could not be made', `p${page.pageNumber}`); return null }
  const ctx = canvas.getContext('2d')!
  const { data } = ctx.getImageData(0, 0, canvas.width, canvas.height)
  let x1 = canvas.width, y1 = canvas.height, x2 = -1, y2 = -1
  for (let y = 0; y < canvas.height; y++) {
    const row = y * canvas.width * 4
    for (let x = 0; x < canvas.width; x++) {
      if (data[row + x * 4 + 3] === 0) continue
      if (x < x1) x1 = x
      if (x > x2) x2 = x
      if (y < y1) y1 = y
      y2 = y
    }
  }
  if (x2 < 0) return null
  const perWorld = canvas.width / world.w
  let out = canvas
  let rect = { x: 0, y: 0, w: world.w, h: world.h }
  if (!request.whole) {
    x1 = Math.max(0, x1 - 2); y1 = Math.max(0, y1 - 2); x2 = Math.min(canvas.width - 1, x2 + 2); y2 = Math.min(canvas.height - 1, y2 + 2)
    out = document.createElement('canvas')
    out.width = x2 - x1 + 1
    out.height = y2 - y1 + 1
    out.getContext('2d')!.drawImage(canvas, x1, y1, out.width, out.height, 0, 0, out.width, out.height)
    rect = { x: x1 / perWorld, y: y1 / perWorld, w: out.width / perWorld, h: out.height / perWorld }
  }
  const opaque = request.whole && !hasAlpha(data)
  const blob = await canvasBlob(out, opaque ? 'image/jpeg' : 'image/png', 0.9)
  return { bytes: new Uint8Array(await blob.arrayBuffer()), mediaType: blob.type || 'image/png', rect }
}

function converterOptions(page: PDFPageProxy, inline: unknown[], opCount: number, choices: Pick<PdfImportChoices, 'mode' | 'unit' | 'rasterEdge'>, report: ImportReport, fileBase: string, idPrefix: string, encode: boolean): PdfConvertOptions {
  const base = page.getViewport({ scale: 1 })
  const scale = pageScale(base.width, base.height)
  const world = { w: Math.round(base.width * scale), h: Math.round(base.height * scale) }
  return {
    mode: choices.mode, unit: choices.unit, pathLimit: PATH_LIMIT, idPrefix, fileBase,
    pageTitle: (n) => aiwsT('aiworkspace.pdf.pageTitle', { n }), imageText: () => aiwsT('aiworkspace.pdf.imageMissing'),
    encodeImage: encode ? (request) => encodePicture(page, inline, request) : () => Promise.resolve({ bytes: new Uint8Array(0), mediaType: 'image/png' }),
    renderRaster: encode ? (request) => renderRaster(page, opCount, world, request, choices.mode === 'raster' ? choices.rasterEdge : RASTER_EDGE, report)
      // the estimate only asks whether a raster would be needed
      : () => Promise.resolve({ bytes: new Uint8Array(0), mediaType: 'image/png', rect: { x: 0, y: 0, w: 1, h: 1 } }),
  }
}

const fileBaseOf = (name: string) => name.replace(/\.pdf$/i, '').replace(/[\\/:*?"<>|]+/g, '_').slice(0, 80) || 'pdf'

/** What the dialog shows before anything is written. */
export async function inspectPdf(bytes: Uint8Array, signal?: AbortSignal): Promise<PdfInspection> {
  const { doc, close } = await openPdfDocument(bytes, signal)
  try {
    const page = await doc.getPage(1)
    const base = page.getViewport({ scale: 1 })
    let preview: Blob | null = null
    try { preview = await canvasBlob(await renderPage(page, 480), 'image/jpeg', 0.85) } catch { /* no picture */ }
    let estimate: PdfInspection['estimate'] = null
    let title: string | null = null
    try {
      const { page: data, raw } = await extractPage(doc as unknown as PdfjsDocument, page as unknown as PdfjsPage, new Map(), null)
      const plan = await convertPdfPage(data, new ImportReport(), converterOptions(page, raw.argsArray, data.fnArray.length, { mode: 'editable', unit: 'paragraph', rasterEdge: 2560 }, new ImportReport(), 'pdf', 'x', false))
      estimate = plan.counts
    } catch { /* the import tells */ }
    try {
      const info = await doc.getMetadata()
      const t = (info.info as { Title?: unknown } | undefined)?.Title
      title = typeof t === 'string' && t.trim() ? t.trim().slice(0, 120) : null
    } catch { /* none */ }
    return { pageCount: doc.numPages, size: { w: base.width, h: base.height }, preview, estimate, title }
  } finally {
    close()
  }
}

export interface PdfImportProgress { stage: 'convert' | 'upload' | 'commit'; done: number; total: number }

/** Upload pictures by content (each distinct content once, a few at a time). */
class Uploader {
  private readonly byContent = new Map<string, Promise<UploadedAsset | null>>()
  private running = 0
  private readonly queue: (() => void)[] = []
  private readonly store: WorkspaceStore
  private readonly report: ImportReport
  constructor(store: WorkspaceStore, report: ImportReport) { this.store = store; this.report = report }
  private async slot<T>(run: () => Promise<T>): Promise<T> {
    if (this.running >= UPLOADS_AT_ONCE) await new Promise<void>((resolve) => this.queue.push(resolve))
    this.running += 1
    try { return await run() } finally { this.running -= 1; this.queue.shift()?.() }
  }
  async upload(bytes: Uint8Array, mediaType: string, name: string): Promise<UploadedAsset | null> {
    const digest = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes as BufferSource)), (b) => b.toString(16).padStart(2, '0')).join('')
    let pending = this.byContent.get(digest)
    if (!pending) {
      pending = this.slot(async () => {
        try {
          const staged = await this.store.session.uploadAsset(new Blob([bytes as BlobPart], { type: mediaType }), name)
          return { object_id: staged.object_id, file_name: name }
        } catch {
          this.report.add('image_missing', 'Images that could not be stored replaced by dashed boxes', name)
          return null
        }
      })
      this.byContent.set(digest, pending)
    }
    return pending
  }
}


/** Import pages of a PDF into a new canvas. `progress` is told what is happening; `signal` cancels before the commit. */
export async function importPdf(store: WorkspaceStore, source: PdfImportSource, choices: PdfImportChoices, preview: Blob | null, progress: (p: PdfImportProgress) => void, signal: AbortSignal): Promise<ImportOutcome> {
  const name = source.name.replace(/\.pdf$/i, '') || 'PDF'
  const fileBase = fileBaseOf(source.name)
  const report = new ImportReport()
  const outcome: ImportOutcome = { name, report, surfaceId: null, created: [], bounds: null, kind: 'pdf', pathId: null }
  let doc: PDFDocumentProxy
  let close: () => void
  try {
    ({ doc, close } = await openPdfDocument(source.bytes, signal))
  } catch (error) {
    return { ...outcome, error: aiwsT(error instanceof Error && error.message === 'password' ? 'aiworkspace.pdf.error.password' : 'aiworkspace.pdf.error.unreadable', { reason: error instanceof Error ? error.message : String(error) }) }
  }
  const pageCount = doc.numPages
  const uploader = new Uploader(store, report)
  const plans: PdfPagePlan[] = []
  const assets = new Map<string, UploadedAsset | null>()
  const counts = { text: 0, images: 0, shapes: 0, rasters: 0 }
  try {
    const bookmarks = await readBookmarks(doc as unknown as PdfjsDocument)
    let oc: { isVisible(group: unknown): boolean } | null = null
    try { oc = await doc.getOptionalContentConfig() as unknown as { isVisible(group: unknown): boolean } } catch { /* none */ }
    const from = Math.max(1, Math.min(choices.from, pageCount)), to = Math.max(from, Math.min(choices.to, pageCount))
    const idPrefix = `pdf${randomId().slice(0, 6)}_`
    for (let n = from; n <= to; n++) {
      signal.throwIfAborted()
      progress({ stage: 'convert', done: n - from, total: to - from + 1 })
      const page = await doc.getPage(n)
      let plan: PdfPagePlan
      try {
        const { page: data, raw } = await extractPage(doc as unknown as PdfjsDocument, page as unknown as PdfjsPage, bookmarks, oc)
        const complex = choices.mode === 'editable' && choices.complex === 'whole' && data.fnArray.filter((fn) => fn === OPS.constructPath).length > PATH_LIMIT
        if (complex) report.add('pdf_complex_whole', 'Pages with very many paths imported whole as pictures', `p${n}`)
        plan = await convertPdfPage(data, report, converterOptions(page, raw.argsArray, data.fnArray.length, { ...choices, mode: complex ? 'raster' : choices.mode }, report, fileBase, idPrefix, true))
      } catch (error) {
        if (signal.aborted) throw error
        // the page whole as a picture
        report.add('pdf_page_failed', 'Pages that could not be taken apart imported whole as pictures', `p${n}`)
        const fallback: PdfPage = { number: n, width: page.getViewport({ scale: 1 }).width, height: page.getViewport({ scale: 1 }).height, transform: page.getViewport({ scale: 1 }).transform, fnArray: [], argsArray: [], text: [], fonts: {}, patterns: {}, images: {}, struct: null, links: [], bookmark: bookmarks.get(n) ?? null }
        plan = await convertPdfPage(fallback, report, { ...converterOptions(page, [], -1, { ...choices, mode: 'raster' }, report, fileBase, idPrefix, true), renderRaster: async () => {
          const base = page.getViewport({ scale: 1 })
          const s = pageScale(base.width, base.height)
          const canvas = await renderPage(page, RASTER_EDGE)
          const blob = await canvasBlob(canvas, 'image/jpeg', 0.9)
          return { bytes: new Uint8Array(await blob.arrayBuffer()), mediaType: 'image/jpeg', rect: { x: 0, y: 0, w: Math.round(base.width * s), h: Math.round(base.height * s) } }
        } })
      }
      // the page's pictures go up now, and their bytes are let go
      progress({ stage: 'upload', done: n - from, total: to - from + 1 })
      await Promise.all(plan.images.map(async (image) => {
        if (!assets.has(image.key) && image.bytes) assets.set(image.key, await uploader.upload(image.bytes, image.mediaType, image.name))
        delete image.bytes
      }))
      counts.text += plan.counts.text
      counts.images += plan.counts.images
      counts.shapes += plan.counts.shapes
      if (plan.counts.raster) counts.rasters += 1
      plans.push(plan)
      page.cleanup()
    }
  } catch (error) {
    close()
    if (signal.aborted) return { ...outcome, error: aiwsT('aiworkspace.pdf.cancelled') }
    return { ...outcome, error: aiwsT('aiworkspace.pdf.error.unreadable', { reason: error instanceof Error ? error.message : String(error) }) }
  }
  close()
  if (signal.aborted) return { ...outcome, error: aiwsT('aiworkspace.pdf.cancelled') }

  // ---- commit: the canvas, its folder and the original; then the pages; then the path
  const title = name.slice(0, 120)
  const { ops: surfaceOps, surfaceId } = createSurfaceOps(store, title, 'free')
  const folderId = `${surfaceId}-content`
  const firstOps: Operation[] = [...surfaceOps]
  let contentAfter: string | null = null
  if (choices.keepOriginal) {
    progress({ stage: 'commit', done: 0, total: 1 })
    let original: { object_id: string } | null = source.asset ?? null
    let posterId: string | null = null
    try {
      if (!original && source.bytes.length <= ASSET_LIMIT) original = await store.session.uploadAsset(new Blob([source.bytes as BlobPart], { type: 'application/pdf' }), source.name)
      if (preview) posterId = (await store.session.uploadAsset(preview, `${fileBase}.poster.jpg`)).object_id
    } catch {
      report.add('pdf_original_missing', 'The original PDF could not be stored', source.name)
    }
    if (original) {
      contentAfter = keysAfter(null, 1)[0]
      const payload: Record<string, Json> = { object_id: original.object_id, file_name: source.name.slice(0, 255), page_count: pageCount, ...(posterId ? { poster: posterId } : {}) }
      firstOps.push({ op: 'entity.create', entity_id: randomId('d'), type_id: 'buckyos.asset-ref', parent_id: folderId, order_key: contentAfter, payload })
    }
  }
  const label = aiwsT('aiworkspace.pdf.importLabel', { name })
  const first = await store.submit({ editId: `import:pdf:${surfaceId}`, label, operations: firstOps })
  if (first.status !== 'accepted' && first.status !== 'saved_locally') {
    return { ...outcome, error: aiwsT('aiworkspace.pdf.error.canvasFailed', { reason: first.status === 'rejected' || first.status === 'conflict' ? first.code : first.status }) }
  }
  outcome.surfaceId = surfaceId
  const sizes = plans.map((p) => p.pageSize ?? { w: p.bounds.w, h: p.bounds.h })
  const origins = gridOrigins(sizes, Math.max(1, Math.min(choices.columns, 12)))
  const all: ImportOp[] = []
  const frames: { id: string; page: number }[] = []
  const pageOf = new Map<string, number>()
  let afterKey: string | null = null
  for (const [i, plan] of plans.entries()) {
    const origin = { x: origins[i].x + MARGIN, y: origins[i].y + MARGIN }
    const built = buildOperations(plan, { parentId: surfaceId, origin, afterKey, contentFolderId: folderId, contentAfterKey: contentAfter, assets })
    const groupId = built.ids.get(plan.id)!
    afterKey = built.objects.find((o) => o.entity_id === groupId)?.order_key ?? afterKey
    for (const o of built.objects) if (o.parent_id === folderId) contentAfter = o.order_key
    for (const o of [...built.objects, ...built.lines]) pageOf.set(o.entity_id, i)
    all.push(...built.objects, ...built.lines)
    const frameId = built.ids.get(`${plan.id}~frame`)
    if (frameId) frames.push({ id: frameId, page: i })
    outcome.created.push(groupId)
    const b = { x: origin.x, y: origin.y, w: sizes[i].w, h: sizes[i].h }
    outcome.bounds = outcome.bounds ? { x: Math.min(outcome.bounds.x, b.x), y: Math.min(outcome.bounds.y, b.y), w: Math.max(outcome.bounds.x + outcome.bounds.w, b.x + b.w) - Math.min(outcome.bounds.x, b.x), h: Math.max(outcome.bounds.y + outcome.bounds.h, b.y + b.h) - Math.min(outcome.bounds.y, b.y) } : b
  }
  const parts = batches(all)
  let committedPages = 0
  for (const [k, operations] of parts.entries()) {
    progress({ stage: 'commit', done: k, total: parts.length })
    const result = await store.submit({ editId: `import:pdf:${surfaceId}:${k}`, label: parts.length > 1 ? aiwsT('aiworkspace.pdf.importBatchLabel', { name, part: k + 1, parts: parts.length }) : label, operations: operations as unknown as Operation[] })
    if (result.status !== 'accepted' && result.status !== 'saved_locally') {
      const reason = result.status === 'rejected' || result.status === 'conflict' ? (result.detail ? aiwsT('aiworkspace.canvas.codeWithDetail', { code: result.code, detail: String(result.detail) }) : result.code) : result.status
      return { ...outcome, error: aiwsT('aiworkspace.pdf.error.stopped', { pages: committedPages, reason }) }
    }
    // pages wholly written so far (a page's operations end in this batch or before)
    const lastPage = pageOf.get(operations.at(-1)!.entity_id) ?? 0
    committedPages = k === parts.length - 1 ? plans.length : lastPage
  }
  if (choices.createPath && frames.length) {
    const steps: ShowStep[] = frames.map((f) => ({ id: randomId('st'), target: { kind: 'frame', entity_id: f.id }, transition: 'cut' }))
    const path = createPathOp(store, title, 'presentation', { stage: stageOf(sizes), background: '#000000', steps })
    const result = await store.submit({ editId: `import:pdf:${surfaceId}:path`, label: aiwsT('aiworkspace.pdf.pathLabel', { name }), operations: [path.op] })
    if (result.status === 'accepted' || result.status === 'saved_locally') outcome.pathId = path.id
    else report.add('pdf_path_failed', 'The presentation path could not be created', title)
  }
  report.objects = counts.text + counts.images + counts.shapes
  outcome.summary = aiwsT('aiworkspace.pdf.summary', {
    pages: aiwsT('aiworkspace.drawio.count.pages', { count: plans.length }), text: aiwsT('aiworkspace.pdf.count.text', { count: counts.text }),
    images: aiwsT('aiworkspace.drawio.count.images', { count: counts.images }), shapes: aiwsT('aiworkspace.pdf.count.shapes', { count: counts.shapes }),
    rasters: aiwsT('aiworkspace.pdf.count.rasters', { count: counts.rasters }),
  })
  return outcome
}
