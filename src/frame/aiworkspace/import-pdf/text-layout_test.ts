import { OPS, type PdfPage, type PdfTextItem } from './data.ts'
import { groupText, readStruct } from './text.ts'
import type { Mat } from './interpret.ts'
import { readPdfTextLayout } from './text-layout.ts'

const assert = (value: unknown, message = 'assertion failed') => { if (!value) throw new Error(message) }
const near = (actual: number, expected: number) => assert(Math.abs(actual - expected) < 1e-8, `${actual} != ${expected}`)
const item = (str: string, x: number, width: number, extra: Partial<PdfTextItem> = {}): PdfTextItem => ({ str, transform: [12, 0, 0, 12, x, 30], width, height: 12, fontName: 'f', hasEOL: false, mc: null, ...extra })
const page = (text: PdfTextItem[]): PdfPage => ({ number: 1, width: 240, height: 240, transform: [1, 0, 0, 1, 0, 0], fnArray: [], argsArray: [], text, fonts: {}, patterns: {}, images: {}, struct: null, links: [], bookmark: null })
const paragraphs = (text: PdfTextItem[], world: Mat = [1, 0, 0, 1, 0, 0]) => groupText(page(text), world, [], readStruct(null), { unit: 'line' }).paragraphs

for (const gap of [1.7, 2, 10]) {
  Deno.test(`fragment geometry retains a ${gap} unit gap independently of inferred spaces`, () => {
    const [p] = paragraphs([item('hello', 10, 25), item('world', 35 + gap, 29)])
    assert(p.layout.spans.length === 2)
    const [a, b] = p.layout.spans
    near(b.x - (a.x + a.width), gap)
    near(a.width, 25)
    near(b.width, 29)
    near(a.y, b.y)
  })
}

Deno.test('horizontal compression retains the source advance at the original font size', () => {
  const [p] = paragraphs([item('MMMMMMMM', 10, 36, { transform: [6, 0, 0, 12, 10, 30] })])
  near(p.layout.spans[0].width, 36)
  near(p.layout.spans[0].size, 12)
})

Deno.test('rotated text retains local fragment positions and widths', () => {
  const input = [item('hello', 10, 25), item('world', 45, 29)]
  const [normal] = paragraphs(input)
  const [rotated] = paragraphs(input, [0, 1, -1, 0, 0, 0])
  near(rotated.rotation, 90)
  for (let i = 0; i < normal.layout.spans.length; i++) {
    near(rotated.layout.spans[i].x, normal.layout.spans[i].x)
    near(rotated.layout.spans[i].y, normal.layout.spans[i].y)
    near(rotated.layout.spans[i].width, normal.layout.spans[i].width)
  }
})

for (const empty of [false, true]) {
  Deno.test(`explicit line endings survive neighbouring-baseline merging (${empty ? 'empty marker' : 'text item'})`, () => {
    const first = item('LEFT', 10, 20, { hasEOL: !empty })
    const end = item('', 30, 0, { hasEOL: true })
    const p = paragraphs([first, ...(empty ? [end] : []), item('RIGHT', 40, 30)])
    assert(p.length === 2)
    assert(p[0].text === 'LEFT' && p[1].text === 'RIGHT')
  })
}

Deno.test('source layout is used only while text and layout properties still match', () => {
  const [p] = paragraphs([item('hello', 10, 25)])
  const config = { pdf_source: { kind: 'text', text_layout: p.layout } }
  const look = { text: p.text, fontSize: p.fontSize, align: p.align, lineHeight: p.lineHeight, wrap: false, valign: 'top', labelPosition: 'inside', labelFill: null, fontColor: p.style.color, bold: p.style.bold, italic: p.style.italic, fontFamily: p.style.family, opacity: p.style.alpha }
  assert(readPdfTextLayout(config, look) !== null)
  for (const patch of [{ text: 'edited' }, { fontSize: 14 }, { align: 'center' }, { lineHeight: 2 }, { wrap: true }, { valign: 'bottom' }, { labelPosition: 'top' }, { labelFill: '#ffffff' }, { fontColor: '#ff0000' }, { bold: true }, { italic: true }, { fontFamily: 'mono' }, { opacity: 0.5 }]) assert(readPdfTextLayout(config, { ...look, ...patch }) === null)
  assert(readPdfTextLayout({}, look) === null)
})

Deno.test('invalid and oversized source layouts fall back to normal text', () => {
  const [p] = paragraphs([item('hello', 10, 25)])
  const look = { text: p.text, fontSize: p.fontSize, align: p.align, lineHeight: p.lineHeight, wrap: false, valign: 'top', labelPosition: 'inside', labelFill: null, fontColor: p.style.color, bold: p.style.bold, italic: p.style.italic, fontFamily: p.style.family, opacity: p.style.alpha }
  const span = p.layout.spans[0]
  for (const patch of [{ width: 0 }, { height: Infinity }, { spans: [] }, { spans: Array(4097).fill(span) }, { spans: [{ ...span, x: NaN }] }, { spans: [{ ...span, width: -1 }] }, { spans: [{ ...span, text: null }] }, { spans: [{ ...span, text: 'different' }] }]) {
    assert(readPdfTextLayout({ pdf_source: { kind: 'text', text_layout: { ...p.layout, ...patch } } }, look) === null)
  }
})

Deno.test('span styles survive validation and reject malformed paints and alpha', () => {
  const [p] = paragraphs([item('hello', 10, 25)])
  const look = { text: p.text, fontSize: p.fontSize, align: p.align, lineHeight: p.lineHeight, wrap: false, valign: 'top', labelPosition: 'inside', labelFill: null, fontColor: p.style.color, bold: p.style.bold, italic: p.style.italic, fontFamily: p.style.family, opacity: p.style.alpha }
  const style = { color: '#a01020', alpha: 0.4, bold: true, italic: true, family: 'mono' }
  const config = { pdf_source: { kind: 'text', text_layout: { ...p.layout, spans: p.layout.spans.map(span => ({ ...span, style })) } } }
  const layout = readPdfTextLayout(config, look)
  assert(JSON.stringify(layout?.spans[0].style) === JSON.stringify(style))
  assert(readPdfTextLayout({ ...config, pdf_source: { ...config.pdf_source, text_layout: { ...config.pdf_source.text_layout, style: undefined } } }, look) === null)
  for (const patch of [{ color: 'url(https://invalid)' }, { alpha: -1 }, { alpha: Infinity }, { bold: 'true' }, { italic: 1 }, { family: 'arbitrary' }]) {
    const invalid = { ...config, pdf_source: { ...config.pdf_source, text_layout: { ...config.pdf_source.text_layout, spans: [{ ...p.layout.spans[0], style: { ...style, ...patch } }] } } }
    assert(readPdfTextLayout(invalid, look) === null)
  }
})

Deno.test('a showText containing readable and private glyphs stays atomic in the raster', () => {
  const data = page([item('ABC', 10, 18), item('\uE123', 28, 6)])
  data.fnArray = [OPS.showText]
  data.argsArray = [[[[1500, 0, 'ABC'], [500, 0, '\uE123']]]]
  const result = groupText(data, [1, 0, 0, 1, 0, 0], [{ index: 0, start: { x: 10, y: 30 }, end: { x: 34, y: 30 }, size: 12, font: 'f', fill: { kind: 'color', color: '#000000' }, alpha: 1, mode: 0, mcid: null, complex: null, blank: false }], readStruct(null), { unit: 'paragraph' })
  assert(result.unreadable === 1 && result.total === 4)
  assert(result.converted.size === 0 && result.paragraphs.length === 0)
})
