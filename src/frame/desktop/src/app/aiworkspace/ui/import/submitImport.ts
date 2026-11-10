/* One import is one Commit (导入原子化): everything an importer creates goes out in a single `store.submit` — the
 * client sends a large Commit in parts and the service assembles it — so the import is there as a whole or not at
 * all, and the undo stack gets one entry. An import past what one Commit may hold is refused before anything is
 * sent. */

import { commitOverLimit } from 'buckyos'
import type { Operation } from '../../api/types'
import { aiwsT } from '../../i18n'
import type { WorkspaceStore } from '../../state/store'

/** `commit`: request `done + 1` of `total` is going up; `write`: the last one is in, the service is writing. */
export interface ImportCommitProgress { stage: 'commit' | 'write'; done: number; total: number }

/** What the user can do about an import that is too large: fewer pages, or several files. */
export type ImportTooLargeHint = 'pages' | 'files'

/** `null` when the Commit was accepted (or saved locally); the reason otherwise. Nothing was written then. */
export async function submitImport(
  store: WorkspaceStore,
  options: { editId: string; label: string; operations: Operation[] },
  hint: ImportTooLargeHint,
  progress?: (p: ImportCommitProgress) => void,
): Promise<string | null> {
  const over = commitOverLimit(options.operations)
  if (over) {
    const what = aiwsT(over.limit === 'operations' ? 'aiworkspace.import.overLimit.operations' : 'aiworkspace.import.overLimit.bytes', {
      value: over.limit === 'operations' ? over.value : Math.ceil(over.value / (1024 * 1024)),
      max: over.limit === 'operations' ? over.max : Math.floor(over.max / (1024 * 1024)),
    })
    return `${what} ${aiwsT(hint === 'pages' ? 'aiworkspace.import.overLimit.hintPages' : 'aiworkspace.import.overLimit.hintFiles')}`
  }
  progress?.({ stage: 'write', done: 0, total: 1 })
  const result = await store.submit({
    ...options,
    onPart: (part, total) => progress?.(part < total ? { stage: 'commit', done: part - 1, total: total - 1 } : { stage: 'write', done: 0, total: 1 }),
  })
  if (result.status === 'accepted' || result.status === 'saved_locally') return null
  if (result.status === 'rejected' || result.status === 'conflict') {
    return result.detail ? aiwsT('aiworkspace.canvas.codeWithDetail', { code: result.code, detail: String(result.detail) }) : result.code
  }
  return 'detail' in result && result.detail ? String(result.detail) : result.status
}
