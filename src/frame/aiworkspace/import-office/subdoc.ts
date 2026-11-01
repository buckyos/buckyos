/* One-object sub-documents (Office 导入 TODO §5.3, Q5): what the editable path cannot draw — a chart, SmartArt
 * without its drawing, an OLE object, an EMF/WMF picture, a formula, or a whole slide as a last resort — is copied
 * with every part it reaches into the smallest package that still opens: a docx whose only paragraph holds it on a
 * page of its own size (no margins), or a pptx whose only slide holds it at (0, 0) on a slide of its size with an
 * empty background. The host has LibreOffice turn that into a PDF and renders the page as a picture.
 *
 * Parts are copied by reachability: from the object's relationship ids, then each copied part's relationships that
 * its XML actually mentions; a slide master keeps only the layout in use. The zip is written stored (no compression:
 * the parts are small and LibreOffice reads it at once). */

import { el, toXml, type XmlElement, type XmlNode } from '../import-drawio/xml.ts'
import { kid, kids, local, relsPathOf, type OfficePackage, type Relationship } from './package.ts'
import { EMU_PER_PT } from './units.ts'

// ---- zip (stored)

const CRC_TABLE = (() => {
  const t = new Uint32Array(256)
  for (let n = 0; n < 256; n++) {
    let c = n
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1
    t[n] = c >>> 0
  }
  return t
})()

export function crc32(bytes: Uint8Array): number {
  let c = 0xffffffff
  for (let i = 0; i < bytes.length; i++) c = CRC_TABLE[(c ^ bytes[i]) & 0xff] ^ (c >>> 8)
  return (c ^ 0xffffffff) >>> 0
}

/** A zip of stored entries, in the given order (`[Content_Types].xml` first, as Office writes it). */
export function writeZip(files: { name: string; bytes: Uint8Array }[]): Uint8Array {
  const enc = new TextEncoder()
  const locals: Uint8Array[] = []
  const central: Uint8Array[] = []
  let offset = 0
  for (const f of files) {
    const name = enc.encode(f.name)
    const crc = crc32(f.bytes)
    const head = new Uint8Array(30 + name.length)
    const v = new DataView(head.buffer)
    v.setUint32(0, 0x04034b50, true); v.setUint16(4, 20, true); v.setUint16(6, 0x0800, true); v.setUint16(8, 0, true)
    v.setUint32(14, crc, true); v.setUint32(18, f.bytes.length, true); v.setUint32(22, f.bytes.length, true); v.setUint16(26, name.length, true)
    head.set(name, 30)
    const dir = new Uint8Array(46 + name.length)
    const d = new DataView(dir.buffer)
    d.setUint32(0, 0x02014b50, true); d.setUint16(4, 20, true); d.setUint16(6, 20, true); d.setUint16(8, 0x0800, true)
    d.setUint32(16, crc, true); d.setUint32(20, f.bytes.length, true); d.setUint32(24, f.bytes.length, true); d.setUint16(28, name.length, true)
    d.setUint32(42, offset, true)
    dir.set(name, 46)
    locals.push(head, f.bytes)
    central.push(dir)
    offset += head.length + f.bytes.length
  }
  const dirSize = central.reduce((a, b) => a + b.length, 0)
  const end = new Uint8Array(22)
  const e = new DataView(end.buffer)
  e.setUint32(0, 0x06054b50, true); e.setUint16(8, files.length, true); e.setUint16(10, files.length, true); e.setUint32(12, dirSize, true); e.setUint32(16, offset, true)
  const out = new Uint8Array(offset + dirSize + 22)
  let at = 0
  for (const part of [...locals, ...central, end]) { out.set(part, at); at += part.length }
  return out
}

// ---- copying parts

const XML_HEAD = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
const RELS_NS = 'http://schemas.openxmlformats.org/package/2006/relationships'
const REL = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships'
const enc = new TextEncoder()

/** Relationships a part has without naming them in its XML: a master's theme, a layout's master, a chart's style and
 * colours, a document's styles … (a sub-document without them loses the theme colours a chart is drawn in). */
const IMPLICIT = new Set(['theme', 'themeOverride', 'slideMaster', 'chartStyle', 'chartColorStyle', 'styles', 'stylesWithEffects', 'settings', 'fontTable', 'numbering', 'webSettings', 'diagramDrawing', 'tableStyles'])

/** Relationship ids an element's subtree mentions (`r:embed`, `r:id`, `r:link`, `r:dm` …). */
export function relIdsOf(node: XmlElement, out = new Set<string>()): Set<string> {
  for (const [k, v] of Object.entries(node.attrs)) {
    if (k.startsWith('r:') || (k.includes(':') && /^(embed|id|link|pict|dm|lo|qs|cs|href)$/.test(local(k)) && /^rId|^R[0-9a-f]/.test(v))) out.add(v)
  }
  for (const c of node.children) if (typeof c !== 'string') relIdsOf(c, out)
  return out
}

class PartCopier {
  readonly files = new Map<string, Uint8Array>()
  private readonly pkg: OfficePackage
  constructor(pkg: OfficePackage) { this.pkg = pkg }

  /** Write a part's relationships, keeping only `ids`; returns the kept ones. */
  rels(path: string, all: Map<string, Relationship>, ids: Set<string>, extra: Relationship[] = []): Relationship[] {
    const kept = [...all.values()].filter((r) => ids.has(r.id) || IMPLICIT.has(r.type)).concat(extra)
    const slash = path.lastIndexOf('/')
    const dir = slash < 0 ? '' : path.slice(0, slash + 1)
    const rel = (target: string) => {
      // relative to the source part's folder
      const from = dir.split('/').filter(Boolean)
      const to = target.split('/')
      let common = 0
      while (common < from.length && common < to.length - 1 && from[common] === to[common]) common += 1
      return [...from.slice(common).map(() => '..'), ...to.slice(common)].join('/')
    }
    const body = kept.map((r) => toXml(el('Relationship', { Id: r.id, Type: `${REL}/${r.type}`, Target: r.external ? r.target : rel(r.target), TargetMode: r.external ? 'External' : undefined }))).join('')
    this.files.set(relsPathOf(path), enc.encode(`${XML_HEAD}<Relationships xmlns="${RELS_NS}">${body}</Relationships>`))
    return kept
  }

  /** Copy a part as it is, with the relationships its XML mentions, recursively. `rewrite` may change the XML. */
  async copy(path: string, rewrite?: (doc: XmlElement, rels: Map<string, Relationship>) => Promise<void> | void): Promise<void> {
    if (this.files.has(path)) return
    const bytes = await this.pkg.bytes(path)
    if (!bytes) return
    const type = this.pkg.contentType(path) ?? ''
    const isXml = /\.(xml|rels|vml)$/i.test(path) || type.endsWith('+xml') || type === 'application/xml' || type === 'text/xml'
    if (!isXml) { this.files.set(path, bytes); return }
    const doc = await this.pkg.xml(path)
    if (!doc) { this.files.set(path, bytes); return }
    const all = await this.pkg.rels(path)
    if (rewrite) await rewrite(doc, all)
    this.files.set(path, rewrite ? enc.encode(XML_HEAD + toXml(doc)) : bytes)
    if (all.size === 0) return
    const kept = this.rels(path, all, relIdsOf(doc))
    for (const r of kept) if (!r.external) await this.copy(r.target)
  }

  /** `[Content_Types].xml` reduced to the parts copied. */
  async contentTypes(): Promise<Uint8Array> {
    const doc = await this.pkg.xml('[Content_Types].xml')
    const present = new Set([...this.files.keys()].map((p) => p.toLowerCase()))
    const children = (doc?.children ?? []).filter((c) => typeof c === 'string' || local(c.name) !== 'Override' || present.has((c.attrs.PartName ?? '').replace(/^\//, '').toLowerCase()))
    const root = doc ? { ...doc, children } : el('Types', { xmlns: 'http://schemas.openxmlformats.org/package/2006/content-types' })
    return enc.encode(XML_HEAD + toXml(root))
  }

  async zip(): Promise<Uint8Array> {
    const types = await this.contentTypes()
    return writeZip([{ name: '[Content_Types].xml', bytes: types }, ...[...this.files].map(([name, bytes]) => ({ name, bytes }))])
  }
}

const rootRels = (main: string) => enc.encode(`${XML_HEAD}<Relationships xmlns="${RELS_NS}"><Relationship Id="rId1" Type="${REL}/officeDocument" Target="${main}"/></Relationships>`)

// ---- docx

const PT_TWIPS = 20

/** A drawing anchored to the page, made inline (an anchor's position would put it off the small page). */
function inlineDrawing(drawing: XmlElement): XmlElement {
  const anchor = kid(drawing, 'anchor')
  if (!anchor) return drawing
  const keep = anchor.children.filter((c): c is XmlElement => typeof c !== 'string' && ['extent', 'effectExtent', 'docPr', 'cNvGraphicFramePr', 'graphic'].includes(local(c.name)))
  const prefix = anchor.name.includes(':') ? anchor.name.slice(0, anchor.name.indexOf(':') + 1) : ''
  return { ...drawing, children: [{ name: `${prefix}inline`, attrs: { distT: '0', distB: '0', distL: '0', distR: '0' }, children: keep }] }
}

/** A docx whose only paragraph holds `object` (a `w:drawing`, `w:pict`, `w:object` or an `m:oMath`/`m:oMathPara`)
 * on a page `widthPt` × `heightPt` without margins. */
export async function docxSubdoc(pkg: OfficePackage, object: XmlElement, widthPt: number, heightPt: number): Promise<Uint8Array> {
  const copier = new PartCopier(pkg)
  const main = pkg.main
  const doc = await pkg.xml(main)
  if (!doc) throw new Error('no document part')
  const w = Math.max(72, Math.round(widthPt * PT_TWIPS) + 4)
  const h = Math.max(72, Math.round(heightPt * PT_TWIPS) + 4)
  const name = local(object.name)
  const wp = (n: string, attrs: Record<string, string> = {}, children: XmlNode[] = []): XmlElement => ({ name: `w:${n}`, attrs, children })
  const obj = name === 'drawing' ? inlineDrawing(object) : object
  const inRun = name === 'oMath' || name === 'oMathPara' ? obj : wp('r', {}, [obj])
  const para = wp('p', {}, [wp('pPr', {}, [wp('spacing', { 'w:before': '0', 'w:after': '0', 'w:line': '240', 'w:lineRule': 'auto' }), wp('jc', { 'w:val': 'left' })]), inRun])
  const sect = wp('sectPr', {}, [wp('pgSz', { 'w:w': String(w), 'w:h': String(h) }), wp('pgMar', { 'w:top': '0', 'w:right': '0', 'w:bottom': '0', 'w:left': '0', 'w:header': '0', 'w:footer': '0', 'w:gutter': '0' })])
  const root: XmlElement = { name: doc.name, attrs: { ...doc.attrs }, children: [wp('body', {}, [para, sect])] }
  copier.files.set(main, enc.encode(XML_HEAD + toXml(root)))
  const all = await pkg.rels(main)
  const ids = relIdsOf(obj)
  // styles and theme keep fonts and colours as the document has them
  for (const r of all.values()) if (r.type === 'styles' || r.type === 'theme' || r.type === 'fontTable' || r.type === 'settings') ids.add(r.id)
  const kept = copier.rels(main, all, ids)
  for (const r of kept) if (!r.external) await copier.copy(r.target)
  copier.files.set('_rels/.rels', rootRels(main))
  return copier.zip()
}

// ---- pptx

/** A pptx whose only slide holds `shapes` (from `slidePath`), moved so `box` (EMU) is the slide. */
export async function pptxSubdoc(pkg: OfficePackage, slidePath: string, shapes: XmlElement[], box: { x: number; y: number; w: number; h: number }, keepBackground = false): Promise<Uint8Array> {
  const copier = new PartCopier(pkg)
  const pres = await pkg.xml(pkg.main)
  const slide = await pkg.xml(slidePath)
  if (!pres || !slide) throw new Error('no presentation part')
  const slideRels = await pkg.rels(slidePath)
  const layout = [...slideRels.values()].find((r) => r.type === 'slideLayout')
  const layoutRels = layout ? await pkg.rels(layout.target) : new Map<string, Relationship>()
  const master = [...layoutRels.values()].find((r) => r.type === 'slideMaster')
  const shift = (node: XmlElement): XmlElement => {
    // the shape's own offset moves by the box's corner (groups: their children move with them)
    const copy: XmlElement = JSON.parse(JSON.stringify(node))
    const xfrm = kid(kid(copy, 'spPr'), 'xfrm') ?? kid(kid(copy, 'grpSpPr'), 'xfrm') ?? kid(copy, 'xfrm')
    const off = kid(xfrm, 'off')
    if (off) { off.attrs.x = String(Math.round(Number(off.attrs.x ?? 0) - box.x)); off.attrs.y = String(Math.round(Number(off.attrs.y ?? 0) - box.y)) }
    return copy
  }
  const cSld = kid(slide, 'cSld')
  const tree = kid(cSld, 'spTree')
  const treeHead = (tree?.children ?? []).filter((c): c is XmlElement => typeof c !== 'string' && (local(c.name) === 'nvGrpSpPr' || local(c.name) === 'grpSpPr'))
  const p = (n: string, attrs: Record<string, string> = {}, children: XmlNode[] = []): XmlElement => ({ name: `p:${n}`, attrs, children })
  const a = (n: string): XmlElement => ({ name: `a:${n}`, attrs: {}, children: [] })
  const bg = keepBackground ? kids(cSld, 'bg') : [p('bg', {}, [p('bgPr', {}, [a('noFill'), a('effectLst')])])]
  const newSlide: XmlElement = {
    name: slide.name, attrs: { ...slide.attrs, show: '1' },
    children: [p('cSld', {}, [...bg, p('spTree', {}, [...treeHead, ...shapes.map(shift)])]), ...kids(slide, 'clrMapOvr')],
  }
  // the master first, keeping only the layout in use (the layout would otherwise pull it in whole)
  if (master && layout) {
    await copier.copy(master.target, (doc, rels) => {
      const list = kid(doc, 'sldLayoutIdLst')
      if (!list) return
      list.children = list.children.filter((c) => typeof c === 'string' || rels.get(c.attrs['r:id'] ?? '')?.target === layout.target)
    })
  }
  const newSlidePath = 'ppt/slides/slide1.xml'
  copier.files.set(newSlidePath, enc.encode(XML_HEAD + toXml(newSlide)))
  const slideIds = relIdsOf(newSlide)
  if (layout) slideIds.add(layout.id)
  for (const r of copier.rels(newSlidePath, slideRels, slideIds)) if (!r.external) await copier.copy(r.target)
  // presentation.xml: that master, one slide, the box's size
  const presRels = await pkg.rels(pkg.main)
  const masterRel = [...presRels.values()].find((r) => master && r.target === master.target)
  const themeRel = [...presRels.values()].find((r) => r.type === 'theme')
  const masterIds = kids(kid(pres, 'sldMasterIdLst'), 'sldMasterId')
  const masterId = masterIds.find((m) => masterRel && m.attrs['r:id'] === masterRel.id) ?? masterIds[0]
  const cx = Math.max(914400 / 4, Math.round(box.w)), cy = Math.max(914400 / 4, Math.round(box.h))
  const keep = pres.children.filter((c): c is XmlElement => typeof c !== 'string' && ['defaultTextStyle'].includes(local(c.name)))
  const newPres: XmlElement = {
    name: pres.name, attrs: { ...pres.attrs },
    children: [
      p('sldMasterIdLst', {}, masterId ? [{ ...masterId, attrs: { ...masterId.attrs, 'r:id': 'rIdM' } }] : []),
      p('sldIdLst', {}, [p('sldId', { id: '256', 'r:id': 'rIdS' })]),
      p('sldSz', { cx: String(cx), cy: String(cy) }),
      p('notesSz', { cx: String(cy), cy: String(cx) }),
      ...keep,
    ],
  }
  copier.files.set(pkg.main, enc.encode(XML_HEAD + toXml(newPres)))
  const extra: Relationship[] = [{ id: 'rIdS', type: 'slide', target: newSlidePath, external: false }]
  if (master) extra.push({ id: 'rIdM', type: 'slideMaster', target: master.target, external: false })
  copier.rels(pkg.main, presRels, new Set(themeRel ? [themeRel.id] : []), extra)
  if (themeRel) await copier.copy(themeRel.target)
  copier.files.set('_rels/.rels', rootRels(pkg.main))
  return copier.zip()
}

export const emuToPtBox = (b: { w: number; h: number }) => ({ w: b.w / EMU_PER_PT, h: b.h / EMU_PER_PT })
