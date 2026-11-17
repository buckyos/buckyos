import type { FileEntry } from '../types'
import { toUiError } from './state'
import type { UiError } from './state'
import { entryNameSchema } from './schemas'
import { hostConnectionOf } from './hostRealm'
import { DEVICE_SCHEME } from './urls'

export type ConflictChoice = 'keep-both' | 'skip' | 'cancel'
export interface OperationConflict {
  source: FileEntry
  target: FileEntry
  targetPath: string
}
export interface OperationCounts { success: number; failed: number; skipped: number; cancelled: number; pending: number; bytes: number }
export interface OperationResult {
  resultId?: number
  itemKey?: string
  entry: FileEntry
  targetPath?: string
  status: 'success' | 'failed' | 'skipped' | 'cancelled'
  error?: UiError
}
export interface BatchOptions {
  requestKey?: string
  retryOf?: string
  onTask?: (task: { taskId: string; total: number; cancelling: boolean; summary?: OperationCounts; loadMore?: () => Promise<void> }) => void
  onCopyConflict?: (conflict: OperationConflict) => Promise<{ choice: ConflictChoice; apply: boolean }>
  signal?: AbortSignal
  onProgress?: (results: OperationResult[]) => void
  onConflict?: (conflict: OperationConflict) => Promise<ConflictChoice>
}
export interface FolderWriteOps {
  readonly supportsCopy: boolean
  readonly copyUnavailableReason?: string
  copyEntries(entries: FileEntry[], toParentPath: string, options?: BatchOptions): Promise<OperationResult[]>
  resumeCopy?: (taskId: string, options?: BatchOptions) => Promise<OperationResult[]>
  listCopies?: () => Promise<{ task_id: string; name: string; phase: string }[]>
  nameExists(parentPath: string, name: string): Promise<boolean>
  statEntry(parentPath: string, name: string): Promise<FileEntry | null>
  createFolder(parentPath: string, name: string): Promise<void>
  renameEntry(entry: FileEntry, name: string): Promise<void>
  deleteEntries(entries: FileEntry[], options?: BatchOptions): Promise<OperationResult[]>
  moveEntries(entries: FileEntry[], toParentPath: string, options?: BatchOptions): Promise<OperationResult[]>
  downloadUrl(entry: FileEntry): string | null
  /** The bytes of the file at a DFS path, read with the user's session. */
  readContent(path: string): Promise<Blob>
}

export function operationError(code: string, fallback: string): UiError {
  return { code, messageKey: `filebrowser.operation.${code}`, fallback, retryable: true }
}

export async function availableName(parent: string, name: string, folder = false): Promise<string> {
  const dot = folder ? -1 : name.lastIndexOf('.')
  const extension = dot > 0 ? name.slice(dot) : ''
  const base = dot > 0 ? name.slice(0, dot) : name
  for (let i = 2; i < 10000; i++) {
    const suffix = ` (${i})${extension}`
    let stem = base
    while (stem && new TextEncoder().encode(stem + suffix).length > 255) stem = [...stem].slice(0, -1).join('')
    const candidate = entryNameSchema.parse(stem + suffix)
    if (!await folderOps().nameExists(parent, candidate)) return candidate
  }
  throw operationError('CONFLICT', 'Could not find an unused name')
}

export async function runEntryBatch(
  entries: FileEntry[],
  mutate: (entry: FileEntry, name: string) => Promise<void>,
  target?: string,
  options: BatchOptions = {},
  mode: 'move' | 'copy' = 'move',
): Promise<OperationResult[]> {
  const results: OperationResult[] = []
  let cancelled = false
  for (const original of entries) {
    const entry = { ...original }
    let result: OperationResult = { entry, targetPath: target, status: 'success' }
    if (cancelled || options.signal?.aborted) result.status = 'cancelled'
    else try {
      let name = entry.name
      if (target) {
        const parent = entry.path.slice(0, entry.path.lastIndexOf('/')) || '/'
        if (parent === target && mode === 'move') { result.status = 'skipped'; result.error = operationError('SAME_FOLDER', 'Already in this folder') }
        else {
          if (entry.kind === 'folder' && (target === entry.path || target.startsWith(`${entry.path}/`))) throw operationError('DESCENDANT', `A folder cannot be ${mode === 'copy' ? 'copied' : 'moved'} into itself or its descendants`)
          const existing = await folderOps().statEntry(target, name)
          if (existing) {
            if (!options.onConflict) throw operationError('CONFLICT', 'This name already exists in the destination')
            const choice = await options.onConflict({ source: entry, target: existing, targetPath: target })
            if (choice === 'cancel') { cancelled = true; result.status = 'cancelled' }
            else if (choice === 'skip') result.status = 'skipped'
            else name = await availableName(target, name, entry.kind === 'folder')
          }
        }
        result.targetPath = `${target === '/' ? '' : target}/${name}`
      }
      if (options.signal?.aborted) result.status = 'cancelled'
      if (result.status === 'success') await mutate(entry, name)
    } catch (err) {
      result = err instanceof Error && err.name === 'AbortError' ? { ...result, status: 'cancelled' } : { ...result, status: 'failed', error: toUiError(err) }
    }
    results.push(result)
    options.onProgress?.([...results])
  }
  return results
}

const unsupported = () => Promise.reject(operationError('UNSUPPORTED', 'File operations are not available'))
const noOps: FolderWriteOps = {
  supportsCopy: false,
  nameExists: unsupported, statEntry: unsupported, createFolder: unsupported,
  renameEntry: unsupported, deleteEntries: unsupported, moveEntries: unsupported, copyEntries: unsupported, downloadUrl: () => null, readContent: unsupported,
}
let active: FolderWriteOps = noOps
export function registerFolderOps(ops: FolderWriteOps): () => void {
  active = ops
  return () => { if (active === ops) active = noOps }
}

// ─── Realm routing: computer paths (device://) go to the window's sudo connection ───

const isDevice = (path: string) => path.startsWith(DEVICE_SCHEME)
function opsFor(path: string): FolderWriteOps {
  return isDevice(path) ? hostConnectionOf(path).ops : active
}
const routed = <T>(path: string, call: (ops: FolderWriteOps) => Promise<T>): Promise<T> => {
  try { return call(opsFor(path)) } catch (err) { return Promise.reject(err) }
}
/** The connection that can submit a copy touching the computer (null = DFS only). */
function hostSide(entries: FileEntry[], target: string): FolderWriteOps | null {
  const device = isDevice(target) ? target : entries.find((entry) => isDevice(entry.path))?.path
  return device ? hostConnectionOf(device).ops : null
}

/** Between the computer and the Zone a move is a copy, then the originals that copied are deleted. */
async function moveAcross(entries: FileEntry[], target: string, options: BatchOptions = {}): Promise<OperationResult[]> {
  const copied = await (hostSide(entries, target) ?? active).copyEntries(entries, target, options)
  const results: OperationResult[] = []
  for (const result of copied) {
    const original = entries.find((entry) => entry.path === result.entry.path)
    if (result.status !== 'success' || !original) { results.push(result); continue }
    const [removed] = await opsFor(original.path).deleteEntries([original]).catch((err: unknown) => [{ entry: original, status: 'failed' as const, error: toUiError(err) }])
    results.push(removed?.status === 'success' ? { ...result, entry: original } : { ...result, entry: original, status: 'failed', error: operationError('MOVE_SOURCE_KEPT', 'Copied, but the original could not be removed') })
    options.onProgress?.([...results])
  }
  return results
}

const router: FolderWriteOps = {
  get supportsCopy() { return active.supportsCopy },
  get copyUnavailableReason() { return active.copyUnavailableReason },
  copyEntries(entries, target, options) {
    try { return (hostSide(entries, target) ?? active).copyEntries(entries, target, options) } catch (err) { return Promise.reject(err) }
  },
  resumeCopy: (taskId, options) => active.resumeCopy ? active.resumeCopy(taskId, options) : Promise.reject(operationError('UNSUPPORTED', 'Copy tasks are not available')),
  listCopies: () => active.listCopies ? active.listCopies() : Promise.resolve([]),
  nameExists: (parent, name) => routed(parent, (ops) => ops.nameExists(parent, name)),
  statEntry: (parent, name) => routed(parent, (ops) => ops.statEntry(parent, name)),
  createFolder: (parent, name) => routed(parent, (ops) => ops.createFolder(parent, name)),
  renameEntry: (entry, name) => routed(entry.path, (ops) => ops.renameEntry(entry, name)),
  deleteEntries: (entries, options) => routed(entries[0]?.path ?? '', (ops) => ops.deleteEntries(entries, options)),
  async moveEntries(entries, target, options) {
    const across = entries.some((entry) => isDevice(entry.path) !== isDevice(target))
    if (across) return moveAcross(entries, target, options)
    const results = await routed(target, (ops) => ops.moveEntries(entries, target, options))
    if (!isDevice(target)) return results
    // On the computer a rename across disks is refused; those items move as copy + delete.
    const acrossDisks = results.filter((result) => result.status === 'failed' && result.error?.code === 'UNSUPPORTED')
    if (!acrossDisks.length) return results
    const retried = await moveAcross(acrossDisks.map((result) => result.entry), target, options)
    return results.map((result) => retried.find((moved) => moved.entry.path === result.entry.path && acrossDisks.includes(result)) ?? result)
  },
  downloadUrl(entry) {
    try { return opsFor(entry.path).downloadUrl(entry) } catch { return null }
  },
  readContent: (path) => routed(path, (ops) => ops.readContent(path)),
}
export function folderOps(): FolderWriteOps { return router }
