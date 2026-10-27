/* A PDF's first page as a picture (多媒体对象 §3.3), with pdf.js loaded only when a PDF is inserted. Fonts a PDF
 * does not embed fall back to the browser's; predefined CJK CMaps are not shipped (a known gap). */

import workerUrl from 'pdfjs-dist/build/pdf.worker.min.mjs?url'

export async function pdfFirstPage(bytes: Uint8Array, maxEdge: number, signal: AbortSignal): Promise<{ canvas: HTMLCanvasElement; pageCount: number }> {
  const pdfjs = await import('pdfjs-dist')
  pdfjs.GlobalWorkerOptions.workerSrc = workerUrl
  // pdf.js takes the buffer over: hand it a copy
  const task = pdfjs.getDocument({ data: bytes.slice() })
  const abort = () => { void task.destroy() }
  signal.addEventListener('abort', abort)
  try {
    const pdf = await task.promise
    const page = await pdf.getPage(1)
    const base = page.getViewport({ scale: 1 })
    const viewport = page.getViewport({ scale: maxEdge / Math.max(base.width, base.height, 1) })
    const canvas = document.createElement('canvas')
    canvas.width = Math.max(1, Math.round(viewport.width))
    canvas.height = Math.max(1, Math.round(viewport.height))
    // pdf.js paints a white page under the content
    await page.render({ canvas, viewport }).promise
    return { canvas, pageCount: pdf.numPages }
  } finally {
    signal.removeEventListener('abort', abort)
    void task.destroy()
  }
}
