/* Importing Office documents (Office 导入 TODO §3–§6): the browser host of the shared converter (`@office/*`, run in
 * officeReader.worker.ts). It answers the converter's requests — LibreOffice pictures of charts, SmartArt, OLE objects,
 * EMF / WMF pictures and formulas (§5.3: the one-object sub-document goes to the content server's `repr` as a
 * transient upload, comes back as a PDF and is drawn with pdf.js), cropped pictures (canvas) — uploads pictures
 * (each content once, BMP as PNG), checks rich text through the WASM core and commits:
 *
 *   docx      a new flow page: rich text and picture Blocks in document order, the original file last (§3.2)
 *   pptx      a new free canvas: one page group per slide in a grid, a presentation path (§4.1), the original in the
 *             page's content folder (like a PDF import)
 *   faithful  the original uploaded, LibreOffice makes a PDF of it, the PDF importer takes over with the original as
 *             its file (§5.2, Q12)
 * Every import is one Commit: cancelling or a refused Commit leaves the workspace untouched, undo takes it back whole. */

import { allocateKeys, buildOperations, ImportReport, type ImportOp, type UploadedAsset } from '@drawio/mod'
import { keysAfter } from '@drawio/keys'
import { buildDocOperations, fileBaseOf, type CropRequest, type DocPlan, type EncodedImage, type OfficePagePlan, type RasterRequest } from '@office/mod'
import { gridOrigins, stageOf } from '@pdf/layout'
import { NfspClient } from 'buckyos/nfsp'
import { randomId } from '../../api/ids'
import type { Operation, ShowStep } from '../../api/types'
import { aiwsT } from '../../i18n'
import { createPathOp } from '../../presentation/pathOps'
import type { WorkspaceStore } from '../../state/store'
import { openPdfDocument, renderPage } from '../media/pdfDoc'
import type { ImportOutcome } from '../canvas/drawioImport'
import { ASSET_LIMIT, Uploader } from '../canvas/pdfImport'
import { createSurfaceOps } from '../canvas/surfaceOps'
import type { OfficeImportSource } from './officeImportRequest'
import { submitImport, type ImportCommitProgress } from './submitImport'
import type { DocxJob, OfficeReaderRequest, OfficeReaderResponse, ReportData, SlideJob } from './officeReader.worker'

export { ASSET_LIMIT }
const RASTER_EDGE = 2560
const MARGIN = 80
const PROFILE = 'preview-document'

export interface OfficeInspection {
  kind: 'docx' | 'pptx'
  title: string
  slides: { number: number; hidden: boolean }[]
  /** Slide size in points (pptx). */
  size: { w: number; h: number } | null
}

export interface OfficeImportChoices {
  mode: 'editable' | 'faithful'
  includeHidden: boolean
  /** Q4's option: deleted revisions kept struck through. */
  keepDeleted: boolean
  keepOriginal: boolean
  createPath: boolean
  columns: number
}

export type OfficeImportProgress = { stage: 'read' | 'convert' | 'draw' | 'upload'; done: number; total: number } | ImportCommitProgress

export function defaultChoices(inspection: OfficeInspection | null, bytes: number): OfficeImportChoices {
  const shown = inspection ? inspection.slides.filter((s) => !s.hidden).length : 0
  return { mode: 'editable', includeHidden: false, keepDeleted: false, keepOriginal: bytes <= ASSET_LIMIT, createPath: true, columns: Math.min(8, Math.max(1, Math.ceil(Math.sqrt(Math.max(1, shown))))) }
}

// ---- LibreOffice on the content server

const clients = new WeakMap<object, Promise<NfspClient>>()

/** The content server the Workspace uploads go to, as the same principal (its `hello` done once per session). */
function contentClient(store: WorkspaceStore): Promise<NfspClient> {
  let pending = clients.get(store.session)
  if (!pending) {
    pending = store.session.contentServer().then(async ({ endpoint, sessionToken }) => {
      const client = new NfspClient({ baseUrl: endpoint, sessionToken: async () => sessionToken })
      await client.hello()
      return client
    })
    pending.catch(() => clients.delete(store.session))
    clients.set(store.session, pending)
  }
  return pending
}

/** Whether the content server converts Office documents (`hello.repr.profiles` has `preview-document`, Q7). */
export async function libreOfficeAvailable(store: WorkspaceStore): Promise<boolean> {
  try {
    const capability = (await contentClient(store)).reprCapability
    return Boolean(capability?.available && capability.profiles.includes(PROFILE))
  } catch {
    return false
  }
}

const sleep = (ms: number, signal?: AbortSignal) => new Promise<void>((resolve, reject) => {
  const timer = setTimeout(resolve, ms)
  signal?.addEventListener('abort', () => { clearTimeout(timer); reject(new DOMException('aborted', 'AbortError')) }, { once: true })
})

/** A stored Office file as the PDF LibreOffice makes of it (nfs_server `repr`, profile preview-document). */
export async function libreOfficePdf(store: WorkspaceStore, objectId: string, signal?: AbortSignal): Promise<Uint8Array> {
  const raw = await contentClient(store)
  let record = await raw.repr(null, { purpose: 'preview', source: { obj_id: objectId }, runtime: { accept: ['pdf'] } })
  const started = Date.now()
  while (record.state === 'processing') {
    signal?.throwIfAborted()
    if (Date.now() - started > 10 * 60_000) throw new Error('LibreOffice took too long')
    await sleep(Math.min(3000, Math.max(400, record.retry_after_ms ?? 800)), signal)
    record = await raw.getRepr(record.work_key)
  }
  if (record.state === 'unsupported') throw new Error(record.detail ?? record.reason)
  if (record.state === 'failed') throw new Error(record.error.message || record.error.code)
  const url = /^[a-z][a-z0-9+.-]*:/i.test(record.result.read_url) ? record.result.read_url : `${raw.baseUrl}${record.result.read_url}`
  const response = await fetch(url, { signal })
  if (!response.ok) throw new Error(`HTTP ${response.status}`)
  return new Uint8Array(await response.arrayBuffer())
}

const canvasBlob = (canvas: HTMLCanvasElement, type: string, quality?: number) =>
  new Promise<Blob>((resolve, reject) => canvas.toBlob((blob) => (blob ? resolve(blob) : reject(new Error('encode'))), type, quality))

/** §5.3: a one-object sub-document → a transient upload → LibreOffice's PDF → its first page as a picture (transparent
 * where nothing is drawn, white margins cut when asked), at twice the object's size, long side ≤ 2560 px. */
function rasterizer(store: WorkspaceStore, signal: AbortSignal, onDraw: () => void): (request: RasterRequest) => Promise<EncodedImage | null> {
  return async (request) => {
    signal.throwIfAborted()
    onDraw()
    try {
      const type = request.subdocKind === 'docx' ? 'application/vnd.openxmlformats-officedocument.wordprocessingml.document' : 'application/vnd.openxmlformats-officedocument.presentationml.presentation'
      const uploaded = await store.session.uploadTransient(new Blob([request.subdoc as BlobPart], { type }), `${fileBaseOf(request.name)}.${request.subdocKind}`)
      const pdf = await libreOfficePdf(store, uploaded.object_id, signal)
      const { doc, close } = await openPdfDocument(pdf, signal)
      try {
        const page = await doc.getPage(1)
        const edge = Math.min(RASTER_EDGE, Math.max(64, Math.round(Math.max(request.width, request.height) * 2 * (96 / 72))))
        let canvas = await renderPage(page, edge, { transparent: true })
        if (request.trim) {
          const ctx = canvas.getContext('2d')!
          const { data } = ctx.getImageData(0, 0, canvas.width, canvas.height)
          let x1 = canvas.width, y1 = canvas.height, x2 = -1, y2 = -1
          for (let y = 0; y < canvas.height; y++) for (let x = 0; x < canvas.width; x++) {
            const i = (y * canvas.width + x) * 4
            // transparent, or white paper
            if (data[i + 3] === 0 || (data[i] > 250 && data[i + 1] > 250 && data[i + 2] > 250)) continue
            if (x < x1) x1 = x
            if (x > x2) x2 = x
            if (y < y1) y1 = y
            y2 = y
          }
          if (x2 < 0) return null
          const out = document.createElement('canvas')
          out.width = x2 - x1 + 5
          out.height = y2 - y1 + 5
          out.getContext('2d')!.drawImage(canvas, Math.max(0, x1 - 2), Math.max(0, y1 - 2), out.width, out.height, 0, 0, out.width, out.height)
          canvas = out
        }
        const blob = await canvasBlob(canvas, 'image/png')
        return { bytes: new Uint8Array(await blob.arrayBuffer()), mediaType: 'image/png' }
      } finally {
        close()
      }
    } catch (error) {
      if (signal.aborted) throw error
      return null
    }
  }
}

/** `a:srcRect`: the kept part of a picture, in its own format (a photo stays JPEG). */
async function cropPicture(request: CropRequest): Promise<EncodedImage | null> {
  const bitmap = await createImageBitmap(new Blob([request.bytes as BlobPart], { type: request.mediaType }))
  const { l, t, r, b } = request.crop
  const sx = Math.round(bitmap.width * l), sy = Math.round(bitmap.height * t)
  const sw = Math.max(1, Math.round(bitmap.width * (1 - l - r))), sh = Math.max(1, Math.round(bitmap.height * (1 - t - b)))
  const canvas = document.createElement('canvas')
  canvas.width = sw
  canvas.height = sh
  canvas.getContext('2d')!.drawImage(bitmap, sx, sy, sw, sh, 0, 0, sw, sh)
  bitmap.close()
  const jpeg = request.mediaType === 'image/jpeg'
  const blob = await canvasBlob(canvas, jpeg ? 'image/jpeg' : 'image/png', 0.92)
  return { bytes: new Uint8Array(await blob.arrayBuffer()), mediaType: blob.type || (jpeg ? 'image/jpeg' : 'image/png') }
}

/** A BMP as PNG (browsers draw BMP but a Workspace picture is PNG / JPEG / GIF / WebP / SVG). */
async function webPicture(bytes: Uint8Array, mediaType: string): Promise<{ bytes: Uint8Array; mediaType: string }> {
  if (mediaType !== 'image/bmp') return { bytes, mediaType }
  try {
    const bitmap = await createImageBitmap(new Blob([bytes as BlobPart], { type: mediaType }))
    const canvas = document.createElement('canvas')
    canvas.width = bitmap.width
    canvas.height = bitmap.height
    canvas.getContext('2d')!.drawImage(bitmap, 0, 0)
    bitmap.close()
    return { bytes: new Uint8Array(await (await canvasBlob(canvas, 'image/png')).arrayBuffer()), mediaType: 'image/png' }
  } catch {
    return { bytes, mediaType }
  }
}

// ---- the reader Worker

export class OfficeReader {
  private worker: Worker | null = null
  private next = 0
  private readonly pending = new Map<number, { resolve: (r: Extract<OfficeReaderResponse, { ok: true }>) => void; reject: (e: Error) => void }>()
  /** How the Worker's requests are answered (set per conversion). */
  hooks: { rasterize?: (r: RasterRequest) => Promise<EncodedImage | null>; crop?: (r: CropRequest) => Promise<EncodedImage | null> } = {}

  private send(request: { op: 'open'; bytes: Uint8Array } | { op: 'docx'; job: DocxJob } | { op: 'slide'; number: number; job: SlideJob }): Promise<Extract<OfficeReaderResponse, { ok: true }>> {
    if (!this.worker) {
      this.worker = new Worker(new URL('./officeReader.worker.ts', import.meta.url), { type: 'module', name: 'aiworkspace-office-reader' })
      this.worker.onmessage = (event: MessageEvent<OfficeReaderResponse>) => {
        const message = event.data
        if ('call' in message) {
          const answer = message.name === 'rasterize' ? this.hooks.rasterize?.(message.request) : this.hooks.crop?.(message.request)
          void (answer ?? Promise.resolve(null)).catch(() => null).then((result) => this.worker?.postMessage({ reply: message.call, result } satisfies OfficeReaderRequest))
          return
        }
        const p = this.pending.get(message.id)
        if (!p) return
        this.pending.delete(message.id)
        if (message.ok) p.resolve(message)
        else p.reject(Object.assign(new Error(message.error), { code: message.code }))
      }
      this.worker.onerror = (event) => { for (const p of this.pending.values()) p.reject(new Error(event.message)); this.pending.clear() }
    }
    const id = ++this.next
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject })
      this.worker!.postMessage({ ...request, id })
    })
  }

  async open(bytes: Uint8Array): Promise<OfficeInspection> {
    const r = await this.send({ op: 'open', bytes })
    return { kind: r.kind ?? 'docx', title: r.title ?? '', slides: r.slides ?? [], size: r.size ?? null }
  }
  async docx(job: DocxJob): Promise<{ plan: DocPlan; report: ReportData }> {
    const r = await this.send({ op: 'docx', job })
    return { plan: r.plan as DocPlan, report: r.report! }
  }
  async slide(number: number, job: SlideJob): Promise<{ plan: OfficePagePlan; report: ReportData }> {
    const r = await this.send({ op: 'slide', number, job })
    return { plan: r.plan as OfficePagePlan, report: r.report! }
  }
  close() {
    this.worker?.terminate()
    this.worker = null
    for (const p of this.pending.values()) p.reject(new Error('closed'))
    this.pending.clear()
  }
}

/** Why a file cannot be read, in the user's words. */
export function readError(error: unknown): string {
  const code = (error as { code?: string } | null)?.code
  if (code === 'encrypted') return aiwsT('aiworkspace.office.error.encrypted')
  return aiwsT('aiworkspace.office.error.unreadable', { reason: error instanceof Error ? error.message : String(error) })
}

function mergeReport(into: ImportReport, from: ReportData) {
  for (const n of from.notes) {
    const note = into.notes.get(n.kind)
    if (!note) { into.notes.set(n.kind, { ...n, samples: [...n.samples] }); continue }
    note.count += n.count
    for (const s of n.samples) if (note.samples.length < 5 && !note.samples.includes(s)) note.samples.push(s)
  }
  into.pages += from.pages
  into.objects += from.objects
  into.images += from.images
}

const PLACEHOLDERS = () => ({
  image: aiwsT('aiworkspace.office.placeholder.image'), chart: aiwsT('aiworkspace.office.placeholder.chart'), smartart: aiwsT('aiworkspace.office.placeholder.smartart'),
  ole: aiwsT('aiworkspace.office.placeholder.ole'), media: aiwsT('aiworkspace.office.placeholder.media'), object: aiwsT('aiworkspace.office.placeholder.object'),
})

/** The original file as a Workspace asset (the one an Office file Block already is, or uploaded now). */
async function originalAsset(store: WorkspaceStore, source: OfficeImportSource, report: ImportReport): Promise<{ object_id: string; file_name: string } | null> {
  if (source.asset) return { object_id: source.asset.object_id, file_name: source.name }
  if (source.bytes.length > ASSET_LIMIT) return null
  try {
    const staged = await store.session.uploadAsset(new Blob([source.bytes as BlobPart]), source.name)
    return { object_id: staged.object_id, file_name: source.name }
  } catch {
    report.add('office_original_missing', 'The original file could not be stored', source.name)
    return null
  }
}

/** Upload a plan's pictures (each content once); they leave the plan once stored. */
async function uploadImages(images: { key: string; name: string; mediaType: string; bytes?: Uint8Array }[], uploader: Uploader, assets: Map<string, UploadedAsset | null>) {
  await Promise.all(images.map(async (image) => {
    if (assets.has(image.key) || !image.bytes) return
    const web = await webPicture(image.bytes, image.mediaType)
    assets.set(image.key, await uploader.upload(web.bytes, web.mediaType, web.mediaType !== image.mediaType ? image.name.replace(/\.bmp$/i, '.png') : image.name))
    delete image.bytes
  }))
}

// ---- docx → flow page

async function importDocx(store: WorkspaceStore, reader: OfficeReader, source: OfficeImportSource, choices: OfficeImportChoices, raster: boolean, progress: (p: OfficeImportProgress) => void, signal: AbortSignal): Promise<ImportOutcome> {
  const report = new ImportReport()
  const name = source.name.replace(/\.[^.]+$/, '') || 'Document'
  const outcome: ImportOutcome = { name, report, surfaceId: null, created: [], bounds: null, kind: 'office' }
  let drawn = 0
  reader.hooks = { rasterize: raster ? rasterizer(store, signal, () => progress({ stage: 'draw', done: drawn++, total: drawn })) : undefined }
  progress({ stage: 'convert', done: 0, total: 1 })
  let plan: DocPlan
  try {
    const got = await reader.docx({ fileName: source.name, idPrefix: `docx${randomId().slice(0, 6)}_`, keepDeleted: choices.keepDeleted, formulaText: aiwsT('aiworkspace.office.formula'), raster })
    plan = got.plan
    mergeReport(report, got.report)
  } catch (error) {
    if (signal.aborted) return { ...outcome, error: aiwsT('aiworkspace.office.cancelled') }
    return { ...outcome, error: readError(error) }
  }
  // the core checks the rich text again; here it is checked before anything is written
  for (const segment of plan.segments) {
    if (segment.kind !== 'richtext') continue
    try { store.core.richtext_block_index(JSON.stringify(segment.content)) } catch (error) {
      return { ...outcome, error: aiwsT('aiworkspace.office.error.richtext', { reason: error instanceof Error ? error.message : String(error) }) }
    }
  }
  progress({ stage: 'upload', done: 0, total: 1 })
  const assets = new Map<string, UploadedAsset | null>()
  await uploadImages(plan.images, new Uploader(store, report), assets)
  if (signal.aborted) return { ...outcome, error: aiwsT('aiworkspace.office.cancelled') }
  const original = choices.keepOriginal ? await originalAsset(store, source, report) : null
  const title = plan.title.slice(0, 120) || name
  outcome.name = title
  const { ops: surfaceOps, surfaceId } = createSurfaceOps(store, title, 'flow')
  const label = aiwsT('aiworkspace.office.importLabel', { name: title })
  const built = buildDocOperations(plan, { surfaceId, contentFolderId: `${surfaceId}-content`, afterKey: null, contentAfterKey: null, assets, original })
  const failed = await submitImport(store, { editId: `import:office:${surfaceId}`, label, operations: [...surfaceOps, ...(built.ops as unknown as Operation[])] }, 'files', progress)
  if (failed) return { ...outcome, error: aiwsT('aiworkspace.office.error.failed', { reason: failed }) }
  outcome.surfaceId = surfaceId
  outcome.created = built.blocks
  report.objects = built.blocks.length
  const texts = plan.segments.filter((s) => s.kind === 'richtext').length
  outcome.summary = aiwsT('aiworkspace.office.summaryDoc', {
    blocks: aiwsT('aiworkspace.office.count.textBlocks', { count: texts }), images: aiwsT('aiworkspace.drawio.count.images', { count: plan.counts.images - built.missing }),
    tables: aiwsT('aiworkspace.office.count.tables', { count: plan.counts.tables }),
  })
  return outcome
}

// ---- pptx → canvas pages

async function importPptx(store: WorkspaceStore, reader: OfficeReader, source: OfficeImportSource, inspection: OfficeInspection, choices: OfficeImportChoices, raster: boolean, progress: (p: OfficeImportProgress) => void, signal: AbortSignal): Promise<ImportOutcome> {
  const report = new ImportReport()
  const name = (inspection.title || source.name.replace(/\.[^.]+$/, '') || 'Slides').slice(0, 120)
  const outcome: ImportOutcome = { name, report, surfaceId: null, created: [], bounds: null, kind: 'office', pathId: null }
  const slides = inspection.slides.filter((s) => choices.includeHidden || !s.hidden)
  const skipped = inspection.slides.length - slides.length
  if (skipped) report.add('pptx_hidden_skipped', 'Hidden slides left out', String(skipped))
  if (!slides.length) return { ...outcome, error: aiwsT('aiworkspace.office.error.noSlides') }
  let drawn = 0
  reader.hooks = { rasterize: raster ? rasterizer(store, signal, () => progress({ stage: 'draw', done: drawn++, total: drawn })) : undefined, crop: cropPicture }
  const uploader = new Uploader(store, report)
  const assets = new Map<string, UploadedAsset | null>()
  const plans: OfficePagePlan[] = []
  const counts = { text: 0, images: 0, shapes: 0, rasterized: 0, placeholders: 0 }
  const idPrefix = `pptx${randomId().slice(0, 6)}_`
  const job: SlideJob = { fileName: source.name, idPrefix, pageTitle: aiwsT('aiworkspace.office.slideTitle', { n: '{n}' }), placeholders: PLACEHOLDERS(), raster, crop: true }
  try {
    for (const [k, slide] of slides.entries()) {
      signal.throwIfAborted()
      progress({ stage: 'convert', done: k, total: slides.length })
      const got = await reader.slide(slide.number, job)
      mergeReport(report, got.report)
      progress({ stage: 'upload', done: k, total: slides.length })
      await uploadImages(got.plan.images, uploader, assets)
      for (const key of Object.keys(counts) as (keyof typeof counts)[]) counts[key] += got.plan.counts[key]
      plans.push(got.plan)
    }
  } catch (error) {
    if (signal.aborted) return { ...outcome, error: aiwsT('aiworkspace.office.cancelled') }
    return { ...outcome, error: readError(error) }
  }
  const { ops: surfaceOps, surfaceId } = createSurfaceOps(store, name, 'free')
  const folderId = `${surfaceId}-content`
  const firstOps: Operation[] = [...surfaceOps]
  let contentAfter: string | null = null
  if (choices.keepOriginal) {
    const original = await originalAsset(store, source, report)
    if (original) {
      contentAfter = keysAfter(null, 1)[0]
      firstOps.push({ op: 'entity.create', entity_id: randomId('d'), type_id: 'buckyos.asset-ref', parent_id: folderId, order_key: contentAfter, payload: { object_id: original.object_id, file_name: original.file_name.slice(0, 255) } })
    }
  }
  const label = aiwsT('aiworkspace.office.importLabel', { name })
  const sizes = plans.map((p) => p.pageSize ?? { w: p.bounds.w, h: p.bounds.h })
  const origins = gridOrigins(sizes, Math.max(1, Math.min(choices.columns, 12)))
  const all: ImportOp[] = []
  const frames: string[] = []
  const keys = allocateKeys(plans, assets, null, contentAfter)
  for (const [i, plan] of plans.entries()) {
    const origin = { x: origins[i].x + MARGIN, y: origins[i].y + MARGIN }
    const built = buildOperations(plan, { parentId: surfaceId, origin, afterKey: null, contentFolderId: folderId, contentAfterKey: contentAfter, assets, keys: keys[i] })
    const groupId = built.ids.get(plan.id)!
    all.push(...built.objects, ...built.lines)
    const frameId = built.ids.get(`${plan.id}~frame`)
    if (frameId) frames.push(frameId)
    outcome.created.push(groupId)
    const b = { x: origin.x, y: origin.y, w: sizes[i].w, h: sizes[i].h }
    outcome.bounds = outcome.bounds ? { x: Math.min(outcome.bounds.x, b.x), y: Math.min(outcome.bounds.y, b.y), w: Math.max(outcome.bounds.x + outcome.bounds.w, b.x + b.w) - Math.min(outcome.bounds.x, b.x), h: Math.max(outcome.bounds.y + outcome.bounds.h, b.y + b.h) - Math.min(outcome.bounds.y, b.y) } : b
  }
  let pathId: string | null = null
  const operations: Operation[] = [...firstOps, ...(all as unknown as Operation[])]
  if (choices.createPath && frames.length) {
    const steps: ShowStep[] = frames.map((id) => ({ id: randomId('st'), target: { kind: 'frame', entity_id: id }, transition: 'cut' }))
    const path = createPathOp(store, name, 'presentation', { stage: stageOf(sizes), background: '#000000', steps })
    operations.push(path.op)
    pathId = path.id
  }
  const failed = await submitImport(store, { editId: `import:office:${surfaceId}`, label, operations }, 'pages', progress)
  if (failed) return { ...outcome, created: [], bounds: null, error: aiwsT('aiworkspace.office.error.failed', { reason: failed }) }
  outcome.surfaceId = surfaceId
  outcome.pathId = pathId
  report.objects = counts.text + counts.images + counts.shapes + counts.placeholders
  outcome.summary = aiwsT('aiworkspace.office.summarySlides', {
    pages: aiwsT('aiworkspace.drawio.count.pages', { count: plans.length }), text: aiwsT('aiworkspace.pdf.count.text', { count: counts.text }),
    images: aiwsT('aiworkspace.drawio.count.images', { count: counts.images }), shapes: aiwsT('aiworkspace.pdf.count.shapes', { count: counts.shapes }),
  })
  return outcome
}

const LIBREOFFICE_WOULD_DRAW = new Set(['docx_chart', 'docx_smartart', 'docx_unsupported_image', 'docx_formula', 'pptx_chart', 'pptx_smartart', 'pptx_unsupported_image', 'pptx_ole_missing'])

/** Import an Office document with the editable path (docx → flow page, pptx → canvas pages). */
export async function importOffice(store: WorkspaceStore, reader: OfficeReader, source: OfficeImportSource, inspection: OfficeInspection, choices: OfficeImportChoices, libreOffice: boolean, progress: (p: OfficeImportProgress) => void, signal: AbortSignal): Promise<ImportOutcome> {
  try {
    const outcome = inspection.kind === 'pptx'
      ? await importPptx(store, reader, source, inspection, choices, libreOffice, progress, signal)
      : await importDocx(store, reader, source, choices, libreOffice, progress, signal)
    // what LibreOffice would have turned into pictures
    if (!libreOffice && [...outcome.report.notes.keys()].some((k) => LIBREOFFICE_WOULD_DRAW.has(k))) outcome.hint = aiwsT('aiworkspace.office.report.hintLibreOffice')
    return outcome
  } finally {
    reader.hooks = {}
  }
}

/** The faithful path (§5.2): the original stored (it stays the file of the import), LibreOffice's PDF of it. */
export async function faithfulPdf(store: WorkspaceStore, source: OfficeImportSource, progress: (p: OfficeImportProgress) => void, signal: AbortSignal): Promise<{ name: string; bytes: Uint8Array; original: { object_id: string; file_name: string } }> {
  progress({ stage: 'upload', done: 0, total: 1 })
  const original = source.asset ? { object_id: source.asset.object_id, file_name: source.name } : { object_id: (await store.session.uploadAsset(new Blob([source.bytes as BlobPart]), source.name)).object_id, file_name: source.name }
  progress({ stage: 'draw', done: 0, total: 1 })
  const bytes = await libreOfficePdf(store, original.object_id, signal)
  return { name: `${source.name.replace(/\.[^.]+$/, '')}.pdf`, bytes, original }
}

