/* PresentationML → canvas pages (Office 导入 TODO §4): every slide one page plan of the PDF importer's shape — a group
 * the size of the slide, a Frame covering it (the page colour, `presentation.background`, the slide's title as its
 * name), then what the slide shows in paint order: the master's and the layout's own shapes, then the slide's.
 *
 *   layout       slide size → long side 1920 world units (`pageScale`); groups flattened through chOff / chExt,
 *                their turn and mirror applied to the children; `show="0"` slides left out unless asked for
 *   inheritance  a placeholder takes position, geometry, fill, line, body properties and list styles from the
 *                layout's placeholder of the same idx (else type) and the master's of the same type; text styles
 *                come down defaultTextStyle → master title / body / other style → placeholder list styles → own
 *   colours      the theme through the master's colour map (layout / slide overrides); a shape's style matrix
 *                (`p:style` fillRef / lnRef / fontRef) for what the shape does not say
 *   shapes       rect / roundRect / ellipse / diamond → those kinds; a straight line without arrows → a line;
 *                every other preset, custom geometry, gradients and arrowheads → an SVG shape (geometry.ts);
 *                text one style per shape (the first run's), bullets and numbers as prefixes, autofit applied
 *   pictures     png / jpeg / gif / webp / svg (svgBlip preferred); crops by the host (`cropImage`); EMF / WMF /
 *                TIFF through `rasterize`, else a dashed box with the file name
 *   tables       a grid of rectangles with their text; fills from the cell or the table style (tableStyles.xml)
 *   charts …     charts, SmartArt without its drawing, OLE without a preview, through `rasterize` (§5.3), else a
 *                dashed box; SmartArt with its drawing (dsp:drawing, written since Office 2010) as shapes
 *   dropped      notes (reported), animations, transitions, effects (shadow, glow, 3-D), hyperlinks */

import { ImportReport, type Json, type PlanImage, type PlanItem } from '../import-drawio/convert.ts'
import { normalizeRotation, type Rect } from '../import-drawio/geometry.ts'
import type { XmlElement } from '../import-drawio/xml.ts'
import { pageScale } from '../import-pdf/layout.ts'
import { adjustOf, customDef, evaluate, presetPaths, type GeomPath } from './geometry.ts'
import { attr, coreTitle, descendants, kid, kidPath, kids, local, mediaTypeOf, WEB_IMAGE, type OfficePackage, type Relationship } from './package.ts'
import { extensionOf, fileBaseOf, type EncodedImage, type OfficePagePlan, type Rasterize } from './plan.ts'
import { pptxSubdoc } from './subdoc.ts'
import { readParagraphs, readLevels, runColor, runFace, shapeText, type LevelStyles, type TextPara } from './text.ts'
import { colorIn, readClrMap, readTheme, type ColorContext, type Rgba, type Theme } from './theme.ts'
import { angle, EMU_PER_PT, fraction, int, onOff } from './units.ts'

export const PAGE_FRAME_COLOR = '#9ca3af'
/** More objects than this on a slide: the slide whole as a picture when it can be had, else the first ones (§4.4). */
export const SLIDE_OBJECT_LIMIT = 2000
/** An SVG body larger than this is not kept as a shape (the Block's config is limited to 64 KiB). */
const SVG_LIMIT = 48 * 1024

export interface CropRequest { bytes: Uint8Array; mediaType: string; /** Fractions to cut from each side. */ crop: { l: number; t: number; r: number; b: number } }

export interface PptxOptions {
  fileName: string
  /** Unique within the import: plan image keys and SVG element ids start with it. */
  idPrefix: string
  rasterize?: Rasterize
  /** Cuts a picture (`a:srcRect`); absent or null: the whole picture is kept and the crop reported. */
  cropImage?: (request: CropRequest) => Promise<EncodedImage | null>
  /** The host's words: a slide without a title, a picture or object that could not be had. */
  pageTitle: (n: number) => string
  placeholderText?: (kind: 'image' | 'chart' | 'smartart' | 'ole' | 'media' | 'object') => string
}

export interface DeckSlide { number: number; path: string; hidden: boolean }
export interface Deck {
  pkg: OfficePackage
  /** Slide size in EMU. */
  width: number
  height: number
  slides: DeckSlide[]
  title: string
  defaultText: LevelStyles
}

/** The relationship id of a list entry (`r:id`, whatever the prefix; `id` alone is the entry's number). */
const relIdOf = (el: XmlElement) => Object.entries(el.attrs).find(([k]) => k.includes(':') && local(k) === 'id' && !k.startsWith('xmlns'))?.[1] ?? ''

export async function readDeck(pkg: OfficePackage): Promise<Deck> {
  const pres = await pkg.xml(pkg.main)
  if (!pres) throw new Error('no presentation part')
  const rels = await pkg.rels(pkg.main)
  const size = kid(pres, 'sldSz')
  const slides: DeckSlide[] = []
  for (const s of kids(kid(pres, 'sldIdLst'), 'sldId')) {
    const rel = rels.get(relIdOf(s))
    if (!rel || rel.external) continue
    const doc = await pkg.xml(rel.target)
    if (!doc) continue
    slides.push({ number: slides.length + 1, path: rel.target, hidden: attr(doc, 'show') === '0' || attr(doc, 'show') === 'false' })
  }
  return { pkg, width: int(attr(size, 'cx'), 12192000), height: int(attr(size, 'cy'), 6858000), slides, title: await coreTitle(pkg), defaultText: readLevels(kid(pres, 'defaultTextStyle')) }
}

// ---- layers

interface Layer { kind: 'slide' | 'layout' | 'master'; path: string; doc: XmlElement; rels: Map<string, Relationship>; tree: XmlElement | undefined }
interface Master { layer: Layer; theme: Theme; themeDoc: XmlElement | null; /** The theme part (its fill styles' pictures resolve against it). */ themeLayer: Layer | null; clrMap: Record<string, string>; title: LevelStyles; body: LevelStyles; other: LevelStyles }

async function layerOf(pkg: OfficePackage, path: string, kind: Layer['kind']): Promise<Layer | null> {
  const doc = await pkg.xml(path)
  if (!doc) return null
  return { kind, path, doc, rels: await pkg.rels(path), tree: kidPath(doc, 'cSld', 'spTree') }
}

const nvOf = (el: XmlElement) => kid(el, 'nvSpPr') ?? kid(el, 'nvPicPr') ?? kid(el, 'nvGraphicFramePr') ?? kid(el, 'nvCxnSpPr') ?? kid(el, 'nvGrpSpPr')
interface Ph { type: string; idx: string | null }
function phOf(el: XmlElement): Ph | null {
  const ph = kidPath(nvOf(el), 'nvPr', 'ph')
  return ph ? { type: attr(ph, 'type') ?? 'obj', idx: attr(ph, 'idx') ?? null } : null
}
const masterType = (t: string) => (t === 'ctrTitle' || t === 'title' ? 'title' : t === 'dt' || t === 'ftr' || t === 'sldNum' || t === 'hdr' ? t : 'body')

function findPh(layer: Layer | null | undefined, ph: Ph, master: boolean): XmlElement | undefined {
  if (!layer?.tree) return undefined
  const all = layer.tree.children.filter((c): c is XmlElement => typeof c !== 'string' && phOf(c) !== null)
  if (master) return all.find((c) => masterType(phOf(c)!.type) === masterType(ph.type))
  if (ph.idx !== null) { const byIdx = all.find((c) => phOf(c)!.idx === ph.idx); if (byIdx) return byIdx }
  return all.find((c) => { const t = phOf(c)!.type; return t === ph.type || (masterType(t) === 'title' && masterType(ph.type) === 'title') })
}

// ---- transforms

interface Box { x: number; y: number; w: number; h: number; rot: number; flipH: boolean; flipV: boolean }
type Xf = (b: Box) => Box

function boxOfXfrm(xfrm: XmlElement | undefined): Box | null {
  if (!xfrm) return null
  const off = kid(xfrm, 'off'), ext = kid(xfrm, 'ext')
  if (!off || !ext) return null
  return { x: int(attr(off, 'x')), y: int(attr(off, 'y')), w: int(attr(ext, 'cx')), h: int(attr(ext, 'cy')), rot: angle(attr(xfrm, 'rot')), flipH: onOff(attr(xfrm, 'flipH'), attr(xfrm, 'flipH') !== undefined), flipV: onOff(attr(xfrm, 'flipV'), attr(xfrm, 'flipV') !== undefined) }
}

function rotateAbout(px: number, py: number, cx: number, cy: number, deg: number): [number, number] {
  const a = (deg * Math.PI) / 180
  const dx = px - cx, dy = py - cy
  return [cx + dx * Math.cos(a) - dy * Math.sin(a), cy + dx * Math.sin(a) + dy * Math.cos(a)]
}

/** Children of a group into the group's parent: scaled by ext / chExt, mirrored and turned about its centre. */
function groupXf(xfrm: XmlElement | undefined, parent: Xf): Xf {
  const g = boxOfXfrm(xfrm)
  if (!g) return parent
  const chOff = kid(xfrm, 'chOff'), chExt = kid(xfrm, 'chExt')
  const cx0 = int(attr(chOff, 'x'), g.x), cy0 = int(attr(chOff, 'y'), g.y)
  const cw = int(attr(chExt, 'cx'), g.w) || 1, ch = int(attr(chExt, 'cy'), g.h) || 1
  const sx = g.w / cw, sy = g.h / ch
  const gcx = g.x + g.w / 2, gcy = g.y + g.h / 2
  return (b) => {
    let w = b.w * sx, h = b.h * sy
    let cx = g.x + (b.x + b.w / 2 - cx0) * sx, cy = g.y + (b.y + b.h / 2 - cy0) * sy
    let rot = b.rot, flipH = b.flipH, flipV = b.flipV
    // a turned child's box keeps its own proportions under uneven scaling only approximately
    if (Math.abs(normalizeRotation(b.rot)) % 180 === 90) { w = b.w * sy; h = b.h * sx }
    if (g.flipH) { cx = 2 * gcx - cx; flipH = !flipH; rot = -rot }
    if (g.flipV) { cy = 2 * gcy - cy; flipV = !flipV; rot = -rot }
    if (g.rot) { [cx, cy] = rotateAbout(cx, cy, gcx, gcy, g.rot); rot += g.rot }
    return parent({ x: cx - w / 2, y: cy - h / 2, w, h, rot, flipH, flipV })
  }
}

// ---- paint

type Paint =
  | { kind: 'none' }
  | { kind: 'solid'; color: Rgba }
  | { kind: 'gradient'; stops: { pos: number; color: Rgba }[]; angle: number; radial: boolean }
  | { kind: 'picture'; blipFill: XmlElement; layer: Layer }

interface Stroke { color: Rgba | null; width: number; dash: 'solid' | 'dashed' | 'dotted'; head: string; tail: string }

const FILL_NAMES = new Set(['noFill', 'solidFill', 'gradFill', 'blipFill', 'pattFill', 'grpFill'])

interface SlideCtx {
  deck: Deck
  master: Master
  layout: Layer | null
  slide: Layer
  colors: ColorContext
  /** World units per EMU, per point. */
  k: number
  perPt: number
  items: PlanItem[]
  images: PlanImage[]
  report: ImportReport
  options: PptxOptions
  n: number
  groupKey: string
  counts: OfficePagePlan['counts']
  next: number
  svgIds: number
  imageCache: Map<string, PlanImage | null>
  tableStyles: XmlElement | null
}

const r2 = (v: number) => Math.round(v * 100) / 100
const hexOf = (c: Rgba | null | undefined) => (c ? c.hex : 'none')

function themeStyle(ctx: SlideCtx, list: 'fillStyleLst' | 'lnStyleLst' | 'bgFillStyleLst', idx: number): XmlElement | undefined {
  const scheme = kidPath(ctx.master.themeDoc ?? undefined, 'themeElements', 'fmtScheme')
  const entries = (kid(scheme, list)?.children ?? []).filter((c): c is XmlElement => typeof c !== 'string')
  return entries[idx - 1]
}

function paintOfFill(fill: XmlElement, ctx: SlideCtx, colors: ColorContext, layer: Layer, groupFill: Paint | null): Paint {
  switch (local(fill.name)) {
    case 'noFill': return { kind: 'none' }
    case 'solidFill': { const c = colorIn(fill, colors); return c ? { kind: 'solid', color: c } : { kind: 'none' } }
    case 'gradFill': {
      const stops = kids(kid(fill, 'gsLst'), 'gs').map((gs) => ({ pos: fraction(attr(gs, 'pos')), color: colorIn(gs, colors) })).filter((s): s is { pos: number; color: Rgba } => s.color !== null).sort((a, b) => a.pos - b.pos)
      if (!stops.length) return { kind: 'none' }
      const lin = kid(fill, 'lin')
      return { kind: 'gradient', stops, angle: lin ? angle(attr(lin, 'ang')) : 90, radial: !lin && Boolean(kid(fill, 'path')) }
    }
    case 'pattFill': {
      ctx.report.add('pptx_pattern_fill', 'Pattern fills imported as their foreground colour')
      const c = colorIn(kid(fill, 'fgClr'), colors)
      return c ? { kind: 'solid', color: c } : { kind: 'none' }
    }
    case 'blipFill': return { kind: 'picture', blipFill: fill, layer }
    case 'grpFill': return groupFill ?? { kind: 'none' }
  }
  return { kind: 'none' }
}

/** A shape's fill: its own (nearest spPr first), else its style's fillRef. */
function paintOf(spPrs: (XmlElement | undefined)[], style: XmlElement | undefined, ctx: SlideCtx, layer: Layer, groupFill: Paint | null): Paint {
  for (const spPr of spPrs) {
    const fill = spPr?.children.find((c): c is XmlElement => typeof c !== 'string' && FILL_NAMES.has(local(c.name)))
    if (fill) return paintOfFill(fill, ctx, ctx.colors, layer, groupFill)
  }
  const ref = kid(style, 'fillRef')
  const idx = int(attr(ref, 'idx'))
  if (!ref || idx === 0) return { kind: 'none' }
  const entry = idx >= 1001 ? themeStyle(ctx, 'bgFillStyleLst', idx - 1000) : themeStyle(ctx, 'fillStyleLst', idx)
  const phClr = colorIn(ref, ctx.colors)
  if (!entry) return phClr ? { kind: 'solid', color: phClr } : { kind: 'none' }
  return paintOfFill(entry, ctx, { ...ctx.colors, phClr }, ctx.master.themeLayer ?? layer, groupFill)
}

function strokeOf(spPrs: (XmlElement | undefined)[], style: XmlElement | undefined, ctx: SlideCtx): Stroke {
  const ln = spPrs.map((s) => kid(s, 'ln')).find(Boolean)
  const ref = kid(style, 'lnRef')
  const refIdx = int(attr(ref, 'idx'))
  const themeLn = ref && refIdx > 0 ? themeStyle(ctx, 'lnStyleLst', refIdx) : undefined
  const refColors = { ...ctx.colors, phClr: colorIn(ref, ctx.colors) }
  const pick = (name: string) => kid(ln, name) ?? kid(themeLn, name)
  const fillEl = ln?.children.find((c): c is XmlElement => typeof c !== 'string' && FILL_NAMES.has(local(c.name))) ?? themeLn?.children.find((c): c is XmlElement => typeof c !== 'string' && FILL_NAMES.has(local(c.name)))
  const fromTheme = fillEl !== undefined && !ln?.children.some((c) => c === fillEl)
  let color: Rgba | null = null
  if (fillEl && local(fillEl.name) === 'solidFill') color = colorIn(fillEl, fromTheme ? refColors : ctx.colors)
  else if (fillEl && local(fillEl.name) === 'gradFill') color = colorIn(kid(kid(fillEl, 'gsLst'), 'gs'), fromTheme ? refColors : ctx.colors)
  else if (!fillEl && ref && refIdx > 0) color = refColors.phClr ?? null
  const width = int(attr(ln, 'w'), int(attr(themeLn, 'w'), 9525))
  const dashVal = attr(pick('prstDash'), 'val') ?? 'solid'
  const dash = dashVal === 'solid' ? 'solid' : /dot/i.test(dashVal) && !/dash/i.test(dashVal) ? 'dotted' : 'dashed'
  return { color: ln || themeLn || ref ? color : null, width, dash, head: attr(kid(ln, 'headEnd'), 'type') ?? 'none', tail: attr(kid(ln, 'tailEnd'), 'type') ?? 'none' }
}

// ---- text

const SERIF = /times|georgia|garamond|cambria|palatino|book antiqua|century|baskerville|serif|宋|明|song|mincho|fangsong|仿宋|楷|kai/i
const MONO = /courier|consol|mono|menlo|code|typewriter/i
const familyOf = (face: string) => (MONO.test(face) ? 'mono' : SERIF.test(face) ? 'serif' : null)

interface TextLook { config: Record<string, Json>; text: string; mixed: boolean }

/** The text colour: the shape's own (its runs, paragraphs, list style), else its style's font colour (`fontRef`, a
 * table style's text colour), else what the master and the defaults give, else tx1. */
function textLook(paras: TextPara[], ownParas: TextPara[], bodyPr: Record<string, string>, autofit: { font: number; spacing: number }, fontRefColor: Rgba | null, ctx: SlideCtx): TextLook | null {
  const st = shapeText(paras)
  if (st.empty) return null
  const size = (st.style.size ?? 18) * autofit.font
  const color = runColor(shapeText(ownParas).style, ctx.colors) ?? fontRefColor ?? runColor(st.style, ctx.colors) ?? colorIn({ name: 'a:schemeClr', attrs: { val: 'tx1' }, children: [] }, ctx.colors)
  const face = runFace(st.style, st.text, ctx.master.theme)
  const family = familyOf(face)
  const config: Record<string, Json> = {
    text: st.text, font_size: r2(Math.max(4, size * ctx.perPt)), font_color: hexOf(color),
    align: st.align === 'ctr' ? 'center' : st.align === 'r' ? 'right' : 'left',
    valign: bodyPr.anchor === 'ctr' ? 'middle' : bodyPr.anchor === 'b' ? 'bottom' : 'top',
    ...(st.style.bold ? { bold: true } : {}), ...(st.style.italic ? { italic: true } : {}), ...(st.style.underline ? { underline: true } : {}),
    ...(family ? { font_family: family } : {}),
    ...(bodyPr.wrap === 'none' ? { wrap: false } : {}),
  }
  const ls = st.lineSpacing
  if (ls && 'pct' in ls) config.line_height = r2(Math.min(3, Math.max(0.8, ls.pct * 1.2 * autofit.spacing)))
  else if (ls && 'pt' in ls) config.line_height = r2(Math.min(3, Math.max(0.8, ls.pt / Math.max(1, size))))
  if (bodyPr.vert && bodyPr.vert !== 'horz') ctx.report.add('pptx_vertical_text', 'Vertical text imported horizontally', st.text.slice(0, 32))
  return { config, text: st.text, mixed: st.mixed }
}

// ---- items

const firstLine = (text: string) => text.split('\n').find((l) => l.trim())?.trim().replace(/^[  ]+/, '').slice(0, 256) ?? ''

function toWorld(b: Box, ctx: SlideCtx): Rect {
  return { x: r2(b.x * ctx.k), y: r2(b.y * ctx.k), w: Math.max(1, r2(b.w * ctx.k)), h: Math.max(1, r2(b.h * ctx.k)) }
}

function push(ctx: SlideCtx, item: Omit<PlanItem, 'key' | 'parent'>, kind: 'text' | 'images' | 'shapes' | 'placeholders') {
  ctx.items.push({ key: `${ctx.groupKey}~${ctx.next++}`, parent: ctx.groupKey, ...item })
  ctx.counts[kind] += 1
}

function mix(hex: string, with_: string, k: number): string {
  const a = parseInt(hex.slice(1), 16), b = parseInt(with_.slice(1), 16)
  const ch = (s: number) => Math.round(((a >> s) & 255) * (1 - k) + ((b >> s) & 255) * k).toString(16).padStart(2, '0')
  return `#${ch(16)}${ch(8)}${ch(0)}`
}

/** Points of an SVG path's first and last segments (for arrowheads). */
function ends(d: string): { start: [number, number]; startDir: [number, number]; end: [number, number]; endDir: [number, number] } | null {
  const pts: [number, number][] = []
  for (const m of d.matchAll(/([MLQCA])([^MLQCAZ]*)/g)) {
    const nums = m[2].trim().split(/[\s,]+/).filter(Boolean).map(Number)
    if (m[1] === 'A') pts.push([nums[5], nums[6]])
    else for (let i = 0; i + 1 < nums.length; i += 2) pts.push([nums[i], nums[i + 1]])
  }
  if (pts.length < 2) return null
  return { start: pts[0], startDir: pts[1], end: pts[pts.length - 1], endDir: pts[pts.length - 2] }
}

function arrowHead(tip: [number, number], from: [number, number], size: number, kind: string): string {
  let dx = tip[0] - from[0], dy = tip[1] - from[1]
  const len = Math.hypot(dx, dy) || 1
  dx /= len; dy /= len
  const bx = tip[0] - dx * size, by = tip[1] - dy * size
  const px = -dy * size * 0.5, py = dx * size * 0.5
  if (kind === 'oval') return `<circle cx="${r2(tip[0])}" cy="${r2(tip[1])}" r="${r2(size / 2)}" fill="var(--aiws-shape-stroke)"/>`
  if (kind === 'diamond') return `<path d="M${r2(tip[0] + dx * size / 2)} ${r2(tip[1] + dy * size / 2)}L${r2(tip[0] + px)} ${r2(tip[1] + py)}L${r2(tip[0] - dx * size / 2)} ${r2(tip[1] - dy * size / 2)}L${r2(tip[0] - px)} ${r2(tip[1] - py)}Z" fill="var(--aiws-shape-stroke)"/>`
  if (kind === 'arrow') return `<path d="M${r2(bx + px)} ${r2(by + py)}L${r2(tip[0])} ${r2(tip[1])}L${r2(bx - px)} ${r2(by - py)}" fill="none" stroke="var(--aiws-shape-stroke)" stroke-linejoin="round"/>`
  return `<path d="M${r2(tip[0])} ${r2(tip[1])}L${r2(bx + px)} ${r2(by + py)}L${r2(bx - px)} ${r2(by - py)}Z" fill="var(--aiws-shape-stroke)"/>`
}

/** An SVG shape body from evaluated paths. */
function svgBody(paths: GeomPath[], paint: Paint, stroke: Stroke, strokeWidth: number, ctx: SlideCtx): { body: string; fill: string; pad: number } {
  const defs: string[] = []
  let fillRef = 'none'
  let base = 'none'
  if (paint.kind === 'solid') { fillRef = 'var(--aiws-shape-fill)'; base = paint.color.hex }
  else if (paint.kind === 'gradient') {
    const id = `${ctx.options.idPrefix}${ctx.n}_g${ctx.svgIds++}`
    const stops = paint.stops.map((s) => `<stop offset="${r2(s.pos)}" stop-color="${s.color.hex}"${s.color.alpha < 0.995 ? ` stop-opacity="${r2(s.color.alpha)}"` : ''}/>`).join('')
    if (paint.radial) defs.push(`<radialGradient id="${id}" cx="0.5" cy="0.5" r="0.7">${stops}</radialGradient>`)
    else {
      const a = (paint.angle * Math.PI) / 180
      const c = Math.cos(a) / 2, s = Math.sin(a) / 2
      defs.push(`<linearGradient id="${id}" x1="${r2(0.5 - c)}" y1="${r2(0.5 - s)}" x2="${r2(0.5 + c)}" y2="${r2(0.5 + s)}">${stops}</linearGradient>`)
    }
    fillRef = `url(#${id})`
    base = paint.stops[0].color.hex
  }
  const dash = stroke.dash === 'dashed' ? ` stroke-dasharray="${r2(strokeWidth * 4)} ${r2(strokeWidth * 3)}"` : stroke.dash === 'dotted' ? ` stroke-dasharray="${r2(strokeWidth)} ${r2(strokeWidth * 2)}"` : ''
  const parts: string[] = []
  const fillAlpha = paint.kind === 'solid' && paint.color.alpha < 0.995 ? ` fill-opacity="${r2(paint.color.alpha)}"` : ''
  for (const p of paths) {
    let f = 'none'
    if (p.fill !== 'none' && base !== 'none') {
      if (p.fill === 'norm') f = fillRef
      else f = mix(base, p.fill.startsWith('lighten') ? '#ffffff' : '#000000', p.fill.endsWith('Less') ? 0.2 : 0.4)
    }
    const s = p.stroke && stroke.color ? ` stroke="var(--aiws-shape-stroke)" stroke-width="${r2(strokeWidth)}"${dash}${stroke.color.alpha < 0.995 ? ` stroke-opacity="${r2(stroke.color.alpha)}"` : ''}` : ''
    parts.push(`<path d="${p.d}" fill="${f}"${f === fillRef ? fillAlpha : ''}${s}/>`)
  }
  // arrowheads at the first path's ends
  let pad = stroke.color ? strokeWidth / 2 : 0
  const heads = stroke.color && (stroke.head !== 'none' || stroke.tail !== 'none') ? ends(paths.find((p) => p.stroke)?.d ?? '') : null
  if (heads) {
    const size = Math.max(strokeWidth * 3.5, 6)
    if (stroke.head !== 'none') parts.push(arrowHead(heads.start, heads.startDir, size, stroke.head))
    if (stroke.tail !== 'none') parts.push(arrowHead(heads.end, heads.endDir, size, stroke.tail))
    pad = Math.max(pad, size)
  }
  const inner = `${defs.length ? `<defs>${defs.join('')}</defs>` : ''}${parts.join('')}`
  return { body: pad > 0 ? `<g transform="translate(${r2(pad)} ${r2(pad)})">${inner}</g>` : inner, fill: base, pad }
}

interface ShapeInput {
  el: XmlElement
  layer: Layer
  box: Box
  spPrs: (XmlElement | undefined)[]
  style: XmlElement | undefined
  bodyPr: Record<string, string>
  autofit: { font: number; spacing: number }
  paras: TextPara[]
  /** The paragraphs with only the shape's own styles (for the colour's precedence). */
  ownParas: TextPara[]
  geom: XmlElement | undefined
  placeholder: boolean
  name: string
  groupFill: Paint | null
  /** The text frame of a SmartArt shape, when it differs from the shape. */
  textBox?: Box | null
}

async function shapeItem(s: ShapeInput, ctx: SlideCtx): Promise<void> {
  const paint = paintOf(s.spPrs, s.style, ctx, s.layer, s.groupFill)
  const stroke = strokeOf(s.spPrs, s.style, ctx)
  const fontRefColor = colorIn(kid(s.style, 'fontRef'), ctx.colors)
  const look = textLook(s.paras, s.ownParas, s.bodyPr, s.autofit, fontRefColor, ctx)
  if (look?.mixed) ctx.report.add('pptx_mixed_style', 'Text with several styles in one shape imported in its first style', firstLine(look.text).slice(0, 48))
  const prst = local(s.geom?.name ?? '') === 'prstGeom' ? attr(s.geom, 'prst') ?? 'rect' : local(s.geom?.name ?? '') === 'custGeom' ? 'custom' : 'rect'
  if (paint.kind === 'picture') {
    // a picture-filled shape: the picture in the shape's box (the outline is not kept)
    if (prst !== 'rect') ctx.report.add('pptx_picture_fill', 'Pictures filling a shape imported in its bounding box', s.name)
    await pictureItem({ ...s, blipFill: paint.blipFill, layer: paint.layer }, ctx)
    if (look) push(ctx, { type: 'shape', rect: toWorld(s.box, ctx), rotation: normalizeRotation(s.box.rot), config: { shape: 'text', fill: 'none', stroke: 'none', ...look.config, office_source: { slide: ctx.n, kind: 'text' } }, title: firstLine(look.text) }, 'text')
    return
  }
  const visibleFill = paint.kind !== 'none'
  const visibleStroke = stroke.color !== null && stroke.width > 0
  if (!look && !visibleFill && !visibleStroke) return
  const rect = toWorld(s.box, ctx)
  const rotation = normalizeRotation(s.box.rot)
  const strokeWidth = visibleStroke ? Math.max(0.5, stroke.width * ctx.k) : 0
  const source = { slide: ctx.n, kind: look ? 'text' : 'shape', ...(s.layer.kind !== 'slide' ? { layer: s.layer.kind } : {}) }
  const title = look ? firstLine(look.text) : undefined
  const common: Record<string, Json> = {
    fill: paint.kind === 'solid' ? paint.color.hex : 'none', stroke: hexOf(stroke.color), stroke_width: r2(strokeWidth),
    ...(stroke.dash !== 'solid' ? { dash: stroke.dash } : {}),
    ...(paint.kind === 'solid' && paint.color.alpha < 0.995 && !visibleStroke && !look ? { opacity: r2(paint.color.alpha) } : {}),
    ...(look?.config ?? {}), office_source: source,
  }
  const kindOf = look && !visibleFill && !visibleStroke && (prst === 'rect' || prst === 'roundRect') ? 'text' : null
  const isLine = prst === 'line' || prst === 'straightConnector1'
  const arrows = stroke.head !== 'none' || stroke.tail !== 'none'
  if (kindOf === 'text') {
    // the text frame's insets moved into the box (the canvas draws text at the box's edge)
    const ins = (k: string, d: number) => int(s.bodyPr[k], d)
    const tb = s.textBox ?? s.box
    const inset = { ...tb, x: tb.x + ins('lIns', 91440), y: tb.y + ins('tIns', 45720), w: Math.max(EMU_PER_PT, tb.w - ins('lIns', 91440) - ins('rIns', 91440)), h: Math.max(EMU_PER_PT, tb.h - ins('tIns', 45720) - ins('bIns', 45720)) }
    push(ctx, { type: 'shape', rect: toWorld(inset, ctx), rotation, config: { ...common, shape: 'text', fill: 'none', stroke: 'none' }, title }, 'text')
    return
  }
  if (isLine && !arrows && paint.kind !== 'gradient') {
    // a straight line: its two ends, turned and mirrored as the shape is
    const b = s.box
    let [x1, y1, x2, y2] = [b.x, b.y, b.x + b.w, b.y + b.h]
    if (b.flipH) [x1, x2] = [x2, x1]
    if (b.flipV) [y1, y2] = [y2, y1]
    const cx = b.x + b.w / 2, cy = b.y + b.h / 2
    ;[x1, y1] = rotateAbout(x1, y1, cx, cy, b.rot)
    ;[x2, y2] = rotateAbout(x2, y2, cx, cy, b.rot)
    const len = Math.hypot(x2 - x1, y2 - y1) * ctx.k
    const thick = Math.max(4, strokeWidth * 2)
    const mx = ((x1 + x2) / 2) * ctx.k, my = ((y1 + y2) / 2) * ctx.k
    push(ctx, { type: 'shape', rect: { x: r2(mx - len / 2), y: r2(my - thick / 2), w: Math.max(1, r2(len)), h: r2(thick) }, rotation: normalizeRotation((Math.atan2(y2 - y1, x2 - x1) * 180) / Math.PI), config: { ...common, shape: 'line', fill: 'none' } }, 'shapes')
    return
  }
  const basic: Record<string, string> = { rect: 'rect', roundRect: 'rect', ellipse: 'ellipse', diamond: 'diamond' }
  if (basic[prst] && paint.kind !== 'gradient') {
    const radius = prst === 'roundRect' ? r2(Math.min(rect.w, rect.h) * fraction(adjustOf(s.geom).adj?.replace(/^val\s+/, ''), 0.16667)) : 0
    push(ctx, { type: 'shape', rect, rotation, config: { ...common, shape: basic[prst], ...(radius ? { radius } : {}) }, ...(title ? { title } : {}) }, look ? 'text' : 'shapes')
    return
  }
  // anything else: the geometry as an SVG body
  const w = rect.w, h = rect.h
  const paths = prst === 'custom' && s.geom ? evaluate(customDef(s.geom), w, h, {}, s.box.flipH, s.box.flipV) : presetPaths(prst, w, h, adjustOf(s.geom), s.box.flipH, s.box.flipV)
  if (!paths || !paths.length) {
    ctx.report.add('pptx_unknown_geometry', 'Shapes of unknown geometry imported as rectangles', prst)
    push(ctx, { type: 'shape', rect, rotation, config: { ...common, shape: 'rect' }, ...(title ? { title } : {}) }, look ? 'text' : 'shapes')
    return
  }
  const svg = svgBody(paths, paint, stroke, strokeWidth, ctx)
  if (svg.body.length > SVG_LIMIT) {
    ctx.report.add('pptx_svg_too_large', 'Shapes too complex for an SVG shape imported as rectangles', s.name)
    push(ctx, { type: 'shape', rect, rotation, config: { ...common, shape: 'rect' }, ...(title ? { title } : {}) }, look ? 'text' : 'shapes')
    return
  }
  const pad = svg.pad
  const padded = { x: r2(rect.x - pad), y: r2(rect.y - pad), w: r2(rect.w + 2 * pad), h: r2(rect.h + 2 * pad) }
  push(ctx, { type: 'shape', rect: padded, rotation, config: { ...common, fill: svg.fill, shape: 'svg', svg: { viewBox: `0 0 ${padded.w} ${padded.h}`, body: svg.body, aspect: 'stretch' } }, ...(title ? { title } : {}) }, look ? 'text' : 'shapes')
}

// ---- pictures

async function imageOf(ctx: SlideCtx, layer: Layer, relId: string): Promise<{ path: string; bytes: Uint8Array; mediaType: string } | null> {
  const rel = layer.rels.get(relId)
  if (!rel || rel.external) return null
  const bytes = await ctx.deck.pkg.bytes(rel.target)
  return bytes ? { path: rel.target, bytes, mediaType: mediaTypeOf(ctx.deck.pkg, rel.target) } : null
}

/** A copy of a shape placed at `box` on the slide (a grouped shape is moved out of its group's coordinates). */
function placed(el: XmlElement, box: Box): XmlElement {
  const copy: XmlElement = JSON.parse(JSON.stringify(el))
  const holder = kid(copy, 'spPr') ?? kid(copy, 'grpSpPr')
  let xfrm = kid(holder, 'xfrm') ?? kid(copy, 'xfrm')
  const prefix = local(copy.name) === 'graphicFrame' ? 'p' : 'a'
  if (!xfrm) {
    xfrm = { name: `${prefix}:xfrm`, attrs: {}, children: [] }
    if (holder) holder.children.unshift(xfrm)
    else copy.children.splice(1, 0, xfrm)
  }
  xfrm.attrs = { ...(box.rot ? { rot: String(Math.round(box.rot * 60000)) } : {}), ...(box.flipH ? { flipH: '1' } : {}), ...(box.flipV ? { flipV: '1' } : {}) }
  xfrm.children = [{ name: 'a:off', attrs: { x: String(Math.round(box.x)), y: String(Math.round(box.y)) }, children: [] }, { name: 'a:ext', attrs: { cx: String(Math.round(box.w)), cy: String(Math.round(box.h)) }, children: [] }]
  return copy
}

async function rasterItem(ctx: SlideCtx, kind: 'chart' | 'smartart' | 'ole' | 'image' | 'media', el: XmlElement, layer: Layer, box: Box, name: string, title?: string): Promise<boolean> {
  if (!ctx.options.rasterize || layer.kind !== 'slide') return false
  let subdoc: Uint8Array
  try { subdoc = await pptxSubdoc(ctx.deck.pkg, layer.path, [placed(el, { ...box, rot: 0 })], { x: box.x, y: box.y, w: box.w, h: box.h }) } catch { return false }
  const got = await ctx.options.rasterize({ kind: kind === 'media' ? 'ole' : kind, subdoc, subdocKind: 'pptx', width: box.w / EMU_PER_PT, height: box.h / EMU_PER_PT, name, trim: false }).catch(() => null)
  if (!got) return false
  const key = `${ctx.options.idPrefix}s${ctx.n}r${ctx.counts.rasterized + 1}`
  const image: PlanImage = { key, name: `${fileBaseOf(ctx.options.fileName)}-s${String(ctx.n).padStart(3, '0')}-${kind}${ctx.counts.rasterized + 1}.${extensionOf(got.mediaType)}`, mediaType: got.mediaType, bytes: got.bytes }
  ctx.images.push(image)
  ctx.counts.rasterized += 1
  ctx.report.add('pptx_rasterized', 'Charts, diagrams and pictures in other formats imported as pictures (LibreOffice)', name)
  ctx.items.push({ key: `${ctx.groupKey}~${ctx.next++}`, parent: ctx.groupKey, type: 'asset', rect: toWorld(box, ctx), rotation: normalizeRotation(box.rot), config: { office_source: { slide: ctx.n, kind } }, image, ...(title ? { title } : {}), fallback: placeholderConfig(ctx, kind, name) })
  ctx.counts.images += 1
  return true
}

function placeholderConfig(ctx: SlideCtx, kind: 'image' | 'chart' | 'smartart' | 'ole' | 'media' | 'object', name: string): { config: Record<string, Json>; title?: string } {
  const label = ctx.options.placeholderText?.(kind) ?? kind
  return { config: { shape: 'rect', fill: 'none', stroke: '#9ca3af', stroke_width: 1, dash: 'dashed', text: name ? `${label}\n${name}` : label, font_size: r2(12 * ctx.perPt), font_color: '#6b7280', office_source: { slide: ctx.n, kind: 'placeholder' } } }
}

function placeholderItem(ctx: SlideCtx, kind: 'image' | 'chart' | 'smartart' | 'ole' | 'media' | 'object', box: Box, name: string) {
  const p = placeholderConfig(ctx, kind, name)
  push(ctx, { type: 'shape', rect: toWorld(box, ctx), rotation: normalizeRotation(box.rot), config: p.config }, 'placeholders')
}

async function pictureItem(s: ShapeInput & { blipFill: XmlElement }, ctx: SlideCtx): Promise<void> {
  const blip = kid(s.blipFill, 'blip')
  const embed = attr(blip, 'embed') ?? attr(blip, 'link') ?? ''
  const svgBlip = descendants(blip ?? s.blipFill, 'svgBlip')[0]
  const src = kid(s.blipFill, 'srcRect')
  const crop = src ? { l: fraction(attr(src, 'l')), t: fraction(attr(src, 't')), r: fraction(attr(src, 'r')), b: fraction(attr(src, 'b')) } : null
  const cropping = crop && (crop.l > 0.001 || crop.t > 0.001 || crop.r > 0.001 || crop.b > 0.001) && crop.l >= 0 && crop.t >= 0 && crop.r >= 0 && crop.b >= 0
  const descr = attr(kid(nvOf(s.el), 'cNvPr'), 'descr') || undefined
  const source = (!cropping && svgBlip ? await imageOf(ctx, s.layer, attr(svgBlip, 'embed') ?? '') : null) ?? (embed ? await imageOf(ctx, s.layer, embed) : null)
  if (!source) {
    if (attr(blip, 'link')) ctx.report.add('pptx_external_image', 'Linked (not embedded) pictures shown as boxes', s.name)
    placeholderItem(ctx, 'image', s.box, s.name)
    return
  }
  const fileName = source.path.replace(/.*\//, '')
  const cacheKey = `${source.path}|${cropping ? JSON.stringify(crop) : ''}`
  let image = ctx.imageCache.get(cacheKey)
  if (image === undefined) {
    image = null
    if (WEB_IMAGE.has(source.mediaType) || source.mediaType === 'image/bmp') {
      let bytes = source.bytes, mediaType = source.mediaType
      if (cropping && crop) {
        const cut = ctx.options.cropImage ? await ctx.options.cropImage({ bytes, mediaType, crop }).catch(() => null) : null
        if (cut) { bytes = cut.bytes; mediaType = cut.mediaType } else ctx.report.add('pptx_crop_kept', 'Cropped pictures imported whole', fileName)
      }
      const n = ctx.images.length + 1
      image = { key: `${ctx.options.idPrefix}s${ctx.n}i${n}`, name: `${fileBaseOf(ctx.options.fileName)}-s${String(ctx.n).padStart(3, '0')}-img${n}.${extensionOf(mediaType)}`, mediaType, bytes }
      ctx.images.push(image)
    }
    ctx.imageCache.set(cacheKey, image)
  }
  if (!image) {
    if (await rasterItem(ctx, 'image', s.el, s.layer, s.box, fileName, descr)) return
    ctx.report.add('pptx_unsupported_image', 'Pictures in formats a browser cannot show (EMF, WMF, TIFF) shown as boxes', fileName)
    placeholderItem(ctx, 'image', s.box, fileName)
    return
  }
  if (s.box.flipH || s.box.flipV) ctx.report.add('pptx_image_flip', 'Mirrored pictures imported unmirrored', fileName)
  ctx.items.push({ key: `${ctx.groupKey}~${ctx.next++}`, parent: ctx.groupKey, type: 'asset', rect: toWorld(s.box, ctx), rotation: normalizeRotation(s.box.rot), config: { office_source: { slide: ctx.n, kind: 'image' } }, image, ...(descr ? { title: descr.slice(0, 256) } : {}), fallback: placeholderConfig(ctx, 'image', fileName) })
  ctx.counts.images += 1
}

// ---- tables

function tableStyleOf(ctx: SlideCtx, id: string): XmlElement | undefined {
  return kids(ctx.tableStyles ?? undefined, 'tblStyle').find((s) => attr(s, 'styleId') === id)
}

function cellPaint(part: XmlElement | undefined, ctx: SlideCtx): Paint | null {
  const fill = kidPath(part, 'tcStyle', 'fill')
  const solid = kid(fill, 'solidFill')
  if (solid) { const c = colorIn(solid, ctx.colors); return c ? { kind: 'solid', color: c } : null }
  if (kid(fill, 'noFill')) return { kind: 'none' }
  const ref = kidPath(part, 'tcStyle', 'fillRef')
  if (ref) { const c = colorIn(ref, ctx.colors); return c ? { kind: 'solid', color: c } : null }
  return null
}

/** A cell's border: its own (`a:lnL` … `a:lnB`), else the table style's (`tcBdr`); null when it has none. One
 * colour and width for all four sides (a rectangle Block has one outline). */
function borderOf(tcPr: XmlElement | undefined, parts: (XmlElement | undefined)[], ctx: SlideCtx): { color: Rgba; width: number } | null {
  const fromLn = (ln: XmlElement | undefined): { color: Rgba; width: number } | null | undefined => {
    if (!ln) return undefined
    if (kid(ln, 'noFill')) return null
    const c = colorIn(kid(ln, 'solidFill'), ctx.colors)
    return c ? { color: c, width: int(attr(ln, 'w'), 12700) } : undefined
  }
  for (const side of ['lnB', 'lnT', 'lnL', 'lnR']) { const b = fromLn(kid(tcPr, side)); if (b !== undefined) return b }
  for (const part of parts) {
    const bdr = kidPath(part, 'tcStyle', 'tcBdr')
    for (const side of ['bottom', 'top', 'left', 'right', 'insideH', 'insideV']) {
      const edge = kid(bdr, side)
      if (!edge) continue
      const b = fromLn(kid(edge, 'ln'))
      if (b !== undefined) return b
      const ref = kid(edge, 'lnRef')
      const c = ref ? colorIn(ref, ctx.colors) : null
      if (c) return { color: c, width: 12700 }
    }
  }
  return null
}

async function tableItems(frame: XmlElement, tbl: XmlElement, box: Box, ctx: SlideCtx): Promise<void> {
  const tblPr = kid(tbl, 'tblPr')
  const flags = { firstRow: onOff(attr(tblPr, 'firstRow'), attr(tblPr, 'firstRow') !== undefined), bandRow: onOff(attr(tblPr, 'bandRow'), attr(tblPr, 'bandRow') !== undefined), firstCol: onOff(attr(tblPr, 'firstCol'), attr(tblPr, 'firstCol') !== undefined) }
  const styleId = kid(tblPr, 'tableStyleId')?.children.filter((c): c is string => typeof c === 'string').join('').trim() ?? ''
  const style = styleId ? tableStyleOf(ctx, styleId) : undefined
  const cols = kids(kid(tbl, 'tblGrid'), 'gridCol').map((c) => int(attr(c, 'w')))
  const rows = kids(tbl, 'tr')
  const heights = rows.map((r) => int(attr(r, 'h')))
  const sumW = cols.reduce((a, b) => a + b, 0) || 1, sumH = heights.reduce((a, b) => a + b, 0) || 1
  const sx = box.w > 0 ? box.w / sumW : 1, sy = box.h > sumH ? box.h / sumH : 1
  const xs = [0], ys = [0]
  for (const w of cols) xs.push(xs[xs.length - 1] + w * sx)
  for (const h of heights) ys.push(ys[ys.length - 1] + h * sy)
  const fallbackStyle = !style && styleId !== ''
  if (fallbackStyle) ctx.report.add('pptx_table_style', 'Table styles not in the file approximated', styleId)
  const accent = ctx.master.theme.colors.get('accent1') ?? '#4472c4'
  for (const [r, tr] of rows.entries()) {
    let c = 0
    for (const tc of kids(tr, 'tc')) {
      const col = c
      c += 1
      if (onOff(attr(tc, 'hMerge'), attr(tc, 'hMerge') !== undefined) || onOff(attr(tc, 'vMerge'), attr(tc, 'vMerge') !== undefined)) continue
      const span = Math.max(1, int(attr(tc, 'gridSpan'), 1)), rspan = Math.max(1, int(attr(tc, 'rowSpan'), 1))
      const x = xs[Math.min(col, xs.length - 1)], x2 = xs[Math.min(col + span, xs.length - 1)]
      const y = ys[Math.min(r, ys.length - 1)], y2 = ys[Math.min(r + rspan, ys.length - 1)]
      const cell: Box = { x: box.x + x, y: box.y + y, w: Math.max(1, x2 - x), h: Math.max(1, y2 - y), rot: 0, flipH: false, flipV: false }
      const header = flags.firstRow && r === 0
      const tcPr = kid(tc, 'tcPr')
      const own = tcPr?.children.find((x2_): x2_ is XmlElement => typeof x2_ !== 'string' && FILL_NAMES.has(local(x2_.name)))
      let paint: Paint | null = own ? paintOfFill(own, ctx, ctx.colors, ctx.slide, null) : null
      if (!paint && style) {
        const band = flags.bandRow && (r - (flags.firstRow ? 1 : 0)) % 2 === 0 ? kid(style, 'band1H') : null
        paint = (header ? cellPaint(kid(style, 'firstRow'), ctx) : null) ?? (band ? cellPaint(band, ctx) : null) ?? cellPaint(kid(style, 'wholeTbl'), ctx)
      }
      if (!paint && fallbackStyle) paint = { kind: 'solid', color: { hex: header ? accent : mix(accent, '#ffffff', (r - (flags.firstRow ? 1 : 0)) % 2 === 0 ? 0.8 : 0.9), alpha: 1 } }
      const txStyle = header && style ? kidPath(style, 'firstRow', 'tcTxStyle') : style ? kidPath(style, 'wholeTbl', 'tcTxStyle') : undefined
      const txColor = colorIn(txStyle, ctx.colors) ?? (header && fallbackStyle ? { hex: '#ffffff', alpha: 1 } : null)
      const paras = readParagraphs(kid(tc, 'txBody'), [ctx.deck.defaultText, ctx.master.other])
      const bold = header && (attr(txStyle, 'b') === 'on' || fallbackStyle)
      const look = textLook(paras, readParagraphs(kid(tc, 'txBody'), []), { anchor: attr(tcPr, 'anchor') ?? 't' }, { font: 1, spacing: 1 }, txColor, ctx)
      if (look && bold) look.config.bold = true
      const parts = style ? [header ? kid(style, 'firstRow') : undefined, kid(style, 'band1H'), kid(style, 'wholeTbl')] : []
      const border = borderOf(tcPr, parts, ctx) ?? (fallbackStyle ? { color: { hex: '#ffffff', alpha: 1 }, width: 12700 } : null)
      const config: Record<string, Json> = {
        shape: 'rect', fill: paint?.kind === 'solid' ? paint.color.hex : 'none', stroke: border ? border.color.hex : 'none', stroke_width: border ? r2(Math.max(0.5, border.width * ctx.k)) : 0,
        ...(look?.config ?? {}), office_source: { slide: ctx.n, kind: 'table' },
      }
      push(ctx, { type: 'shape', rect: toWorld(cell, ctx), rotation: 0, config, ...(look ? { title: firstLine(look.text) } : {}) }, look ? 'text' : 'shapes')
    }
  }
  void frame
}

// ---- graphic frames

async function graphicFrame(el: XmlElement, layer: Layer, xf: Xf, ctx: SlideCtx): Promise<void> {
  const raw = boxOfXfrm(kid(el, 'xfrm'))
  if (!raw) return
  const box = xf(raw)
  const data = kidPath(el, 'graphic', 'graphicData')
  const uri = attr(data, 'uri') ?? ''
  const name = attr(kid(nvOf(el), 'cNvPr'), 'name') ?? ''
  const descr = attr(kid(nvOf(el), 'cNvPr'), 'descr') || undefined
  if (/\/table$/.test(uri)) {
    const tbl = kid(data, 'tbl')
    if (tbl) await tableItems(el, tbl, box, ctx)
    return
  }
  if (/chart/.test(uri)) {
    if (await rasterItem(ctx, 'chart', el, layer, box, name, descr)) return
    ctx.report.add('pptx_chart', 'Charts shown as boxes (no LibreOffice to draw them)', name)
    placeholderItem(ctx, 'chart', box, name)
    return
  }
  if (/diagram/.test(uri)) {
    // SmartArt: the drawing PowerPoint saves next to the data (Office 2010+), placed in the frame
    const relIds = kid(data, 'relIds')
    const dmRel = layer.rels.get(relIds?.attrs['r:dm'] ?? attr(relIds, 'dm') ?? '')
    const dm = dmRel ? await ctx.deck.pkg.xml(dmRel.target) : null
    const ext = dm ? descendants(dm, 'dataModelExt')[0] : undefined
    const drawingRel = layer.rels.get(attr(ext, 'relId') ?? '') ?? [...layer.rels.values()].find((r) => r.type === 'diagramDrawing' && dmRel && r.target.replace(/drawing/, 'data') === dmRel.target)
    const drawing = drawingRel ? await ctx.deck.pkg.xml(drawingRel.target) : null
    const tree = drawing ? kid(drawing, 'spTree') : undefined
    if (tree) {
      const inner: Xf = (b) => xf({ ...b, x: raw.x + b.x, y: raw.y + b.y })
      const drawingLayer: Layer = { ...layer, rels: drawingRel ? await ctx.deck.pkg.rels(drawingRel.target) : layer.rels }
      for (const c of tree.children) if (typeof c !== 'string') await element(c, drawingLayer, inner, ctx, null)
      return
    }
    if (await rasterItem(ctx, 'smartart', el, layer, box, name, descr)) return
    ctx.report.add('pptx_smartart', 'SmartArt diagrams without their drawing shown as boxes', name)
    placeholderItem(ctx, 'smartart', box, name)
    return
  }
  if (/ole/.test(uri)) {
    // an OLE object: its preview picture
    const pic = data ? descendants(data, 'pic')[0] : undefined
    if (pic) { await element(pic, layer, (b) => (b.w || b.h ? xf(b) : box), ctx, null, box); ctx.report.add('pptx_ole', 'Embedded objects (OLE) imported as their preview pictures', name); return }
    if (await rasterItem(ctx, 'ole', el, layer, box, name, descr)) return
    ctx.report.add('pptx_ole_missing', 'Embedded objects without a preview shown as boxes', name)
    placeholderItem(ctx, 'ole', box, name)
    return
  }
  ctx.report.add('pptx_object', 'Other embedded objects shown as boxes', uri.replace(/.*\//, ''))
  placeholderItem(ctx, 'object', box, name)
}

// ---- shapes of a tree

function bodyPrOf(chain: (XmlElement | undefined)[]): { attrs: Record<string, string>; autofit: { font: number; spacing: number } } {
  const attrs: Record<string, string> = {}
  let autofit = { font: 1, spacing: 1 }
  for (const b of [...chain].reverse()) {
    if (!b) continue
    for (const [k, v] of Object.entries(b.attrs)) if (!k.startsWith('xmlns')) attrs[local(k)] = v
    const norm = kid(b, 'normAutofit')
    if (norm) autofit = { font: fraction(attr(norm, 'fontScale'), 1), spacing: 1 - fraction(attr(norm, 'lnSpcReduction'), 0) }
    else if (kid(b, 'noAutofit') || kid(b, 'spAutoFit')) autofit = { font: 1, spacing: 1 }
  }
  return { attrs, autofit }
}

async function element(el: XmlElement, layer: Layer, xf: Xf, ctx: SlideCtx, groupFill: Paint | null, forcedBox?: Box): Promise<void> {
  const name = local(el.name)
  const cNvPr = kid(nvOf(el) ?? el, 'cNvPr')
  if (cNvPr && onOff(attr(cNvPr, 'hidden'), attr(cNvPr, 'hidden') !== undefined)) return
  if (name === 'AlternateContent') {
    const pick = kid(el, 'Fallback') ?? kid(el, 'Choice')
    for (const c of pick?.children ?? []) if (typeof c !== 'string') await element(c, layer, xf, ctx, groupFill)
    return
  }
  if (name === 'grpSp') {
    const grpSpPr = kid(el, 'grpSpPr')
    const inner = groupXf(kid(grpSpPr, 'xfrm'), xf)
    const fill = grpSpPr?.children.find((c): c is XmlElement => typeof c !== 'string' && FILL_NAMES.has(local(c.name)))
    const gf = fill ? paintOfFill(fill, ctx, ctx.colors, layer, groupFill) : groupFill
    for (const c of el.children) if (typeof c !== 'string') await element(c, layer, inner, ctx, gf)
    return
  }
  if (name === 'graphicFrame') { await graphicFrame(el, layer, xf, ctx); return }
  if (name === 'contentPart') { ctx.report.add('pptx_ink', 'Ink not imported'); return }
  if (name !== 'sp' && name !== 'cxnSp' && name !== 'pic') return
  const ph = phOf(el)
  // a master's or layout's placeholders show on a slide only through the slide's own
  if (ph && layer.kind !== 'slide') return
  const layoutPh = ph ? findPh(ctx.layout, ph, false) : undefined
  const masterPh = ph ? findPh(ctx.master.layer, layoutPh ? phOf(layoutPh) ?? ph : ph, true) : undefined
  const spPr = kid(el, 'spPr')
  const spPrs = [spPr, kid(layoutPh, 'spPr'), kid(masterPh, 'spPr')]
  const xfrmEl = spPrs.map((s) => kid(s, 'xfrm')).find((x) => boxOfXfrm(x) !== null)
  const own = boxOfXfrm(kid(spPr, 'xfrm'))
  // a placeholder's position comes from the layout or master in their own (slide) coordinates
  const raw = own ? xf(own) : xfrmEl ? boxOfXfrm(xfrmEl) : null
  const box = forcedBox ?? raw
  if (!box) return
  const geom = spPrs.map((s) => kid(s, 'prstGeom') ?? kid(s, 'custGeom')).find(Boolean)
  const style = kid(el, 'style') ?? kid(layoutPh, 'style') ?? kid(masterPh, 'style')
  const shapeName = attr(cNvPr, 'name') ?? name
  if (name === 'pic') {
    const blipFill = kid(el, 'blipFill')
    if (!blipFill) return
    if (kidPath(nvOf(el), 'nvPr', 'videoFile') || kidPath(nvOf(el), 'nvPr', 'audioFile') || descendants(kid(nvOf(el), 'nvPr') ?? el, 'media').length) ctx.report.add('pptx_media', 'Video and sound imported as their poster pictures', shapeName)
    await pictureItem({ el, layer, box, spPrs, style, bodyPr: {}, autofit: { font: 1, spacing: 1 }, paras: [], ownParas: [], geom, placeholder: Boolean(ph), name: shapeName, groupFill, blipFill }, ctx)
    return
  }
  const txBody = kid(el, 'txBody')
  const type = ph?.type ?? null
  const master = ctx.master
  const levels: LevelStyles[] = [ctx.deck.defaultText]
  if (ph) {
    levels.push(masterType(ph.type) === 'title' ? master.title : masterType(ph.type) === 'body' ? master.body : master.other)
    levels.push(readLevels(kid(kid(masterPh, 'txBody'), 'lstStyle')), readLevels(kid(kid(layoutPh, 'txBody'), 'lstStyle')))
  } else levels.push(master.other)
  const paras = readParagraphs(txBody, levels)
  const { attrs: bodyPr, autofit } = bodyPrOf([kid(txBody, 'bodyPr'), kid(kid(layoutPh, 'txBody'), 'bodyPr'), kid(kid(masterPh, 'txBody'), 'bodyPr')])
  if (type && masterType(type) === 'title' && !bodyPr.anchor) bodyPr.anchor = 'ctr'
  const txXfrm = boxOfXfrm(kid(el, 'txXfrm'))
  await shapeItem({ el, layer, box, spPrs, style, bodyPr, autofit, paras, ownParas: readParagraphs(txBody, []), geom, placeholder: Boolean(ph), name: shapeName, groupFill, textBox: txXfrm ? xf(txXfrm) : null }, ctx)
}

// ---- background

async function background(ctx: SlideCtx, W: number, H: number): Promise<string> {
  const layers = [ctx.slide, ctx.layout, ctx.master.layer].filter((l): l is Layer => l !== null)
  for (const layer of layers) {
    const bg = kidPath(layer.doc, 'cSld', 'bg')
    if (!bg) continue
    const bgPr = kid(bg, 'bgPr')
    let paint: Paint | null = null
    if (bgPr) {
      const fill = bgPr.children.find((c): c is XmlElement => typeof c !== 'string' && FILL_NAMES.has(local(c.name)))
      paint = fill ? paintOfFill(fill, ctx, ctx.colors, layer, null) : null
    } else {
      const ref = kid(bg, 'bgRef')
      if (ref) paint = paintOf([], { name: 'p:style', attrs: {}, children: [{ ...ref, name: 'a:fillRef' }] }, ctx, layer, null)
    }
    if (!paint) continue
    const page: Box = { x: 0, y: 0, w: W, h: H, rot: 0, flipH: false, flipV: false }
    if (paint.kind === 'solid') return paint.color.hex
    if (paint.kind === 'none') return '#ffffff'
    if (paint.kind === 'gradient') {
      const rect = toWorld(page, ctx)
      const svg = svgBody(presetPaths('rect', rect.w, rect.h, {}) ?? [], paint, { color: null, width: 0, dash: 'solid', head: 'none', tail: 'none' }, 0, ctx)
      push(ctx, { type: 'shape', rect, rotation: 0, config: { shape: 'svg', svg: { viewBox: `0 0 ${rect.w} ${rect.h}`, body: svg.body, aspect: 'stretch' }, fill: svg.fill, stroke: 'none', stroke_width: 0, office_source: { slide: ctx.n, kind: 'background' } } }, 'shapes')
      return paint.stops[0].color.hex
    }
    if (paint.kind === 'picture') {
      await pictureItem({ el: { name: 'p:pic', attrs: {}, children: [] }, layer: paint.layer, box: page, spPrs: [], style: undefined, bodyPr: {}, autofit: { font: 1, spacing: 1 }, paras: [], ownParas: [], geom: undefined, placeholder: false, name: 'background', groupFill: null, blipFill: paint.blipFill }, ctx)
      return '#ffffff'
    }
  }
  return '#ffffff'
}

// ---- slides

const masters = new WeakMap<OfficePackage, Map<string, Promise<Master | null>>>()

async function masterOf(pkg: OfficePackage, path: string): Promise<Master | null> {
  let cache = masters.get(pkg)
  if (!cache) { cache = new Map(); masters.set(pkg, cache) }
  let pending = cache.get(path)
  if (!pending) {
    pending = (async () => {
      const layer = await layerOf(pkg, path, 'master')
      if (!layer) return null
      const themeRel = [...layer.rels.values()].find((r) => r.type === 'theme')
      const themeDoc = themeRel ? await pkg.xml(themeRel.target) : null
      const themeLayer = themeRel && themeDoc ? { kind: 'master' as const, path: themeRel.target, doc: themeDoc, rels: await pkg.rels(themeRel.target), tree: undefined } : null
      const tx = kid(layer.doc, 'txStyles')
      return { layer, theme: readTheme(themeDoc), themeDoc, themeLayer, clrMap: readClrMap(kid(layer.doc, 'clrMap')), title: readLevels(kid(tx, 'titleStyle')), body: readLevels(kid(tx, 'bodyStyle')), other: readLevels(kid(tx, 'otherStyle')) }
    })()
    cache.set(path, pending)
  }
  return pending
}

const overrideMap = (doc: XmlElement, base: Record<string, string>) => {
  const over = kidPath(doc, 'clrMapOvr', 'overrideClrMapping')
  return over ? readClrMap(over, base) : base
}

/** Convert one slide into a page plan. */
export async function convertSlide(deck: Deck, slide: DeckSlide, report: ImportReport, options: PptxOptions): Promise<OfficePagePlan> {
  const pkg = deck.pkg
  const slideLayer = await layerOf(pkg, slide.path, 'slide')
  if (!slideLayer) throw new Error(`missing ${slide.path}`)
  const layoutRel = [...slideLayer.rels.values()].find((r) => r.type === 'slideLayout')
  const layout = layoutRel ? await layerOf(pkg, layoutRel.target, 'layout') : null
  const masterRel = layout ? [...layout.rels.values()].find((r) => r.type === 'slideMaster') : undefined
  const master = (masterRel ? await masterOf(pkg, masterRel.target) : null) ?? { layer: { kind: 'master', path: '', doc: { name: 'p:sldMaster', attrs: {}, children: [] }, rels: new Map(), tree: undefined } as Layer, theme: readTheme(null), themeDoc: null, themeLayer: null, clrMap: readClrMap(undefined), title: [], body: [], other: [] }
  const clrMap = overrideMap(slideLayer.doc, layout ? overrideMap(layout.doc, master.clrMap) : master.clrMap)
  const perPt = pageScale(deck.width / EMU_PER_PT, deck.height / EMU_PER_PT)
  const k = perPt / EMU_PER_PT
  const W = Math.round(deck.width * k), H = Math.round(deck.height * k)
  const groupKey = `s${slide.number}`
  const tsRel = await pkg.relOfType(pkg.main, 'tableStyles')
  const ctx: SlideCtx = {
    deck, master, layout, slide: slideLayer, colors: { theme: master.theme, map: clrMap }, k, perPt, items: [], images: [], report, options, n: slide.number, groupKey,
    counts: { text: 0, images: 0, shapes: 0, rasterized: 0, placeholders: 0 }, next: 0, svgIds: 0, imageCache: new Map(), tableStyles: tsRel ? await pkg.xml(tsRel.target) : null,
  }
  const pageRect: Rect = { x: 0, y: 0, w: W, h: H }
  const content: PlanItem[] = []
  ctx.items = content
  const pageColor = await background(ctx, deck.width, deck.height)
  const showMaster = (doc: XmlElement) => attr(kid(doc, 'cSld') ?? doc, 'showMasterSp') !== '0' && attr(doc, 'showMasterSp') !== '0'
  const identity: Xf = (b) => b
  if (showMaster(slideLayer.doc) && (!layout || showMaster(layout.doc))) for (const c of master.layer.tree?.children ?? []) if (typeof c !== 'string') await element(c, master.layer, identity, ctx, null)
  if (layout && showMaster(slideLayer.doc)) for (const c of layout.tree?.children ?? []) if (typeof c !== 'string') await element(c, layout, identity, ctx, null)
  for (const c of slideLayer.tree?.children ?? []) if (typeof c !== 'string') await element(c, slideLayer, identity, ctx, null)

  // the title placeholder's first line names the page
  const titleSp = (slideLayer.tree?.children ?? []).find((c): c is XmlElement => typeof c !== 'string' && local(c.name) === 'sp' && ['title', 'ctrTitle'].includes(phOf(c)?.type ?? ''))
  const titleText = titleSp ? firstLine(shapeText(readParagraphs(kid(titleSp, 'txBody'), [])).text) : ''
  const name = titleText || options.pageTitle(slide.number)

  let items = content
  if (items.length > SLIDE_OBJECT_LIMIT) {
    const all = (slideLayer.tree?.children ?? []).filter((c): c is XmlElement => typeof c !== 'string' && !['nvGrpSpPr', 'grpSpPr'].includes(local(c.name)))
    let raster: PlanItem | null = null
    if (options.rasterize) {
      const subdoc = await pptxSubdoc(pkg, slide.path, all, { x: 0, y: 0, w: deck.width, h: deck.height }, true).catch(() => null)
      const got = subdoc ? await options.rasterize({ kind: 'slide', subdoc, subdocKind: 'pptx', width: deck.width / EMU_PER_PT, height: deck.height / EMU_PER_PT, name, trim: false }).catch(() => null) : null
      if (got) {
        const image: PlanImage = { key: `${options.idPrefix}s${slide.number}whole`, name: `${fileBaseOf(options.fileName)}-s${String(slide.number).padStart(3, '0')}.${extensionOf(got.mediaType)}`, mediaType: got.mediaType, bytes: got.bytes }
        ctx.images.splice(0, ctx.images.length, image)
        raster = { key: `${groupKey}~whole`, parent: groupKey, type: 'asset', rect: pageRect, rotation: 0, config: { office_source: { slide: slide.number, kind: 'slide' } }, image }
      }
    }
    if (raster) { report.add('pptx_slide_raster', 'Slides with very many objects imported whole as pictures', `${slide.number}`); items = [raster] } else { report.add('pptx_too_many', 'Slides with very many objects: only the first ones imported', `${slide.number}`); items = items.slice(0, SLIDE_OBJECT_LIMIT) }
  }
  const notes = [...slideLayer.rels.values()].find((r) => r.type === 'notesSlide')
  if (notes) {
    const doc = await pkg.xml(notes.target)
    const body = doc ? descendants(doc, 'sp').find((s) => phOf(s)?.type === 'body') : undefined
    if (body && shapeText(readParagraphs(kid(body, 'txBody'), [])).text.trim()) report.add('pptx_notes_dropped', 'Speaker notes not imported', `${slide.number}`)
  }
  const out: PlanItem[] = [
    { key: groupKey, parent: null, type: 'group', rect: pageRect, rotation: 0, config: {} },
    { key: `${groupKey}~frame`, parent: groupKey, type: 'frame', rect: pageRect, rotation: 0, title: name, config: { color: PAGE_FRAME_COLOR, fill: pageColor, stroke_width: 1, radius: 0, office_source: { file: options.fileName.slice(0, 255), slide: slide.number } }, payload: { presentation: { background: pageColor } } },
    ...items,
  ]
  report.pages += 1
  report.objects += out.length - 1
  report.images += ctx.images.length
  return { id: groupKey, name, background: pageColor, pageSize: { w: W, h: H }, items: out, bounds: pageRect, images: ctx.images, slide: slide.number, pageColor, counts: ctx.counts }
}

export type { TextPara }
