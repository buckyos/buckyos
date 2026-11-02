/* Package, theme and rich text checks of the Office importer (Office 导入 TODO O1). Run: `deno test -A`. */

import { DEFAULT_THEME, LIMITS, OfficeError, colorOf, officeKind, openPackage, readTheme, validateDoc, type AstNode } from './mod.ts'
import { crc32, writeZip } from './subdoc.ts'
import { assert, equal, fixture, here } from './test_util.ts'

Deno.test('officeKind by bytes and name', async () => {
  equal(officeKind('a.docx', await fixture('docx/basic.docx')), 'docx')
  equal(officeKind('noext', await fixture('docx/basic.lo.docx')), 'docx')
  equal(officeKind('a.pptx', await fixture('pptx/basic.pptx')), 'pptx')
  equal(officeKind('x', await fixture('pptx/basic.odp')), 'odp')
  equal(officeKind('x', await fixture('docx/basic.odt')), 'odt')
  equal(officeKind('a.rtf', new TextEncoder().encode('{\\rtf1\\ansi hello}')), 'rtf')
  const ole = new Uint8Array(64); ole.set([0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1])
  equal(officeKind('old.doc', ole), 'doc')
  equal(officeKind('old.ppt', ole), 'ppt')
  equal(officeKind('book.xls', ole), null)
  equal(officeKind('secret.docx', ole), 'docx')
  equal(officeKind('book.xlsx', await Deno.readFile(`${here}../fixtures/table/xlsx/workbook.xlsx`)), null)
  equal(officeKind('a.zip', writeZip([{ name: 'readme.txt', bytes: new Uint8Array([65]) }])), null)
})

Deno.test('openPackage: main part, relationships, content types', async () => {
  const pkg = await openPackage(await fixture('docx/basic.docx'))
  equal(pkg.kind, 'docx')
  equal(pkg.main, 'word/document.xml')
  const rels = [...(await pkg.rels(pkg.main)).values()]
  assert(rels.some((r) => r.type === 'styles' && r.target === 'word/styles.xml'), 'styles relationship')
  assert(rels.some((r) => r.type === 'hyperlink' && r.external && r.target === 'https://example.com/report'), 'external hyperlink')
  const image = rels.find((r) => r.type === 'image')
  assert(image && image.target.startsWith('word/media/'), 'image resolved against word/')
  equal(pkg.contentType(image.target), 'image/png')
  const deck = await openPackage(await fixture('pptx/basic.pptx'))
  equal(deck.kind, 'pptx')
  const layoutRel = (await deck.relOfType('ppt/slides/slide1.xml', 'slideLayout'))!
  assert(/^ppt\/slideLayouts\/slideLayout\d+\.xml$/.test(layoutRel.target), `../ resolved: ${layoutRel.target}`)
})

Deno.test('openPackage refuses what is not a readable package', async () => {
  const ole = new Uint8Array(64); ole.set([0xd0, 0xcf, 0x11, 0xe0])
  for (const [bytes, code] of [[ole, 'encrypted'], [new Uint8Array([1, 2, 3, 4, 5, 6]), 'not_office'], [writeZip([{ name: 'a.txt', bytes: new Uint8Array(1) }]), 'not_office']] as const) {
    let got = ''
    try { await openPackage(bytes) } catch (error) { got = error instanceof OfficeError ? error.code : String(error) }
    equal(got, code)
  }
})

Deno.test('zip writer: CRC and a round trip through the reader', async () => {
  equal(crc32(new TextEncoder().encode('123456789')).toString(16), 'cbf43926')
  const zip = writeZip([{ name: '[Content_Types].xml', bytes: new TextEncoder().encode('<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>') }, { name: '_rels/.rels', bytes: new TextEncoder().encode('<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>') }, { name: 'word/document.xml', bytes: new TextEncoder().encode('<w:document xmlns:w="w"><w:body/></w:document>') }])
  const pkg = await openPackage(zip)
  equal(pkg.main, 'word/document.xml')
  equal((await pkg.xml('word/document.xml'))?.name, 'w:document')
})

// Office's own values for the theme colour picker of the 2013–2022 theme (accent 1 = #4472C4)
Deno.test('theme colours: lumMod / lumOff as Office shows them', () => {
  const ctx = { theme: DEFAULT_THEME, map: { bg1: 'lt1', tx1: 'dk1' } }
  const c = (mods: string) => colorOf({ name: 'a:schemeClr', attrs: { val: 'accent1' }, children: mods ? mods.split(';').map((m) => { const [n, v] = m.split('='); return { name: `a:${n}`, attrs: { val: v }, children: [] } }) : [] }, ctx)?.hex
  equal(c(''), '#4472c4')
  equal(c('lumMod=60000;lumOff=40000'), '#8faadc')
  equal(c('lumMod=20000;lumOff=80000'), '#dae3f3')
  equal(c('lumMod=75000'), '#2f5597')
  equal(c('lumMod=50000'), '#203864')
  equal(colorOf({ name: 'a:schemeClr', attrs: { val: 'tx1' }, children: [] }, ctx)?.hex, '#000000')
  const alpha = colorOf({ name: 'a:srgbClr', attrs: { val: 'FF0000' }, children: [{ name: 'a:alpha', attrs: { val: '50000' }, children: [] }] }, ctx)
  equal(alpha, { hex: '#ff0000', alpha: 0.5 })
})

Deno.test('readTheme: colours and fonts of a theme part', async () => {
  const pkg = await openPackage(await fixture('pptx/real/with_japanese.pptx'))
  const theme = readTheme(await pkg.xml('ppt/theme/theme1.xml'))
  assert(/^#[0-9a-f]{6}$/.test(theme.colors.get('accent1') ?? ''), 'accent1')
  assert(theme.minor.latin.length > 0, 'minor latin face')
  assert(theme.minor.ea.length > 0, 'an East Asian face from the script fonts')
})

Deno.test('rich text: limits match the schema, validateDoc catches what the core refuses', async () => {
  const schema = JSON.parse(await Deno.readTextFile(`${here}../schemas/richtext.basic.v1.json`))
  equal(schema.limits.max_blocks, LIMITS.max_blocks)
  equal(schema.limits.max_text_utf16, LIMITS.max_text_utf16)
  equal(schema.limits.max_depth, LIMITS.max_depth)
  equal(schema.nodes.heading.attrs.level.max, 3)
  const p = (id: string, text?: string): AstNode => ({ type: 'paragraph', attrs: { block_id: id }, ...(text ? { content: [{ type: 'text', text }] } : {}) })
  equal(validateDoc({ type: 'doc', content: [p('a', 'x')] }), [])
  assert(validateDoc({ type: 'doc', content: [p('a'), p('a')] }).some((m) => m.includes('duplicate')), 'duplicate ids')
  assert(validateDoc({ type: 'doc', content: [{ type: 'heading', attrs: { block_id: 'h', level: 4 } }] }).some((m) => m.includes('level')), 'heading level 4')
  assert(validateDoc({ type: 'doc', content: [{ type: 'bullet_list', attrs: { block_id: 'l' }, content: [{ type: 'list_item', attrs: { block_id: 'i' }, content: [{ type: 'bullet_list', attrs: { block_id: 'x' }, content: [] }] }] }] }).length > 0, 'item must start with a paragraph')
  assert(validateDoc({ type: 'doc', content: [{ type: 'table', attrs: { block_id: 't' }, content: [{ type: 'table_row', content: [{ type: 'table_cell', content: [p('q', 'no')] }] }] }] }).length > 0, 'cells hold inline content')
  assert(validateDoc({ type: 'doc', content: [{ type: 'paragraph', attrs: { block_id: 'c' }, content: [{ type: 'text', text: 'x', marks: [{ type: 'code' }, { type: 'strong' }] }] }] }).length > 0, 'code is exclusive')
})
