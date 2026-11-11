/* pdf.js, loaded only when a PDF is met, with the data files the build serves under aiworkspace/pdfjs/ (CMaps for
 * the CJK text of PDFs with predefined encodings, the 14 standard fonts, the image decoders, the CMYK profile).
 * Shared by the Preview renderer and the AI Workspace importer. */

import workerUrl from 'pdfjs-dist/build/pdf.worker.min.mjs?url'

const ASSETS = 'aiworkspace/pdfjs/'

export async function loadPdfjs() {
  const pdfjs = await import('pdfjs-dist')
  pdfjs.GlobalWorkerOptions.workerSrc = workerUrl
  return pdfjs
}

/** `getDocument` options for the data files; the worker fetches them itself, so the URLs are absolute. */
export function pdfjsAssetOptions() {
  const base = new URL(`${import.meta.env.BASE_URL}${ASSETS}`, window.location.href).href
  return { cMapUrl: `${base}cmaps/`, cMapPacked: true, standardFontDataUrl: `${base}standard_fonts/`, wasmUrl: `${base}wasm/`, iccUrl: `${base}iccs/` }
}
