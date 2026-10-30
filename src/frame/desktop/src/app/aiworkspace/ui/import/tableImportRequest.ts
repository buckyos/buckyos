/* A request to import table data (表格数据导入 TODO §4, §5): where it comes from (a paste, a file), whether the user
 * is asked first (a dropped or pasted single file: a table, or a file), and where the table goes — a canvas (its
 * content folder, with a table Block) or a folder of the data tree (no Block). TableImportDialog.tsx shows it at the
 * workspace shell level, so the data-tree mode can open it too; the requester says how an import ends. */

import { Emitter } from '../../state/emitter'
import type { ImportOutcome } from '../canvas/drawioImport'

export type TableImportSource =
  /** A paste: the clipboard's HTML and plain text (one of them holds the table). */
  | { kind: 'clipboard'; html: string | null; text: string | null }
  | { kind: 'file'; name: string; bytes: Uint8Array }

export interface CanvasDestination {
  surfaceId: string
  isFree: boolean
  contentFolderId: string
  /** World point of the first Block's top-left (a drop or paste point), or null: centred on `center`. */
  at: { x: number; y: number } | null
  center: { x: number; y: number }
  /** A flow page: the Block goes after this one (the selected Block), else last. */
  afterId?: string | null
}

export type TableDestination = { canvas: CanvasDestination } | { folderId: string }

export interface TableImportRequest {
  source: TableImportSource
  step: 'ask' | 'wizard'
  destination: TableDestination
  insertAsFile?: () => void
  /** Called once the import has run (select the new Blocks, fit the view). */
  finish?: (outcome: ImportOutcome) => void
}

class RequestSink {
  private request: TableImportRequest | null = null
  private readonly emitter = new Emitter()
  readonly subscribe = this.emitter.subscribe
  snapshot = () => this.request
  open(request: TableImportRequest | null) { this.request = request; this.emitter.emit() }
  get busy() { return this.request !== null }
}
export const tableImports = new RequestSink()
