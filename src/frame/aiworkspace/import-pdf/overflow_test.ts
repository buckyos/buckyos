import { ImportReport } from '../import-drawio/convert.ts'
import { overflowTextPdf } from '../fixtures/pdf/src/overflow.ts'
import { convertPdfPage, type RasterRequest } from './convert.ts'
import { extractPage } from './extract.ts'
import { available, openPdf } from './tools/pdfjs.ts'

Deno.test({ name: 'real PDF: an opaque background cannot cover text after layout overflow', ignore: !(await available()), sanitizeOps: false, sanitizeResources: false, fn: async () => {
  const { doc, oc, close } = await openPdf(overflowTextPdf())
  try {
    const { page } = await extractPage(doc, await doc.getPage(1), new Map(), oc)
    if (page.text.length < 1200) throw new Error('The overflow fixture must contain the real PDF.js text fragments')
    const report = new ImportReport()
    const rasters: RasterRequest[] = []
    const plan = await convertPdfPage(page, report, {
      mode: 'editable', unit: 'paragraph', pathLimit: 3000, idPrefix: 'overflow', fileBase: 'overflow', pageTitle: () => 'Overflow',
      renderRaster: (request) => {
        rasters.push(request)
        return Promise.resolve({ bytes: new Uint8Array([1]), mediaType: 'image/png', rect: { x: 0, y: 0, w: 1440, h: 1920 } })
      },
    })
    if (!report.notes.has('pdf_text_layout_limit')) throw new Error('The 60 KiB layout budget must be exceeded')
    if (rasters.length !== 1 || !rasters[0].whole || rasters[0].skip.size !== 0) throw new Error('The whole original page, including background and text, must remain in paint order')
    if (JSON.stringify(plan.items.map((item) => item.type)) !== '["group","frame","asset"]') throw new Error('No extracted background may cover the fallback text')
    if (plan.counts.text !== 0 || plan.counts.shapes !== 0 || !plan.counts.raster) throw new Error('Overflow must produce one complete page raster')
  } finally { await close() }
} })
