/* A request to import an Office document (Office 导入 TODO §6): who asked (the insert menu, a dropped or pasted file,
 * an Office file Block's "expand") and what happens after. ImportOfficeDialog.tsx shows it from the Workspace shell;
 * the canvas being shown registers how an import ends (switch to the new page, notify, report). */

import type { OfficeKind } from '@office/package'
import { Emitter } from '../../state/emitter'
import type { ImportOutcome } from '../canvas/drawioImport'

export interface OfficeImportSource {
  name: string
  bytes: Uint8Array
  kind: OfficeKind
  /** The file is already an asset (an Office file Block): it is not uploaded again. */
  asset?: { object_id: string } | null
}

export interface OfficeImportRequest {
  source: OfficeImportSource
  /** `ask`: a dropped or pasted file (insert as a file, or import). */
  step: 'ask' | 'options'
  insertAsFile?: () => void
  /** Called with the result once the import has run; the shown canvas's `finisher` when absent. */
  finish?: (outcome: ImportOutcome) => void
}

class RequestSink {
  private request: OfficeImportRequest | null = null
  private readonly emitter = new Emitter()
  readonly subscribe = this.emitter.subscribe
  /** How the canvas being shown ends an import (set while it is mounted). */
  finisher: ((outcome: ImportOutcome) => void) | null = null
  snapshot = () => this.request
  open(request: OfficeImportRequest | null) { this.request = request; this.emitter.emit() }
  get busy() { return this.request !== null }
}
export const officeImports = new RequestSink()
