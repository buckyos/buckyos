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

export async function openPdf(bytes: Uint8Array): Promise<{ doc: PdfjsDocument; oc: { isVisible(g: unknown): boolean } | null }> {
  const lib = await pdfjs()
  const doc = await lib.getDocument({
    data: bytes.slice(), fontExtraProperties: true, isEvalSupported: false, disableFontFace: true, useWorkerFetch: false,
    standardFontDataUrl: new URL('standard_fonts/', PDFJS_DIR).pathname, cMapUrl: new URL('cmaps/', PDFJS_DIR).pathname, cMapPacked: true, verbosity: 0,
  }).promise
  let oc = null
  try { oc = await doc.getOptionalContentConfig() } catch { /* none */ }
  return { doc, oc }
}
