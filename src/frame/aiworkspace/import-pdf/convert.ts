/* One PDF page → an import plan (PDF 导入方案 §3, §4): a group the size of the page whose first child is a Frame
 * covering it (the path's step target, the page colour as its fill and `presentation.background`), then — in the
 * order the PDF paints — a raster of what could not be taken apart, the text, pictures and shapes. The plan is the
 * draw.io importer's (`buildOperations` writes it); the host encodes pictures and renders the raster on request.
 *
 *   editable   §4.1–§4.5; text a page cannot be read from (§4.1 第 6 条) → the page whole as a raster with its
 *              pictures still taken out; more paths than `pathLimit` → the vectors go to the raster
 *   raster     the high-fidelity mode (§4.6): one raster of the whole page
 *
 * The raster never holds what became an object: the host renders the page without the operators in `skip`. */

import { ImportReport, type PagePlan, type PlanImage, type PlanItem } from '../import-drawio/convert.ts'
import { normalizeRotation, type Rect } from '../import-drawio/geometry.ts'
import type { Json, PdfPage } from './data.ts'
import { apply, interpret, mul, scaleOf, type ImageMark, type Mat } from './interpret.ts'
import { groupText, readStruct, type Paragraph } from './text.ts'
import { canJoin, candidateOf, intersect, joinGroup, openGroup, primitiveObject, svgObject, type Candidate, type ShapeObject, type SvgGroup } from './vector.ts'
import { pageScale } from './layout.ts'

export interface ImageRequest {
  ref: string
  /** Mirror left to right before use (a picture drawn mirrored). */
  flipX: boolean
  /** The part to keep, as fractions of the picture (top-left origin), or null for all of it. */
  crop: Rect | null
  /** Width / height as drawn: a picture the PDF stretches is stretched the same when encoded. */
  aspect: number
}
export interface EncodedImage { bytes: Uint8Array; mediaType: string }

export interface RasterRequest {
  /** Operators not to draw (they became objects). */
  skip: Set<number>
  /** The whole page (no cropping to what is drawn). */
  whole: boolean
}
/** A rendered raster and where it goes (world units on the page), or null when nothing was drawn. */
export interface RasterResult { bytes: Uint8Array; mediaType: string; rect: Rect }

export interface PdfConvertOptions {
  mode: 'editable' | 'raster'
  unit: 'paragraph' | 'line'
  /** More filled or stroked paths than this: the page's vectors go to the raster (§4.3). */
  pathLimit: number
  /** Unique within the import: plan image keys and SVG element ids start with it. */
  idPrefix: string
  /** The file name without extension, for picture names. */
  fileBase: string
  encodeImage?: (request: ImageRequest) => Promise<EncodedImage | null>
  renderRaster?: (request: RasterRequest) => Promise<RasterResult | null>
  /** The host's words: a page without a heading or bookmark, a picture that could not be stored. */
  pageTitle: (n: number) => string
  imageText?: () => string
}

export interface PdfPagePlan extends PagePlan {
  /** The page's colour (Frame background). */
  pageColor: string
  /** World units per point. */
  scale: number
  counts: { text: number; images: number; shapes: number; raster: boolean }
}

export const PAGE_FRAME_COLOR = '#9ca3af'
const r2 = (v: number) => Math.round(v * 100) / 100
const roundRect = (r: Rect): Rect => ({ x: r2(r.x), y: r2(r.y), w: Math.max(1, r2(r.w)), h: Math.max(1, r2(r.h)) })
const UNREADABLE_SHARE = 0.3

const firstLine = (text: string) => text.split('\n').find((l) => l.trim())?.trim().replace(/^[  ]+/, '').slice(0, 256) ?? ''

/** The world transform of a page (user space → world units, top-left origin). */
export function worldOf(page: PdfPage): { world: Mat; scale: number } {
  const scale = pageScale(page.width, page.height)
  return { world: mul([scale, 0, 0, scale, 0, 0], page.transform), scale }
}

interface Placed { order: number; item: PlanItem; image?: PlanImage; kind: 'text' | 'image' | 'shape' }

function textItem(p: Paragraph, key: string, parent: string, source: Record<string, Json>): PlanItem {
  const config: Record<string, Json> = {
    shape: 'text', fill: 'none', stroke: 'none', text: p.text, font_size: r2(p.fontSize), font_color: p.style.color, align: p.align, valign: 'top', wrap: false,
    ...(p.style.bold ? { bold: true } : {}), ...(p.style.italic ? { italic: true } : {}), ...(p.style.family !== 'sans' ? { font_family: p.style.family } : {}),
    ...(p.lineHeight !== null ? { line_height: p.lineHeight } : {}), ...(p.style.alpha < 0.995 ? { opacity: r2(p.style.alpha) } : {}),
    pdf_source: { ...source, kind: 'text', ...(p.role ? { role: p.role } : {}) },
  }
  return { key, parent, type: 'shape', rect: roundRect(p.rect), rotation: normalizeRotation(p.rotation), config, ...(firstLine(p.text) ? { title: firstLine(p.text) } : {}) }
}

function shapeItem(o: ShapeObject, key: string, parent: string): PlanItem {
  return { key, parent, type: 'shape', rect: roundRect(o.rect), rotation: normalizeRotation(o.rotation), config: o.config }
}

/** A picture's box and how it is turned or mirrored; null when it is sheared too much to place. */
function placeImage(mark: ImageMark): { rect: Rect; rotation: number; flipX: boolean; sheared: boolean } {
  const [c0, c1, c2, c3] = mark.corners
  // the picture's top row is at unit y = 1: top-left c3, top-right c2, bottom-left c0
  let tl = c3, tr = c2, bl = c0
  let ux = tr.x - tl.x, uy = tr.y - tl.y, vx = bl.x - tl.x, vy = bl.y - tl.y
  const flipX = ux * vy - uy * vx < 0
  if (flipX) { tl = c2; tr = c3; bl = c1; ux = tr.x - tl.x; uy = tr.y - tl.y; vx = bl.x - tl.x; vy = bl.y - tl.y }
  const w = Math.hypot(ux, uy), h = Math.hypot(vx, vy)
  const cx = (c0.x + c1.x + c2.x + c3.x) / 4, cy = (c0.y + c1.y + c2.y + c3.y) / 4
  const sheared = w > 0 && h > 0 && Math.abs(ux * vx + uy * vy) / (w * h) > 0.03
  if (sheared) {
    const xs = mark.corners.map((p) => p.x), ys = mark.corners.map((p) => p.y)
    const x = Math.min(...xs), y = Math.min(...ys)
    return { rect: { x, y, w: Math.max(...xs) - x, h: Math.max(...ys) - y }, rotation: 0, flipX, sheared }
  }
  return { rect: { x: cx - w / 2, y: cy - h / 2, w, h }, rotation: normalizeRotation((Math.atan2(uy, ux) * 180) / Math.PI), flipX, sheared }
}

/** Convert one page. Pictures and the raster go through the host's callbacks. */
export async function convertPdfPage(page: PdfPage, report: ImportReport, options: PdfConvertOptions): Promise<PdfPagePlan> {
  const { world, scale } = worldOf(page)
  const n = page.number
  // whole world units: a page and the stage of its proportions match exactly
  const W = Math.round(page.width * scale), H = Math.round(page.height * scale)
  const pageRect: Rect = { x: 0, y: 0, w: W, h: H }
  const groupKey = `p${n}`
  const source = { page: n }
  const items: PlanItem[] = []
  const images: PlanImage[] = []
  const counts = { text: 0, images: 0, shapes: 0, raster: false }
  const unit = scaleOf(world)
  const marks = interpret(page, world)
  const struct = readStruct(page.struct)
  const skip = new Set<number>()

  // the page colour: full-page solid fills before anything else is drawn (§3)
  let pageColor = '#ffffff'
  const firstOther = Math.min(marks.runs.find((r) => !r.blank)?.index ?? Infinity, marks.images[0]?.index ?? Infinity, marks.other[0] ?? Infinity)
  for (const p of marks.paths) {
    if (p.index > firstOther) break
    const full = p.fill?.kind === 'color' && !p.stroke && p.fillAlpha >= 0.995 && p.complex === null && p.box.w * p.box.h >= 0.95 * W * H && p.clips.every((c) => c.rect)
    if (!full) break
    pageColor = (p.fill as { color: string }).color
    skip.add(p.index)
  }

  const text = groupText(page, world, marks.runs, struct, { unit: options.unit })
  const unreadable = text.total >= 10 && text.unreadable / text.total > UNREADABLE_SHARE
  const title = text.paragraphs.find((p) => p.role && /^(H[1-6]?|Title)$/.test(p.role))?.text ?? page.bookmark ?? null
  const frameTitle = (title ? firstLine(title) : '') || page.bookmark || options.pageTitle(n)
  items.push({ key: groupKey, parent: null, type: 'group', rect: pageRect, rotation: 0, config: {} })
  items.push({ key: `${groupKey}~frame`, parent: groupKey, type: 'frame', rect: pageRect, rotation: 0, title: frameTitle, config: { color: PAGE_FRAME_COLOR, fill: pageColor, stroke_width: 1, radius: 0, pdf_source: { ...source, kind: 'page' } }, payload: { presentation: { background: pageColor } } })

  const result = (raster: PlanItem | null): PdfPagePlan => {
    if (raster) items.splice(2, 0, raster)
    report.pages += 1
    report.objects += items.length - 1
    report.images += images.length
    return { id: groupKey, name: frameTitle, background: pageColor, pageSize: { w: W, h: H }, items, bounds: pageRect, images, pageColor, scale, counts }
  }
  const rasterItem = async (request: RasterRequest): Promise<PlanItem | null> => {
    if (!options.renderRaster) return null
    const got = await options.renderRaster(request)
    if (!got) return null
    const image: PlanImage = { key: `${options.idPrefix}p${n}r`, name: `${options.fileBase}-p${String(n).padStart(3, '0')}-raster.png`, mediaType: got.mediaType, bytes: got.bytes }
    images.push(image)
    counts.raster = true
    return { key: `${groupKey}~raster`, parent: groupKey, type: 'asset', rect: roundRect(got.rect), rotation: 0, config: { pdf_source: { ...source, kind: 'raster' } }, image, fallback: { config: { shape: 'rect', fill: 'none', stroke: 'auto', stroke_width: 1, dash: 'dashed', text: options.imageText?.() ?? 'Image', font_size: 14 } } }
  }

  if (options.mode === 'raster') {
    return result(await rasterItem({ skip: new Set(), whole: true }))
  }

  // pictures (§4.2)
  const placed: Placed[] = []
  const imageCache = new Map<string, PlanImage | null>()
  let imageCount = 0
  for (const mark of marks.images) {
    if (mark.complex) continue
    const at = placeImage(mark)
    if (at.sheared) report.add('pdf_image_sheared', 'Sheared pictures imported upright in their bounding box', `p${n}`)
    if (at.rect.w * at.rect.h < 4 * unit * unit) continue
    if (mark.alpha < 0.995) { report.add('pdf_image_alpha', 'Translucent pictures kept in the page raster', `p${n}`); continue }
    const cuts = mark.clips.filter((c) => !(c.rect && c.box.x <= at.rect.x + 0.5 && c.box.y <= at.rect.y + 0.5 && c.box.x + c.box.w >= at.rect.x + at.rect.w - 0.5 && c.box.y + c.box.h >= at.rect.y + at.rect.h - 0.5))
    let crop: Rect | null = null
    let rect = at.rect
    if (cuts.length) {
      if (!cuts.every((c) => c.rect) || at.rotation !== 0 || at.sheared) { report.add('pdf_image_clipped', 'Pictures cut by a shape kept in the page raster', `p${n}`); continue }
      let cut: Rect | null = at.rect
      for (const c of cuts) cut = cut && intersect(cut, c.box)
      if (!cut) continue
      crop = { x: (cut.x - at.rect.x) / at.rect.w, y: (cut.y - at.rect.y) / at.rect.h, w: cut.w / at.rect.w, h: cut.h / at.rect.h }
      rect = cut
    }
    const aspect = Math.round((rect.w / Math.max(rect.h, 0.01)) * 1000) / 1000
    const cacheKey = `${mark.ref}|${at.flipX}|${aspect}|${crop ? [crop.x, crop.y, crop.w, crop.h].map((v) => v.toFixed(4)).join(',') : ''}`
    let image = imageCache.get(cacheKey)
    if (image === undefined) {
      const encoded = options.encodeImage ? await options.encodeImage({ ref: mark.ref, flipX: at.flipX, crop, aspect }).catch(() => null) : null
      imageCount += 1
      image = encoded ? { key: `${options.idPrefix}p${n}i${imageCount}`, name: `${options.fileBase}-p${String(n).padStart(3, '0')}-img${imageCount}.${encoded.mediaType === 'image/jpeg' ? 'jpg' : 'png'}`, mediaType: encoded.mediaType, bytes: encoded.bytes } : null
      imageCache.set(cacheKey, image)
      if (image) images.push(image)
    }
    if (!image) { report.add('pdf_image_unreadable', 'Pictures that could not be decoded kept in the page raster', `p${n}`); continue }
    const alt = mark.mcid !== null ? struct.figureAlt.get(mark.mcid) : undefined
    placed.push({
      order: mark.index, kind: 'image', image,
      item: { key: `${groupKey}~i${placed.length}`, parent: groupKey, type: 'asset', rect: roundRect(rect), rotation: at.rotation, config: { pdf_source: { ...source, kind: 'image' } }, ...(alt ? { title: alt.slice(0, 256) } : {}), image,
        fallback: { config: { shape: 'rect', fill: 'none', stroke: 'auto', stroke_width: 1, dash: 'dashed', text: options.imageText?.() ?? 'Image', font_size: 14, pdf_source: { ...source, kind: 'image' } } } },
    })
    skip.add(mark.index)
  }

  if (unreadable) {
    // §4.1 第 6 条: the whole page as a raster (its text in it), the pictures still taken out
    report.add('pdf_unreadable_text', 'Pages whose text cannot be read imported as pictures (pictures on them still separate)', `p${n}`)
    const raster = await rasterItem({ skip, whole: true })
    for (const p of placed) { items.push(p.item); counts.images += 1 }
    return result(raster)
  }

  // text (§4.1)
  const links = page.links.map((l) => {
    const a = apply(world, l.rect[0], l.rect[1]), b = apply(world, l.rect[2], l.rect[3])
    return { rect: { x: Math.min(a.x, b.x), y: Math.min(a.y, b.y), w: Math.abs(a.x - b.x), h: Math.abs(a.y - b.y) }, page: l.page, url: l.url }
  })
  for (const [k, p] of text.paragraphs.entries()) {
    // the link covering most of the paragraph's text (boxes of neighbouring lines overlap a little)
    const area = (r: Rect | null) => (r ? r.w * r.h : 0)
    const link = links.map((l) => ({ l, a: area(intersect(l.rect, p.rect)) })).filter(({ l, a }) => a > 0.3 * Math.min(area(p.rect), area(l.rect))).sort((x, y) => y.a - x.a)[0]?.l
    const item = textItem(p, `${groupKey}~t${k}`, groupKey, { ...source, ...(link ? { link: link.page ? { page: link.page } : { url: link.url } } : {}) })
    placed.push({ order: p.order, item, kind: 'text' })
    if (p.mixed) report.add('pdf_mixed_style', 'Paragraphs with several styles imported in their main style', firstLine(p.text).slice(0, 48))
    for (const r of p.runs) skip.add(r)
  }
  if (text.invisible) report.add('pdf_invisible_text', 'Invisible text (a scan’s recognised text) not imported', `p${n}`)
  if (text.vertical) report.add('pdf_vertical_text', 'Vertical text kept in the page raster', `p${n}`)

  // shapes (§4.3), joined in paint order; pictures and text in between close an SVG shape
  const paths = marks.paths.filter((p) => !skip.has(p.index) && p.complex === null)
  const candidates: Candidate[] = []
  if (paths.length > options.pathLimit) report.add('pdf_complex_page', 'Pages with very many paths: their drawing kept in the page raster', `p${n} (${paths.length})`)
  else for (const p of paths) { const c = candidateOf(p, unit); if (c) candidates.push(c) }
  if (marks.paths.some((p) => p.complex !== null && p.complex !== 'annotation')) report.add('pdf_effects', 'Patterns, masks, blend modes and mesh shadings kept in the page raster', `p${n}`)
  const shapes: ShapeObject[] = []
  let group: SvgGroup | null = null
  const ids = `${options.idPrefix}${n}_`
  let svgCount = 0
  const close = () => {
    if (!group) return
    const g = group
    group = null
    if (g.members.length === 1 && g.members[0].prim) { shapes.push(primitiveObject(g.members[0], source)); return }
    const one = svgObject(g, page.patterns, `${ids}${svgCount++}`, source)
    if (one) { shapes.push(one); return }
    for (const m of g.members) {
      if (m.prim) { shapes.push(primitiveObject(m, source)); continue }
      const single = svgObject(openGroup(m), page.patterns, `${ids}${svgCount++}`, source)
      if (single) shapes.push(single)
      else report.add('pdf_svg_too_large', 'Drawings too large for an SVG shape kept in the page raster', `p${n}`)
    }
  }
  const breaks = placed.map((p) => p.order).sort((a, b) => a - b)
  let b = 0
  for (const c of candidates) {
    while (b < breaks.length && breaks[b] < c.mark.index) { close(); b += 1 }
    if (c.prim && !c.small) { close(); shapes.push(primitiveObject(c, source)); continue }
    if (group && canJoin(group, c, unit)) joinGroup(group, c)
    else { close(); group = openGroup(c) }
  }
  close()
  // a filled rectangle and the outline drawn right after it on its edge are one rectangle
  for (let k = shapes.length - 2; k >= 0; k--) {
    const a = shapes[k], b = shapes[k + 1]
    const ac = a.config, bc = b.config
    const sw = Number(bc.stroke_width) || 0
    if (a.kind === 'rect' && b.kind === 'rect' && ac.stroke === 'none' && bc.fill === 'none' && ac.opacity === undefined && bc.opacity === undefined
      && Math.abs(a.rect.x - b.rect.x) <= sw && Math.abs(a.rect.y - b.rect.y) <= sw && Math.abs(a.rect.x + a.rect.w - b.rect.x - b.rect.w) <= sw && Math.abs(a.rect.y + a.rect.h - b.rect.y - b.rect.h) <= sw) {
      shapes.splice(k, 2, { ...b, ops: [...a.ops, ...b.ops], order: a.order, config: { ...bc, fill: ac.fill } })
    }
  }
  for (const [k, o] of shapes.entries()) {
    placed.push({ order: o.order, item: shapeItem(o, `${groupKey}~s${k}`, groupKey), kind: 'shape' })
    for (const op of o.ops) skip.add(op)
  }

  placed.sort((a, b) => a.order - b.order)
  for (const p of placed) {
    items.push(p.item)
    if (p.kind === 'text') counts.text += 1
    else if (p.kind === 'image') counts.images += 1
    else counts.shapes += 1
  }

  // the raster: whatever is still drawn (§4.5)
  const onPage = (r: Rect) => intersect(r, pageRect) !== null
  const left = marks.paths.some((p) => !skip.has(p.index) && (p.fill?.kind !== 'none' || p.stroke?.kind !== 'none') && (p.shading && !p.segs.length ? true : onPage({ ...p.box, w: Math.max(p.box.w, 1), h: Math.max(p.box.h, 1) })))
    || marks.runs.some((r) => !skip.has(r.index) && !r.blank && (r.mode & 3) !== 3 && onPage({ x: Math.min(r.start.x, r.end.x) - r.size, y: Math.min(r.start.y, r.end.y) - r.size, w: Math.abs(r.end.x - r.start.x) + 2 * r.size, h: Math.abs(r.end.y - r.start.y) + 2 * r.size }))
    || marks.images.some((m) => !skip.has(m.index)) || marks.other.length > 0
  const raster = left ? await rasterItem({ skip, whole: false }) : null
  if (raster) report.add('pdf_raster_parts', 'Parts of pages that cannot be taken apart kept as a picture under the objects', `p${n}`)
  return result(raster)
}
