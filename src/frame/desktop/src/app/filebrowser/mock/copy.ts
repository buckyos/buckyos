import type { FileEntry } from '../types'
import { operationError, runEntryBatch, type BatchOptions, type OperationResult } from '../data/folderOps'
import { mockAddEntry, mockEntriesAtPath, mockEntryById, mockNameExists } from './data'
import { invalidateMockPath, mockDelay } from '../data/mockReader'

interface MockCopyRecord { sourceId: string; targetId: string; targetPath: string; complete: boolean }
const tasks = new Map<string, Map<string, MockCopyRecord>>()
const failures = new Set<string>()
export async function mockCopyEntries(entries: FileEntry[], target: string, options: BatchOptions = {}) {
  const taskId = crypto.randomUUID()
  const previous = options.retryOf ? tasks.get(options.retryOf) : undefined
  const records = new Map(previous)
  tasks.set(taskId, records)
  const details: OperationResult[] = []
  const copy = async (entry: FileEntry, path: string): Promise<void> => {
    options.signal?.throwIfAborted()
    await mockDelay(new URLSearchParams(location.search).has('fbSlowOps') ? 900 : 20, new URLSearchParams(location.search).has('fbSlowOps') ? 900 : 40)
    const live = mockEntryById(entry.id)
    if (!live || live.path !== entry.path) throw operationError('STALE', 'The source moved or was replaced')
    if (entry.name.startsWith('denied-')) throw operationError('PERMISSION_DENIED', 'The source is not readable')
    if (entry.name.startsWith('fail-once-') && !failures.has(entry.id)) { failures.add(entry.id); throw operationError('NETWORK', 'The request was interrupted. Please retry.') }
    const record = records.get(entry.path)
    if (record) {
      if (mockEntryById(record.targetId)?.path !== record.targetPath) throw operationError('STALE', 'The recorded copy directory changed')
      path = record.targetPath
      if (record.complete) return
    } else {
      const parent = path.slice(0, path.lastIndexOf('/')) || '/'
      if (!mockEntriesAtPath(parent)) throw operationError('NOT_FOUND', 'The destination folder no longer exists')
      if (parent.startsWith('/readonly')) throw operationError('PERMISSION_DENIED', 'The destination is read-only')
      const name = path.split('/').pop()!
      if (mockNameExists(parent, name)) throw operationError('CONFLICT', 'The destination name already exists')
      const targetId = crypto.randomUUID()
      mockAddEntry({ id: targetId, name, path, kind: entry.kind, modifiedAt: live.modifiedAt, sizeBytes: live.sizeBytes })
      records.set(entry.path, { sourceId: entry.id, targetId, targetPath: path, complete: false })
      invalidateMockPath(parent)
    }
    let failed = false
    if (entry.kind === 'folder') {
      for (const child of [...mockEntriesAtPath(entry.path) ?? []]) {
        const result: OperationResult = { entry: { ...child }, targetPath: `${path}/${child.name}`, status: 'success' }
        try { await copy(child, result.targetPath!) } catch (error) {
          result.status = options.signal?.aborted ? 'cancelled' : 'failed'
          result.error = error as ReturnType<typeof operationError>
          failed = true
        }
        details.push(result)
        options.onProgress?.([...details])
      }
    }
    if (failed) throw operationError('PARTIAL_FAILURE', 'Some children could not be copied; created children remain')
    records.get(entry.path)!.complete = true
  }
  options.onTask?.({ taskId, total: entries.length, cancelling: false })
  const result = await runEntryBatch(entries, async (entry, name) => {
    await copy(entry, previous?.get(entry.path)?.targetPath ?? `${target === '/' ? '' : target}/${name}`)
  }, previous ? undefined : target, options, 'copy')
  const results = [...result.map((item) => ({ ...item, targetPath: records.get(item.entry.path)?.targetPath ?? item.targetPath })), ...details]
  options.onTask?.({ taskId, total: results.length, cancelling: false })
  options.onProgress?.(results)
  return results
}
