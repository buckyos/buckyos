import { reviewFollowupPdf, syntheticCmykPdf } from '../fixtures/pdf/src/review-followup.ts'
import { ImportReport } from '../import-drawio/convert.ts'
import { exportDiagram } from '../import-drawio/export.ts'
import { readLook } from '../import-drawio/shapes.ts'
import { convertPdfPage, worldOf } from './convert.ts'
import { extractPage } from './extract.ts'
import { interpret } from './interpret.ts'
import { readPdfTextLayout } from './text-layout.ts'
import { available, openPdf, PDFJS_DIR } from './tools/pdfjs.ts'

const assert = (value: unknown, message: string) => { if (!value) throw new Error(message) }
const ignore = !(await available())

Deno.test({ name: 'synthetic tiny text keeps mixed paints on canvas and in editable Draw.io labels', ignore, fn: async () => {
  const { doc, oc, close } = await openPdf(reviewFollowupPdf())
  try {
    const { page } = await extractPage(doc, await doc.getPage(1), new Map(), oc)
    const report = new ImportReport()
    const plan = await convertPdfPage(page, report, { mode: 'editable', unit: 'paragraph', pathLimit: 3000, idPrefix: 'test', fileBase: 'review', pageTitle: () => 'Review' })
    const tiny = plan.items.find(item => String(item.config.text).includes('TINY'))!
    assert(tiny && readLook(tiny.config).fontSize === 4, 'Fixture must exercise the Shape inspector clamp')
    const layout = readPdfTextLayout(tiny.config, readLook(tiny.config))
    assert(layout?.spans.length === 3 && layout.spans.every(span => span.size < 4), 'Source spans must keep their small sizes')
    assert(plan.items.some(item => item.config.text === 'VISIBLE'), 'Visible text must stay editable')
    assert(!plan.items.some(item => String(item.config.text).includes('HIDDEN OCR')), 'Overlapping invisible OCR must remain hidden')
    assert(report.notes.has('pdf_invisible_text'), 'Invisible text report expected')
    const { xml } = exportDiagram([{ id: 'tiny', parent: null, kind: 'cell', view: 'shape', placement: tiny.rect, config: tiny.config }], { name: 'Review' })
    for (const css of ['color:#d90d0d', 'color:#0d26d9', 'font-weight:700', 'font-style:italic', 'font-size:3.33px']) assert(xml.includes(css), `Draw.io label missing ${css}`)
    assert(xml.includes('TINY') && xml.includes(' RED') && xml.includes(' BLUE') && xml.includes('html=1'), 'Draw.io must keep text and word boundaries')
  } finally { await close() }
} })

Deno.test({ name: 'a synthetic mixed paragraph fits the real layout budget with a style table', ignore, fn: async () => {
  const { doc, oc, close } = await openPdf(reviewFollowupPdf())
  try {
    const { page } = await extractPage(doc, await doc.getPage(2), new Map(), oc)
    const report = new ImportReport()
    const plan = await convertPdfPage(page, report, { mode: 'editable', unit: 'paragraph', pathLimit: 3000, idPrefix: 'test', fileBase: 'review', pageTitle: () => 'Review' })
    const native = plan.items.filter(item => item.config.shape === 'text')
    assert(native.length === 1 && !report.notes.has('pdf_text_layout_limit'), 'One long paragraph must stay editable')
    const config = native[0].config
    const layout = readPdfTextLayout(config, readLook(config))!
    assert(layout.spans.length === 768, 'All source fragments must survive')
    assert(layout.spans.filter(span => span.style).length === 384, 'Half the spans must have an override')
    const size = (value: unknown) => new TextEncoder().encode(JSON.stringify(value)).length
    const source = config.pdf_source as Record<string, unknown>
    assert(size(config) < 60 * 1024, 'Packed config must fit the 60 KiB budget')
    assert(size({ ...config, pdf_source: { ...source, text_layout: layout } }) > 60 * 1024, 'The same fixture must exceed the old inline budget')
  } finally { await close() }
} })

Deno.test({ name: 'synthetic DeviceCMYK and embedded ICC use the same profile in Deno extraction', ignore, fn: async () => {
  const profile = await Deno.readFile(new URL('iccs/CGATS001Compat-v2-micro.icc', PDFJS_DIR))
  const { doc, oc, close } = await openPdf(syntheticCmykPdf(profile))
  try {
    const { page } = await extractPage(doc, await doc.getPage(1), new Map(), oc)
    const fills = interpret(page, worldOf(page).world).paths.map(path => path.fill)
    assert(JSON.stringify(fills) === JSON.stringify([{ kind: 'color', color: '#9e557e' }, { kind: 'color', color: '#9e557e' }]), JSON.stringify(fills))
  } finally { await close() }
} })
