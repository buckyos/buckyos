import type { PendingChangeRecord } from './types'

export function summarizePendingChanges(changes: PendingChangeRecord[]) {
  return {
    create: changes.filter((change) => change.action === 'create').length,
    update: changes.filter((change) => change.action === 'update').length,
    delete: changes.filter((change) => change.action === 'delete').length,
    disable: changes.filter((change) => change.action === 'disable').length,
  }
}
