/**
 * Folder write operations — the extension point behind toolbar/menu mutations
 * on real storage locations (new folder, rename, delete, move, download).
 * Collection membership stays on CollectionReader; uploads stay on the
 * transfer executor. The mock registers in-memory-index operations; the NFSP
 * adapter registers mkdir/move/delete/readUrl (UI_DATAMODEL.md §8.3).
 *
 * Implementations throw UiError-shaped values (data/state.ts) so callers can
 * surface normalized, display-safe messages.
 */

import type { FileEntry } from '../types'

export interface FolderWriteOps {
  /** True when `name` already exists directly under `parentPath`. */
  nameExists(parentPath: string, name: string): Promise<boolean>
  createFolder(parentPath: string, name: string): Promise<void>
  renameEntry(entry: FileEntry, name: string): Promise<void>
  /** Destroy semantics — never used for collection references (§6.3). */
  deleteEntries(entries: FileEntry[]): Promise<void>
  moveEntries(entries: FileEntry[], toParentPath: string): Promise<void>
  /**
   * Data-plane download URL for a file entry, or null when the backend has
   * no data plane (mock) — callers fall back to an informational toast.
   */
  downloadUrl(entry: FileEntry): string | null
}

const unsupported = () =>
  Promise.reject({
    code: 'UNSUPPORTED',
    messageKey: 'filebrowser.error.noWriteBackend',
    fallback: 'File operations are not available',
    retryable: false,
  })

const noOps: FolderWriteOps = {
  nameExists: () => Promise.resolve(false),
  createFolder: unsupported,
  renameEntry: unsupported,
  deleteEntries: unsupported,
  moveEntries: unsupported,
  downloadUrl: () => null,
}

let active: FolderWriteOps = noOps

export function registerFolderOps(ops: FolderWriteOps): () => void {
  active = ops
  return () => {
    if (active === ops) active = noOps
  }
}

export function folderOps(): FolderWriteOps {
  return active
}
