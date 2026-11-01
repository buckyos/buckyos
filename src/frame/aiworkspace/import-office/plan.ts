/* What the converters produce (Office 导入 TODO §2.1): a docx becomes a `DocPlan` — the flow page's Blocks in order,
 * rich text and pictures — a pptx a list of page plans of the draw.io importer's shape (`OfficePagePlan`). Pictures
 * travel as `PlanImage`s (bytes until the host uploads them); what no TS code can draw (charts, SmartArt, OLE,
 * EMF/WMF, formulas) goes to the host's `rasterize` as a one-object sub-document (§5.3) and comes back as a picture,
 * or stays a placeholder when the host cannot (no LibreOffice). */

import type { PagePlan, PlanImage } from '../import-drawio/convert.ts'
import type { AstNode } from './richtext.ts'

export interface EncodedImage { bytes: Uint8Array; mediaType: string }

/** What a host is asked to turn into a picture (§5.3). */
export interface RasterRequest {
  /** chart / smartart / ole / image (EMF, WMF, TIFF …) / formula / slide (a whole page). */
  kind: 'chart' | 'smartart' | 'ole' | 'image' | 'formula' | 'slide'
  /** A minimal docx or pptx holding only the object (`subdoc.ts`). */
  subdoc: Uint8Array
  subdocKind: 'docx' | 'pptx'
  /** Its size in points (the sub-document's page). */
  width: number
  height: number
  /** For reports and file names. */
  name: string
  /** Crop the white margin of the rendered page (a docx object sits on a page of its own size, but may not fill it). */
  trim: boolean
}

export type Rasterize = (request: RasterRequest) => Promise<EncodedImage | null>

export type DocSegment =
  | { kind: 'richtext'; content: AstNode }
  /** A picture Block: `image` is a plan image key, `size` its size in points as the document shows it. */
  | { kind: 'image'; image: string; title?: string; size: { w: number; h: number } | null; pixels: { width: number; height: number } | null }

export interface DocPlan {
  title: string
  segments: DocSegment[]
  images: PlanImage[]
  counts: { paragraphs: number; headings: number; lists: number; tables: number; images: number; rasterized: number }
}

export interface OfficePagePlan extends PagePlan {
  /** 1-based slide number. */
  slide: number
  pageColor: string
  counts: { text: number; images: number; shapes: number; rasterized: number; placeholders: number }
}

/** A file name without its extension, safe for picture names. */
export const fileBaseOf = (name: string) => name.replace(/\.[^.]+$/, '').replace(/[\\/:*?"<>|]+/g, '_').slice(0, 80) || 'office'

/** The extension for a picture's media type. */
export function extensionOf(mediaType: string): string {
  const map: Record<string, string> = { 'image/png': 'png', 'image/jpeg': 'jpg', 'image/gif': 'gif', 'image/webp': 'webp', 'image/svg+xml': 'svg', 'image/bmp': 'bmp', 'image/tiff': 'tif', 'image/emf': 'emf', 'image/wmf': 'wmf' }
  return map[mediaType] ?? 'bin'
}
