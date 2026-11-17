/* The page model of printing (打印 TODO §3.2, T1): page sizes, the content area of a page with and without a PDF Frame,
 * which PDF Frames fit which page, and how a free canvas step is fitted into the content area. No dependencies: the
 * Desktop's dialog, the print renderer (in the package, in Chromium) and the tests share it. Lengths are millimetres. */

export type PaperName = 'A3' | 'A4' | 'A5' | 'Letter' | '16:9' | '4:3'
export type Orientation = 'portrait' | 'landscape'

/** The page sizes of the dialog, each in its natural orientation (`landscape` for 16:9 and 4:3). */
export const PAPERS: Record<PaperName, { width_mm: number; height_mm: number; natural: Orientation }> = {
  A3: { width_mm: 297, height_mm: 420, natural: 'portrait' },
  A4: { width_mm: 210, height_mm: 297, natural: 'portrait' },
  A5: { width_mm: 148, height_mm: 210, natural: 'portrait' },
  Letter: { width_mm: 215.9, height_mm: 279.4, natural: 'portrait' },
  // the width of a landscape A4: a slide prints on the same sheet width as an A4
  '16:9': { width_mm: 297, height_mm: 167.06, natural: 'landscape' },
  '4:3': { width_mm: 297, height_mm: 222.75, natural: 'landscape' },
}
export const PAPER_NAMES = Object.keys(PAPERS) as PaperName[]

/** A page size as printed: already in its orientation. */
export interface Paper {
  name: PaperName | 'custom'
  width_mm: number
  height_mm: number
}

export interface Rect { x: number; y: number; w: number; h: number }

/** No PDF Frame on a flow canvas: 20 mm on every side (Q5). */
export const FLOW_MARGIN_MM = 20
/** Two aspect ratios this close are the same page shape (A3, A4 and A5 are one shape). */
export const RATIO_TOLERANCE = 0.01

export function paperOf(name: PaperName, orientation: Orientation): Paper {
  const p = PAPERS[name]
  const swap = orientation !== orientationOf(p.width_mm, p.height_mm)
  return { name, width_mm: swap ? p.height_mm : p.width_mm, height_mm: swap ? p.width_mm : p.height_mm }
}

export function orientationOf(w: number, h: number): Orientation {
  return w > h ? 'landscape' : 'portrait'
}

/** `media.page`'s `paper`: always the custom size, so the orientation is already in it (`landscape` stays false). */
export function paperArgument(paper: Paper): string {
  const mm = (v: number) => String(Math.round(v * 100) / 100)
  return `${mm(paper.width_mm)}x${mm(paper.height_mm)}mm`
}

export function sameShape(a: { w: number; h: number }, b: { w: number; h: number }): boolean {
  if (a.w <= 0 || a.h <= 0 || b.w <= 0 || b.h <= 0) return false
  const r = (a.w / a.h) / (b.w / b.h)
  return Math.abs(r - 1) <= RATIO_TOLERANCE
}

/** The default page of a canvas (Q5, §3.2): A4 portrait for a flow canvas; a free canvas takes its path's stage ratio
 * (the A or 16:9 / 4:3 size of that shape, else A4 in the stage's orientation). */
export function defaultPaper(mode: 'flow' | 'free', stage?: { w: number; h: number } | null): Paper {
  if (mode === 'flow' || !stage || stage.w <= 0 || stage.h <= 0) return paperOf('A4', 'portrait')
  const o = orientationOf(stage.w, stage.h)
  for (const name of ['16:9', '4:3', 'A4', 'Letter'] as PaperName[]) {
    const p = paperOf(name, o)
    if (sameShape({ w: p.width_mm, h: p.height_mm }, stage)) return p
  }
  return paperOf('A4', o)
}

/** A PDF Frame as the print needs it (T3): its box on the canvas and, inside it, where the page content goes. Header
 * and footer are what is above and below the content area. In the canvas units of the frame. */
export interface FrameGeometry {
  w: number
  h: number
  content: Rect
}

/** May this PDF Frame be used for this page? Only a frame of the page's shape (Q12): it is scaled, never stretched. */
export function frameFits(frame: { w: number; h: number }, paper: Paper): boolean {
  return sameShape(frame, { w: paper.width_mm, h: paper.height_mm })
}

/** The page's content area in millimetres: a PDF Frame's content area scaled to the page, else the whole page on a free
 * canvas and the page less 20 mm on each side on a flow canvas. */
export function contentArea(paper: Paper, mode: 'flow' | 'free', frame?: FrameGeometry | null): Rect {
  if (frame && frame.w > 0 && frame.h > 0) {
    const s = paper.width_mm / frame.w
    return { x: frame.content.x * s, y: frame.content.y * s, w: frame.content.w * s, h: frame.content.h * s }
  }
  if (mode === 'free') return { x: 0, y: 0, w: paper.width_mm, h: paper.height_mm }
  const m = FLOW_MARGIN_MM
  return { x: m, y: m, w: Math.max(1, paper.width_mm - 2 * m), h: Math.max(1, paper.height_mm - 2 * m) }
}

/** A free canvas step on its page (§3.2): the step's rectangle scaled to fit the content area, centred, never cropped.
 * `scale` maps the step's canvas units to the content area's units. */
export function fitStep(step: { w: number; h: number }, area: Rect): { scale: number; x: number; y: number; w: number; h: number } {
  if (step.w <= 0 || step.h <= 0) return { scale: 1, x: area.x, y: area.y, w: 0, h: 0 }
  const scale = Math.min(area.w / step.w, area.h / step.h)
  const w = step.w * scale
  const h = step.h * scale
  return { scale, x: area.x + (area.w - w) / 2, y: area.y + (area.h - h) / 2, w, h }
}

/** CSS pixels per millimetre (96 dpi). */
export const PX_PER_MM = 96 / 25.4

/** The values a PDF Frame label may show (§3.3), and the made-up ones of the canvas while it is edited. */
export interface PageContext {
  page: number
  pages: number
  title: string
  date: string
}

export const SAMPLE_PAGE_CONTEXT: PageContext = { page: 1, pages: 12, title: '', date: '' }

/** `{page}`, `{pages}`, `{title}`, `{date}` in a label's text, replaced by the page's values. Anything else is kept. */
export function fillPageContext(text: string, ctx: PageContext): string {
  return text.replace(/\{(page|pages|title|date)\}/g, (_m, key: keyof PageContext) => String(ctx[key]))
}

export function hasPageContext(text: string): boolean {
  return /\{(page|pages|title|date)\}/.test(text)
}

/** The default file name of the PDF: the canvas title without what a file name cannot hold. */
export function pdfFileName(title: string): string {
  // eslint-disable-next-line no-control-regex -- control characters are what is being removed
  const stem = title.replace(/[\\/:*?"<>|\u0000-\u001f]+/g, '-').replace(/^[\s.-]+|[\s.]+$/g, '').slice(0, 120) || 'canvas'
  return `${stem}.pdf`
}
