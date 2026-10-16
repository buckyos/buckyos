/* Importing draw.io into the canvas (draw.io 导入 TODO §3): read the file (any carrier), convert each page with the
 * shared module, upload its pictures, then commit objects before lines in batches of 2 000 operations under one
 * import label. One page lands on the free canvas being shown (centred in the view, or at a drop point); several
 * pages — or a flow page as target — become new free canvases named after the pages, with their background and
 * page size. What could not be kept is collected in the report the canvas shows afterwards. */

import { buildOperations, batches, convertPage, ImportReport, readDrawio, type ConvertOptions, type DrawioPage, type PagePlan, type UploadedAsset } from '@drawio/mod'
import { decodeStencilStyle, stencilToSvg } from '@drawio/stencil'
import { describeError } from '../../api/session'
import { aiwsT } from '../../i18n'
import type { Json, Operation } from '../../api/types'
import type { WorkspaceStore } from '../../state/store'
import { Emitter } from '../../state/emitter'
import { formatErrorText, imageFallbackText } from './drawioText'
import { libraryShape } from './stencilLibrary'
import { createSurfaceOps } from './surfaceOps'

export interface ImportTarget {
  /** The canvas being shown and whether it is a free one. */
  surfaceId: string | null
  isFree: boolean
  /** World point where the page's top-left goes (a drop), or null: centred on `center`. */
  at: { x: number; y: number } | null
  center: { x: number; y: number }
  /** Moves a spot that would cover existing objects to free space (the canvas knows what is there). */
  clear?: (rect: { x: number; y: number; w: number; h: number }) => { x: number; y: number }
}

export interface ImportOutcome {
  name: string
  report: ImportReport
  /** The canvas showing the first imported page, and the new top-level objects on it. */
  surfaceId: string | null
  created: string[]
  /** World bounds of the import on that canvas. */
  bounds: { x: number; y: number; w: number; h: number } | null
  /** Why it stopped (a refused commit, an unreadable file), if it did. */
  error?: string
}

/** The report of the last import, shown until closed: it outlives the canvas view (an import may switch canvases). */
class ReportSink {
  private outcome: ImportOutcome | null = null
  private readonly emitter = new Emitter()
  readonly subscribe = this.emitter.subscribe
  snapshot = () => this.outcome
  show(outcome: ImportOutcome | null) { this.outcome = outcome; this.emitter.emit() }
}
export const importReports = new ReportSink()

const OPTIONS: ConvertOptions = {
  stencil: (name) => libraryShape(name),
  inlineStencil: async (value) => {
    const shape = await decodeStencilStyle(value)
    if (!shape) return null
    const s = stencilToSvg(shape)
    return { title: s.title, svg: s.svg, anchors: s.anchors, unsupported: s.unsupported }
  },
  imageText: (url) => imageFallbackText(url),
}

const MARGIN = 40

/** Upload the plan's pictures: data URIs as they are, links when the browser may fetch them (else the fallback). */
async function uploadImages(store: WorkspaceStore, plans: PagePlan[], report: ImportReport): Promise<Map<string, UploadedAsset | null>> {
  const out = new Map<string, UploadedAsset | null>()
  const byContent = new Map<string, Promise<UploadedAsset | null>>()
  for (const plan of plans) {
    for (const image of plan.images) {
      const content = image.url ?? `${image.mediaType}:${image.bytes?.length}:${image.bytes ? Array.from(image.bytes.subarray(0, 64)).join(',') : ''}:${image.name}`
      let upload = byContent.get(content)
      if (!upload) {
        upload = (async () => {
          try {
            let blob: Blob
            if (image.bytes) blob = new Blob([image.bytes as BlobPart], { type: image.mediaType })
            else {
              const response = await fetch(image.url!, { mode: 'cors' })
              if (!response.ok) throw new Error(`HTTP ${response.status}`)
              blob = await response.blob()
            }
            const uploaded = await store.session.uploadAsset(blob, image.name)
            return { object_id: uploaded.object_id, file_name: image.name }
          } catch {
            report.add('image_missing', 'Images that could not be fetched replaced by dashed boxes with a link', image.url ?? image.name)
            return null
          }
        })()
        byContent.set(content, upload)
      }
      out.set(image.key, await upload)
    }
  }
  return out
}

function lastKey(store: WorkspaceStore, parentId: string): string | null {
  return store.outline.childrenOf(parentId).at(-1)?.order_key ?? null
}

/** Import a draw.io file (bytes or text). */
export async function importDrawio(store: WorkspaceStore, input: { name: string; bytes?: Uint8Array; text?: string }, target: ImportTarget): Promise<ImportOutcome> {
  const name = input.name.replace(/\.(drawio\.(png|svg|xml)|drawio|xml|png|svg)$/i, '') || 'draw.io'
  const report = new ImportReport()
  const outcome: ImportOutcome = { name, report, surfaceId: null, created: [], bounds: null }
  let pages: DrawioPage[]
  try {
    const file = await readDrawio(input)
    // a shape library: its entries side by side on one canvas
    pages = file.pages.length ? file.pages : file.library.flatMap((entry, i) => (entry.model ? [{ id: `lib-${i}`, name: entry.title || aiwsT('aiworkspace.drawio.libraryEntry', { n: i + 1 }), model: entry.model }] : []))
    if (pages.length === 0) throw new Error(aiwsT('aiworkspace.drawio.nothingToImport'))
  } catch (error) {
    return { ...outcome, error: aiwsT('aiworkspace.drawio.unreadable', { reason: formatErrorText(error) ?? describeError(error) }) }
  }
  const library = pages[0]?.id.startsWith('lib-')
  const plans: PagePlan[] = []
  for (const page of pages) plans.push(await convertPage(page, report, OPTIONS))
  const assets = await uploadImages(store, plans, report)
  const intoCurrent = target.surfaceId !== null && target.isFree && (plans.length === 1 || library)
  // where each plan goes: the current canvas (library entries in a row), or a new canvas per page
  let row = 0
  let firstBase: { x: number; y: number } | null = null
  const totalW = library ? plans.reduce((w, p) => w + p.bounds.w + MARGIN, -MARGIN) : 0
  const label = aiwsT('aiworkspace.drawio.importLabel', { name })
  for (const [i, plan] of plans.entries()) {
    let surfaceId: string
    let contentFolderId: string
    let origin: { x: number; y: number }
    if (intoCurrent) {
      surfaceId = target.surfaceId!
      contentFolderId = store.outline.get(surfaceId)?.content_folder_id ?? ''
      const w = library ? totalW : plan.bounds.w
      let base = target.at ?? { x: Math.round(target.center.x - w / 2), y: Math.round(target.center.y - plan.bounds.h / 2) }
      if (!target.at && i === 0 && target.clear) base = target.clear({ ...base, w, h: plan.bounds.h })
      if (i === 0) firstBase = base
      else base = firstBase ?? base
      origin = { x: base.x + row, y: base.y }
      row += plan.bounds.w + MARGIN
    } else {
      const config: Record<string, Json> = { ...(plan.background ? { background: plan.background } : {}), ...(plan.pageSize ? { page_size: plan.pageSize } : {}) }
      const { ops, surfaceId: created } = createSurfaceOps(store, plan.name.slice(0, 120) || `${name} ${i + 1}`, 'free')
      const surfaceOp = ops.find((op) => op.op === 'entity.create' && op.entity_id === created) as (Operation & { payload: Record<string, Json> }) | undefined
      if (surfaceOp && Object.keys(config).length) surfaceOp.payload.config = config
      const result = await store.submit({ editId: `import:${created}`, label: aiwsT('aiworkspace.drawio.importNewCanvasLabel', { name, canvas: plan.name }), operations: ops })
      if (result.status !== 'accepted' && result.status !== 'saved_locally') return { ...outcome, error: aiwsT('aiworkspace.drawio.newCanvasFailed', { reason: result.status === 'rejected' || result.status === 'conflict' ? result.code : result.status }) }
      surfaceId = created
      contentFolderId = `${created}-content`
      origin = { x: MARGIN, y: MARGIN }
    }
    const built = buildOperations(plan, { parentId: surfaceId, origin, afterKey: lastKey(store, surfaceId), contentFolderId, contentAfterKey: lastKey(store, contentFolderId), assets })
    const parts = [...batches(built.objects), ...batches(built.lines)]
    for (const [k, operations] of parts.entries()) {
      const result = await store.submit({ editId: `import:${surfaceId}:${i}:${k}`, label: parts.length > 1 ? aiwsT('aiworkspace.drawio.importBatchLabel', { name, part: k + 1, parts: parts.length }) : label, operations: operations as unknown as Operation[] })
      if (result.status !== 'accepted' && result.status !== 'saved_locally') {
        const reason = result.status === 'rejected' || result.status === 'conflict' ? (result.detail ? aiwsT('aiworkspace.canvas.codeWithDetail', { code: result.code, detail: String(result.detail) }) : result.code) : result.status
        return { ...outcome, error: aiwsT('aiworkspace.drawio.importStopped', { part: k + 1, parts: parts.length, reason }) }
      }
    }
    if (outcome.surfaceId === null || outcome.surfaceId === surfaceId) {
      outcome.surfaceId = surfaceId
      outcome.created.push(...built.topLevel)
      const b = { x: origin.x, y: origin.y, w: plan.bounds.w, h: plan.bounds.h }
      outcome.bounds = outcome.bounds ? { x: Math.min(outcome.bounds.x, b.x), y: Math.min(outcome.bounds.y, b.y), w: Math.max(outcome.bounds.x + outcome.bounds.w, b.x + b.w) - Math.min(outcome.bounds.x, b.x), h: Math.max(outcome.bounds.y + outcome.bounds.h, b.y + b.h) - Math.min(outcome.bounds.y, b.y) } : b
    }
  }
  return outcome
}

/** What a dropped or pasted file is. */
export type FileKind = 'drawio' | 'svg' | 'image' | 'other'

export async function classifyFile(file: File): Promise<{ kind: FileKind; bytes: Uint8Array; text?: string }> {
  const bytes = new Uint8Array(await file.arrayBuffer())
  const name = file.name.toLowerCase()
  if (/\.(drawio|drawio\.xml)$/.test(name)) return { kind: 'drawio', bytes }
  if (name.endsWith('.png') || file.type === 'image/png') {
    const { pngTextChunks } = await import('@drawio/bytes')
    const chunks = await pngTextChunks(bytes)
    return { kind: chunks.has('mxfile') || chunks.has('mxGraphModel') ? 'drawio' : 'image', bytes }
  }
  if (name.endsWith('.svg') || file.type === 'image/svg+xml') {
    const text = new TextDecoder().decode(bytes)
    return { kind: /<svg[^>]*\scontent=/.test(text.slice(0, 4000)) ? 'drawio' : 'svg', bytes, text }
  }
  if (name.endsWith('.xml')) {
    const text = new TextDecoder().decode(bytes)
    const { looksLikeDrawio } = await import('@drawio/file')
    return { kind: looksLikeDrawio(text) ? 'drawio' : 'other', bytes, text }
  }
  if (file.type.startsWith('image/')) return { kind: 'image', bytes }
  return { kind: 'other', bytes }
}
