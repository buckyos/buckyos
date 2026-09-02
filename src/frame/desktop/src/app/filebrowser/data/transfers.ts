/**
 * Upload/transfer progress model (UI_DATAMODEL.md §4.7).
 *
 * The store owns the task list and its observable lifecycle
 * (queued → hashing → probing → uploading → committing → success, plus
 * error/cancelled); a pluggable executor performs the actual work — the mock
 * executor simulates stages, the real one drives NFSP probe/tus/commit.
 * Browser `File` objects are stashed out-of-band by `localId` and never enter
 * shared state.
 */

import type { FileEntry, LocationUrl } from '../types'
import type { TransferStatus, TransferTask } from './state'
import { toUiError } from './state'
import type { UploadCandidateInput } from './schemas'
import { uploadCandidateSchema } from './schemas'

export interface TransferControls {
  /** Executors poll this between stages and abort with CancelledError. */
  isCancelled(): boolean
  setStatus(status: Exclude<TransferStatus, 'success' | 'error' | 'cancelled'>): void
  setProgress(bytesSent: number): void
}

export interface TransferExecutor {
  /** Resolves with the committed entry once the destination exposes it. */
  run(task: TransferTask, controls: TransferControls): Promise<FileEntry>
}

export class TransferCancelledError extends Error {
  constructor() {
    super('transfer cancelled')
  }
}

let executor: TransferExecutor | null = null

export function registerTransferExecutor(next: TransferExecutor): () => void {
  executor = next
  return () => {
    if (executor === next) executor = null
  }
}

// ─── Out-of-band File stash (§3: never serialized into shared state) ───

const localFiles = new Map<string, File>()

export function stashLocalFile(localId: string, file: File) {
  localFiles.set(localId, file)
}

export function takeLocalFile(localId: string): File | undefined {
  return localFiles.get(localId)
}

// ─── Store ───

export interface RejectedUpload {
  name: string
  /** i18n message keys from uploadCandidateSchema. */
  messageKeys: string[]
}

const tasks: TransferTask[] = []
const listeners = new Set<() => void>()
const cancelRequested = new Set<string>()
let snapshotVersion = 0
let taskCounter = 0

function emit() {
  snapshotVersion += 1
  for (const listener of listeners) listener()
}

function taskById(id: string): TransferTask | undefined {
  return tasks.find((task) => task.id === id)
}

function runTask(task: TransferTask) {
  if (!executor) {
    task.status = 'error'
    task.error = {
      code: 'UNSUPPORTED',
      messageKey: 'filebrowser.transfer.noExecutor',
      fallback: 'Uploads are not available',
      retryable: false,
    }
    emit()
    return
  }
  cancelRequested.delete(task.id)
  const controls: TransferControls = {
    isCancelled: () => cancelRequested.has(task.id),
    setStatus: (status) => {
      task.status = status
      emit()
    },
    setProgress: (bytesSent) => {
      task.bytesSent = Math.min(bytesSent, task.totalBytes)
      emit()
    },
  }
  executor
    .run(task, controls)
    .then((entry) => {
      task.status = 'success'
      task.bytesSent = task.totalBytes
      task.committedAt = new Date().toISOString()
      task.committedEntry = entry
      task.error = null
      localFiles.delete(task.candidate.localId)
      emit()
    })
    .catch((err: unknown) => {
      if (err instanceof TransferCancelledError || cancelRequested.has(task.id)) {
        task.status = 'cancelled'
        task.error = null
      } else {
        task.status = 'error'
        task.error = toUiError(err)
      }
      emit()
    })
}

export const transferStore = {
  subscribe(listener: () => void): () => void {
    listeners.add(listener)
    return () => listeners.delete(listener)
  },

  snapshot(): number {
    return snapshotVersion
  },

  tasks(): readonly TransferTask[] {
    return tasks
  },

  /**
   * Validate candidates and start accepted ones. Invalid candidates come back
   * with their schema message keys so the caller can surface them.
   */
  enqueue(
    targetUrl: LocationUrl,
    candidates: UploadCandidateInput[],
  ): { accepted: TransferTask[]; rejected: RejectedUpload[] } {
    const accepted: TransferTask[] = []
    const rejected: RejectedUpload[] = []
    for (const raw of candidates) {
      const parsed = uploadCandidateSchema.safeParse(raw)
      if (!parsed.success) {
        rejected.push({
          name: raw.name || raw.localId,
          messageKeys: parsed.error.issues.map((issue) => issue.message),
        })
        continue
      }
      taskCounter += 1
      const task: TransferTask = {
        id: `transfer-${taskCounter}`,
        targetUrl,
        candidate: parsed.data,
        status: 'queued',
        bytesSent: 0,
        totalBytes: parsed.data.sizeBytes,
        error: null,
      }
      tasks.push(task)
      accepted.push(task)
    }
    if (accepted.length) emit()
    for (const task of accepted) runTask(task)
    return { accepted, rejected }
  },

  cancel(id: string) {
    const task = taskById(id)
    if (!task) return
    if (task.status === 'success' || task.status === 'error' || task.status === 'cancelled') {
      return
    }
    // The executor notices at its next stage boundary and settles the task.
    cancelRequested.add(id)
  },

  retry(id: string) {
    const task = taskById(id)
    if (!task || (task.status !== 'error' && task.status !== 'cancelled')) return
    task.status = 'queued'
    task.bytesSent = 0
    task.error = null
    emit()
    runTask(task)
  },

  dismiss(id: string) {
    const index = tasks.findIndex((task) => task.id === id)
    if (index === -1) return
    const task = tasks[index]
    if (task.status !== 'success' && task.status !== 'error' && task.status !== 'cancelled') {
      return
    }
    localFiles.delete(task.candidate.localId)
    tasks.splice(index, 1)
    emit()
  },
}
