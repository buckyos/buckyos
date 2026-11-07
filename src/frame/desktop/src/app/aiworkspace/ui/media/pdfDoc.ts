/* pdf.js for the workspace (多媒体对象 §3.3, PDF 导入方案 §5): loaded only when a PDF is met, with the CMaps (CJK
 * text of PDFs that use predefined encodings) and the standard font data (PDFs that do not embed the 14 base fonts)
 * the build serves under aiworkspace/pdfjs/ — fetched only by the pages that need them. The poster of an inserted
 * PDF and the importer open documents here. */

import type { PdfOperatorList } from '@pdf/mod'
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
  signal?.throwIfAborted()
  // the worker fetches the data files itself: absolute URLs
  const base = new URL(`${import.meta.env.BASE_URL}${ASSETS}`, window.location.href).href
  const task: PDFDocumentLoadingTask = pdfjs.getDocument({
    data: bytes.slice(), cMapUrl: `${base}cmaps/`, cMapPacked: true, standardFontDataUrl: `${base}standard_fonts/`, wasmUrl: `${base}wasm/`, iccUrl: `${base}iccs/`,
    // font names, serif and monospace flags for the importer (PDF 导入方案 §5.1)
    fontExtraProperties: true,
  })
  let closed = false
  const close = () => {
    if (closed) return
    closed = true
    signal?.removeEventListener('abort', close)
    void task.destroy().catch(() => undefined)
  }
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
export async function renderPage(page: PDFPageProxy, maxEdge: number, options: { transparent?: boolean; filter?: (index: number, list: PdfOperatorList) => boolean; importWork?: boolean; signal?: AbortSignal } = {}): Promise<HTMLCanvasElement> {
  options.signal?.throwIfAborted()
  const base = page.getViewport({ scale: 1 })
  const viewport = page.getViewport({ scale: maxEdge / Math.max(base.width, base.height, 1) })
  const canvas = document.createElement('canvas')
  canvas.width = Math.max(1, Math.round(viewport.width))
  canvas.height = Math.max(1, Math.round(viewport.height))
  // pdf.js asks for an opaque context (`alpha: false`), where nothing drawn is black; it gets the one made first
  if (options.transparent) canvas.getContext('2d', { alpha: true, willReadFrequently: true })
  const task = page.render({ canvas, viewport, ...(options.transparent ? { background: 'rgba(0,0,0,0)' } : {}), ...(options.filter ? { operationsFilter: options.filter as never } : {}) })
  if (!options.importWork) {
    await task.promise
    return canvas
  }
  const scheduler = (task as unknown as { _internalRenderTask?: { _useRequestAnimationFrame?: boolean } })._internalRenderTask
  if (typeof scheduler?._useRequestAnimationFrame === 'boolean') scheduler._useRequestAnimationFrame = false
  const channel = new MessageChannel()
  let resume: (() => void) | null = null
  channel.port1.onmessage = () => {
    const next = resume
    resume = null
    next?.()
  }
  task.onContinue = (next: () => void) => {
    resume = next
    channel.port2.postMessage(null)
  }
  const cancel = () => task.cancel()
  let timedOut = false
  const timeout = window.setTimeout(() => { timedOut = true; cancel() }, 30_000)
  options.signal?.addEventListener('abort', cancel, { once: true })
  try {
    await task.promise
  } catch (error) {
    options.signal?.throwIfAborted()
    if (timedOut) throw new Error('PDF page render timed out')
    throw error
  } finally {
    window.clearTimeout(timeout)
    channel.port1.close()
    channel.port2.close()
    options.signal?.removeEventListener('abort', cancel)
  }

  return canvas
}

export async function renderOperators(page: PDFPageProxy, signal?: AbortSignal): Promise<PdfOperatorList | null> {
  const pdfjs = await loadPdfjs()
  let operators: PdfOperatorList | null = null
  await renderPage(page, 1, { transparent: true, importWork: true, signal, filter: (index, list) => {
    operators = list
    return list.fnArray[index] === pdfjs.OPS.dependency
  } })
  return operators
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
