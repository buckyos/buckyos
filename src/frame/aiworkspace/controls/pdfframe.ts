/* The PDF Frame control (打印 TODO §3.3, T3): a page shape — the A series (A3, A4, A5 share it), Letter, 16:9, 4:3 or a
 * custom one, portrait or landscape — with a header band and a footer band. The frame's box always has that shape: its
 * width is stored, its height follows from the ratio. Its only children are two groups, `slot` `header` and `footer`,
 * placed by the frame (full width, at the top and at the bottom, as high as `config.header` / `config.footer`); the
 * Blocks of the header and the footer sit in them like Blocks in any group. The content area is what lies between
 * the bands, inside the side margins (`config.margin`). Printing scales the whole frame onto the chosen page (only a
 * page of the same shape may use it, Q12), so lengths are in the frame's own canvas units. Pure: the canvas, the print
 * renderer, the core's checks (mirrored in `types.rs`) and Agents read the same rules. */

import type { ControlDescriptor } from './catalog.ts'
import { num } from './placement.ts'
import type { ControlSource, SolvedControl, SolveOptions } from './solve.ts'
import { mindmapConfig, type LaidNode, type Rect } from './tree.ts'

export const PDF_FRAME = 'pdf-frame'

export type FramePage = 'A' | 'Letter' | '16:9' | '4:3' | 'custom'
export type FrameOrientation = 'portrait' | 'landscape'
export type FrameBand = 'header' | 'footer'

export const FRAME_PAGES: readonly FramePage[] = ['A', 'Letter', '16:9', '4:3', 'custom']
export const FRAME_BANDS: readonly FrameBand[] = ['header', 'footer']

export interface PdfFrameConfig {
  page: FramePage
  orientation: FrameOrientation
  /** A custom page's width and height in millimetres (its shape, and its natural size). */
  custom_w: number
  custom_h: number
  /** Heights of the bands and the left / right margin of the content area, in the frame's units (may be 0). */
  header: number
  footer: number
  margin: number
}

/** The largest band height or margin the core accepts. */
export const MAX_FRAME_LENGTH = 20000
/** The largest custom page side (mm). */
export const MAX_CUSTOM_MM = 10000

/** Each page shape at its natural size in whole points (1 unit = 1 pt on that page): A4, Letter, and 16:9 / 4:3 on the
 * width of a landscape A4 (as the print dialog's pages). Portrait first, except the slide shapes. */
const NATURAL: Record<Exclude<FramePage, 'custom'>, { w: number; h: number }> = {
  A: { w: 595, h: 842 },
  Letter: { w: 612, h: 792 },
  '16:9': { w: 842, h: 842 * 9 / 16 },
  '4:3': { w: 842, h: 842 * 3 / 4 },
}
const PT_PER_MM = 72 / 25.4

export const DEFAULT_FRAME_CONFIG: PdfFrameConfig = { page: 'A', orientation: 'portrait', custom_w: 210, custom_h: 297, header: 56, footer: 56, margin: 56 }

const length = (v: unknown, fallback: number, max = MAX_FRAME_LENGTH) => (typeof v === 'number' && Number.isFinite(v) && v >= 0 && v <= max ? v : fallback)

/** A frame's `config` with every key, unknown or invalid values taking the defaults. */
export function pdfFrameConfig(raw: unknown): PdfFrameConfig {
  const c = (raw && typeof raw === 'object' && !Array.isArray(raw) ? raw : {}) as Record<string, unknown>
  const d = DEFAULT_FRAME_CONFIG
  const page = FRAME_PAGES.includes(c.page as FramePage) ? c.page as FramePage : d.page
  const custom = (v: unknown, fallback: number) => (typeof v === 'number' && Number.isFinite(v) && v > 0 && v <= MAX_CUSTOM_MM ? v : fallback)
  return {
    page,
    orientation: c.orientation === 'portrait' || c.orientation === 'landscape' ? c.orientation : naturalOrientation(page, c),
    custom_w: custom(c.custom_w, d.custom_w),
    custom_h: custom(c.custom_h, d.custom_h),
    header: length(c.header, d.header),
    footer: length(c.footer, d.footer),
    margin: length(c.margin, d.margin),
  }
}

/** The orientation a page shape comes in (a custom one: its sides'), what choosing that shape turns the frame to. */
export function naturalOrientation(page: FramePage, c: Record<string, unknown> = {}): FrameOrientation {
  if (page === 'custom') return typeof c.custom_w === 'number' && typeof c.custom_h === 'number' && c.custom_w > c.custom_h ? 'landscape' : 'portrait'
  return page === '16:9' || page === '4:3' ? 'landscape' : 'portrait'
}

/** The page at its natural size (points), in the frame's orientation. */
export function naturalSize(c: PdfFrameConfig): { w: number; h: number } {
  const n = c.page === 'custom' ? { w: c.custom_w * PT_PER_MM, h: c.custom_h * PT_PER_MM } : NATURAL[c.page]
  const long = Math.max(n.w, n.h), short = Math.min(n.w, n.h)
  return c.orientation === 'landscape' ? { w: long, h: short } : { w: short, h: long }
}

/** Width over height of the frame's page. */
export function frameRatio(c: PdfFrameConfig): number {
  const n = naturalSize(c)
  return n.w / n.h
}

/** The frame's box from its stored width (absent: the natural width): the height always follows the shape. */
export function frameBox(width: number | undefined, c: PdfFrameConfig): { w: number; h: number } {
  const w = width !== undefined && width > 0 ? width : naturalSize(c).w
  return { w, h: w / frameRatio(c) }
}

/** The bands and the content area in a box of the frame (relative to its top-left): a band never reaches past the box,
 * the footer never over the header; the content area may be empty. */
export function frameAreas(box: { w: number; h: number }, c: PdfFrameConfig): { header: Rect; footer: Rect; content: Rect } {
  const header = Math.min(c.header, box.h)
  const footer = Math.min(c.footer, box.h - header)
  const margin = Math.min(c.margin, box.w / 2)
  return {
    header: { x: 0, y: 0, w: box.w, h: header },
    footer: { x: 0, y: box.h - footer, w: box.w, h: footer },
    content: { x: margin, y: header, w: box.w - 2 * margin, h: box.h - header - footer },
  }
}

/** The frame in a size that keeps its scale after its shape changes: `before`'s width over its natural width, applied to
 * `after`'s (turning a portrait A4 frame to landscape swaps its sides). */
export function rescaledWidth(width: number, before: PdfFrameConfig, after: PdfFrameConfig): number {
  return Math.round(width * (naturalSize(after).w / naturalSize(before).w))
}

/** The band a child of the frame is (a group with `slot` header / footer). */
export function bandOf(e: { type_id: string; kind?: string | null; slot?: string | null }): FrameBand | null {
  return e.type_id === 'buckyos.container' && e.kind === 'group' && (e.slot === 'header' || e.slot === 'footer') ? e.slot : null
}

/** The frame's layout: its box, and its two bands where the config puts them. Anything else under it (the core allows
 * none) is not laid out. */
export function solvePdfFrame(src: ControlSource, containerId: string, _opts: SolveOptions): SolvedControl {
  const container = src.get(containerId)
  const config = pdfFrameConfig(container?.config)
  const box = frameBox(num(container?.placement?.w), config)
  const areas = frameAreas(box, config)
  const nodes = new Map<string, LaidNode>()
  const seen = new Set<FrameBand>()
  for (const e of src.childrenOf(containerId)) {
    const band = bandOf(e)
    if (!band || seen.has(band)) continue
    seen.add(band)
    nodes.set(e.entity_id, { id: e.entity_id, ...areas[band], parent: null, depth: 0, grow: 'right', pinned: false, hasChildren: false, collapsed: false })
  }
  return { id: containerId, config: mindmapConfig(undefined), layout: { nodes, bounds: { x: 0, y: 0, w: box.w, h: box.h } }, nested: new Map() }
}

/** The operations an Agent (or a client) writes for a new frame with its two bands, in one Commit. */
export function pdfFrameOperations(args: { id: string; parentId: string; orderKey: string; x: number; y: number; config?: Partial<PdfFrameConfig>; title?: string; headerId: string; footerId: string }): Record<string, unknown>[] {
  const config = pdfFrameConfig(args.config ?? {})
  const box = frameBox(undefined, config)
  const areas = frameAreas(box, config)
  const round = (r: Rect) => ({ x: Math.round(r.x), y: Math.round(r.y), w: Math.max(1, Math.round(r.w)), h: Math.max(1, Math.round(r.h)) })
  return [
    { op: 'entity.create', entity_id: args.id, type_id: 'buckyos.container', parent_id: args.parentId, order_key: args.orderKey,
      placement: { x: Math.round(args.x), y: Math.round(args.y), w: Math.round(box.w), h: Math.round(box.h) },
      payload: { kind: PDF_FRAME, ...(args.title ? { title: args.title } : {}), config: { ...config } } },
    { op: 'entity.create', entity_id: args.headerId, type_id: 'buckyos.container', parent_id: args.id, order_key: 'a', placement: round(areas.header), payload: { kind: 'group', slot: 'header' } },
    { op: 'entity.create', entity_id: args.footerId, type_id: 'buckyos.container', parent_id: args.id, order_key: 'b', placement: round(areas.footer), payload: { kind: 'group', slot: 'footer' } },
  ]
}

const AGENT = [
  '# PDF Frame',
  '',
  'A page design for printing a canvas to PDF: a page shape with a header and a footer that every printed page repeats. The',
  'print dialog lists the frames of the Workspace whose shape matches the chosen page; the frame itself is never printed as',
  'content (a flow canvas skips it when paginating).',
  '',
  '`buckyos.container { kind: "pdf-frame", config, title? }` with a numeric placement (never `"auto"`): `x`, `y` and `w`; the',
  'height always follows the page shape (`w` / ratio), whatever `h` says. `config` (each key optional, unknown keys refused):',
  '`page` `A` (A3 / A4 / A5) | `Letter` | `16:9` | `4:3` | `custom` (`custom_w`, `custom_h` in mm), `orientation` `portrait` |',
  '`landscape`, `header` / `footer` (band heights) and `margin` (left and right) in the frame\'s units, 0 … 20000. At its natural',
  'width (A4 portrait 595, landscape 842; Letter 612 / 792; 16:9 and 4:3 842 landscape) one unit is one point of the page:',
  'a label of font size 10 prints at 10 pt on an A4; a frame twice as wide prints its header at half the size.',
  '',
  'Its only children are two groups, `slot: "header"` and `slot: "footer"`: the frame places them (full width, top and bottom,',
  'as high as the config says) — their stored placement is only what an older client shows. The header and the footer are',
  'ordinary Blocks inside those groups (labels, pictures, shapes), placed relative to their band. A label\'s text may hold',
  '`{page}`, `{pages}`, `{title}` (the printed canvas) and `{date}`: printing fills them per page; the canvas shows samples.',
  'The content area is between the bands, inside the margins: leave it empty.',
  '',
  'Example (doc.commit): `{ op: "entity.create", entity_id: "pf", type_id: "buckyos.container", parent_id: <surface>, order_key: "m",',
  'placement: { x: 40, y: 40, w: 595, h: 842 }, payload: { kind: "pdf-frame", config: { page: "A", orientation: "portrait", header: 56,',
  'footer: 56, margin: 56 } } }`, `{ op: "entity.create", entity_id: "pf-h", type_id: "buckyos.container", parent_id: "pf", order_key: "a",',
  'placement: { x: 0, y: 0, w: 595, h: 56 }, payload: { kind: "group", slot: "header" } }`, the same for `pf-f` (`slot: "footer"`,',
  '`y: 786`), then `{ op: "entity.create", entity_id: "pn", type_id: "buckyos.cell", parent_id: "pf-f", order_key: "a",',
  'placement: { x: 248, y: 16, w: 100, h: 24 }, payload: { view: { type: "label" }, title: "{page} / {pages}", config: { align: "center", font_size: 10 } } }`.',
  'Change the shape with `entity.set_keys` on `config` (and `tree.place` the frame\'s width to keep its scale).',
].join('\n')

export const PDF_FRAME_CONTROL: ControlDescriptor = {
  kind: PDF_FRAME,
  title: 'PDF Frame',
  summary: 'A page shape with a header and a footer (labels, pictures, shapes; page numbers) that printing a canvas to PDF repeats on every page.',
  when: 'To give printed PDFs of a canvas a header, a footer and page numbers; one frame per page shape or design.',
  formats: [],
  agent: AGENT,
}
