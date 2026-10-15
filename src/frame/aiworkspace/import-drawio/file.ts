/* Reading draw.io files: `.drawio` / `.xml` (`<mxfile>` with one `<diagram>` per page, each page plain or
 * compressed: base64 → raw inflate → URL decoding), a bare `<mxGraphModel>` (what draw.io puts on the clipboard),
 * `.drawio.png` (the `mxfile` text chunk), `.drawio.svg` (the root's `content` attribute) and `<mxlibrary>`
 * (a JSON list of shapes, each a compressed or plain model). */

import { base64ToBytes, inflateRaw, isPng, latin1, pngTextChunks, utf8 } from './bytes.ts'
import { elements, firstElement, parseXml, textOf, type XmlElement } from './xml.ts'

export interface DrawioPage {
  id: string
  name: string
  /** The page's `<mxGraphModel>`. */
  model: XmlElement
}

export interface LibraryEntry { title: string; w: number; h: number; model: XmlElement | null; image?: string }

export interface DrawioFile {
  /** Where the pages came from. */
  source: 'mxfile' | 'model' | 'png' | 'svg' | 'library'
  pages: DrawioPage[]
  library: LibraryEntry[]
}

export class DrawioFormatError extends Error {}

/** `Graph.decompress`: base64 → inflateRaw → decodeURIComponent. Plain XML passes through. */
export async function decompressDiagram(text: string): Promise<string> {
  const t = text.trim()
  if (!t || t.startsWith('<')) return t
  const bytes = await inflateRaw(base64ToBytes(t))
  const decoded = latin1.decode(bytes)
  try { return decodeURIComponent(decoded) } catch { return utf8.decode(bytes) }
}

async function modelOf(diagram: XmlElement): Promise<XmlElement | null> {
  const inline = firstElement(diagram, 'mxGraphModel')
  if (inline) return inline
  const text = textOf(diagram).trim()
  if (!text) return null
  const xml = await decompressDiagram(text)
  return xml ? parseXml(xml) : null
}

async function fromRoot(root: XmlElement, source: DrawioFile['source']): Promise<DrawioFile> {
  if (root.name === 'mxGraphModel') return { source: source === 'mxfile' ? 'model' : source, pages: [{ id: 'page-1', name: 'Page-1', model: root }], library: [] }
  if (root.name === 'mxfile') {
    const pages: DrawioPage[] = []
    let n = 0
    for (const diagram of elements(root, 'diagram')) {
      n += 1
      const model = await modelOf(diagram)
      if (!model || model.name !== 'mxGraphModel') continue
      pages.push({ id: diagram.attrs.id || `page-${n}`, name: diagram.attrs.name || `Page-${n}`, model })
    }
    if (pages.length === 0) throw new DrawioFormatError('文件中没有可读的 draw.io 页面')
    return { source, pages, library: [] }
  }
  if (root.name === 'mxlibrary') {
    let list: { xml?: string; w?: number; h?: number; title?: string; data?: string }[]
    try { list = JSON.parse(textOf(root)) } catch { throw new DrawioFormatError('图形库内容不是 JSON') }
    const library: LibraryEntry[] = []
    for (const item of Array.isArray(list) ? list : []) {
      let model: XmlElement | null = null
      if (typeof item.xml === 'string') {
        try { const xml = await decompressDiagram(item.xml); model = xml ? parseXml(xml) : null } catch { model = null }
      }
      library.push({ title: item.title ?? '', w: Number(item.w) || 80, h: Number(item.h) || 80, model, ...(typeof item.data === 'string' ? { image: item.data } : {}) })
    }
    return { source: 'library', pages: [], library }
  }
  if (root.name === 'svg') {
    const content = root.attrs.content
    if (!content) throw new DrawioFormatError('这个 SVG 不含 draw.io 图（没有 content 属性）')
    const xml = content.trim().startsWith('<') ? content : await decompressDiagram(content)
    return fromRoot(parseXml(xml), 'svg')
  }
  throw new DrawioFormatError(`不认识的根元素 <${root.name}>`)
}

/** Is this text a draw.io model or file (a paste, a dropped file's text)? */
export function looksLikeDrawio(text: string): boolean {
  const head = text.trimStart().slice(0, 400)
  return /^(<\?xml[^>]*>\s*)?<(mxfile|mxGraphModel|mxlibrary)[\s>]/.test(head) || /^%3C(mxfile|mxGraphModel)/i.test(head)
}

/** Read a draw.io file from its bytes (any of the supported carriers) or text. */
export async function readDrawio(input: { bytes?: Uint8Array; text?: string }): Promise<DrawioFile> {
  let text = input.text
  if (input.bytes && isPng(input.bytes)) {
    const chunks = await pngTextChunks(input.bytes)
    const value = chunks.get('mxfile') ?? chunks.get('mxGraphModel')
    if (!value) throw new DrawioFormatError('这张 PNG 不含 draw.io 图（没有 mxfile 文本块）')
    let xml = value
    try { xml = decodeURIComponent(value) } catch { /* already plain */ }
    return fromRoot(parseXml(xml), 'png')
  }
  if (text === undefined && input.bytes) text = utf8.decode(input.bytes)
  if (text === undefined) throw new DrawioFormatError('没有内容')
  let t = text.trim()
  if (/^%3C/i.test(t)) { try { t = decodeURIComponent(t) } catch { /* keep */ } }
  return fromRoot(parseXml(t), 'mxfile')
}
