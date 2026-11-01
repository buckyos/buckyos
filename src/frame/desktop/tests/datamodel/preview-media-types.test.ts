// Real HEIC / HEVC samples (libheif / x265) and Office documents shared with nfs_server's Pipeline tests.
import { readFileSync } from 'node:fs'
import { classifyMedia, decideDirect, heifImageSize, isPipelineDocument, rendererForMediaType, sniffMagic } from '../../src/components/preview/mediaTypes.ts'
import type { PreviewRuntimeProfile } from '../../src/components/preview/types.ts'

const FIXTURES = new URL('../../../nfs_server/tests/fixtures/preview/', import.meta.url)
const bytes = (name: string) => new Uint8Array(readFileSync(new URL(name, FIXTURES)))

function equal(actual: unknown, expected: unknown, what: string) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error(`${what}: expected ${JSON.stringify(expected)}, received ${JSON.stringify(actual)}`)
}

const chromium: PreviewRuntimeProfile = {
  acceptTypes: ['image', 'svg', 'text', 'html', 'audio', 'video', 'pdf'],
  imageMediaTypes: ['image/png', 'image/jpeg', 'image/gif', 'image/webp', 'image/avif'],
  videoMediaTypes: ['video/mp4', 'video/webm', 'video/ogg'],
  audioMediaTypes: ['audio/mpeg', 'audio/mp4'],
  videoCodecs: { h264: true, hevc: false },
  heic: false,
  pdfInline: true,
}
const safari: PreviewRuntimeProfile = { ...chromium, imageMediaTypes: [...chromium.imageMediaTypes, 'image/heic', 'image/heif'], heic: true }

equal(sniffMagic(bytes('portrait_grid.heic')), 'image/heic', 'HEIC magic')
equal(heifImageSize(bytes('portrait_grid.heic')), { width: 768, height: 1024 }, 'portrait ispe + irot')
equal(heifImageSize(bytes('landscape_grid.heic')), { width: 1024, height: 768 }, 'landscape ispe')
equal(heifImageSize(bytes('h264_aac.mov')), null, 'no HEIF meta in a MOV')
equal(sniffMagic(bytes('hevc_aac.mov')), 'video/quicktime', 'MOV magic')

const mif1 = new Uint8Array([0, 0, 0, 0x18, ...'ftypmif1'.split('').map((c) => c.charCodeAt(0)), 0, 0, 0, 0, ...'mif1heic'.split('').map((c) => c.charCodeAt(0))])
equal(sniffMagic(mif1), 'image/heic', 'mif1 major brand with heic compatible')
const generic = new Uint8Array([0, 0, 0, 0x14, ...'ftypmif1'.split('').map((c) => c.charCodeAt(0)), 0, 0, 0, 0, ...'mif1'.split('').map((c) => c.charCodeAt(0))])
equal(sniffMagic(generic), 'image/heif', 'generic HEIF')
for (const brand of ['hevx', 'heim', 'heis', 'hevm', 'hevs']) {
  const b = new Uint8Array([0, 0, 0, 0x10, ...`ftyp${brand}`.split('').map((c) => c.charCodeAt(0)), 0, 0, 0, 0])
  equal(sniffMagic(b), 'image/heic', `brand ${brand}`)
}

// Upper-case suffix, empty File.type, octet-stream: the bytes decide.
const heic = classifyMedia({ name: 'IMG_0001.HEIC', hints: ['application/octet-stream'], contentType: '', magic: bytes('portrait_grid.heic') })
equal([heic.mediaType, heic.rendererType, heic.confirmed], ['image/heic', 'image', true], 'HEIC classification')
equal(decideDirect(heic, 4338, chromium), { ok: false, reason: 'runtime-unsupported' }, 'Chromium sends HEIC to the Pipeline')
equal(decideDirect(heic, 4338, safari), { ok: true }, 'Safari shows HEIC directly')

// Video Direct by container: canPlayType("video/quicktime") is "" on Chromium yet H.264 MOV plays.
const mov = classifyMedia({ name: 'IMG_0002.MOV', magic: bytes('h264_aac.mov') })
equal(decideDirect(mov, 32970, chromium), { ok: true }, 'MOV is tried directly')
const avi = classifyMedia({ name: 'clip.avi', contentType: 'video/x-msvideo' })
equal(decideDirect(avi, 1, chromium), { ok: false, reason: 'runtime-unsupported' }, 'AVI is not a Direct container')

// Office / ODF / RTF: the pdf renderer, but only for the Pipeline's PDF (nfs_server `preview-document`).
const docx = classifyMedia({ name: 'Q3 Report.DOCX', hints: ['application/octet-stream'], magic: bytes('office.docx') })
equal([docx.mediaType, docx.rendererType, docx.direct], ['application/vnd.openxmlformats-officedocument.wordprocessingml.document', 'pdf', false], 'DOCX classification')
equal(decideDirect(docx, 5656, chromium), { ok: false, reason: 'no-renderer' }, 'a DOCX never goes to the PDF viewer as is')
const pptx = classifyMedia({ name: 'deck.pptx', magic: bytes('office.pptx') })
equal([pptx.rendererType, pptx.direct, decideDirect(pptx, 10321, chromium).ok], ['pdf', false, false], 'PPTX goes to the Pipeline')
const odt = classifyMedia({ name: 'notes.odt', magic: bytes('office.odt') })
equal([odt.mediaType, odt.rendererType, odt.direct], ['application/vnd.oasis.opendocument.text', 'pdf', false], 'ODT classification')
for (const [name, type] of [
  ['old.doc', 'application/msword'],
  ['old.ppt', 'application/vnd.ms-powerpoint'],
  ['old.xls', 'application/vnd.ms-excel'],
  ['book.xlsx', 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet'],
  ['slides.odp', 'application/vnd.oasis.opendocument.presentation'],
  ['sheet.ods', 'application/vnd.oasis.opendocument.spreadsheet'],
  ['letter.rtf', 'application/rtf'],
]) {
  const doc = classifyMedia({ name })
  equal([doc.mediaType, doc.rendererType, doc.direct], [type, 'pdf', false], `${name} by extension`)
}
equal(rendererForMediaType('text/rtf'), 'pdf', 'text/rtf is a document, not text')
equal(classifyMedia({ name: 'memo', contentType: 'application/msword' }).rendererType, 'pdf', 'MIME type alone')
equal([isPipelineDocument('application/pdf'), isPipelineDocument('application/zip')], [false, false], 'PDF and ZIP are not converted documents')
const pdf = classifyMedia({ name: 'paper.pdf', magic: new TextEncoder().encode('%PDF-1.7\n') })
equal([pdf.rendererType, pdf.direct, decideDirect(pdf, 9, chromium).ok], ['pdf', true, true], 'a PDF stays Direct')
equal(classifyMedia({ name: 'bundle.zip', magic: bytes('office.docx') }).direct, false, 'a ZIP has no renderer')

console.log('preview media types: ok')
