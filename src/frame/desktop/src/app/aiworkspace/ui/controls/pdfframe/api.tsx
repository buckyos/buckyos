/* eslint-disable react-refresh/only-export-components -- the PDF Frame's interface to printing: constants, readers and its decoration */
/* The PDF Frame control as printing uses it (打印 TODO §3.3, T3): which PDF Frames a Workspace has, their page shape and
 * content area, the page context their labels read, and the header and footer drawn on a printed page. */

import type { FrameGeometry, PageContext } from '@print/page.ts'
import type { EntityEnvelope } from '../../../api/types'
import { aiwsT } from '../../../i18n'
import { FrameArt } from './Art'
import { framesIn, isFrameEntity, PDF_FRAME } from './model'

/** A PDF Frame or anything inside it: not page content (`outline` is the store's). */
export { partOfPdfFrame } from './model'

export { PageContextValue } from './context'

export const PDF_FRAME_KIND = PDF_FRAME

export interface PdfFrameInfo {
  id: string
  title: string
  geometry: FrameGeometry
  surfaceId: string | null
}

export function isPdfFrame(entity: EntityEnvelope): boolean {
  return isFrameEntity(entity)
}

/** Every live PDF Frame among `entities` (the outline: what the caller reads), with the Surface it is drawn on. */
export function listPdfFrames(entities: readonly EntityEnvelope[]): PdfFrameInfo[] {
  return framesIn(entities).map((f) => ({ ...f, title: f.title ?? aiwsT('aiworkspace.pdfframe.title') }))
}

/** The header and footer of frame `frameId` on a page of `widthPx` × `heightPx` (the frame scaled to the page's width;
 * the page has its shape), the content area transparent; its labels read `context`. Runs in the print renderer. */
export function PdfFrameDecoration(props: { frameId: string; widthPx: number; heightPx: number; context: PageContext }) {
  return <FrameArt frameId={props.frameId} widthPx={props.widthPx} heightPx={props.heightPx} mode="print" context={props.context} testId={`aiws-pdfframe-decoration-${props.frameId}`} />
}
