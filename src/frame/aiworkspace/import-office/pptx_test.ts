/* pptx → canvas pages (Office 导入 TODO O3, §4): every fixture converts to the plans recorded in
 * ../fixtures/office/expected (`OFFICE_REGEN=1 deno test -A`), then the rules one by one and the sub-documents of
 * §5.3. Run: `deno test -A`. */

import { ImportReport, type PlanItem } from '../import-drawio/convert.ts'
import { buildOperations } from '../import-drawio/operations.ts'
import { convertSlide, openPackage, readDeck, type CropRequest, type OfficePagePlan, type PptxOptions, type RasterRequest } from './mod.ts'
import { assert, equal, fixture, golden, soffice, toPdf } from './test_util.ts'

async function convert(path: string, options: Partial<PptxOptions> = {}): Promise<{ pages: OfficePagePlan[]; report: ImportReport; hidden: number[] }> {
  const deck = await readDeck(await openPackage(await fixture(path)))
  const report = new ImportReport()
  const pages: OfficePagePlan[] = []
  for (const slide of deck.slides) pages.push(await convertSlide(deck, slide, report, { fileName: path.replace(/.*\//, ''), idPrefix: 't', pageTitle: (n) => `Slide ${n}`, ...options }))
  return { pages, report, hidden: deck.slides.filter((s) => s.hidden).map((s) => s.number) }
}

const summary = (pages: OfficePagePlan[], report: ImportReport, hidden: number[]) => ({
  hidden,
  pages: pages.map((p) => ({ name: p.name, pageColor: p.pageColor, pageSize: p.pageSize, counts: p.counts, items: p.items.map((i) => ({ ...i, image: i.image ? { key: i.image.key, name: i.image.name, mediaType: i.image.mediaType, bytes: i.image.bytes?.length ?? 0 } : undefined })) })),
  report: report.list().map((n) => ({ kind: n.kind, count: n.count, samples: n.samples })),
})

const FILES = ['pptx/basic.pptx', 'pptx/basic.lo.pptx', ...['shapes', 'SmartArt', 'smartart-simple', 'bar-chart', 'table_test', 'backgrounds', 'with_japanese', 'sample', 'placeholder-layout-color'].map((n) => `pptx/real/${n}.pptx`)]
for (const path of FILES) {
  Deno.test(`golden ${path}`, async () => {
    const { pages, report, hidden } = await convert(path)
    await golden(path.replace(/^pptx\//, 'pptx-').replace(/\//g, '-').replace(/\.pptx$/, ''), summary(pages, report, hidden))
  })
}

const content = (p: OfficePagePlan) => p.items.slice(2)
const cfg = (i: PlanItem) => i.config as Record<string, unknown>

Deno.test('basic: pages, titles, frames, hidden slide, page colour', async () => {
  const { pages, report, hidden } = await convert('pptx/basic.pptx')
  equal(pages.length, 9)
  equal(hidden, [8])
  equal(pages.map((p) => p.name), ['Office Import Deck', 'Agenda', 'Shapes', 'Pictures', 'Table', 'Slide 6', 'Chart', 'Hidden slide', 'Dark background'])
  for (const p of pages) {
    equal(p.pageSize, { w: 1920, h: 1080 })
    equal([p.items[0].type, p.items[1].type], ['group', 'frame'])
    equal((p.items[1].payload as { presentation: { background: string } }).presentation.background, p.pageColor)
  }
  equal(pages[8].pageColor, '#0f172a')
  assert(report.notes.has('pptx_notes_dropped'), 'notes reported')
})

Deno.test('basic: placeholders inherit position and style, shapes keep their look', async () => {
  const { pages } = await convert('pptx/basic.pptx')
  const title = content(pages[0])[0]
  equal([cfg(title).shape, cfg(title).font_size, cfg(title).align, cfg(title).valign], ['text', 88, 'center', 'middle'])
  const body = content(pages[1]).find((i) => String(cfg(i).text).includes('Editable'))!
  equal(String(cfg(body).text).split('\n'), ['• Why import Office files', '  – Editable pages', '    • Pictures and shapes', '• High fidelity with LibreOffice'])
  equal(cfg(body).bold, undefined, 'the style most of the text has (only the first run is bold)')
  const shapes = content(pages[2])
  const blue = shapes.find((i) => cfg(i).text === 'Blue box')!
  equal([cfg(blue).shape, cfg(blue).fill, cfg(blue).stroke, cfg(blue).font_color], ['rect', '#2563eb', '#1e3a8a', '#ffffff'], 'style fontRef lt1 beats the default text colour')
  equal(cfg(shapes.find((i) => cfg(i).text === 'Accent 2, lighter')!).fill, '#d99694', 'accent 2 lighter 40%')
  const arrow = shapes.find((i) => i.rotation === 30)!
  equal(cfg(arrow).shape, 'svg')
  const line = shapes.find((i) => cfg(i).shape === 'line')!
  assert(Math.abs(line.rotation - 32.47) < 0.1, `line turned to its ends: ${line.rotation}`)
  const red = shapes.find((i) => String(cfg(i).text).startsWith('Right aligned'))!
  equal([cfg(red).align, cfg(red).font_color, cfg(red).bold, cfg(red).font_size], ['right', '#dc2626', true, 56])
  const star = shapes.find((i) => String((cfg(i).svg as { body?: string } | undefined)?.body ?? '').includes('linearGradient'))
  assert(star, 'a gradient becomes an SVG shape')
})

Deno.test('basic: pictures, table grid, group scale, chart placeholder', async () => {
  const { pages, report } = await convert('pptx/basic.pptx')
  equal(content(pages[3]).filter((i) => i.type === 'asset').length, 2)
  assert(report.notes.has('pptx_crop_kept'), 'no host crop: the whole picture, reported')
  const cells = content(pages[4]).filter((i) => (cfg(i).office_source as { kind: string }).kind === 'table')
  equal(cells.length, 8, '9 cells, one merged')
  equal(cells.filter((c) => cfg(c).bold).length, 3, 'header row bold')
  const group = content(pages[5])
  equal(group.length, 2)
  equal(group[0].rect.w, group[1].rect.w)
  assert(group[0].rect.x > 280 && group[0].rect.w > 570, `scaled by chExt: ${JSON.stringify(group[0].rect)}`)
  const chart = content(pages[6])[1]
  equal([chart.type, cfg(chart).dash], ['shape', 'dashed'])
})

Deno.test('host hooks: crops and rasterized charts', async () => {
  const crops: CropRequest[] = []
  const rasters: RasterRequest[] = []
  const png = new Uint8Array([0x89, 0x50, 0x4e, 0x47])
  const { pages, report } = await convert('pptx/basic.pptx', {
    cropImage: (r) => { crops.push(r); return Promise.resolve({ bytes: png, mediaType: 'image/png' }) },
    rasterize: (r) => { rasters.push(r); return Promise.resolve({ bytes: png, mediaType: 'image/png' }) },
  })
  equal(crops.length, 1)
  assert(Math.abs(crops[0].crop.l - 0.25) < 0.001 && Math.abs(crops[0].crop.r - 0.1) < 0.001, 'crop fractions')
  equal(rasters.map((r) => r.kind), ['chart'])
  equal(content(pages[6])[1].type, 'asset')
  assert(!report.notes.has('pptx_chart') && report.notes.has('pptx_rasterized'), 'rasterized, not a placeholder')
  const sub = await openPackage(rasters[0].subdoc)
  equal(sub.kind, 'pptx')
  const deck = await readDeck(sub)
  equal(deck.slides.length, 1)
  assert(Math.abs(deck.width - 8 * 914400) < 2 && Math.abs(deck.height - 5 * 914400) < 2, 'the slide is the chart\'s size')
  assert(sub.paths().some((p) => /charts\/chart\d+\.xml$/.test(p)), 'chart copied')
  equal(sub.paths().filter((p) => /slideLayouts\/slideLayout\d+\.xml$/.test(p)).length, 1, 'only the layout in use')
  const bin = await soffice()
  if (bin) assert((await toPdf(bin, rasters[0].subdoc, 'pptx'))?.length, 'soffice converts the chart sub-document')
})

Deno.test('real decks: SmartArt drawing, table style borders, East Asian text', async () => {
  const smart = await convert('pptx/real/SmartArt.pptx')
  equal(content(smart.pages[0]).length, 5, 'SmartArt drawn from its saved drawing')
  equal(smart.report.list().length, 0)
  const grid = await convert('pptx/real/shapes.pptx')
  const cells = content(grid.pages[3]).filter((i) => (cfg(i).office_source as { kind: string }).kind === 'table')
  assert(cells.every((c) => cfg(c).stroke === '#000000'), 'Table Grid style borders')
  const ja = await convert('pptx/real/with_japanese.pptx')
  assert(content(ja.pages[0]).some((i) => /[぀-ヿ一-鿿]/.test(String(cfg(i).text ?? ''))), 'Japanese text kept')
})

Deno.test('a page plan writes through the draw.io importer', async () => {
  const { pages } = await convert('pptx/basic.pptx')
  const plan = pages[3]
  const assets = new Map(plan.images.map((i) => [i.key, { object_id: `o-${i.key}`, file_name: i.name }]))
  let n = 0
  const built = buildOperations(plan, { parentId: 'sf', origin: { x: 80, y: 80 }, afterKey: null, contentFolderId: 'sf-content', contentAfterKey: null, assets, newId: (p) => `${p}${++n}` })
  equal(built.topLevel.length, 1)
  equal(built.objects.filter((o) => o.type_id === 'buckyos.asset-ref').length, 2)
  const frame = built.objects.find((o) => (o.payload.view as { type?: string } | undefined)?.type === 'frame')!
  equal((frame.payload.presentation as { background: string }).background, '#ffffff')
})
