/* Reading Office documents off the main thread (Office 导入 TODO §2.1, O4): the zip and XML of a .docx / .pptx are
 * parsed and converted here (`@office/*`) and only plans go back. What needs the page — LibreOffice pictures of
 * charts and other objects (§5.3), cutting cropped pictures — is asked of the main thread by `call` messages and
 * answered by `reply`. One document at a time: `open` replaces what was open before. */

import { ImportReport } from '@drawio/convert'
import { convertDocx, convertSlide, OfficeError, openPackage, readDeck, type CropRequest, type Deck, type DocPlan, type EncodedImage, type OfficePackage, type OfficePagePlan, type RasterRequest } from '@office/mod'

export interface ReportData { notes: ReturnType<ImportReport['list']>; pages: number; objects: number; images: number }

export interface DocxJob { fileName: string; idPrefix: string; keepDeleted: boolean; formulaText: string; raster: boolean }
export interface SlideJob { fileName: string; idPrefix: string; pageTitle: string; placeholders: Record<string, string>; raster: boolean; crop: boolean }

export type OfficeReaderRequest =
  | { id: number; op: 'open'; bytes: Uint8Array }
  | { id: number; op: 'docx'; job: DocxJob }
  | { id: number; op: 'slide'; number: number; job: SlideJob }
  | { reply: number; result: EncodedImage | null }

export type OfficeReaderResponse =
  | { id: number; ok: true; kind?: 'docx' | 'pptx'; title?: string; slides?: { number: number; hidden: boolean }[]; size?: { w: number; h: number } | null; plan?: DocPlan | OfficePagePlan; report?: ReportData }
  | { id: number; ok: false; error: string; code?: string }
  | { call: number; name: 'rasterize'; request: RasterRequest }
  | { call: number; name: 'crop'; request: CropRequest }

let pkg: OfficePackage | null = null
let deck: Deck | null = null
let calls = 0
const waiting = new Map<number, (result: EncodedImage | null) => void>()

function ask(message: { name: 'rasterize'; request: RasterRequest } | { name: 'crop'; request: CropRequest }): Promise<EncodedImage | null> {
  const call = ++calls
  return new Promise((resolve) => {
    waiting.set(call, resolve)
    ;(self as unknown as Worker).postMessage({ call, ...message })
  })
}

const reportData = (report: ImportReport): ReportData => ({ notes: report.list(), pages: report.pages, objects: report.objects, images: report.images })

async function handle(request: Exclude<OfficeReaderRequest, { reply: number }>): Promise<OfficeReaderResponse> {
  switch (request.op) {
    case 'open': {
      pkg = await openPackage(request.bytes)
      deck = null
      if (pkg.kind === 'pptx') {
        deck = await readDeck(pkg)
        return { id: request.id, ok: true, kind: 'pptx', title: deck.title, slides: deck.slides.map((s) => ({ number: s.number, hidden: s.hidden })), size: { w: deck.width / 12700, h: deck.height / 12700 } }
      }
      if (pkg.kind === 'docx') return { id: request.id, ok: true, kind: 'docx', title: '', size: null }
      throw new OfficeError('not_office', 'neither a Word nor a PowerPoint document')
    }
    case 'docx': {
      if (!pkg) throw new Error('no document open')
      const report = new ImportReport()
      const { job } = request
      const plan = await convertDocx(pkg, report, {
        fileName: job.fileName, idPrefix: job.idPrefix, keepDeleted: job.keepDeleted, formulaText: () => job.formulaText,
        ...(job.raster ? { rasterize: (r: RasterRequest) => ask({ name: 'rasterize', request: r }) } : {}),
      })
      return { id: request.id, ok: true, plan, report: reportData(report) }
    }
    case 'slide': {
      if (!deck) throw new Error('no presentation open')
      const slide = deck.slides.find((s) => s.number === request.number)
      if (!slide) throw new Error(`no slide ${request.number}`)
      const report = new ImportReport()
      const { job } = request
      const plan = await convertSlide(deck, slide, report, {
        fileName: job.fileName, idPrefix: job.idPrefix, pageTitle: (n) => job.pageTitle.replace('{n}', String(n)), placeholderText: (kind) => job.placeholders[kind] ?? kind,
        ...(job.raster ? { rasterize: (r: RasterRequest) => ask({ name: 'rasterize', request: r }) } : {}),
        ...(job.crop ? { cropImage: (r: CropRequest) => ask({ name: 'crop', request: r }) } : {}),
      })
      return { id: request.id, ok: true, plan, report: reportData(report) }
    }
  }
}

self.onmessage = (event: MessageEvent<OfficeReaderRequest>) => {
  const request = event.data
  if ('reply' in request) {
    waiting.get(request.reply)?.(request.result)
    waiting.delete(request.reply)
    return
  }
  handle(request).then(
    (response) => (self as unknown as Worker).postMessage(response),
    (error: unknown) => (self as unknown as Worker).postMessage({ id: request.id, ok: false, error: error instanceof Error ? error.message : String(error), code: (error as { code?: string }).code } satisfies OfficeReaderResponse),
  )
}
