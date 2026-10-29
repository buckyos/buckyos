/* pdf.js for the workspace (多媒体对象 §3.3, PDF 导入方案 §5): loaded only when a PDF is met, with the CMaps (CJK
 * text of PDFs that use predefined encodings) and the standard font data (PDFs that do not embed the 14 base fonts)
 * the build serves under aiworkspace/pdfjs/ — fetched only by the pages that need them. The poster of an inserted
 * PDF and the importer open documents here. */

import workerUrl from 'pdfjs-dist/build/pdf.worker.min.mjs?url'
import type { PDFDocumentLoadingTask, PDFDocumentProxy, PDFPageProxy } from 'pdfjs-dist'

export type { PDFDocumentProxy, PDFPageProxy }

const ASSETS = 'aiworkspace/pdfjs/'

export async function loadPdfjs() {
  const pdfjs = await import('pdfjs-dist')
  pdfjs.GlobalWorkerOptions.workerSrc = workerUrl
  return pdfjs
}

export class PdfPasswordError extends Error {}

/** Open a PDF (pdf.js takes the buffer over: it is handed a copy). `signal` aborts the loading and later work. */
export async function openPdfDocument(bytes: Uint8Array, signal?: AbortSignal): Promise<{ doc: PDFDocumentProxy; close: () => void }> {
  const pdfjs = await loadPdfjs()
  // the worker fetches the data files itself: absolute URLs
  const base = new URL(`${import.meta.env.BASE_URL}${ASSETS}`, window.location.href).href
  const task: PDFDocumentLoadingTask = pdfjs.getDocument({
    data: bytes.slice(), cMapUrl: `${base}cmaps/`, cMapPacked: true, standardFontDataUrl: `${base}standard_fonts/`,
    // font names, serif and monospace flags for the importer (PDF 导入方案 §5.1)
    fontExtraProperties: true,
  })
  const close = () => { void task.destroy() }
  signal?.addEventListener('abort', close, { once: true })
  try {
    return { doc: await task.promise, close }
  } catch (error) {
    close()
    if ((error as { name?: string } | null)?.name === 'PasswordException') throw new PdfPasswordError('password')
    throw error
  }
}

/** A page drawn on a new canvas, `maxEdge` pixels on its long side. `transparent`: no white paper under it;
 * `filter`: operators to draw (pdf.js `operationsFilter`, by index in the page's operator list). */
export async function renderPage(page: PDFPageProxy, maxEdge: number, options: { transparent?: boolean; filter?: (index: number, list: { fnArray: number[] }) => boolean } = {}): Promise<HTMLCanvasElement> {
  const base = page.getViewport({ scale: 1 })
  const viewport = page.getViewport({ scale: maxEdge / Math.max(base.width, base.height, 1) })
  const canvas = document.createElement('canvas')
  canvas.width = Math.max(1, Math.round(viewport.width))
  canvas.height = Math.max(1, Math.round(viewport.height))
  // pdf.js asks for an opaque context (`alpha: false`), where nothing drawn is black; it gets the one made first
  if (options.transparent) canvas.getContext('2d', { alpha: true, willReadFrequently: true })
  await page.render({ canvas, viewport, ...(options.transparent ? { background: 'rgba(0,0,0,0)' } : {}), ...(options.filter ? { operationsFilter: options.filter as never } : {}) }).promise
  return canvas
}

/** A PDF's first page as a picture (the poster of an inserted PDF) and its page count. */
export async function pdfFirstPage(bytes: Uint8Array, maxEdge: number, signal: AbortSignal): Promise<{ canvas: HTMLCanvasElement; pageCount: number }> {
  const { doc, close } = await openPdfDocument(bytes, signal)
  try {
    // pdf.js paints a white page under the content
    return { canvas: await renderPage(await doc.getPage(1), maxEdge), pageCount: doc.numPages }
  } finally {
    close()
  }
}
