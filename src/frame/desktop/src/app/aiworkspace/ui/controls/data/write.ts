/* Writes of a data control (控件机制 TODO G4): the operations of a gesture go out as one Commit (undoable, conflicts kept
 * in the save-state list), the change shows at once as a local overlay of the records, and a refused or conflicting
 * Commit puts the service's values back with a notice saying why. */

import type { Json, Operation } from '../../../api/types'
import { aiwsT } from '../../../i18n'
import type { WorkspaceStore } from '../../../state/store'
import type { RecordSet } from './records'

export interface DataWrite {
  editId: string
  label: string
  operations: Operation[]
  /** Values shown at once, per record (undefined unsets). */
  overlay?: Record<string, Record<string, Json | undefined>>
  /** Records added (shown at once with these values). */
  created?: Record<string, Record<string, Json | undefined>>
  /** Records removed (hidden at once). */
  removed?: string[]
}

/** True when the service accepted (or this device saved it for later). */
export async function submitData(store: WorkspaceStore, set: RecordSet | null, write: DataWrite): Promise<boolean> {
  if (write.operations.length === 0) return true
  const touched = [...Object.keys(write.overlay ?? {}), ...Object.keys(write.created ?? {}), ...(write.removed ?? [])]
  for (const [id, values] of Object.entries(write.overlay ?? {})) set?.overlay(id, values)
  for (const [id, values] of Object.entries(write.created ?? {})) set?.overlay(id, values, true)
  if (write.removed?.length) set?.hide(write.removed)
  const outcome = await store.submit({ editId: write.editId, label: write.label, operations: write.operations as Operation[] })
  if (outcome.status === 'accepted' || outcome.status === 'saved_locally') return true
  set?.dropOverlay(touched)
  if (outcome.status === 'conflict') store.notify('info', aiwsT('aiworkspace.datactl.conflict'))
  else if (outcome.status === 'rejected') store.notify('error', aiwsT('aiworkspace.datactl.rejected', { reason: outcome.errors?.[0]?.detail ?? outcome.detail ?? outcome.code }))
  return false
}
