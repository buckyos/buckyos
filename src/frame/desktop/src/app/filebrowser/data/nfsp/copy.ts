import type { CopyInput, CopyItem, CopyView } from '../../../../api/nfs_copy'
import type { FileEntry } from '../../types'
import type { BatchOptions, OperationResult } from '../folderOps'
import { operationError } from '../folderOps'
import { copyEntryOf as entryOf, copyResultOf as resultOf } from '../copyResult'
import { ensureSession, nfspClient } from './client'
import { notifyDfsPath } from './invalidation'
import { NfspError, type WireRef } from '../../../../api/nfsp_client'

const call = async <T>(method: string, args: Record<string, unknown>): Promise<T> => {
  await ensureSession()
  return nfspClient().copyCall<T>(method, args)
}
export async function listCopies() {
  return (await call<{ tasks: { task_id: string; name: string; phase: string }[] }>('copy_list', {})).tasks
}
const pendingSubmissions = new Map<string, CopyInput>()
export async function copyEntries(entries: FileEntry[], target: string, options: BatchOptions = {}) {
  const key = options.requestKey ?? crypto.randomUUID()
  let input: CopyInput
  const pendingInput = pendingSubmissions.get(key)
  if (pendingInput) input = pendingInput
  else if (options.retryOf) {
    const previous = await call<CopyView>('copy_get', { task_id: options.retryOf })
    input = { ...previous.task.input as unknown as CopyInput, retry_of: options.retryOf }
  } else {
    const destination = await nfspClient().stat(target, { cache: 'no-cache' })
    if (!destination) throw operationError('NOT_FOUND', 'The destination folder no longer exists')
    input = {
      sources: entries.map((entry) => ({ source_ref: entry.copyRef ? JSON.parse(entry.copyRef) as WireRef : { type: 'live', node_id: entry.id }, source_path: entry.path, name: entry.name })),
      destination_ref: destination.copy_ref ?? destination.ref, conflict: 'ask', retry_of: null,
    }
  }
  options.signal?.throwIfAborted()
  pendingSubmissions.set(key, input)
  const args = { input, idempotency_key: key }
  let taskId = ''
  for (let attempt = 0; attempt < 3; attempt++) {
    try { taskId = (await call<{ task_id: string }>('copy_submit', args)).task_id; break }
    catch (error) { if (error instanceof NfspError || attempt === 2) throw error }
  }
  pendingSubmissions.delete(key)
  options.onTask?.({ taskId, total: input.sources.length, cancelling: false })
  return resumeCopy(taskId, options)
}
export async function resumeCopy(taskId: string, options: BatchOptions = {}): Promise<OperationResult[]> {
  let cancelSent = false
  let next: number | null = null
  const rows = new Map<number, CopyItem>()
  const completed = () => [...rows.values()].filter((item) => ['success', 'failed', 'skipped', 'cancelled'].includes(item.status)).map(resultOf)
  const loadMore = async () => {
    if (next === null) return
    const page = await call<CopyView>('copy_get', { task_id: taskId, after: next })
    page.items.forEach((item) => rows.set(item.id, item))
    next = page.next
    options.onProgress?.(completed())
    report(page)
  }
  const report = (view: CopyView) => options.onTask?.({
    taskId, summary: view.summary, total: view.summary.success + view.summary.failed + view.summary.skipped + view.summary.cancelled + view.summary.pending,
    cancelling: !!view.task.pending_control || (cancelSent && view.task.phase !== 'Terminal'),
    loadMore: next !== null ? loadMore : undefined,
  })
  let errors = 0
  for (;;) {
    let view: CopyView
    try {
      if (options.signal?.aborted && !cancelSent) {
        await call('copy_cancel', { task_id: taskId, request_id: `cancel:${taskId}` })
        cancelSent = true
      }
      view = await call<CopyView>('copy_get', { task_id: taskId })
      errors = 0
    } catch (error) {
      if (++errors > 30) throw error
      await new Promise((resolve) => setTimeout(resolve, 1000))
      continue
    }
    view.items.forEach((item) => {
      const previous = rows.get(item.id)
      rows.set(item.id, item)
      if (item.identity && previous?.status !== item.status) {
        const parent = item.target_path.slice(0, item.target_path.lastIndexOf('/')) || '/'
        void nfspClient().invalidateContainer(parent).then(() => notifyDfsPath(parent))
      }
    })
    if (rows.size <= 100) next = view.next
    report(view)
    options.onProgress?.(completed())
    if (view.task.phase === 'Terminal') {
      const lastLoaded = [...rows.keys()].reduce((max, id) => Math.max(max, id), 0)
      let after = view.next
      while (after !== null && after < lastLoaded) {
        const page = await call<CopyView>('copy_get', { task_id: taskId, after })
        page.items.forEach((item) => rows.set(item.id, item))
        after = page.next
      }
      options.onProgress?.(completed())
      const input = view.task.input as unknown as CopyInput
      for (const source of input.sources) {
        const result = view.items.find((item) => item.source_path === source.source_path)
        if (result) notifyDfsPath(result.target_path.slice(0, result.target_path.lastIndexOf('/')) || '/')
      }
      if (!view.items.length && view.task.error) throw operationError(view.task.error.code, view.task.error.message)
      return completed()
    }
    if (view.conflict && !cancelSent) {
      const source = entryOf(view.conflict)
      const target = entryOf({ ...view.conflict, source_path: view.conflict.target_path, kind: view.conflict.error?.target_kind ?? 'unknown', size: view.conflict.error?.target_size, mtime: view.conflict.error?.target_mtime })
      const conflict = { source, target, targetPath: view.conflict.target_path }
      if (!options.onCopyConflict && !options.onConflict) throw operationError('CONFLICT', 'Copy is waiting for a conflict decision; reopen the task to continue')
      const decision = options.onCopyConflict ? await options.onCopyConflict(conflict) : { choice: await options.onConflict!(conflict), apply: false }
      await call('copy_decide', { task_id: taskId, item_id: view.conflict.id, ...decision })
    }
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
}
export async function copyCapability(): Promise<boolean> {
  return (await call<{ supported: boolean }>('copy_capabilities', {})).supported
}
