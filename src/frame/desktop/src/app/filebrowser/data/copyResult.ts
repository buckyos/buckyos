import type { CopyItem } from '../../../api/nfs_copy'
import type { FileEntry } from '../types'
import type { OperationResult } from './folderOps'
import { operationError } from './folderOps'
import { classifyFileKind } from './fileKinds'

export const copyEntryOf = (item: CopyItem): FileEntry => ({
  id: String(item.id), path: item.source_path, name: item.source_path.split('/').pop() ?? item.source_path,
  kind: item.kind === 'dir' ? 'folder' : classifyFileKind(item.source_path),
  sizeBytes: item.kind === 'file' ? item.size ?? undefined : undefined,
  modifiedAt: typeof item.mtime === 'number' && Number.isFinite(item.mtime) ? new Date(item.mtime * 1000).toISOString() : '',
})
export const copyResultOf = (item: CopyItem): OperationResult => ({
  resultId: item.id, entry: copyEntryOf(item), targetPath: item.target_path,
  status: ['success', 'skipped', 'cancelled'].includes(item.status) ? item.status as OperationResult['status'] : 'failed',
  error: item.error ? operationError(item.error.code, item.error.message) : undefined,
})
