/* Flow pagination and the page model (打印 TODO T1, T2). Run: `deno test -A` in this folder. */

import { assert, assertEquals } from './test_assert.ts'
import { paginate, type FlowItem } from './paginate.ts'
import { contentArea, defaultPaper, fillPageContext, fitStep, frameFits, paperArgument, paperOf, pdfFileName } from './page.ts'
import { cssStack, missingFonts, readFontSettings } from './fonts.ts'

const spans = (r: ReturnType<typeof paginate>) => r.pages.map((p) => p.fragments.map((f) => `${f.id}:${f.from}-${f.to}${f.header ? `+h${f.header}` : ''}${f.scale ? `*${f.scale.toFixed(2)}` : ''}@${f.top}`))

Deno.test('blocks that fit stay together, the next page starts where one does not', () => {
  const items: FlowItem[] = [{ id: 'a', height: 40 }, { id: 'b', height: 40 }, { id: 'c', height: 40 }]
  assertEquals(spans(paginate(items, 100)), [['a:0-40@0', 'b:0-40@40'], ['c:0-40@0']])
})

Deno.test('gaps are dropped at a page end', () => {
  const items: FlowItem[] = [{ id: 'a', height: 45, gapAfter: 10 }, { id: 'b', height: 45, gapAfter: 10 }, { id: 'c', height: 10 }]
  assertEquals(spans(paginate(items, 100)), [['a:0-45@0', 'b:0-45@55'], ['c:0-10@0']])
})

Deno.test('an unbreakable block moves to the next page whole', () => {
  const items: FlowItem[] = [{ id: 'a', height: 70 }, { id: 'img', height: 50 }]
  assertEquals(spans(paginate(items, 100)), [['a:0-70@0'], ['img:0-50@0']])
})

Deno.test('a breakable block is split at its last legal break that fits', () => {
  const items: FlowItem[] = [{ id: 'a', height: 30 }, { id: 'text', height: 200, breaks: [20, 40, 60, 80, 100, 120, 140, 160, 180] }]
  assertEquals(spans(paginate(items, 100)), [['a:0-30@0', 'text:0-60@30'], ['text:60-160@0'], ['text:160-200@0']])
})

Deno.test('a page never ends inside a block where it declares no break', () => {
  const r = paginate([{ id: 'a', height: 50 }, { id: 't', height: 120, breaks: [90] }], 100)
  // 90 does not fit after a (50 + 90 > 100): t starts on page two, ends there at 90, the rest on page three
  assertEquals(spans(r), [['a:0-50@0'], ['t:0-90@0'], ['t:90-120@0']])
})

Deno.test('a table repeats its header on every page it continues on', () => {
  const rows = Array.from({ length: 12 }, (_, i) => 20 + i * 20) // header 20, 12 rows of 20
  const r = paginate([{ id: 'tbl', height: 260, breaks: rows.slice(0, -1), header: 20 }], 100)
  assertEquals(spans(r), [['tbl:0-100@0'], ['tbl:100-180+h20@0'], ['tbl:180-260+h20@0']])
})

Deno.test('a heading is kept with what follows it', () => {
  const items: FlowItem[] = [{ id: 'p', height: 80 }, { id: 'h', height: 12, keepWithNext: true }, { id: 'img', height: 40 }]
  assertEquals(spans(paginate(items, 100)), [['p:0-80@0'], ['h:0-12@0', 'img:0-40@12']])
})

Deno.test('what cannot be split and is taller than a page is scaled to one page and reported', () => {
  const r = paginate([{ id: 'a', height: 10 }, { id: 'huge', height: 400 }, { id: 'b', height: 10 }], 100)
  assertEquals(spans(r), [['a:0-10@0'], ['huge:0-400*0.25@0'], ['b:0-10@0']])
  assertEquals(r.issues, [{ kind: 'shrunk', id: 'huge', scale: 0.25 }])
})

Deno.test('a part between two breaks taller than a page is scaled, the rest still splits', () => {
  const r = paginate([{ id: 't', height: 300, breaks: [20, 250, 280] }], 100)
  assertEquals(spans(r), [['t:0-20@0'], ['t:20-250*0.43@0'], ['t:250-300@0']])
  assertEquals(r.issues.length, 1)
})

Deno.test('nothing to print is one empty page', () => {
  assertEquals(paginate([], 100).pages, [{ fragments: [] }])
})

Deno.test('paper, orientation and the argument media.page takes', () => {
  assertEquals(paperOf('A4', 'landscape'), { name: 'A4', width_mm: 297, height_mm: 210 })
  assertEquals(paperOf('16:9', 'landscape'), { name: '16:9', width_mm: 297, height_mm: 167.06 })
  assertEquals(paperArgument(paperOf('A5', 'portrait')), '148x210mm')
  assertEquals(defaultPaper('flow'), { name: 'A4', width_mm: 210, height_mm: 297 })
  assertEquals(defaultPaper('free', { w: 1600, h: 900 }).name, '16:9')
  assertEquals(defaultPaper('free', { w: 1024, h: 768 }).name, '4:3')
})

Deno.test('a PDF Frame fits pages of its shape: A3, A4 and A5 share one', () => {
  const a = { w: 595, h: 842 }
  assert(frameFits(a, paperOf('A4', 'portrait')))
  assert(frameFits(a, paperOf('A5', 'portrait')))
  assert(frameFits(a, paperOf('A3', 'portrait')))
  assert(!frameFits(a, paperOf('A4', 'landscape')))
  assert(!frameFits(a, paperOf('Letter', 'portrait')))
  assert(frameFits({ w: 1600, h: 900 }, paperOf('16:9', 'landscape')))
})

Deno.test('content area: margins, the whole page, a PDF Frame scaled to the page', () => {
  const a4 = paperOf('A4', 'portrait')
  assertEquals(contentArea(a4, 'flow'), { x: 20, y: 20, w: 170, h: 257 })
  assertEquals(contentArea(a4, 'free'), { x: 0, y: 0, w: 210, h: 297 })
  const frame = { w: 420, h: 594, content: { x: 40, y: 60, w: 340, h: 474 } }
  assertEquals(contentArea(a4, 'flow', frame), { x: 20, y: 30, w: 170, h: 237 })
  const a5 = contentArea(paperOf('A5', 'portrait'), 'flow', frame)
  assert(Math.abs(a5.w - 340 * (148 / 420)) < 1e-9)
})

Deno.test('a step is fitted into the content area, centred, never cropped', () => {
  assertEquals(fitStep({ w: 1600, h: 900 }, { x: 0, y: 0, w: 297, h: 210 }), { scale: 297 / 1600, x: 0, y: (210 - 900 * 297 / 1600) / 2, w: 297, h: 900 * 297 / 1600 })
  const tall = fitStep({ w: 100, h: 400 }, { x: 10, y: 10, w: 200, h: 200 })
  assertEquals([tall.scale, tall.w, tall.h, tall.x, tall.y], [0.5, 50, 200, 85, 10])
})

Deno.test('page context and file names', () => {
  assertEquals(fillPageContext('{title} · 第 {page} / {pages} 页 · {date} {other}', { page: 3, pages: 9, title: '周报', date: '2026-11-16' }), '周报 · 第 3 / 9 页 · 2026-11-16 {other}')
  assertEquals(pdfFileName('a/b: c'), 'a-b- c.pdf')
  assertEquals(pdfFileName(''), 'canvas.pdf')
})

Deno.test('fonts: settings, missing ones, CSS stacks', () => {
  const s = readFontSettings({ sans: 'Noto Sans Thai', serif: 'Noto Serif CJK SC', mono: 7, extra: 'x' })
  assertEquals(s, { sans: 'Noto Sans Thai', serif: 'Noto Serif CJK SC' })
  assertEquals(missingFonts(s, new Set()), [{ generic: 'sans', family: 'Noto Sans Thai', item: 'noto-sans-thai' }])
  assertEquals(missingFonts(s, new Set(['noto-sans-thai'])), [])
  assertEquals(missingFonts({ mono: 'Comic Mono' }, new Set()), [{ generic: 'mono', family: 'Comic Mono', item: null }])
  assertEquals(cssStack(s, 'sans'), '"Noto Sans Thai", "Noto Sans CJK SC", sans-serif')
  assertEquals(cssStack({}, 'mono'), '"DejaVu Sans Mono", monospace')
})
