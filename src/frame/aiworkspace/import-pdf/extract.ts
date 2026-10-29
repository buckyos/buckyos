/* pdf.js → plain page data (PDF 导入方案 §4, §5.1). Works on a pdf.js document and page in the browser or in Deno;
 * pdf.js is passed in (typed by what is read here), never imported, so this module stays dependency free. The
 * argument arrays are reduced to JSON: glyphs to [advance, space, text], typed arrays to numbers, pictures and
 * patterns to references the host resolves later (`pictureObject`). */

import { OPS, type Json, type PdfFont, type PdfLink, type PdfPage, type PdfStructNode, type PdfTextItem } from './data.ts'

/** What is read of a pdf.js object store (`page.objs`, `page.commonObjs`). */
export interface PdfjsObjects { get(id: string, callback?: (data: unknown) => void): unknown; has(id: string): boolean }

export interface PdfjsPage {
  pageNumber: number
  getViewport(options: { scale: number }): { width: number; height: number; transform: number[] }
  getTextContent(options: { includeMarkedContent: boolean }): Promise<{ items: unknown[]; styles: Record<string, { ascent?: number | null; descent?: number | null; vertical?: boolean }> }>
  getOperatorList(): Promise<{ fnArray: number[]; argsArray: unknown[] }>
  getStructTree(): Promise<unknown>
  getAnnotations(): Promise<unknown[]>
  objs: PdfjsObjects
  commonObjs: PdfjsObjects
}

export interface PdfjsDocument {
  numPages: number
  getPage(n: number): Promise<PdfjsPage>
  getPageIndex(ref: unknown): Promise<number>
  getDestination(name: string): Promise<unknown[] | null>
  getOutline(): Promise<unknown[] | null>
  getOptionalContentConfig?(): Promise<{ isVisible(group: unknown): boolean }>
}

const r4 = (v: number) => Math.round(v * 10000) / 10000
const nums = (v: unknown): number[] | null => {
  if (v === null || v === undefined) return null
  if (ArrayBuffer.isView(v) || Array.isArray(v)) return Array.from(v as ArrayLike<number>, (n) => (typeof n === 'number' ? r4(n) : 0))
  return null
}
const isPlain = (v: unknown): boolean => v === null || ['string', 'number', 'boolean'].includes(typeof v) || (Array.isArray(v) && v.every(isPlain))

/** Where a picture's pixels are: an object id (`img_p0_1`, `g_…` for shared ones) or `op:<index>` (inline). */
export function pictureRef(fn: number, args: unknown[], index: number): string | null {
  if (fn === OPS.paintImageXObject || fn === OPS.paintImageXObjectRepeat) return typeof args[0] === 'string' ? args[0] : null
  if (fn === OPS.paintInlineImageXObject) return `op:${index}`
  return null
}

/** One operator's arguments as JSON, reduced to what the converter reads. */
function compactArgs(fn: number, raw: unknown, index: number, ocVisible: (data: unknown) => boolean): Json {
  const args = Array.isArray(raw) ? raw : raw === null || raw === undefined ? [] : [raw]
  switch (fn) {
    case OPS.dependency: return null
    case OPS.constructPath: {
      const data = args[1] as unknown[] | undefined
      return [args[0] as number, nums(data?.[0]), nums(args[2])]
    }
    case OPS.showText: {
      const glyphs = (args[0] as unknown[]) ?? []
      return [glyphs.map((g): Json => {
        if (typeof g === 'number') return r4(g)
        const glyph = g as { width?: number; isSpace?: boolean; unicode?: string } | null
        return [r4(glyph?.width ?? 0), glyph?.isSpace ? 1 : 0, glyph?.unicode ?? '']
      })]
    }
    case OPS.setTextMatrix: return nums(args[0]) ?? nums(args) ?? null
    case OPS.setFont: return [String(args[0]), Number(args[1]) || 0]
    case OPS.setGState: {
      const states = (args[0] as [string, unknown][]) ?? []
      return [states.map(([key, value]): Json => {
        if (key === 'SMask' || key === 'TR') return [key, Boolean(value)]
        if (key === 'Font') return [key, [String((value as unknown[])[0]), Number((value as unknown[])[1]) || 0]]
        if (key === 'D') return [key, [nums((value as unknown[])[0]) ?? [], Number((value as unknown[])[1]) || 0]]
        return [key, isPlain(value) ? value as Json : null]
      })]
    }
    case OPS.setFillColorN: case OPS.setStrokeColorN: {
      if (args[0] === 'Shading') return ['Shading', String(args[1]), nums(args[2])]
      if (args[0] === 'TilingPattern') return ['TilingPattern']
      return isPlain(args) ? args as Json : ['Unknown']
    }
    case OPS.paintImageXObject: return [String(args[0]), Number(args[1]) || 0, Number(args[2]) || 0]
    case OPS.paintImageXObjectRepeat: return [String(args[0])]
    case OPS.paintInlineImageXObject: {
      const img = args[0] as { width?: number; height?: number } | undefined
      return [`op:${index}`, img?.width ?? 0, img?.height ?? 0]
    }
    case OPS.paintImageMaskXObject: case OPS.paintImageMaskXObjectGroup: case OPS.paintImageMaskXObjectRepeat: case OPS.paintInlineImageXObjectGroup: return []
    case OPS.paintFormXObjectBegin: return [nums(args[0]), nums(args[1])]
    case OPS.beginGroup: {
      const g = (args[0] ?? {}) as { knockout?: boolean; smask?: unknown; isolated?: boolean }
      return [{ knockout: Boolean(g.knockout), smask: Boolean(g.smask), isolated: Boolean(g.isolated) }]
    }
    case OPS.beginAnnotation: return [String(args[0] ?? ''), nums(args[1])]
    case OPS.beginMarkedContentProps:
      if (args[0] === 'OC') return ['OC', ocVisible(args[1])]
      return [String(args[0]), typeof args[1] === 'number' ? args[1] : null]
    default:
      if (isPlain(args)) return args as Json
      return args.map((a) => (ArrayBuffer.isView(a) ? nums(a) : isPlain(a) ? a as Json : null))
  }
}

function fontOf(page: PdfjsPage, id: string, styles: Record<string, { ascent?: number | null; descent?: number | null; vertical?: boolean }>): PdfFont {
  let f: Record<string, unknown> = {}
  try { if (page.commonObjs.has(id)) f = (page.commonObjs.get(id) ?? {}) as Record<string, unknown> } catch { /* not loaded */ }
  const name = typeof f.name === 'string' ? f.name : ''
  const plain = name.replace(/^[A-Z]{6}\+/, '')
  const matrix = f.fontMatrix as number[] | undefined
  return {
    name: plain,
    bold: f.bold === true || f.black === true || /bold|black|heavy|semibold|demibold|-w[6-9]\b/i.test(plain),
    italic: f.italic === true || /italic|oblique/i.test(plain),
    serif: f.isSerifFont === true,
    mono: f.isMonospace === true,
    type3: f.isType3Font === true,
    vertical: f.vertical === true || styles[id]?.vertical === true,
    unit: matrix && typeof matrix[0] === 'number' && matrix[0] !== 0 ? Math.abs(matrix[0]) : 0.001,
    ascent: styles[id]?.ascent ?? null,
    descent: styles[id]?.descent ?? null,
  }
}

function structOf(node: unknown, depth = 0): PdfStructNode | null {
  if (!node || typeof node !== 'object' || depth > 64) return null
  const n = node as { role?: unknown; alt?: unknown; lang?: unknown; children?: unknown[] }
  const out: PdfStructNode = {}
  if (typeof n.role === 'string') out.role = n.role
  if (typeof n.alt === 'string' && n.alt) out.alt = n.alt
  if (typeof n.lang === 'string') out.lang = n.lang
  if (Array.isArray(n.children)) {
    out.children = []
    for (const c of n.children) {
      const child = c as { type?: unknown; id?: unknown }
      if (typeof child?.type === 'string' && typeof child.id === 'string') out.children.push({ type: child.type, id: child.id })
      else { const s = structOf(c, depth + 1); if (s) out.children.push(s) }
    }
  }
  return out
}

async function destPage(doc: PdfjsDocument, dest: unknown): Promise<number | null> {
  try {
    const explicit = typeof dest === 'string' ? await doc.getDestination(dest) : dest
    if (!Array.isArray(explicit) || explicit.length === 0) return null
    const target = explicit[0]
    if (typeof target === 'number') return target + 1
    return (await doc.getPageIndex(target)) + 1
  } catch {
    return null
  }
}

/** The first bookmark (top level and below, in reading order) pointing at each page. */
export async function readBookmarks(doc: PdfjsDocument): Promise<Map<number, string>> {
  const out = new Map<number, string>()
  let outline: unknown[] | null = null
  try { outline = await doc.getOutline() } catch { return out }
  const walk = async (items: unknown[], depth: number) => {
    for (const item of items) {
      const it = item as { title?: unknown; dest?: unknown; items?: unknown[] }
      const page = it.dest ? await destPage(doc, it.dest) : null
      const title = typeof it.title === 'string' ? it.title.trim() : ''
      if (page && title && !out.has(page)) out.set(page, title.slice(0, 256))
      if (Array.isArray(it.items) && depth < 8) await walk(it.items, depth + 1)
    }
  }
  if (outline) await walk(outline, 0)
  return out
}

export interface Extracted {
  page: PdfPage
  /** The operator list as pdf.js gave it (inline pictures are read from it). */
  raw: { fnArray: number[]; argsArray: unknown[] }
}

/** Read one page. `bookmarks` from `readBookmarks`; `oc` the document's optional content configuration, if any. */
export async function extractPage(doc: PdfjsDocument, page: PdfjsPage, bookmarks: Map<number, string>, oc: { isVisible(group: unknown): boolean } | null): Promise<Extracted> {
  const viewport = page.getViewport({ scale: 1 })
  const content = await page.getTextContent({ includeMarkedContent: true })
  const ops = await page.getOperatorList()
  const ocVisible = (data: unknown) => { try { return !oc || !data || oc.isVisible(data) } catch { return true } }
  const text: PdfTextItem[] = []
  const stack: (string | null)[] = []
  for (const raw of content.items) {
    const item = raw as { type?: string; id?: string; str?: string; transform?: number[]; width?: number; height?: number; fontName?: string; hasEOL?: boolean }
    if (item.type === 'beginMarkedContentProps' || item.type === 'beginMarkedContent') { stack.push(item.id ?? stack.at(-1) ?? null); continue }
    if (item.type === 'endMarkedContent') { stack.pop(); continue }
    if (typeof item.str !== 'string' || !item.transform) continue
    text.push({ str: item.str, transform: item.transform.map(r4), width: r4(item.width ?? 0), height: r4(item.height ?? 0), fontName: item.fontName ?? '', hasEOL: item.hasEOL === true, mc: stack.at(-1) ?? null })
  }
  const fnArray = Array.from(ops.fnArray)
  const argsArray: Json[] = fnArray.map((fn, i) => compactArgs(fn, ops.argsArray[i], i, ocVisible))
  const fonts: Record<string, PdfFont> = {}
  const patterns: Record<string, Json> = {}
  const images: Record<string, { width: number; height: number }> = {}
  for (const [i, fn] of fnArray.entries()) {
    const args = argsArray[i] as Json[]
    if (fn === OPS.setFont) { const id = String(args[0]); if (!fonts[id]) fonts[id] = fontOf(page, id, content.styles) }
    if (fn === OPS.setGState) for (const [key, value] of args[0] as [string, Json][]) if (key === 'Font') { const id = String((value as Json[])[0]); if (!fonts[id]) fonts[id] = fontOf(page, id, content.styles) }
    const patternId = fn === OPS.shadingFill ? String(args[0]) : (fn === OPS.setFillColorN || fn === OPS.setStrokeColorN) && args[0] === 'Shading' ? String(args[1]) : null
    if (patternId && !(patternId in patterns)) {
      const store = patternId.startsWith('g_') || patternId.includes('_type3_') ? page.commonObjs : page.objs
      let ir: unknown = null
      try { if (store.has(patternId)) ir = store.get(patternId) } catch { /* not loaded */ }
      const p = ir as unknown[] | null
      patterns[patternId] = Array.isArray(p) && p[0] === 'RadialAxial'
        ? ['RadialAxial', String(p[1]), nums(p[2]), ((p[3] as unknown[]) ?? []).map((s) => { const stop = s as [number, string]; return [r4(stop[0]), String(stop[1])] }), nums(p[4]), nums(p[5]), typeof p[6] === 'number' ? r4(p[6]) : null, typeof p[7] === 'number' ? r4(p[7]) : null]
        : [Array.isArray(p) ? String(p[0]) : 'Unknown']
    }
    if (fn === OPS.paintImageXObject || fn === OPS.paintInlineImageXObject) {
      const ref = String(args[0])
      images[ref] = { width: Number(args[1]) || 0, height: Number(args[2]) || 0 }
    }
  }
  let struct: PdfStructNode | null = null
  try { struct = structOf(await page.getStructTree()) } catch { /* untagged or broken */ }
  const links: PdfLink[] = []
  try {
    for (const a of await page.getAnnotations()) {
      const annot = a as { subtype?: string; rect?: number[]; dest?: unknown; url?: string; unsafeUrl?: string }
      if (annot.subtype !== 'Link' || !annot.rect) continue
      const target = annot.dest ? await destPage(doc, annot.dest) : null
      const url = annot.url ?? annot.unsafeUrl ?? null
      if (target || url) links.push({ rect: annot.rect.map(r4), page: target, url: url && /^https?:|^mailto:/i.test(url) ? url : null })
    }
  } catch { /* no annotations */ }
  return {
    page: {
      number: page.pageNumber, width: r4(viewport.width), height: r4(viewport.height), transform: viewport.transform.map(r4),
      fnArray, argsArray, text, fonts, patterns, images, struct, links, bookmark: bookmarks.get(page.pageNumber) ?? null,
    },
    raw: { fnArray, argsArray: ops.argsArray },
  }
}
