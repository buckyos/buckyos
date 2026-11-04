import { fidelityPdf } from '../fixtures/pdf/src/fidelity.ts'
import { OPS, type PdfPage } from './data.ts'
import { extractPage } from './extract.ts'
import { groupText, readStruct } from './text.ts'
import { interpret } from './interpret.ts'
import { worldOf } from './convert.ts'
import { available, openPdf } from './tools/pdfjs.ts'

const fixture = new URL('../fixtures/pdf/synthetic-pdf-import.pdf', import.meta.url)
const manifest = JSON.parse(await Deno.readTextFile(new URL('../fixtures/pdf/fidelity.manifest.json', import.meta.url)))
const assert = (condition: unknown, message: string) => { if (!condition) throw new Error(message) }
const sha256 = async (bytes: Uint8Array) => Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', new Uint8Array(bytes)))).map(n => n.toString(16).padStart(2, '0')).join('')

Deno.test('synthetic PDF is deterministic and contains only declared fixture resources', async () => {
  const bytes = await Deno.readFile(fixture)
  assert(await sha256(bytes) === await sha256(fidelityPdf()), 'Regeneration must reproduce the committed fixture')
  assert(await sha256(bytes) === manifest.sha256, 'Manifest digest must match')
  const pdf = new TextDecoder('latin1').decode(bytes)
  assert(pdf.includes('/CCITTFaxDecode'), 'Compressed image must exercise the CCITT decoder')
  assert(pdf.includes('<41> <E123>'), 'Custom glyph mapping must be present')
  assert(!/\/(?:FontFile[23]?|EmbeddedFiles|Metadata|Annots|JavaScript|URI)\b/.test(pdf), 'No embedded source fonts, attachments, metadata, links or scripts')
  assert([...pdf.matchAll(/\/BaseFont \/([^ /<>]+)/g)].map(m => m[1]).sort().join(',') === 'Times-Bold,Times-Italic,Times-Roman', 'Only declared PDF standard fonts')
})

Deno.test({ name: 'synthetic PDF extracts its four intended regression triggers', ignore: !(await available()), sanitizeOps: false, sanitizeResources: false, fn: async () => {
  const { doc } = await openPdf(await Deno.readFile(fixture))
  try {
    assert(doc.numPages === 4, 'Exactly four synthetic pages')
    const pages: PdfPage[] = []
    for (let n = 1; n <= 4; n++) pages.push((await extractPage(doc, await doc.getPage(n), new Map(), null)).page)
    const text = pages.flatMap(p => p.text).map(t => t.str).join(' ')
    assert(text.includes('ROW 36 - SAMPLE TEXT'), 'Repeated text operators must survive extraction')
    assert(pages[0].fnArray.filter(op => op === OPS.setTextMatrix).length === 36, 'Repeated text matrix blocks must remain in the source')
    assert(pages[0].fnArray.filter(op => op === OPS.transform).length >= 12, 'Transformed path blocks must exercise the path optimizer')
    assert(text.includes('\uE123'), 'Private-use glyph must survive source extraction')
    assert(pages[1].images && Object.values(pages[1].images).some(image => image.width === 256 && image.height === 192), 'CCITT image dimensions')
    assert(Object.values(pages[2].fonts).some(font => font.type3), 'Handwritten Type3 font must be used')
    const privateWorld = worldOf(pages[2]).world
    const privateMarks = interpret(pages[2], privateWorld)
    const privateText = groupText(pages[2], privateWorld, privateMarks.runs, readStruct(pages[2].struct), { unit: 'paragraph' })
    const privateRun = privateMarks.runs.find(run => JSON.stringify(pages[2].argsArray[run.index]).includes('\uE123'))
    assert(privateRun && !privateText.converted.has(privateRun.index), 'Private-use glyph operator must remain in the raster')
    assert(privateText.paragraphs.some(p => p.text.includes('ALPHA')), 'Neighbouring readable text remains editable')
    const { world } = worldOf(pages[3])
    const marks = interpret(pages[3], world)
    const result = groupText(pages[3], world, marks.runs, readStruct(pages[3].struct), { unit: 'paragraph' })
    assert(result.paragraphs.some(p => p.mixed), 'Mixed styles must remain a regression trigger')
    assert(result.paragraphs.some(p => p.layout.spans.some(span => span.style?.color === '#d90d0d' && span.style.bold && !span.style.italic)), 'Red bold fragment retains its own style')
  } finally { await (doc as unknown as { loadingTask: { destroy(): Promise<void> } }).loadingTask.destroy() }
} })
