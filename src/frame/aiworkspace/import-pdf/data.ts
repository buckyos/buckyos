/* What the converter reads: one PDF page as plain data (PDF 导入方案 §4), taken out of pdf.js by `extract.ts` in the
 * browser or in Deno. Everything here is JSON (the unit tests read recorded pages); pixels stay with the host, which
 * encodes a picture or renders the base raster when the converter asks for it by reference.
 *
 *   operator list   pdf.js's `fnArray` with arguments reduced to what the converter reads (see `compactArgs`)
 *   text            `getTextContent({ includeMarkedContent: true })` items with the marked content they sit in
 *   fonts           the font objects of `setFont` (`fontExtraProperties: true`): name, bold / italic, serif / mono
 *   patterns        shading patterns (`RadialAxial` kept, anything else by kind only)
 *   struct          `getStructTree()` (tagged PDFs)
 *
 * The op codes are pdf.js 6.4's (`OPS`, `DrawOPS` in build/pdf.mjs); `extract_test.ts` checks them against the
 * installed pdf.js. */

export type Json = string | number | boolean | null | Json[] | { [key: string]: Json }

export const OPS = {
  dependency: 1, setLineWidth: 2, setLineCap: 3, setLineJoin: 4, setMiterLimit: 5, setDash: 6, setRenderingIntent: 7, setFlatness: 8,
  setGState: 9, save: 10, restore: 11, transform: 12, moveTo: 13, lineTo: 14, curveTo: 15, curveTo2: 16, curveTo3: 17, closePath: 18,
  rectangle: 19, stroke: 20, closeStroke: 21, fill: 22, eoFill: 23, fillStroke: 24, eoFillStroke: 25, closeFillStroke: 26,
  closeEOFillStroke: 27, endPath: 28, clip: 29, eoClip: 30, beginText: 31, endText: 32, setCharSpacing: 33, setWordSpacing: 34,
  setHScale: 35, setLeading: 36, setFont: 37, setTextRenderingMode: 38, setTextRise: 39, moveText: 40, setLeadingMoveText: 41,
  setTextMatrix: 42, nextLine: 43, showText: 44, showSpacedText: 45, nextLineShowText: 46, nextLineSetSpacingShowText: 47,
  setCharWidth: 48, setCharWidthAndBounds: 49, setStrokeColorSpace: 50, setFillColorSpace: 51, setStrokeColor: 52, setStrokeColorN: 53,
  setFillColor: 54, setFillColorN: 55, setStrokeGray: 56, setFillGray: 57, setStrokeRGBColor: 58, setFillRGBColor: 59,
  setStrokeCMYKColor: 60, setFillCMYKColor: 61, shadingFill: 62, beginInlineImage: 63, beginImageData: 64, endInlineImage: 65,
  paintXObject: 66, markPoint: 67, markPointProps: 68, beginMarkedContent: 69, beginMarkedContentProps: 70, endMarkedContent: 71,
  beginCompat: 72, endCompat: 73, paintFormXObjectBegin: 74, paintFormXObjectEnd: 75, beginGroup: 76, endGroup: 77,
  beginAnnotation: 80, endAnnotation: 81, paintImageMaskXObject: 83, paintImageMaskXObjectGroup: 84, paintImageXObject: 85,
  paintInlineImageXObject: 86, paintInlineImageXObjectGroup: 87, paintImageXObjectRepeat: 88, paintImageMaskXObjectRepeat: 89,
  paintSolidColorImageMask: 90, constructPath: 91, setStrokeTransparent: 92, setFillTransparent: 93, rawFillPath: 94,
} as const

/** Path segments inside a `constructPath`: moveTo x y, lineTo x y, curveTo x1 y1 x2 y2 x y, quadraticCurveTo x1 y1 x y, closePath. */
export const DRAW = { moveTo: 0, lineTo: 1, curveTo: 2, quadraticCurveTo: 3, closePath: 4 } as const

/** One `getTextContent` item. `mc` is the marked content id it sits in (`p12R_mc3`), null outside any. */
export interface PdfTextItem { str: string; transform: number[]; width: number; height: number; fontName: string; hasEOL: boolean; mc: string | null }

export interface PdfFont {
  /** PostScript name (`ABCDEF+Calibri-Bold`), empty when unknown. */
  name: string
  bold: boolean
  italic: boolean
  serif: boolean
  mono: boolean
  type3: boolean
  vertical: boolean
  /** Glyph space → text space (`fontMatrix[0]`, 0.001 for most fonts). */
  unit: number
  ascent: number | null
  descent: number | null
}

export type PdfStructNode = { role?: string; alt?: string; lang?: string; children?: PdfStructChild[] }
export type PdfStructChild = PdfStructNode | { type: string; id: string }

export interface PdfLink { rect: number[]; page: number | null; url: string | null }

export interface PdfPage {
  /** 1-based. */
  number: number
  /** Size at scale 1 (points, after `/Rotate`) and user space → page (top-left origin, y down) at scale 1. */
  width: number
  height: number
  transform: number[]
  fnArray: number[]
  argsArray: Json[]
  text: PdfTextItem[]
  fonts: Record<string, PdfFont>
  patterns: Record<string, Json>
  /** Picture sizes by reference: an XObject's id, or `op:<index>` for an inline image. */
  images: Record<string, { width: number; height: number }>
  struct: PdfStructNode | null
  links: PdfLink[]
  /** The first bookmark pointing at this page, if any. */
  bookmark: string | null
}
