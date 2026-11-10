/* docx → flow page (Office 导入 TODO O2, §3): every fixture converts to the plan recorded in
 * ../fixtures/office/expected (`OFFICE_REGEN=1 deno test -A` rewrites them) and validates against the rich text
 * schema; then the rules one by one, the operations, and the sub-documents of §5.3. Run: `deno test -A`. */

import { ImportReport } from '../import-drawio/convert.ts'
import { buildDocOperations, convertDocx, openPackage, splitDoc, validateDoc, type AstNode, type DocPlan, type DocxOptions, type RasterRequest } from './mod.ts'
import { assert, counter, equal, fixture, golden, soffice, toPdf } from './test_util.ts'

async function convert(path: string, options: Partial<DocxOptions> = {}): Promise<{ plan: DocPlan; report: ImportReport }> {
  const report = new ImportReport()
  const plan = await convertDocx(await openPackage(await fixture(path)), report, { fileName: path.replace(/.*\//, ''), idPrefix: 't', newId: counter(), ...options })
  for (const s of plan.segments) if (s.kind === 'richtext') equal(validateDoc(s.content), [], `${path}: schema`)
  return { plan, report }
}

const summary = (plan: DocPlan, report: ImportReport) => ({
  title: plan.title, counts: plan.counts, segments: plan.segments,
  images: plan.images.map((i) => ({ key: i.key, name: i.name, mediaType: i.mediaType, bytes: i.bytes?.length ?? 0 })),
  report: report.list().map((n) => ({ kind: n.kind, count: n.count, samples: n.samples })),
})

const FILES = ['docx/basic.docx', 'docx/basic.lo.docx', 'docx/features.docx', ...['footnotes', 'delins', 'ComplexNumberedLists', 'heading123', 'VariousPictures', 'shapes-with-text', 'FieldCodes', 'testComment', 'WithGIF', 'chartex'].map((n) => `docx/real/${n}.docx`)]
for (const path of FILES) {
  Deno.test(`golden ${path}`, async () => {
    const { plan, report } = await convert(path)
    await golden(path.replace(/^docx\//, 'docx-').replace(/\//g, '-').replace(/\.docx$/, ''), summary(plan, report))
  })
}

/** Top-level nodes of all rich text segments, in order. */
const nodes = (plan: DocPlan): AstNode[] => plan.segments.flatMap((s) => (s.kind === 'richtext' ? s.content.content ?? [] : []))
const plain = (n: AstNode): string => (n.type === 'text' ? n.text ?? '' : (n.content ?? []).map(plain).join(n.type === 'list_item' || n.type.endsWith('_list') ? '|' : ''))

Deno.test('basic: headings, marks, link, lists, quote, code, table, picture between Blocks', async () => {
  const { plan, report } = await convert('docx/basic.docx')
  equal(plan.title, '季度报告 Quarterly Report')
  const all = nodes(plan)
  equal(all.filter((n) => n.type === 'heading').map((n) => [n.attrs?.level, plain(n)]), [[1, 'Quarterly Report'], [1, 'Overview'], [2, 'Details'], [3, 'Fine print'], [3, 'Finer print']])
  const para = all.find((n) => plain(n).startsWith('Plain, '))!
  const marked = (para.content ?? []).filter((t) => t.marks).map((t) => [t.text, t.marks!.map((m) => m.type).join('+')])
  equal(marked, [['bold', 'strong'], ['italic', 'em'], ['struck', 'strike'], ['bold italic', 'strong+em'], ['example.com', 'link']])
  const bullets = all.find((n) => n.type === 'bullet_list')!
  equal(bullets.content!.length, 3)
  equal(bullets.content![1].content![1].type, 'bullet_list', '"List Bullet 2" nests under the item before it')
  equal(all.find((n) => n.type === 'ordered_list')!.content!.length, 3)
  assert(all.some((n) => n.type === 'blockquote'), 'quote')
  equal(all.find((n) => n.type === 'code_block')?.content?.[0].text, 'fn main() {\n    println!("hi");\n}')
  const table = all.find((n) => n.type === 'table')!
  equal(table.content![0].content!.map((c) => c.attrs?.header), [1, 1, 1])
  equal(table.content!.map((r) => r.content!.length), [3, 3, 3, 3], 'rows padded and spans repeated')
  // the picture is a Block of its own after the paragraph it sat in
  const kinds = plan.segments.map((s) => s.kind)
  equal(kinds, ['richtext', 'image', 'richtext'])
  const picture = plan.segments[1] as Extract<DocPlan['segments'][number], { kind: 'image' }>
  equal(picture.pixels, { width: 400, height: 200 })
  assert(report.notes.has('docx_heading_level') && report.notes.has('docx_merged_cells'), 'reported')
})

Deno.test('features: revisions, comments, notes, fields, content controls, text boxes, formula, list start', async () => {
  const { plan, report } = await convert('docx/features.docx')
  const texts = nodes(plan).map(plain)
  assert(texts.includes('Kept inserted text.'), 'insertion kept, deletion dropped')
  assert(texts.includes('With a footnote[1] and an endnote[2].'), 'note references')
  assert(texts.includes('Visit BuckyOS.'), 'HYPERLINK field result')
  assert(!texts.some((t) => t.includes('Features 1') || t.includes('Numbers 2')), 'TOC entries dropped')
  assert(texts.includes('Inside a content control'), 'block content control')
  assert(texts.includes('Floating box text'), 'text box promoted')
  assert(texts.includes('Energy: [Formula]'), 'formula placeholder without LibreOffice')
  assert(!texts.some((t) => t.includes('hidden text')), 'hidden text dropped')
  const start = nodes(plan).find((n) => n.type === 'ordered_list' && plain(n).startsWith('Five'))
  equal(start?.attrs?.start, 5)
  const notes = nodes(plan).slice(-2)
  equal(notes.map((n) => n.type), ['horizontal_rule', 'ordered_list'])
  equal(plain(notes[1]), 'The footnote text.|The endnote text.')
  for (const kind of ['docx_revisions', 'docx_comments', 'docx_floating_text', 'docx_formula', 'docx_nested_table']) assert(report.notes.has(kind), kind)
  const kept = await convert('docx/features.docx', { keepDeleted: true })
  const line = nodes(kept.plan).find((n) => plain(n).startsWith('Kept'))!
  equal(line.content!.find((t) => t.text === 'deleted ')?.marks, [{ type: 'strike' }], 'Q4 option: deletions struck through')
})

Deno.test('real Word files: restarts, chart fallbacks, unsupported pictures', async () => {
  const lists = nodes((await convert('docx/real/ComplexNumberedLists.docx')).plan).filter((n) => n.type === 'ordered_list')
  equal(lists.map((l) => l.attrs?.start ?? 1), [1, 1, 10, 12])
  const charts = await convert('docx/real/chartex.docx')
  equal(charts.plan.segments.filter((s) => s.kind === 'image').length, 2, "chartex charts use Word's own picture")
  equal(charts.report.notes.get('docx_chart')?.count, 4, 'charts without a picture reported')
  const pictures = await convert('docx/real/VariousPictures.docx')
  equal(pictures.report.notes.get('docx_unsupported_image')?.count, 3)
})

Deno.test('rasterize (§5.3): EMF / WMF pictures and charts go to the host as one-object docx', async () => {
  const requests: RasterRequest[] = []
  const png = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, 0x49, 0x48, 0x44, 0x52, 0, 0, 0, 10, 0, 0, 0, 5])
  const rasterize = (r: RasterRequest) => { requests.push(r); return Promise.resolve({ bytes: png, mediaType: 'image/png' }) }
  const { plan, report } = await convert('docx/real/VariousPictures.docx', { rasterize })
  equal(requests.map((r) => r.kind), ['image', 'image', 'image'])
  equal(plan.segments.filter((s) => s.kind === 'image').length, 5)
  assert(!report.notes.has('docx_unsupported_image') && report.notes.get('docx_rasterized')?.count === 3, 'reported as rasterized')
  const sub = await openPackage(requests[0].subdoc)
  equal(sub.kind, 'docx')
  assert(sub.paths().some((p) => /\.(emf|wmf)$/.test(p)), 'the picture is in the sub-document')
  assert(!sub.paths().some((p) => /\.(jpeg|png)$/.test(p)), 'other pictures are not')
  const charts: RasterRequest[] = []
  await convert('docx/real/chartex.docx', { rasterize: (r) => { charts.push(r); return Promise.resolve({ bytes: png, mediaType: 'image/png' }) } })
  equal(charts.length, 4)
  const chartPkg = await openPackage(charts[0].subdoc)
  assert(chartPkg.paths().some((p) => /charts\/chart\d+\.xml$/.test(p)), 'chart part copied')
  const bin = await soffice()
  if (bin) {
    // LibreOffice opens the sub-documents (the host turns them into pictures through nfs_server's preview-document)
    for (const r of [requests[0], charts[0]]) assert((await toPdf(bin, r.subdoc, 'docx'))?.length, `soffice converts the ${r.kind} sub-document`)
  }
})

Deno.test('splitDoc: Blocks under the limits, at a heading 1 when half full', () => {
  const p = (i: number): AstNode => ({ type: 'paragraph', attrs: { block_id: `p${i}` }, content: [{ type: 'text', text: 'x'.repeat(10) }] })
  const h1 = (i: number): AstNode => ({ type: 'heading', attrs: { block_id: `h${i}`, level: 1 }, content: [{ type: 'text', text: 'H' }] })
  const parts = splitDoc([p(1), p(2), p(3), h1(1), p(4), p(5), p(6), p(7), p(8)], { blocks: 5, text: 1000 })
  equal(parts.map((x) => x.length), [3, 5, 1])
  equal(splitDoc([p(1), p(2), p(3)], { blocks: 2, text: 1000 }).map((x) => x.length), [2, 1])
})

Deno.test('operations: data in the content folder, Cells on the page in order, the original last', async () => {
  const { plan } = await convert('docx/basic.docx')
  const assets = new Map(plan.images.map((i) => [i.key, { object_id: `obj-${i.key}`, file_name: i.name }]))
  const built = buildDocOperations(plan, { surfaceId: 'sf', contentFolderId: 'sf-content', afterKey: null, contentAfterKey: null, assets, original: { object_id: 'orig', file_name: 'basic.docx' }, newId: counter() })
  const cells = built.ops.filter((o) => o.type_id === 'buckyos.cell')
  equal(cells.map((c) => (c.payload.view as { type: string }).type), ['richtext', 'asset', 'richtext', 'asset'])
  assert(cells.every((c) => c.parent_id === 'sf' && c.placement === undefined), 'flow page Cells without placement')
  const keys = cells.map((c) => c.order_key)
  equal([...keys].sort(), keys, 'order keys increase')
  assert(built.ops.filter((o) => o.type_id !== 'buckyos.cell').every((o) => o.parent_id === 'sf-content'), 'data in the content folder')
  equal(built.blocks.length, 4)
  const missing = buildDocOperations(plan, { surfaceId: 'sf', contentFolderId: 'c', afterKey: null, contentAfterKey: null, assets: new Map(), newId: counter() })
  equal(missing.missing, 1)
})
