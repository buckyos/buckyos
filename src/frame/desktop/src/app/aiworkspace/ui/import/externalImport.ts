/* A file another part of the system hands to AI Workspace to import ("Import to AI Workspace" in Open with…, which
 * Preview and the system chooser show). It waits here until a canvas of a workspace this window has open can take it,
 * then goes through the same steps as a file dropped there (CanvasView `handleFiles`): a PDF, Office document or
 * table asks first (a file Block, or an import), a draw.io file is imported. While it waits the app shows what it is
 * waiting for (a workspace, a page, the right to insert) and can drop it: nothing is imported then.
 *
 * Which files are offered at all is read from the importers' own accept lists (the file pickers of "Import draw.io",
 * "Import PDF", "Import Office document" and "Import table"), so the offer cannot drift from what imports. Names that
 * only some files of carry an importable model (a PNG or SVG with a draw.io model inside, a .txt that is a table) are
 * left out: their bytes decide when they are dropped on a canvas. */

import { OFFICE_ACCEPT } from '@office/package'
import { TABULAR_ACCEPT } from '@table/files'
import { Emitter } from '../../state/emitter'

export const DRAWIO_ACCEPT = '.drawio,.xml,.png,.svg,application/xml,text/xml,image/png,image/svg+xml'
export const PDF_ACCEPT = '.pdf,application/pdf'

export type ImportFormat = 'drawio' | 'pdf' | 'office' | 'tabular'

const AMBIGUOUS = new Set(['.xml', '.png', '.svg', '.txt', 'application/xml', 'text/xml', 'image/png', 'image/svg+xml'])
const ACCEPTS: [ImportFormat, string[]][] = (
  [['drawio', `.drawio,${DRAWIO_ACCEPT}`], ['pdf', PDF_ACCEPT], ['office', OFFICE_ACCEPT], ['tabular', TABULAR_ACCEPT]] as const
).map(([format, accept]) => [format, accept.split(',').map((a) => a.trim().toLowerCase()).filter((a) => a && !AMBIGUOUS.has(a))])

/** What an original can be imported as, judged by its name and media type before its bytes are read; null: not offered. */
export function importFormatOf(name: string, mime?: string): ImportFormat | null {
  const lower = name.toLowerCase()
  const type = (mime ?? '').split(';')[0].trim().toLowerCase()
  if (/\.drawio(\.xml)?$/.test(lower)) return 'drawio'
  for (const [format, accept] of ACCEPTS) {
    if (accept.some((a) => (a.startsWith('.') ? lower.endsWith(a) : a === type))) return format
  }
  return null
}

export interface ExternalImport {
  id: number
  file: File
  /** Why the page shown cannot take it now (read-only, presenting, no page), when one is shown. */
  blocked: string | null
}

class ExternalImports {
  private pending: ExternalImport | null = null
  private next = 1
  private readonly emitter = new Emitter()
  readonly subscribe = this.emitter.subscribe
  snapshot = () => this.pending
  /** Hand a file over; it replaces one still waiting. */
  offer(file: File) { this.pending = { id: this.next++, file, blocked: null }; this.emitter.emit() }
  /** A canvas takes the file to import it (null when there is none, or it was taken). */
  take(id: number): File | null {
    if (this.pending?.id !== id) return null
    const { file } = this.pending
    this.pending = null
    this.emitter.emit()
    return file
  }
  /** The canvas shown says why it cannot take the file now. */
  block(id: number, reason: string | null) {
    if (this.pending?.id !== id || this.pending.blocked === reason) return
    this.pending = { ...this.pending, blocked: reason }
    this.emitter.emit()
  }
  cancel() { this.pending = null; this.emitter.emit() }
}

export const externalImports = new ExternalImports()
