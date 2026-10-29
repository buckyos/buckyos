/* Extraction against the real pdf.js (the Desktop's pdfjs-dist; skipped when it is not installed): the op codes
 * the converter reads are pdf.js's, and the recorded fixture pages are what extraction gives today (re-record with
 * `deno run -A tools/record.ts ../fixtures/pdf/<name>.pdf ../fixtures/pdf/<name>.pages.json`). */

import { DRAW, OPS } from './data.ts'
import { extractPage, readBookmarks } from './extract.ts'
import { available, openPdf, pdfjs } from './tools/pdfjs.ts'

function assertEquals(actual: unknown, expected: unknown, message = '') {
  const a = JSON.stringify(actual), e = JSON.stringify(expected)
  if (a !== e) throw new Error(`${message ? `${message}: ` : ''}expected ${e.slice(0, 2000)}\n     got ${a.slice(0, 2000)}`)
}

const FIXTURES = new URL('../fixtures/pdf/', import.meta.url)
const ignore = !(await available())

Deno.test({ name: 'op codes are pdf.js 6.4’s', ignore, fn: async () => {
  const lib = await pdfjs()
  for (const [name, code] of Object.entries(OPS)) assertEquals(lib.OPS[name], code, name)
  const pdf = await Deno.readTextFile(new URL('legacy/build/pdf.mjs', new URL('../../desktop/node_modules/pdfjs-dist/', import.meta.url)))
  const draw = /const DrawOPS = \{([^}]*)\}/.exec(pdf)![1]
  for (const [name, code] of Object.entries(DRAW)) assertEquals(new RegExp(`${name}: (\\d+)`).exec(draw)?.[1], String(code), name)
} })

for (const name of ['slides', 'doc', 'latex', 'scan']) {
  Deno.test({ name: `recorded pages are current: ${name}`, ignore, sanitizeOps: false, sanitizeResources: false, fn: async () => {
    const { doc, oc } = await openPdf(await Deno.readFile(new URL(`${name}.pdf`, FIXTURES)))
    const bookmarks = await readBookmarks(doc)
    const pages = []
    for (let n = 1; n <= doc.numPages; n++) pages.push((await extractPage(doc, await doc.getPage(n), bookmarks, oc)).page)
    const recorded = JSON.parse(await Deno.readTextFile(new URL(`${name}.pages.json`, FIXTURES))).pages
    // font ids carry the number of the document in this process (`g_d3_f1`)
    const same = (v: unknown) => JSON.parse(JSON.stringify(v).replace(/g_d\d+_/g, 'g_d0_'))
    assertEquals(same(pages), same(recorded))
  } })
}
