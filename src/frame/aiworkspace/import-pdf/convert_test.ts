/* PDF import (PDF 导入方案 §11): recorded pages of four fixtures (a 16:9 deck and an A4 document printed by
 * Chromium, an untagged LaTeX-like article, a scan) converted into plans compared with golden files, and the rules
 * one by one: lines and paragraphs, colours and fonts, shapes, pictures, the raster and its filter, the fallbacks.
 * `PDF_REGEN=1 deno test -A convert_test.ts` rewrites fixtures/pdf/expected/*.plan.json after an intended change. */

import { ImportReport } from '../import-drawio/convert.ts'
import { buildOperations } from '../import-drawio/operations.ts'
import type { PdfPage } from './data.ts'
import { convertPdfPage, type PdfConvertOptions, type PdfPagePlan, type RasterRequest } from './convert.ts'
import { defaultColumns, gridOrigins, pageScale, stageOf } from './layout.ts'
import { candidateOf, openGroup, svgObject } from './vector.ts'
import type { PathMark } from './interpret.ts'

function assert(cond: unknown, message = 'assertion failed'): asserts cond {
  if (!cond) throw new Error(message)
}
function assertEquals(actual: unknown, expected: unknown, message = '') {
  const a = JSON.stringify(actual), e = JSON.stringify(expected)
  if (a !== e) throw new Error(`${message ? `${message}: ` : ''}expected ${e.slice(0, 2000)}\n     got ${a.slice(0, 2000)}`)
}

const FIXTURES = new URL('../fixtures/pdf/', import.meta.url)
const REGEN = Deno.env.get('PDF_REGEN') === '1'

async function pages(name: string): Promise<PdfPage[]> {
  return JSON.parse(await Deno.readTextFile(new URL(`${name}.pages.json`, FIXTURES))).pages
}

function options(rasters: RasterRequest[] = [], extra: Partial<PdfConvertOptions> = {}): PdfConvertOptions {
  return {
    mode: 'editable', unit: 'paragraph', pathLimit: 3000, idPrefix: 't', fileBase: 'fixture', pageTitle: (n) => `Page ${n}`, imageText: () => 'Image',
    encodeImage: (request) => Promise.resolve({ bytes: new TextEncoder().encode(request.ref), mediaType: 'image/png' }),
    renderRaster: (request) => { rasters.push(request); return Promise.resolve({ bytes: new Uint8Array([1, 2, 3]), mediaType: 'image/png', rect: request.whole ? { x: 0, y: 0, w: 100, h: 100 } : { x: 10, y: 10, w: 20, h: 20 } }) },
    ...extra,
  }
}

const json = (plan: PdfPagePlan) => JSON.parse(JSON.stringify(plan, (key, value) => (key === 'bytes' && value instanceof Uint8Array ? value.length : value)))
const texts = (plan: PdfPagePlan) => plan.items.filter((i) => i.config.shape === 'text')
const configOf = (plan: PdfPagePlan, text: string) => texts(plan).find((i) => String(i.config.text).includes(text))?.config

for (const name of ['slides', 'doc', 'latex', 'scan']) {
  Deno.test(`golden plan: ${name}`, async () => {
    const report = new ImportReport()
    const plans: unknown[] = []
    for (const page of await pages(name)) plans.push(json(await convertPdfPage(page, report, options())))
    const out = { plans, report: { pages: report.pages, objects: report.objects, images: report.images, notes: report.list() } }
    const file = new URL(`expected/${name}.plan.json`, FIXTURES)
    if (REGEN) await Deno.writeTextFile(file, JSON.stringify(out, null, 1) + '\n')
    assertEquals(out, JSON.parse(await Deno.readTextFile(file)))
  })
}

Deno.test('a page: group, covering Frame with the page colour, objects in paint order', async () => {
  const [page] = await pages('slides')
  const plan = await convertPdfPage(page, new ImportReport(), options())
  assertEquals(plan.items[0].type, 'group')
  assertEquals(plan.items[1].type, 'frame')
  assertEquals(plan.items[1].rect, plan.items[0].rect)
  assertEquals(plan.pageSize, { w: 1920, h: 1080 })
  assertEquals(plan.pageColor, '#1e3a5f')
  assertEquals(plan.items[1].payload, { presentation: { background: '#1e3a5f' } })
  assertEquals(plan.items[1].config.fill, '#1e3a5f')
  // the heading names the page; its text is white and bold
  assertEquals(plan.name, '季度回顾 Quarterly Review')
  const heading = configOf(plan, 'Quarterly Review')!
  assertEquals([heading.font_color, heading.bold, heading.wrap, heading.valign], ['#ffffff', true, false, 'top'])
  assertEquals((heading.pdf_source as Record<string, unknown>).role, 'H1')
  // three list items, a footer in italics, a picture, the box with its border as one rectangle, the gradient as SVG
  assertEquals(texts(plan).filter((i) => (i.config.pdf_source as Record<string, unknown>).role === 'LI').length, 3)
  assertEquals(configOf(plan, 'Confidential')!.italic, true)
  assertEquals(plan.items.filter((i) => i.type === 'asset').length, 1)
  const box = plan.items.find((i) => i.config.shape === 'rect' && i.config.fill === '#f59e0b')!
  assertEquals([box.config.stroke, box.config.stroke_width], ['#b45309', 6])
  const gradient = plan.items.find((i) => i.config.shape === 'svg' && String((i.config.svg as { body: string }).body).includes('linearGradient'))
  assert(gradient, 'a gradient SVG')
  assert(String((gradient.config.svg as { body: string }).body).includes('stop-color="#22c55e"'))
  assertEquals(plan.counts.raster, false)
})

Deno.test('paragraphs: tagged blocks, line height, first-line indent, table cells, CJK, families', async () => {
  const [doc] = await pages('doc')
  const plan = await convertPdfPage(doc, new ImportReport(), options())
  const para = configOf(plan, 'PDF import turns')!
  assertEquals(String(para.text).split('\n').length, 2)
  assert(Number(para.line_height) > 1.2 && Number(para.line_height) < 1.4)
  assertEquals(para.font_family, 'serif')
  assertEquals(configOf(plan, '中文段落')!.font_family, undefined)
  assertEquals(configOf(plan, 'Value')!.text, 'Value')
  assertEquals((configOf(plan, 'Value')!.pdf_source as Record<string, unknown>).role, 'TD')
  assertEquals(plan.name, '1. Introduction')
  const [latex1, latex2] = await pages('latex')
  const article = await convertPdfPage(latex1, new ImportReport(), options())
  const first = String(configOf(article, 'first paragraph')!.text).split('\n')
  assertEquals(first.length, 4)
  assert(first[0].startsWith(' '), 'the first-line indent is kept as em spaces')
  assertEquals(String(configOf(article, 'A second paragraph')!.text).split('\n').length, 2)
  assertEquals(configOf(article, 'A Short Article')!.bold, true)
  assertEquals(configOf(article, 'Figure 1')!.italic, true)
  const vertical = texts(article).find((i) => i.config.text === 'vertical axis')!
  assertEquals(vertical.rotation, 270)
  // table cells of an untagged page are lines of their own
  const results = await convertPdfPage(latex2, new ImportReport(), options())
  for (const cell of ['Name', 'Value', 'Pages']) assertEquals(configOf(results, cell)!.text, cell)
  // one object per line on request (Q2)
  const lines = await convertPdfPage(latex1, new ImportReport(), options([], { unit: 'line' }))
  assert(texts(lines).length > texts(article).length + 3)
})

Deno.test('shapes: rectangles, ellipses, turned lines, a filled box with its outline', async () => {
  const [latex1] = await pages('latex')
  const plan = await convertPdfPage(latex1, new ImportReport(), options())
  const kinds = plan.items.filter((i) => i.type === 'shape' && i.config.shape !== 'text').map((i) => i.config.shape)
  assertEquals(kinds, ['line', 'rect', 'ellipse', 'line'])
  const box = plan.items.find((i) => i.config.shape === 'rect')!
  assertEquals([box.config.fill, box.config.stroke], ['#d9e6ff', '#334c99'])
  const dashed = plan.items.filter((i) => i.config.shape === 'line')[1]
  assertEquals(dashed.config.dash, 'dashed')
  assert(dashed.rotation > 90 && dashed.rotation < 180)
  const circle = plan.items.find((i) => i.config.shape === 'ellipse')!
  assert(Math.abs(circle.rect.w - circle.rect.h) < 1)
})

Deno.test('the raster holds only what did not become an object', async () => {
  const [slide1] = await pages('slides')
  // every vector drawn as shapes: no raster
  const rasters: RasterRequest[] = []
  await convertPdfPage(slide1, new ImportReport(), options(rasters))
  assertEquals(rasters.length, 0)
  // too many paths: the vectors go to the raster, which skips the text and the picture it took out
  const report = new ImportReport()
  const plan = await convertPdfPage(slide1, report, options(rasters, { pathLimit: 2 }))
  assertEquals(rasters.length, 1)
  assertEquals(rasters[0].whole, false)
  const showText = slide1.fnArray.flatMap((fn, i) => (fn === 44 ? [i] : []))
  assert(showText.every((i) => rasters[0].skip.has(i)))
  assertEquals(plan.items[2].key, 'p1~raster')
  assertEquals(plan.items[2].rect, { x: 10, y: 10, w: 20, h: 20 })
  assert(report.notes.has('pdf_complex_page') && report.notes.has('pdf_raster_parts'))
  assertEquals(plan.items.filter((i) => i.config.shape && i.config.shape !== 'text').length, 0)
})

Deno.test('fallbacks: unreadable text, the high-fidelity mode, a scan', async () => {
  const [latex1] = await pages('latex')
  const garbled: PdfPage = { ...latex1, text: latex1.text.map((t) => ({ ...t, str: t.str.replace(/[a-z]/g, '') })) }
  const rasters: RasterRequest[] = []
  const report = new ImportReport()
  const plan = await convertPdfPage(garbled, report, options(rasters))
  assert(report.notes.has('pdf_unreadable_text'))
  assertEquals(rasters.length, 1)
  assertEquals(rasters[0].whole, true)
  assertEquals(rasters[0].skip.size, 0)
  assertEquals(plan.items.map((i) => i.type), ['group', 'frame', 'asset'])

  const whole: RasterRequest[] = []
  const hifi = await convertPdfPage(latex1, new ImportReport(), options(whole, { mode: 'raster' }))
  assertEquals(hifi.items.map((i) => i.key), ['p1', 'p1~frame', 'p1~raster'])
  assertEquals([whole[0].skip.size, whole[0].whole], [0, true])

  const [scan] = await pages('scan')
  const scanReport = new ImportReport()
  const scanned = await convertPdfPage(scan, scanReport, options())
  assertEquals(scanned.items.map((i) => i.type), ['group', 'frame', 'asset'])
  const picture = scanned.items[2].rect
  assert(Math.abs(picture.w - scanned.pageSize!.w) < 1 && Math.abs(picture.h - scanned.pageSize!.h) < 1 && picture.x === 0 && picture.y === 0)
  assert(scanReport.notes.has('pdf_invisible_text'))
})

Deno.test('a plan becomes operations: Frame presentation, pictures, shapes in the group', async () => {
  const [slide1] = await pages('slides')
  const plan = await convertPdfPage(slide1, new ImportReport(), options())
  let n = 0
  const assets = new Map(plan.images.map((i) => [i.key, { object_id: `obj${i.key}`, file_name: i.name }]))
  const built = buildOperations(plan, { parentId: 'sf', origin: { x: 100, y: 200 }, afterKey: null, contentFolderId: 'sf-content', contentAfterKey: null, assets, newId: (p) => `${p}${n++}` })
  const group = built.objects[0]
  assertEquals([group.type_id, group.parent_id, group.placement], ['buckyos.container', 'sf', { x: 100, y: 200, w: 1920, h: 1080 }])
  const frame = built.objects[1]
  assertEquals(frame.payload.view, { type: 'frame' })
  assertEquals(frame.payload.presentation, { background: '#1e3a5f' })
  assertEquals(frame.parent_id, group.entity_id)
  assertEquals(frame.placement, { x: 0, y: 0, w: 1920, h: 1080 })
  assertEquals(built.objects.filter((o) => o.type_id === 'buckyos.asset-ref').length, 1)
  assert(built.objects.every((o) => o.entity_id === group.entity_id || o.parent_id === group.entity_id || o.parent_id === 'sf-content'))
  assertEquals(built.topLevel, [group.entity_id])
})

Deno.test('layout: page scale, grid, stage', () => {
  assertEquals(pageScale(720, 405), 1920 / 720)
  assertEquals(pageScale(595, 842), 1920 / 842)
  assertEquals([1, 2, 4, 5, 10, 100].map(defaultColumns), [1, 2, 2, 3, 4, 8])
  assertEquals(gridOrigins([{ w: 1920, h: 1080 }, { w: 1357, h: 1920 }, { w: 1920, h: 1080 }]), [{ x: 0, y: 0 }, { x: 2080, y: 0 }, { x: 0, y: 2080 }])
  assertEquals(stageOf([{ w: 1920, h: 1080 }, { w: 1920, h: 1080 }, { w: 1357, h: 1920 }]), { w: 1920, h: 1080 })
  assertEquals(stageOf([{ w: 1357, h: 1920 }]), { w: 1920, h: 2717 })
})

Deno.test('vector: an SVG over the size limit is not made', () => {
  const segs = Array.from({ length: 6000 }, (_, i) => ({ c: i === 0 ? 'M' : 'L', p: { x: (i * 13.37) % 970, y: (i * 7.11) % 890 } }) as const)
  const mark: PathMark = { index: 1, segs: [...segs], box: { x: 0, y: 0, w: 970, h: 890 }, fill: { kind: 'color', color: '#ff0000' }, stroke: null, evenOdd: false, fillAlpha: 1, strokeAlpha: 1, lineWidth: 1, dash: [], clips: [], form: 0, shading: false, complex: null }
  const c = candidateOf(mark, 1)!
  assertEquals(c.prim, null)
  assertEquals(svgObject(openGroup(c), {}, 'x', { page: 1 }), null)
  const small = candidateOf({ ...mark, segs: segs.slice(0, 20) }, 1)!
  const svg = svgObject(openGroup(small), {}, 'x', { page: 1 })!
  assertEquals(svg.config.fill, '#ff0000')
  assert(String((svg.config.svg as { body: string }).body).includes('var(--aiws-shape-fill)'))
})
