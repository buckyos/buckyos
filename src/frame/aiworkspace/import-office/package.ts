/* OPC packages (Office 导入 TODO §2.1): a .docx / .pptx is a zip of XML parts tied together by relationship parts.
 * This reads `[Content_Types].xml`, `_rels/.rels` and any part's `_rels/<name>.rels` (a target resolved against the
 * part's folder, or an external URL), finds the main part, and parses parts on demand (each once). The zip reader is
 * the table importer's (stored and deflated entries through `DecompressionStream`, browsers and Deno alike).
 *
 * Element names keep their prefixes (`w:p`, `a:srgbClr`); the readers match local names (`local`) so documents
 * written with other prefixes, or in the Strict namespaces, read the same. */

import { isZip, zipEntries, zipRead } from '../import-table/xlsx.ts'
import { elements, parseXml, type XmlElement } from '../import-drawio/xml.ts'

/** What an Office file is: the two the editable path reads, and those only the faithful path (LibreOffice) takes. */
export type OfficeKind = 'docx' | 'pptx' | 'doc' | 'ppt' | 'odt' | 'odp' | 'rtf'

/** The file picker's `accept` list for "import an Office document". */
export const OFFICE_ACCEPT = [
  '.docx', '.docm', '.dotx', '.pptx', '.pptm', '.potx', '.ppsx', '.doc', '.ppt', '.pps', '.odt', '.odp', '.rtf',
  'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
  'application/vnd.openxmlformats-officedocument.presentationml.presentation',
  'application/msword', 'application/vnd.ms-powerpoint', 'application/rtf', 'text/rtf',
  'application/vnd.oasis.opendocument.text', 'application/vnd.oasis.opendocument.presentation',
].join(',')

/** Kinds the editable path converts (the rest need LibreOffice). */
export const EDITABLE_KINDS: readonly OfficeKind[] = ['docx', 'pptx']
/** Kinds that become a document (a flow page) rather than slides (canvas pages). */
export const isDocumentKind = (kind: OfficeKind) => kind === 'docx' || kind === 'doc' || kind === 'odt' || kind === 'rtf'

export class OfficeError extends Error {
  readonly code: 'not_office' | 'encrypted' | 'corrupt'
  constructor(code: OfficeError['code'], message: string) { super(message); this.code = code }
}

/** The local part of a qualified name (`w:p` → `p`). */
export const local = (name: string) => name.slice(name.indexOf(':') + 1)
/** Child elements with this local name. */
export const kids = (el: XmlElement | null | undefined, name: string): XmlElement[] => (el ? elements(el).filter((c) => local(c.name) === name) : [])
/** The first child element with this local name. */
export const kid = (el: XmlElement | null | undefined, name: string): XmlElement | undefined => el?.children.find((c): c is XmlElement => typeof c !== 'string' && local(c.name) === name)
/** The element reached by a path of local names (`kidPath(sp, 'spPr', 'xfrm', 'off')`). */
export function kidPath(el: XmlElement | null | undefined, ...names: string[]): XmlElement | undefined {
  let at = el ?? undefined
  for (const n of names) at = kid(at, n)
  return at
}
/** Every descendant (the element itself included) with this local name, depth first. */
export function descendants(el: XmlElement, name: string, out: XmlElement[] = []): XmlElement[] {
  if (local(el.name) === name) out.push(el)
  for (const c of el.children) if (typeof c !== 'string') descendants(c, name, out)
  return out
}
/** An attribute by local name (`r:id`, `r:embed`, `w:val` …). */
export function attr(el: XmlElement | undefined, name: string): string | undefined {
  if (!el) return undefined
  if (el.attrs[name] !== undefined) return el.attrs[name]
  for (const [k, v] of Object.entries(el.attrs)) if (local(k) === name && !k.startsWith('xmlns')) return v
  return undefined
}

export interface Relationship {
  id: string
  /** The type's last segment (`image`, `hyperlink`, `slideLayout` …): the Transitional and Strict URIs end alike. */
  type: string
  /** The target part's path inside the package, or the URL of an external target. */
  target: string
  external: boolean
}

export interface OfficePackage {
  /** docx or pptx by the main part's content type (null: some other OPC package). */
  readonly kind: 'docx' | 'pptx' | null
  /** The main part's path (`word/document.xml`, `ppt/presentation.xml`). */
  readonly main: string
  paths(): string[]
  has(path: string): boolean
  /** The part's content type (`[Content_Types].xml`: an override, else the default of its extension). */
  contentType(path: string): string | null
  /** The size the zip directory records for a part (bytes, uncompressed). */
  size(path: string): number
  bytes(path: string): Promise<Uint8Array | null>
  xml(path: string): Promise<XmlElement | null>
  /** The relationships of a part (`''` for the package's own). */
  rels(path: string): Promise<Map<string, Relationship>>
  /** The first relationship of this type from the part. */
  relOfType(path: string, type: string): Promise<Relationship | undefined>
}

const MAIN_TYPES: Record<string, 'docx' | 'pptx'> = {
  'application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml': 'docx',
  'application/vnd.ms-word.document.macroEnabled.main+xml': 'docx',
  'application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml': 'docx',
  'application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml': 'pptx',
  'application/vnd.ms-powerpoint.presentation.macroEnabled.main+xml': 'pptx',
  'application/vnd.openxmlformats-officedocument.presentationml.slideshow.main+xml': 'pptx',
  'application/vnd.openxmlformats-officedocument.presentationml.template.main+xml': 'pptx',
}

/** `dir/../x` and `./` folded; no leading slash. */
export function normalizePath(path: string): string {
  const out: string[] = []
  for (const seg of path.replace(/\\/g, '/').split('/')) {
    if (seg === '' || seg === '.') continue
    if (seg === '..') out.pop()
    else out.push(seg)
  }
  return out.join('/')
}

/** The relationship part of a part (`word/document.xml` → `word/_rels/document.xml.rels`). */
export function relsPathOf(path: string): string {
  const slash = path.lastIndexOf('/')
  return slash < 0 ? `_rels/${path}.rels` : `${path.slice(0, slash)}/_rels/${path.slice(slash + 1)}.rels`
}

/** A relationship target resolved against its source part. */
export function resolveTarget(source: string, target: string): string {
  if (target.startsWith('/')) return normalizePath(target)
  const slash = source.lastIndexOf('/')
  return normalizePath((slash < 0 ? '' : source.slice(0, slash + 1)) + target)
}

const isOle2 = (b: Uint8Array) => b.length > 8 && b[0] === 0xd0 && b[1] === 0xcf && b[2] === 0x11 && b[3] === 0xe0

/** What an Office file is, by its bytes and then its name; null for anything else (a workbook goes to the table
 * importer). Synchronous: only the zip directory and the stored `mimetype` entry of an ODF file are read. */
export function officeKind(name: string, bytes: Uint8Array): OfficeKind | null {
  const lower = name.toLowerCase()
  const ext = /\.([a-z]+)$/.exec(lower)?.[1] ?? ''
  if (bytes.length > 5 && String.fromCharCode(...bytes.subarray(0, 5)) === '{\\rtf') return 'rtf'
  if (isOle2(bytes)) {
    // an encrypted .docx / .pptx is an OLE2 container as well: kept, so the import can say it is encrypted
    if (ext === 'doc' || ext === 'dot' || ext === 'docx' || ext === 'docm') return ext.startsWith('docx') || ext === 'docm' ? 'docx' : 'doc'
    if (ext === 'ppt' || ext === 'pps' || ext === 'pot' || ext === 'pptx' || ext === 'pptm') return ext === 'pptx' || ext === 'pptm' ? 'pptx' : 'ppt'
    return null
  }
  if (!isZip(bytes)) return null
  let entries: ReturnType<typeof zipEntries>
  try { entries = zipEntries(bytes) } catch { return null }
  const names = [...entries.keys()]
  if (entries.has('word/document.xml') || names.some((n) => /^word\/[^/]+\.xml$/.test(n) && /document/i.test(n))) return 'docx'
  if (entries.has('ppt/presentation.xml')) return 'pptx'
  const mimetype = entries.get('mimetype')
  if (mimetype && mimetype.method === 0) {
    const p = mimetype.offset
    const start = p + 30 + (bytes[p + 26] | (bytes[p + 27] << 8)) + (bytes[p + 28] | (bytes[p + 29] << 8))
    const type = new TextDecoder().decode(bytes.subarray(start, start + Math.min(mimetype.compressed, 128)))
    if (type === 'application/vnd.oasis.opendocument.text') return 'odt'
    if (type === 'application/vnd.oasis.opendocument.presentation') return 'odp'
    return null
  }
  if (names.some((n) => n.startsWith('word/'))) return 'docx'
  if (names.some((n) => n.startsWith('ppt/'))) return 'pptx'
  return null
}

/** Open an OPC package. Throws OfficeError (`not_office`, `encrypted`, `corrupt`). */
export async function openPackage(bytes: Uint8Array): Promise<OfficePackage> {
  if (isOle2(bytes)) throw new OfficeError('encrypted', 'an OLE2 container (an encrypted or legacy binary document)')
  if (!isZip(bytes)) throw new OfficeError('not_office', 'not a zip file')
  let entries: ReturnType<typeof zipEntries>
  try { entries = zipEntries(bytes) } catch (error) { throw new OfficeError('corrupt', error instanceof Error ? error.message : String(error)) }
  // part names are case-insensitive in OPC
  const byLower = new Map<string, string>()
  for (const k of entries.keys()) byLower.set(k.toLowerCase(), k)
  const find = (path: string) => entries.get(path) ?? entries.get(byLower.get(path.toLowerCase()) ?? '')
  const dec = new TextDecoder()
  const xmlCache = new Map<string, Promise<XmlElement | null>>()
  const relsCache = new Map<string, Promise<Map<string, Relationship>>>()
  const readBytes = async (path: string): Promise<Uint8Array | null> => {
    const e = find(normalizePath(path))
    if (!e) return null
    try { return await zipRead(bytes, e) } catch (error) {
      if ((error as { code?: string }).code === 'encrypted') throw new OfficeError('encrypted', 'the document is encrypted')
      throw new OfficeError('corrupt', error instanceof Error ? error.message : String(error))
    }
  }
  const readXml = (path: string): Promise<XmlElement | null> => {
    const key = normalizePath(path)
    let pending = xmlCache.get(key)
    if (!pending) {
      pending = readBytes(key).then((b) => {
        if (!b) return null
        // UTF-16 parts carry a BOM
        const text = b[0] === 0xff && b[1] === 0xfe ? new TextDecoder('utf-16le').decode(b) : b[0] === 0xfe && b[1] === 0xff ? new TextDecoder('utf-16be').decode(b) : dec.decode(b)
        try { return parseXml(text) } catch (error) { throw new OfficeError('corrupt', `${key}: ${error instanceof Error ? error.message : String(error)}`) }
      })
      xmlCache.set(key, pending)
    }
    return pending
  }
  const readRels = (path: string): Promise<Map<string, Relationship>> => {
    const key = normalizePath(path)
    let pending = relsCache.get(key)
    if (!pending) {
      pending = readXml(key === '' ? '_rels/.rels' : relsPathOf(key)).then((doc) => {
        const out = new Map<string, Relationship>()
        for (const r of doc ? kids(doc, 'Relationship') : []) {
          const id = r.attrs.Id ?? ''
          const target = r.attrs.Target ?? ''
          const external = r.attrs.TargetMode === 'External'
          out.set(id, { id, type: (r.attrs.Type ?? '').replace(/.*\//, ''), target: external ? target : resolveTarget(key, decodeURIComponent(target)), external })
        }
        return out
      })
      relsCache.set(key, pending)
    }
    return pending
  }
  const typesDoc = await readXml('[Content_Types].xml')
  const defaults = new Map<string, string>()
  const overrides = new Map<string, string>()
  for (const d of kids(typesDoc, 'Default')) defaults.set((d.attrs.Extension ?? '').toLowerCase(), d.attrs.ContentType ?? '')
  for (const o of kids(typesDoc, 'Override')) overrides.set(normalizePath(o.attrs.PartName ?? '').toLowerCase(), o.attrs.ContentType ?? '')
  const contentType = (path: string) => {
    const p = normalizePath(path).toLowerCase()
    return overrides.get(p) ?? defaults.get(/\.([^./]+)$/.exec(p)?.[1] ?? '') ?? null
  }
  const rootRels = await readRels('')
  const officeDoc = [...rootRels.values()].find((r) => r.type === 'officeDocument' && !r.external)
  const main = officeDoc?.target ?? (find('word/document.xml') ? 'word/document.xml' : find('ppt/presentation.xml') ? 'ppt/presentation.xml' : '')
  if (!main || !find(main)) throw new OfficeError('not_office', 'no main document part')
  const kind = MAIN_TYPES[contentType(main) ?? ''] ?? (main.startsWith('word/') ? 'docx' : main.startsWith('ppt/') ? 'pptx' : null)
  return {
    kind, main,
    paths: () => [...entries.keys()],
    has: (path) => find(normalizePath(path)) !== undefined,
    contentType,
    size: (path) => find(normalizePath(path))?.size ?? 0,
    bytes: readBytes,
    xml: readXml,
    rels: readRels,
    relOfType: async (path, type) => [...(await readRels(path)).values()].find((r) => r.type === type),
  }
}

/** `docProps/core.xml` `dc:title`, trimmed (empty when absent). */
export async function coreTitle(pkg: OfficePackage): Promise<string> {
  const rel = await pkg.relOfType('', 'core-properties')
  const doc = await pkg.xml(rel?.target ?? 'docProps/core.xml')
  const title = doc ? kid(doc, 'title') : undefined
  return title ? title.children.filter((c): c is string => typeof c === 'string').join('').trim() : ''
}

/** A picture part's media type, from its content type or extension. */
export function mediaTypeOf(pkg: OfficePackage, path: string): string {
  const ct = pkg.contentType(path) ?? ''
  if (ct.startsWith('image/')) return ct === 'image/jpg' ? 'image/jpeg' : ct === 'image/x-emf' ? 'image/emf' : ct === 'image/x-wmf' ? 'image/wmf' : ct
  const ext = /\.([^./]+)$/.exec(path.toLowerCase())?.[1] ?? ''
  const byExt: Record<string, string> = { png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', jpe: 'image/jpeg', gif: 'image/gif', bmp: 'image/bmp', dib: 'image/bmp', tif: 'image/tiff', tiff: 'image/tiff', emf: 'image/emf', wmf: 'image/wmf', svg: 'image/svg+xml', webp: 'image/webp', wdp: 'image/vnd.ms-photo', jxr: 'image/vnd.ms-photo' }
  return byExt[ext] ?? 'application/octet-stream'
}

/** Pictures every host can upload and show as they are. */
export const WEB_IMAGE = new Set(['image/png', 'image/jpeg', 'image/gif', 'image/webp', 'image/svg+xml'])
