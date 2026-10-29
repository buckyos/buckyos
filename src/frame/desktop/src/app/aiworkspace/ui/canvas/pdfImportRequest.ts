/* A request to import a PDF (PDF 导入方案 §5): who asked (the main menu, a dropped file, a PDF file Block's
 * "expand into pages") and what happens after. ImportPdfDialog.tsx shows it outside the canvas view; the canvas
 * being shown registers how an import ends (switch to the new canvas, fit it, notify). */

import { Emitter } from '../../state/emitter'
import type { ImportOutcome } from './drawioImport'

export interface PdfImportSource { name: string; bytes: Uint8Array; /** The PDF is already an asset (a PDF file Block): no upload of the original. */ asset?: { object_id: string } | null }

export interface PdfImportRequest {
  source: PdfImportSource
  /** `ask`: a dropped or pasted file (insert as a file, or import as pages). */
  step: 'ask' | 'options'
  insertAsFile?: () => void
  /** Called with the result once the import has run; the shown canvas's `finisher` when absent. */
  finish?: (outcome: ImportOutcome) => void
}

class RequestSink {
  private request: PdfImportRequest | null = null
  private readonly emitter = new Emitter()
  readonly subscribe = this.emitter.subscribe
  /** How the canvas being shown ends an import (set while it is mounted). */
  finisher: ((outcome: ImportOutcome) => void) | null = null
  snapshot = () => this.request
  open(request: PdfImportRequest | null) { this.request = request; this.emitter.emit() }
  get busy() { return this.request !== null }
}
export const pdfImports = new RequestSink()
