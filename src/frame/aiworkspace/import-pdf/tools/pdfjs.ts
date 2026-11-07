/* pdf.js for Deno: the Desktop's own copy (src/frame/desktop/node_modules/pdfjs-dist, the legacy build that runs
 * without a DOM), its standard fonts and CMaps from disk. Used by the recording tool and the extraction test. */

import type { PdfjsDocument } from '../extract.ts'

export const PDFJS_DIR = new URL('../../../desktop/node_modules/pdfjs-dist/', import.meta.url)

export async function available(): Promise<boolean> {
  try { await Deno.stat(new URL('legacy/build/pdf.mjs', PDFJS_DIR)); return true } catch { return false }
}

// deno-lint-ignore no-explicit-any
export async function pdfjs(): Promise<any> {
  return await import(new URL('legacy/build/pdf.mjs', PDFJS_DIR).href)
}

export async function openPdf(bytes: Uint8Array): Promise<{ doc: PdfjsDocument; oc: { isVisible(g: unknown): boolean } | null; close: () => Promise<void> }> {
  const lib = await pdfjs()
  const port = new Worker(new URL('./pdfjs-worker.ts', import.meta.url), { type: 'module' })
  try {
    await new Promise<void>((resolve, reject) => {
      const listeners = new AbortController()
      port.addEventListener('message', event => {
        if (event.data !== 'pdfjs-local-worker-ready') return
        listeners.abort()
        resolve()
      }, { signal: listeners.signal })
      port.addEventListener('error', event => {
        event.preventDefault()
        listeners.abort()
        reject(new Error(event.message))
      }, { signal: listeners.signal })
    })
  } catch (error) {
    port.terminate()
    throw error
  }
  const worker = new lib.PDFWorker({ port, verbosity: 0 })
  const task = lib.getDocument({
    data: bytes.slice(), worker, fontExtraProperties: true, isEvalSupported: false, disableFontFace: true, useWorkerFetch: true,
    wasmUrl: new URL('wasm/', PDFJS_DIR).href, iccUrl: new URL('iccs/', PDFJS_DIR).href,
    standardFontDataUrl: new URL('standard_fonts/', PDFJS_DIR).href, cMapUrl: new URL('cmaps/', PDFJS_DIR).href, cMapPacked: true, verbosity: 0,
  })
  const close = async () => {
    try { await task.destroy() } finally { worker.destroy(); port.terminate() }
  }
  try {
    const doc = await task.promise
    let oc = null
    try { oc = await doc.getOptionalContentConfig() } catch { /* none */ }
    return { doc, oc, close }
  } catch (error) {
    await close()
    throw error
  }
}
