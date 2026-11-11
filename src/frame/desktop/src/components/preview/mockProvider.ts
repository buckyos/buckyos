/**
 * Mock Preview provider (dev / e2e runtime).
 *
 * Serves the File Browser mock library (`app/filebrowser/mock/data.ts`) plus a
 * virtual `cyfs:///samples` gallery with generated, deterministic content for
 * every standard result family (PRD §9.2), and simulates the nfs_server
 * built-in Pipeline (§23.6): idempotent `ensure`, `processing → completed |
 * failed` records, retry attempts, negative caching and unsupported planning.
 *
 * Nothing here is a wire contract; the NFSP provider is the real one.
 */

import {
  mockEntriesAtPath,
  mockEntryById,
  mockEntryByPath,
} from '../../app/filebrowser/mock/data'
import type { FileEntry } from '../../app/filebrowser/types'
import { extensionOf, mediaTypeFromExtension } from './mediaTypes'
import { cyfsPathToLocal, normalizeCyfsPath, parentCyfsPath } from './session'
import {
  isBlobRef,
  isCyfsPathRef,
  isObjectIdRef,
  PreviewError,
  type ContentRef,
  type EnsurePreviewWorkRequest,
  type PreviewProvider,
  type PreviewRendererType,
  type PreviewResult,
  type PreviewSessionItemInput,
  type PreviewUnsupportedReason,
  type PreviewWorkState,
  type ResolvedPreviewSource,
} from './types'
import { asBucketObjectRef } from './bucketSource'

// ─── Deterministic helpers ───

function hash32(input: string): number {
  let h = 2166136261
  for (let i = 0; i < input.length; i += 1) {
    h ^= input.charCodeAt(i)
    h = Math.imul(h, 16777619)
  }
  return h >>> 0
}

function mulberry32(seed: number) {
  let a = seed
  return () => {
    a |= 0
    a = (a + 0x6d2b79f5) | 0
    let t = Math.imul(a ^ (a >>> 15), 1 | a)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

const delay = (ms: number) => new Promise<void>((resolve) => window.setTimeout(resolve, ms))

// ─── Content generators ───

async function makePng(label: string, seed: string, width = 1600, height = 1000): Promise<Blob> {
  const canvas = document.createElement('canvas')
  canvas.width = width
  canvas.height = height
  const ctx = canvas.getContext('2d')
  if (!ctx) throw new PreviewError('INTERNAL', 'Canvas unavailable')
  const rnd = mulberry32(hash32(seed))
  const hue = Math.floor(rnd() * 360)
  const gradient = ctx.createLinearGradient(0, 0, width, height)
  gradient.addColorStop(0, `hsl(${hue} 70% 62%)`)
  gradient.addColorStop(1, `hsl(${(hue + 70) % 360} 60% 32%)`)
  ctx.fillStyle = gradient
  ctx.fillRect(0, 0, width, height)
  for (let i = 0; i < 18; i += 1) {
    ctx.beginPath()
    ctx.fillStyle = `hsla(${(hue + rnd() * 120) | 0} 80% ${50 + rnd() * 40}% / ${0.12 + rnd() * 0.3})`
    ctx.arc(rnd() * width, rnd() * height, 40 + rnd() * 260, 0, Math.PI * 2)
    ctx.fill()
  }
  ctx.strokeStyle = 'rgba(255,255,255,0.35)'
  ctx.lineWidth = 2
  for (let x = 0; x < width; x += 200) {
    ctx.beginPath()
    ctx.moveTo(x, 0)
    ctx.lineTo(x, height)
    ctx.stroke()
  }
  ctx.fillStyle = 'rgba(0,0,0,0.35)'
  ctx.fillRect(0, height - 140, width, 140)
  ctx.fillStyle = '#fff'
  ctx.font = 'bold 54px system-ui, sans-serif'
  ctx.fillText(label, 48, height - 70)
  ctx.font = '26px system-ui, sans-serif'
  ctx.fillText(`${width} × ${height} · generated preview fixture`, 48, height - 28)
  return new Promise<Blob>((resolve, reject) => {
    canvas.toBlob((blob) => (blob ? resolve(blob) : reject(new PreviewError('INTERNAL', 'PNG encode failed'))), 'image/png')
  })
}

function makeSvg(label: string, seed: string): Blob {
  const rnd = mulberry32(hash32(seed))
  const hue = Math.floor(rnd() * 360)
  const shapes = Array.from({ length: 12 }, (_, i) => {
    const cx = (rnd() * 800) | 0
    const cy = (rnd() * 500) | 0
    const r = (30 + rnd() * 120) | 0
    return `<circle cx="${cx}" cy="${cy}" r="${r}" fill="hsl(${(hue + i * 25) % 360} 70% 55%)" opacity="0.6"/>`
  }).join('')
  const svg = `<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 800 500" width="800" height="500">
  <rect width="800" height="500" fill="hsl(${hue} 30% 18%)"/>
  ${shapes}
  <text x="40" y="460" font-family="system-ui, sans-serif" font-size="36" fill="white">${label.replace(/[<&]/g, '')}</text>
</svg>`
  return new Blob([svg], { type: 'image/svg+xml' })
}

function makeMarkdown(entry: { name: string; summary?: string; tags?: string[]; path?: string }): Blob {
  const lines = [
    `# ${entry.name.replace(/\.[^.]+$/, '')}`,
    '',
    entry.summary ?? 'Generated text fixture for the Preview Component.',
    '',
    '## Notes',
    '',
    '- Content-first: no toolbar until you move the mouse (Auto UI mode).',
    '- Press `Esc` to exit the preview, `←` / `→` to move within the session.',
    '- `Ctrl/Cmd + F` opens find; `Ctrl/Cmd + +/-` scales the text.',
    '',
    entry.tags?.length ? `Tags: ${entry.tags.map((t) => `#${t}`).join(' ')}` : '',
    '',
    '```ts',
    'const preview = <ContentPreview source={{ kind: "cyfs-path", path }} uiMode="auto" />',
    '```',
    '',
    ...Array.from({ length: 40 }, (_, i) => `Paragraph ${i + 1}. Lorem ipsum dolor sit amet, consectetur adipiscing elit — ${entry.path ?? ''}`),
  ]
  return new Blob([lines.join('\n')], { type: 'text/markdown' })
}

function makeHtml(title: string, note: string, body?: string): Blob {
  const html = `<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>${title}</title>
<style>
  body { font: 15px/1.6 system-ui, sans-serif; color: #222; margin: 0; padding: 32px; background: #fff; }
  h1 { font-size: 26px; margin: 0 0 6px; }
  .note { color: #777; font-size: 12px; margin-bottom: 24px; }
  table { border-collapse: collapse; width: 100%; }
  td, th { border: 1px solid #ddd; padding: 6px 10px; text-align: left; }
  th { background: #f4f4f6; }
  a { color: #3163c9; }
</style></head>
<body>
<h1>${title}</h1>
<div class="note">${note}</div>
${body ?? `<p>This HTML runs inside the Preview sandbox: scripts are disabled and no network requests are allowed.</p>
<p><a href="https://example.invalid">External links</a> are inert until the host decides how to open them.</p>
<script>document.body.innerHTML = 'SCRIPT EXECUTED — sandbox broken'</script>
<img src="https://example.invalid/tracker.png" alt="blocked remote image">
${Array.from({ length: 30 }, (_, i) => `<p>Section ${i + 1} — rich text paragraph with <strong>bold</strong> and <em>emphasis</em>.</p>`).join('\n')}`}
</body></html>`
  return new Blob([html], { type: 'text/html' })
}

const XLSX_TYPE = 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet'

function crc32(bytes: Uint8Array): number {
  let crc = ~0
  for (const byte of bytes) {
    crc ^= byte
    for (let k = 0; k < 8; k += 1) crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1))
  }
  return ~crc >>> 0
}

/** A zip of stored (uncompressed) entries. */
function makeZip(files: [string, string][]): Uint8Array {
  const encoder = new TextEncoder()
  const parts: Uint8Array[] = []
  const directory: Uint8Array[] = []
  let offset = 0
  for (const [name, text] of files) {
    const nameBytes = encoder.encode(name)
    const data = encoder.encode(text)
    const header = new DataView(new ArrayBuffer(30))
    header.setUint32(0, 0x04034b50, true)
    header.setUint16(4, 20, true)
    header.setUint32(14, crc32(data), true)
    header.setUint32(18, data.length, true)
    header.setUint32(22, data.length, true)
    header.setUint16(26, nameBytes.length, true)
    const entry = new DataView(new ArrayBuffer(46))
    entry.setUint32(0, 0x02014b50, true)
    entry.setUint16(4, 20, true)
    entry.setUint16(6, 20, true)
    entry.setUint32(16, crc32(data), true)
    entry.setUint32(20, data.length, true)
    entry.setUint32(24, data.length, true)
    entry.setUint16(28, nameBytes.length, true)
    entry.setUint32(42, offset, true)
    parts.push(new Uint8Array(header.buffer), nameBytes, data)
    directory.push(new Uint8Array(entry.buffer), nameBytes)
    offset += 30 + nameBytes.length + data.length
  }
  const directorySize = directory.reduce((n, part) => n + part.length, 0)
  const end = new DataView(new ArrayBuffer(22))
  end.setUint32(0, 0x06054b50, true)
  end.setUint16(8, files.length, true)
  end.setUint16(10, files.length, true)
  end.setUint32(12, directorySize, true)
  end.setUint32(16, offset, true)
  const out = new Uint8Array(offset + directorySize + 22)
  let at = 0
  for (const part of [...parts, ...directory, new Uint8Array(end.buffer)]) {
    out.set(part, at)
    at += part.length
  }
  return out
}

type MockCell = string | number | { value: number; style: 1 | 2 }
interface MockSheet {
  name: string
  rows: MockCell[][]
  merges?: string[]
  hidden?: boolean
}

const xmlEscape = (text: string) => text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')

function columnName(index: number): string {
  let n = index + 1
  let out = ''
  while (n > 0) {
    out = String.fromCharCode(65 + ((n - 1) % 26)) + out
    n = Math.floor((n - 1) / 26)
  }
  return out
}

/** A real workbook: text, numbers, dates (style 1) and percentages (style 2), merged ranges, hidden sheets. */
export function makeMockXlsx(sheets: MockSheet[]): Blob {
  const sheetXml = (sheet: MockSheet) => {
    const rows = sheet.rows.map((row, r) => {
      const cells = row.map((value, c) => {
        const ref = `${columnName(c)}${r + 1}`
        if (typeof value === 'string') return value ? `<c r="${ref}" t="inlineStr"><is><t>${xmlEscape(value)}</t></is></c>` : ''
        if (typeof value === 'number') return `<c r="${ref}"><v>${value}</v></c>`
        return `<c r="${ref}" s="${value.style}"><v>${value.value}</v></c>`
      })
      return `<row r="${r + 1}">${cells.join('')}</row>`
    })
    const merges = sheet.merges?.length ? `<mergeCells count="${sheet.merges.length}">${sheet.merges.map((ref) => `<mergeCell ref="${ref}"/>`).join('')}</mergeCells>` : ''
    return `<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>${rows.join('')}</sheetData>${merges}</worksheet>`
  }
  const main = 'http://schemas.openxmlformats.org/spreadsheetml/2006/main'
  const rels = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships'
  const files: [string, string][] = [
    ['[Content_Types].xml', `<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>${sheets.map((_, i) => `<Override PartName="/xl/worksheets/sheet${i + 1}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>`).join('')}<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/></Types>`],
    ['_rels/.rels', `<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="${rels}/officeDocument" Target="xl/workbook.xml"/></Relationships>`],
    ['xl/workbook.xml', `<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="${main}" xmlns:r="${rels}"><sheets>${sheets.map((sheet, i) => `<sheet name="${xmlEscape(sheet.name)}" sheetId="${i + 1}" r:id="rId${i + 1}"${sheet.hidden ? ' state="hidden"' : ''}/>`).join('')}</sheets></workbook>`],
    ['xl/_rels/workbook.xml.rels', `<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">${sheets.map((_, i) => `<Relationship Id="rId${i + 1}" Type="${rels}/worksheet" Target="worksheets/sheet${i + 1}.xml"/>`).join('')}<Relationship Id="rId${sheets.length + 1}" Type="${rels}/styles" Target="styles.xml"/></Relationships>`],
    ['xl/styles.xml', `<?xml version="1.0" encoding="UTF-8"?><styleSheet xmlns="${main}"><fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts><fills count="1"><fill><patternFill patternType="none"/></fill></fills><borders count="1"><border/></borders><cellStyleXfs count="1"><xf/></cellStyleXfs><cellXfs count="3"><xf numFmtId="0"/><xf numFmtId="14" applyNumberFormat="1"/><xf numFmtId="10" applyNumberFormat="1"/></cellXfs></styleSheet>`],
    ...sheets.map((sheet, i): [string, string] => [`xl/worksheets/sheet${i + 1}.xml`, sheetXml(sheet)]),
  ]
  return new Blob([makeZip(files) as BlobPart], { type: XLSX_TYPE })
}

function makeBudgetXlsx(title: string): Blob {
  const quarters: MockCell[][] = Array.from({ length: 24 }, (_, r) => {
    const values = [1, 2, 3, 4].map((q) => ((r + 1) * (q + 3) * 17) % 997)
    return [`Item ${r + 1}`, { value: 45292 + r * 7, style: 1 }, ...values, values.reduce((a, b) => a + b, 0), { value: ((r * 7) % 40) / 100 + 0.025, style: 2 }]
  })
  return makeMockXlsx([
    { name: 'Budget', rows: [[title], ['Item', 'Updated', 'Q1', 'Q2', 'Q3', 'Q4', 'Total', 'Growth'], ...quarters], merges: ['A1:H1'] },
    { name: 'Notes', rows: [['Topic', 'Note'], ['Scope', 'Figures are generated for the mock runtime.'], ['币种', '人民币（元）']] },
    { name: 'Internal', rows: [['hidden']], hidden: true },
  ])
}

function makeCsv(): Blob {
  const rows = Array.from({ length: 40 }, (_, i) => `Contact ${i + 1},contact${i + 1}@example.com,"${i % 3 === 0 ? 'Kyoto, Japan' : 'Shenzhen'}",${2020 + (i % 6)}-0${1 + (i % 9)}-15`)
  return new Blob([['Name,Email,City,Joined', ...rows].join('\n')], { type: 'text/csv' })
}

function makeWav(seconds = 4, seed = 'tone'): Blob {
  const sampleRate = 22050
  const frames = seconds * sampleRate
  const buffer = new ArrayBuffer(44 + frames * 2)
  const view = new DataView(buffer)
  const write = (offset: number, text: string) => {
    for (let i = 0; i < text.length; i += 1) view.setUint8(offset + i, text.charCodeAt(i))
  }
  write(0, 'RIFF')
  view.setUint32(4, 36 + frames * 2, true)
  write(8, 'WAVE')
  write(12, 'fmt ')
  view.setUint32(16, 16, true)
  view.setUint16(20, 1, true)
  view.setUint16(22, 1, true)
  view.setUint32(24, sampleRate, true)
  view.setUint32(28, sampleRate * 2, true)
  view.setUint16(32, 2, true)
  view.setUint16(34, 16, true)
  write(36, 'data')
  view.setUint32(40, frames * 2, true)
  const base = 220 + (hash32(seed) % 200)
  for (let i = 0; i < frames; i += 1) {
    const t = i / sampleRate
    const envelope = Math.min(1, t * 8) * Math.min(1, (seconds - t) * 4)
    const v = (Math.sin(2 * Math.PI * base * t) * 0.5 + Math.sin(2 * Math.PI * base * 1.5 * t) * 0.25) * envelope
    view.setInt16(44 + i * 2, Math.max(-1, Math.min(1, v)) * 32767, true)
  }
  return new Blob([buffer], { type: 'audio/wav' })
}

function pdfEscape(text: string): string {
  return text.replace(/\\/g, '\\\\').replace(/\(/g, '\\(').replace(/\)/g, '\\)').replace(/[^\x20-\x7e]/g, '?')
}

/** Minimal PDF (Helvetica) so a PDF viewer has real bytes; pages after the first carry their number. */
function makePdf(title: string, lines: string[], pageCount = 1): Blob {
  const pageContent = (page: number) => [
    `BT /F1 24 Tf 72 720 Td (${pdfEscape(page === 0 ? title : `${title} - page ${page + 1}`)}) Tj ET`,
    ...lines.map((line, i) => `BT /F1 12 Tf 72 ${680 - i * 18} Td (${pdfEscape(line)}) Tj ET`),
  ].join('\n')
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    `<< /Type /Pages /Kids [${Array.from({ length: pageCount }, (_, i) => `${4 + i * 2} 0 R`).join(' ')}] /Count ${pageCount} >>`,
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
  ]
  for (let page = 0; page < pageCount; page += 1) {
    const content = pageContent(page)
    objects.push(
      `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents ${5 + page * 2} 0 R /Resources << /Font << /F1 3 0 R >> >> >>`,
      `<< /Length ${content.length} >>\nstream\n${content}\nendstream`,
    )
  }
  let out = '%PDF-1.4\n'
  const offsets: number[] = []
  objects.forEach((body, i) => {
    offsets.push(out.length)
    out += `${i + 1} 0 obj\n${body}\nendobj\n`
  })
  const xref = out.length
  out += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`
  for (const offset of offsets) out += `${String(offset).padStart(10, '0')} 00000 n \n`
  out += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`
  return new Blob([out], { type: 'application/pdf' })
}

let webmPromise: Promise<Blob | null> | null = null

/** ~1.5 s animated WebM produced by MediaRecorder (null when the Runtime cannot record). */
function makeWebm(): Promise<Blob | null> {
  if (webmPromise) return webmPromise
  webmPromise = (async () => {
    if (typeof MediaRecorder === 'undefined') return null
    const canvas = document.createElement('canvas')
    canvas.width = 640
    canvas.height = 360
    const ctx = canvas.getContext('2d')
    if (!ctx || typeof canvas.captureStream !== 'function') return null
    const stream = canvas.captureStream(30)
    const mime = ['video/webm;codecs=vp9', 'video/webm;codecs=vp8', 'video/webm'].find((m) => MediaRecorder.isTypeSupported(m))
    if (!mime) return null
    const recorder = new MediaRecorder(stream, { mimeType: mime })
    const chunks: Blob[] = []
    recorder.ondataavailable = (event) => {
      if (event.data.size > 0) chunks.push(event.data)
    }
    const done = new Promise<void>((resolve) => {
      recorder.onstop = () => resolve()
    })
    recorder.start(100)
    const start = performance.now()
    await new Promise<void>((resolve) => {
      const frame = () => {
        const t = (performance.now() - start) / 1000
        ctx.fillStyle = `hsl(${(t * 120) % 360} 60% 30%)`
        ctx.fillRect(0, 0, 640, 360)
        ctx.fillStyle = '#fff'
        ctx.beginPath()
        ctx.arc(80 + ((t * 300) % 480), 180 + Math.sin(t * 6) * 80, 40, 0, Math.PI * 2)
        ctx.fill()
        ctx.font = 'bold 28px system-ui, sans-serif'
        ctx.fillText(`Pipeline transmux · ${t.toFixed(1)}s`, 24, 330)
        if (t < 1.6) requestAnimationFrame(frame)
        else resolve()
      }
      requestAnimationFrame(frame)
    })
    recorder.stop()
    await done
    stream.getTracks().forEach((track) => track.stop())
    return chunks.length ? new Blob(chunks, { type: 'video/webm' }) : null
  })().catch(() => null)
  return webmPromise
}

export { makePdf as makeMockPdf, makePng as makeMockPng, makeWebm as makeMockWebm }

// ─── Sample gallery (`cyfs:///samples`) ───

interface SampleSpec {
  name: string
  make: () => Promise<Blob> | Blob
  /** Special behaviours exercised by the sample. */
  behaviour?: 'permission-denied' | 'corrupted' | 'pipeline' | 'unsupported'
  size?: number
}

const SAMPLES: SampleSpec[] = [
  { name: 'sunrise-over-kyoto.png', make: () => makePng('Sunrise over Kyoto', 'sunrise') },
  { name: 'harbor-lights.jpg', make: () => makePng('Harbor lights', 'harbor', 1200, 1600) },
  { name: 'system-diagram.svg', make: () => makeSvg('System diagram', 'diagram') },
  { name: 'release-notes.md', make: () => makeMarkdown({ name: 'release-notes.md', summary: 'What changed in this build of the Preview Component.', tags: ['preview', 'release'] }) },
  { name: 'config.json', make: () => new Blob([JSON.stringify({ preview: { uiMode: 'auto', fit: 'contain', autoWindowLimit: 8 }, pipelines: ['office-html', 'media-transmux'] }, null, 2)], { type: 'application/json' }) },
  { name: 'landing-page.html', make: () => makeHtml('Landing page', 'Direct HTML preview inside the sandbox.') },
  { name: 'ambient-tone.wav', make: () => makeWav(4, 'ambient') },
  { name: 'product-brief.pdf', make: () => makePdf('Product brief', ['BuckyOS Preview — content-first viewing for every app.', 'Rendered by the Runtime built-in PDF viewer (PDFIframeRenderer).', 'Zoom, find and print come from the viewer itself in P0.']) },
  { name: 'quarterly-report.docx', make: () => new Blob(['PK\u0003\u0004mock-docx'], { type: 'application/octet-stream' }), behaviour: 'pipeline', size: 240_000 },
  { name: 'ledger.xls', make: () => new Blob(['\u00d0\u00cf\u0011\u00e0mock-xls'], { type: 'application/octet-stream' }), behaviour: 'pipeline', size: 88_000 },
  { name: 'budget.xlsx', make: () => makeBudgetXlsx('Budget 2026') },
  { name: 'contacts.csv', make: makeCsv },
  { name: 'annual-report.pdf', make: () => makePdf('Annual report', ['A longer document: pages are drawn as they scroll into view.'], 12) },
  { name: 'handbook.epub', make: () => new Blob(['PK\u0003\u0004mimetypeapplication/epub+zip'], { type: 'application/octet-stream' }), behaviour: 'pipeline', size: 640_000 },
  { name: 'demo-clip.mp4', make: () => new Blob(['\u0000\u0000\u0000\u0018ftypisom'], { type: 'application/octet-stream' }), behaviour: 'pipeline', size: 12_400_000 },
  { name: 'keynote.pptx', make: () => new Blob(['PK\u0003\u0004mock-pptx'], { type: 'application/octet-stream' }), behaviour: 'unsupported', size: 5_100_000 },
  { name: 'archive.zip', make: () => new Blob(['PK\u0003\u0004mock-zip'], { type: 'application/zip' }), behaviour: 'unsupported', size: 900_000 },
  { name: 'damaged-photo.png', make: () => makeCorruptPng(), behaviour: 'corrupted' },
  { name: 'locked-notes.txt', make: () => new Blob(['secret'], { type: 'text/plain' }), behaviour: 'permission-denied' },
  { name: 'server.log', make: () => new Blob([Array.from({ length: 60000 }, (_, i) => `${new Date(1700000000000 + i * 1000).toISOString()} INFO worker-${i % 7} handled request ${i}`).join('\n')], { type: 'text/plain' }) },
]

function makeCorruptPng(): Blob {
  const bytes = new Uint8Array(4096)
  bytes.set([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])
  const rnd = mulberry32(7)
  for (let i = 8; i < bytes.length; i += 1) bytes[i] = (rnd() * 256) | 0
  return new Blob([bytes], { type: 'image/png' })
}

const SAMPLES_ROOT = 'cyfs:///samples'

/** Refs the Preview App landing page offers in mock mode. */
export function mockSampleItems(): PreviewSessionItemInput[] {
  return SAMPLES.map((sample) => ({
    source: { kind: 'cyfs-path', path: `${SAMPLES_ROOT}/${sample.name}` },
    title: sample.name,
  }))
}

export const MOCK_SAMPLES_CONTAINER: ContentRef = { kind: 'cyfs-path', path: SAMPLES_ROOT }

// ─── Library content (File Browser mock entries) ───

const blobCache = new Map<string, Promise<Blob>>()

function cachedBlob(key: string, make: () => Promise<Blob> | Blob): Promise<Blob> {
  let pending = blobCache.get(key)
  if (!pending) {
    pending = Promise.resolve(make())
    blobCache.set(key, pending)
  }
  return pending
}

function libraryBlob(entry: FileEntry): Promise<Blob> {
  const ext = extensionOf(entry.name)
  const label = entry.name.replace(/\.[^.]+$/, '')
  return cachedBlob(`lib:${entry.id}`, () => {
    switch (ext) {
      case 'jpg':
      case 'jpeg':
      case 'png':
      case 'webp':
      case 'gif':
        return makePng(label, entry.id)
      case 'svg':
        return makeSvg(label, entry.id)
      case 'pdf':
        return makePdf(label, [entry.summary ?? 'Generated fixture document.', `Path: ${entry.path}`, 'Rendered by the Runtime PDF viewer.'])
      case 'html':
      case 'htm':
        return makeHtml(label, entry.summary ?? '')
      case 'xlsx':
        return makeBudgetXlsx(label)
      case 'csv':
        return makeCsv()
      case 'wav':
      case 'mp3':
      case 'm4a':
      case 'flac':
        return makeWav(4, entry.id)
      case 'md':
      case 'txt':
      case 'json':
      case 'yaml':
      case 'yml':
      case 'toml':
      case 'ts':
      case 'js':
      case 'rs':
      case 'py':
        return makeMarkdown(entry)
      default:
        // Office / video / archives: opaque bytes with a recognizable prefix.
        return new Blob([ext === 'mp4' || ext === 'mov' ? '\u0000\u0000\u0000\u0018ftypisom' : 'PK\u0003\u0004mock'], {
          type: 'application/octet-stream',
        })
    }
  })
}

function followLink(entry: FileEntry, depth = 0): FileEntry {
  if (!entry.link || depth > 4) return entry
  if (entry.link.broken) throw new PreviewError('NOT_FOUND', 'The link target no longer exists')
  const target = entry.link.targetUrl.replace(/^dfs:\/\//, '')
  const resolved = mockEntryByPath(target)
  if (!resolved) throw new PreviewError('NOT_FOUND', 'The link target no longer exists')
  return followLink(resolved, depth + 1)
}

async function resolveLibraryPath(source: ContentRef, localPath: string): Promise<ResolvedPreviewSource> {
  if (localPath.startsWith('/home/Private')) {
    throw new PreviewError('PERMISSION_DENIED', 'You do not have access to this content')
  }
  const entry = mockEntryByPath(localPath)
  if (!entry) throw new PreviewError('NOT_FOUND', 'This content no longer exists')
  return resolveLibraryEntry(source, entry)
}

async function resolveLibraryEntry(source: ContentRef, entry: FileEntry): Promise<ResolvedPreviewSource> {
  await delay(40 + (hash32(entry.id) % 60))
  const target = followLink(entry)
  if (target.kind === 'folder') {
    return {
      originalSource: source,
      sourceObjectId: `mock:${entry.id}`,
      displayName: target.name,
      objectType: 'dir',
      mediaTypeHints: [],
      readRef: { kind: 'blob', blob: new Blob([]) },
      containerRef: parentRefOf(target.path),
    }
  }
  const blob = await libraryBlob(target)
  const hint = mediaTypeFromExtension(extensionOf(target.name))
  return {
    originalSource: source,
    sourceObjectId: `mock:${entry.id}`,
    inputObjectId: `mockobj:${hash32(`${target.id}:${blob.size}`).toString(16)}`,
    versionToken: target.modifiedAt,
    displayName: target.name,
    size: blob.size,
    objectType: 'file',
    mediaTypeHints: hint ? [hint] : [],
    readRef: { kind: 'blob', blob },
    containerRef: parentRefOf(entry.path),
  }
}

function parentRefOf(localPath: string): ContentRef | undefined {
  const parent = parentCyfsPath(normalizeCyfsPath(localPath))
  return parent ? { kind: 'cyfs-path', path: parent } : undefined
}

async function resolveSample(source: ContentRef, sample: SampleSpec): Promise<ResolvedPreviewSource> {
  await delay(60)
  if (sample.behaviour === 'permission-denied') {
    throw new PreviewError('PERMISSION_DENIED', 'You do not have access to this content')
  }
  const blob = await cachedBlob(`sample:${sample.name}`, sample.make)
  const hint = mediaTypeFromExtension(extensionOf(sample.name))
  return {
    originalSource: source,
    sourceObjectId: `mock:sample:${sample.name}`,
    inputObjectId: `mockobj:${hash32(`sample:${sample.name}`).toString(16)}`,
    versionToken: 'v1',
    displayName: sample.name,
    size: sample.size ?? blob.size,
    objectType: 'file',
    mediaTypeHints: hint ? [hint] : [],
    readRef: { kind: 'blob', blob },
    containerRef: MOCK_SAMPLES_CONTAINER,
  }
}

// ─── Simulated built-in Pipeline Catalog ───

interface MockPlan {
  pipelineId: string
  pipelineVersion: string
  output: PreviewRendererType
  ticks: number
  /** First attempt fails with a retryable error (exercises Retry / CAS). */
  failFirstAttempt?: boolean
  fidelityNote: string
}

const CATALOG: Record<string, MockPlan> = {
  docx: { pipelineId: 'office-html', pipelineVersion: '1.2.0', output: 'html', ticks: 4, fidelityNote: 'Converted to HTML: fonts, page layout and tracked changes may differ from the original.' },
  doc: { pipelineId: 'office-html', pipelineVersion: '1.2.0', output: 'html', ticks: 4, fidelityNote: 'Converted to HTML: fonts, page layout and tracked changes may differ from the original.' },
  odt: { pipelineId: 'office-html', pipelineVersion: '1.2.0', output: 'html', ticks: 4, fidelityNote: 'Converted to HTML: fonts and page layout may differ from the original.' },
  xls: { pipelineId: 'sheet-xlsx', pipelineVersion: '1.0.0', output: 'sheet', ticks: 3, failFirstAttempt: true, fidelityNote: 'Values only — formulas, charts and conditional formatting are not shown.' },
  ods: { pipelineId: 'sheet-xlsx', pipelineVersion: '1.0.0', output: 'sheet', ticks: 3, fidelityNote: 'Values only — formulas, charts and conditional formatting are not shown.' },
  epub: { pipelineId: 'office-html', pipelineVersion: '1.2.0', output: 'html', ticks: 3, fidelityNote: 'Converted to a web page.' },
  mp4: { pipelineId: 'media-transmux', pipelineVersion: '2.0.0', output: 'video', ticks: 5, fidelityNote: 'Transmuxed to WebM for playback; bitrate and audio tracks may be reduced.' },
  mov: { pipelineId: 'media-transmux', pipelineVersion: '2.0.0', output: 'video', ticks: 5, fidelityNote: 'Transmuxed to WebM for playback; bitrate and audio tracks may be reduced.' },
  mkv: { pipelineId: 'media-transmux', pipelineVersion: '2.0.0', output: 'video', ticks: 5, fidelityNote: 'Transmuxed to WebM for playback; bitrate and audio tracks may be reduced.' },
  avi: { pipelineId: 'media-transmux', pipelineVersion: '2.0.0', output: 'video', ticks: 5, fidelityNote: 'Transmuxed to WebM for playback; bitrate and audio tracks may be reduced.' },
  heic: { pipelineId: 'raster-decode', pipelineVersion: '1.0.0', output: 'image', ticks: 3, fidelityNote: 'Decoded to PNG; HDR gain maps and depth data are dropped.' },
  heif: { pipelineId: 'raster-decode', pipelineVersion: '1.0.0', output: 'image', ticks: 3, fidelityNote: 'Decoded to PNG; HDR gain maps and depth data are dropped.' },
  tiff: { pipelineId: 'raster-decode', pipelineVersion: '1.0.0', output: 'image', ticks: 3, fidelityNote: 'Decoded to PNG (first page only).' },
  tif: { pipelineId: 'raster-decode', pipelineVersion: '1.0.0', output: 'image', ticks: 3, fidelityNote: 'Decoded to PNG (first page only).' },
  psd: { pipelineId: 'raster-decode', pipelineVersion: '1.0.0', output: 'image', ticks: 3, fidelityNote: 'Flattened composite; layers and adjustment effects are not editable.' },
}

const VIDEO_POSTER: MockPlan = { pipelineId: 'video-poster', pipelineVersion: '1.0.0', output: 'image', ticks: 2, fidelityNote: 'Poster frame extracted by the built-in Pipeline.' }
const RASTER_THUMB: MockPlan = { pipelineId: 'raster-thumb', pipelineVersion: '1.0.0', output: 'image', ticks: 2, fidelityNote: 'Downscaled PNG thumbnail.' }
const THUMBNAIL_CATALOG: Record<string, MockPlan> = {
  mp4: VIDEO_POSTER,
  mov: VIDEO_POSTER,
  mkv: VIDEO_POSTER,
  avi: VIDEO_POSTER,
  heic: RASTER_THUMB,
  heif: RASTER_THUMB,
  tiff: RASTER_THUMB,
  tif: RASTER_THUMB,
  psd: RASTER_THUMB,
}

interface MockWork {
  state: PreviewWorkState
  attempts: number
  timer: number | null
  consumers: number
}

const works = new Map<string, MockWork>()
const TICK_MS = 350
const NEGATIVE_CACHE_MS = 2500

function snapshot(work: MockWork): PreviewWorkState {
  return structuredClone(work.state)
}

function workKeyFor(inputObjectId: string, plan: MockPlan, request: EnsurePreviewWorkRequest): string {
  // canonicalTransformParams: purpose, output, quality and a size bucket — never the viewport itself (§9.7).
  const bucket = plan.output === 'image' ? Math.ceil((request.targetProfile.viewport.width * request.targetProfile.viewport.dpr) / 1024) * 1024 : 0
  const params = JSON.stringify({ purpose: request.targetProfile.purpose, output: plan.output, quality: request.targetProfile.quality, bucket })
  return `mock-work/v1:${hash32(`${inputObjectId}|${plan.pipelineId}@${plan.pipelineVersion}|${params}`).toString(16)}`
}

async function produceResult(plan: MockPlan, source: ResolvedPreviewSource): Promise<PreviewResult> {
  const name = source.displayName ?? 'document'
  const title = name.replace(/\.[^.]+$/, '')
  if (plan.pipelineId === 'office-html') {
    const blob = await cachedBlob(`work:${plan.pipelineId}:${source.inputObjectId}`, () =>
      makeHtml(title, `Converted by the built-in "${plan.pipelineId}" Pipeline v${plan.pipelineVersion}.`),
    )
    return { resultType: 'html', readRef: { kind: 'blob', blob }, mediaType: 'text/html', sourceVersion: source.versionToken, fidelityNote: plan.fidelityNote, cacheable: true }
  }
  if (plan.pipelineId === 'sheet-xlsx') {
    const blob = await cachedBlob(`work:${plan.pipelineId}:${source.inputObjectId}`, () => makeBudgetXlsx(title))
    return { resultType: 'sheet', readRef: { kind: 'blob', blob }, mediaType: XLSX_TYPE, sourceVersion: source.versionToken, fidelityNote: plan.fidelityNote, cacheable: true }
  }
  if (plan.pipelineId === 'media-transmux') {
    const blob = await makeWebm()
    if (!blob) throw { code: 'CONVERTER_UNAVAILABLE', message: 'This Runtime cannot produce a playable stream for this video', retryable: false }
    return { resultType: 'video', readRef: { kind: 'blob', blob }, mediaType: 'video/webm', sourceVersion: source.versionToken, fidelityNote: plan.fidelityNote, progressive: true, cacheable: true }
  }
  if (plan.pipelineId === 'video-poster' || plan.pipelineId === 'raster-thumb') {
    const blob = await cachedBlob(`work:${plan.pipelineId}:${source.inputObjectId}`, () => makePng(title, source.inputObjectId ?? name, 640, 360))
    return { resultType: 'image', readRef: { kind: 'blob', blob }, mediaType: 'image/png', sourceVersion: source.versionToken, width: 640, height: 360, fidelityNote: plan.fidelityNote, cacheable: true }
  }
  const blob = await cachedBlob(`work:${plan.pipelineId}:${source.inputObjectId}`, () => makePng(title, source.inputObjectId ?? name))
  return { resultType: 'image', readRef: { kind: 'blob', blob }, mediaType: 'image/png', sourceVersion: source.versionToken, fidelityNote: plan.fidelityNote, cacheable: true }
}

function startAttempt(key: string, work: MockWork, plan: MockPlan, source: ResolvedPreviewSource) {
  work.attempts += 1
  const attemptId = `att-${work.attempts}`
  let completed = 0
  work.state = { workKey: key, state: 'processing', attemptId, taskId: `task-${key.slice(-6)}-${work.attempts}`, progress: { completed: 0, total: plan.ticks, message: 'Preparing preview' }, retryAfterMs: TICK_MS }
  const tick = () => {
    completed += 1
    if (completed < plan.ticks) {
      work.state = { workKey: key, state: 'processing', attemptId, taskId: `task-${key.slice(-6)}-${work.attempts}`, progress: { completed, total: plan.ticks, message: completed === 1 ? 'Reading source object' : 'Converting' }, retryAfterMs: TICK_MS }
      work.timer = window.setTimeout(tick, TICK_MS)
      return
    }
    work.timer = null
    if (plan.failFirstAttempt && work.attempts === 1) {
      work.state = { workKey: key, state: 'failed', attemptId, error: { code: 'CONVERTER_TIMEOUT', message: 'The converter did not finish in time', retryable: true, retryAfter: Date.now() + 200 } }
      return
    }
    void produceResult(plan, source)
      .then((result) => {
        work.state = { workKey: key, state: 'completed', attemptId, result }
      })
      .catch((err: { code?: string; message?: string; retryable?: boolean }) => {
        work.state = { workKey: key, state: 'failed', attemptId, error: { code: err.code ?? 'CONVERTER_ERROR', message: err.message ?? 'The converter failed', retryable: err.retryable ?? false, retryAfter: Date.now() + NEGATIVE_CACHE_MS } }
      })
  }
  work.timer = window.setTimeout(tick, TICK_MS)
}

// ─── Provider ───

export function createMockPreviewProvider(): PreviewProvider {
  return {
    id: 'mock',

    async resolvePreviewSource(source, { signal }) {
      if (signal?.aborted) throw new PreviewError('CANCELLED', 'Cancelled')
      if (isBlobRef(source)) {
        const { blob, name } = source.value
        const hint = blob.type || mediaTypeFromExtension(extensionOf(name))
        return {
          originalSource: source,
          displayName: name ?? 'Dropped content',
          size: blob.size,
          objectType: 'file',
          mediaTypeHints: hint ? [hint] : [],
          readRef: { kind: 'blob', blob },
          inputObjectId: `blob:${hash32(`${name}:${blob.size}:${blob.type}`).toString(16)}`,
        }
      }
      if (isCyfsPathRef(source)) {
        const local = cyfsPathToLocal(source.path)
        if (local === '/samples') {
          return { originalSource: source, displayName: 'samples', objectType: 'dir', mediaTypeHints: [], readRef: { kind: 'blob', blob: new Blob([]) } }
        }
        if (local.startsWith('/samples/')) {
          const name = local.slice('/samples/'.length)
          const sample = SAMPLES.find((s) => s.name === name)
          if (!sample) throw new PreviewError('NOT_FOUND', 'This content no longer exists')
          return resolveSample(source, sample)
        }
        return resolveLibraryPath(source, local)
      }
      const bucketRef = asBucketObjectRef(source)
      if (bucketRef) {
        const sample = SAMPLES.find((s) => s.name === bucketRef.url.split('/').pop())
        if (!sample) throw new PreviewError('NOT_FOUND', 'This content no longer exists')
        return resolveSample(source, sample)
      }
      if (isObjectIdRef(source)) {
        const id = source.objectId
        if (id.startsWith('mock:sample:')) {
          const sample = SAMPLES.find((s) => s.name === id.slice('mock:sample:'.length))
          if (!sample) throw new PreviewError('NOT_FOUND', 'Unknown object')
          return resolveSample(source, sample)
        }
        const entry = mockEntryById(id.startsWith('mock:') ? id.slice(5) : id)
        if (!entry) throw new PreviewError('NOT_FOUND', 'Unknown object')
        return resolveLibraryEntry(source, entry)
      }
      throw new PreviewError('INVALID_SOURCE', `Unsupported source kind "${source.kind}"`)
    },

    async enumerateContainer(container, { signal }) {
      if (signal?.aborted) throw new PreviewError('CANCELLED', 'Cancelled')
      await delay(60)
      if (!isCyfsPathRef(container)) {
        throw new PreviewError('UNSUPPORTED', 'Only path containers can be enumerated in mock mode')
      }
      const local = cyfsPathToLocal(container.path)
      if (local === '/samples') return mockSampleItems()
      if (local.startsWith('/home/Private')) throw new PreviewError('PERMISSION_DENIED', 'You do not have access to this folder')
      const entries = mockEntriesAtPath(local)
      if (!entries) throw new PreviewError('NOT_FOUND', 'This folder no longer exists')
      return entries
        .filter((entry) => entry.kind !== 'folder')
        .sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' }))
        .map((entry) => ({ id: `mock:${entry.id}`, source: { kind: 'cyfs-path', path: normalizeCyfsPath(entry.path) }, title: entry.name }))
    },

    async ensurePreviewWork(request) {
      const { source, options } = request
      const ext = extensionOf(source.displayName)
      const plan = request.targetProfile.purpose === 'thumbnail' ? THUMBNAIL_CATALOG[ext] : CATALOG[ext]
      if (!plan) {
        return { kind: 'unsupported', reason: 'no-pipeline' as PreviewUnsupportedReason, detail: 'No built-in Pipeline accepts this format' }
      }
      if (!request.runtimeProfile.acceptTypes.includes(plan.output)) {
        return { kind: 'unsupported', reason: 'runtime-unsupported', detail: 'The Runtime cannot display the Pipeline output' }
      }
      const inputObjectId = source.inputObjectId ?? `mockobj:${hash32(source.displayName ?? '').toString(16)}`
      const key = workKeyFor(inputObjectId, plan, request)
      await delay(30)
      let work = works.get(key)
      if (!work) {
        work = { state: { workKey: key, state: 'processing' }, attempts: 0, timer: null, consumers: 0 }
        works.set(key, work)
        startAttempt(key, work, plan, source)
        return snapshot(work)
      }
      if (work.state.state === 'failed' && options?.retry) {
        const expected = options.expectedAttemptId
        if (expected && expected !== work.state.attemptId) {
          // Another consumer already retried — return the current attempt (CAS).
          return snapshot(work)
        }
        if (work.state.error.retryable) startAttempt(key, work, plan, source)
      }
      return snapshot(work)
    },

    async getPreviewWork(workKey, { signal }) {
      if (signal?.aborted) throw new PreviewError('CANCELLED', 'Cancelled')
      await delay(20)
      const work = works.get(workKey)
      if (!work) throw new PreviewError('NOT_FOUND', 'Unknown preview work')
      return snapshot(work)
    },
  }
}
