/**
 * Mock transfer executor (UI_DATAMODEL.md §4.7, §7.2 upload fixtures).
 *
 * Simulates the real probe → upload → commit pipeline stage by stage. On
 * commit the entry is inserted into the in-memory index and the destination
 * path invalidated, so the "placeholder is removed once the destination
 * reader exposes the committed entry" loop is fully experienceable.
 *
 * Deterministic scenarios (no timing dependence):
 *   - a file name containing "fail" errors during upload; Retry succeeds
 *   - a name that already exists in the destination commits with a conflict
 */

import type { FileEntry } from '../types'
import type { TransferControls, TransferExecutor } from '../data/transfers'
import { registerTransferExecutor, TransferCancelledError } from '../data/transfers'
import type { TransferTask } from '../data/state'
import { classifyFileKind } from '../data/fileKinds'
import { dfsPathOf } from '../data/urls'
import { invalidateMockPath, mockDelay } from '../data/mockReader'
import { mockAddEntry, mockNameExists } from './data'

function checkCancelled(controls: TransferControls) {
  if (controls.isCancelled()) throw new TransferCancelledError()
}

const failedOnce = new Set<string>()

async function runMockTransfer(
  task: TransferTask,
  controls: TransferControls,
): Promise<FileEntry> {
  const parentPath = dfsPathOf(task.targetUrl)
  if (parentPath === null) {
    throw {
      code: 'UNSUPPORTED',
      messageKey: 'filebrowser.transfer.badTarget',
      fallback: 'This location does not accept uploads',
      retryable: false,
    }
  }

  checkCancelled(controls)
  controls.setStatus('hashing')
  await mockDelay(120, 200)

  checkCancelled(controls)
  controls.setStatus('probing')
  await mockDelay(80, 150)
  if (mockNameExists(parentPath, task.candidate.name)) {
    // Collision requires a user decision (§4.7) — surfaced as a conflict.
    throw {
      code: 'NAMESPACE_CONFLICT',
      messageKey: 'filebrowser.transfer.conflict',
      fallback: `"${task.candidate.name}" already exists here`,
      retryable: false,
    }
  }

  controls.setStatus('uploading')
  const chunks = 5
  for (let i = 1; i <= chunks; i += 1) {
    checkCancelled(controls)
    await mockDelay(90, 160)
    controls.setProgress(Math.round((task.totalBytes * i) / chunks))
    if (
      i === 3 &&
      task.candidate.name.toLowerCase().includes('fail') &&
      !failedOnce.has(task.id)
    ) {
      failedOnce.add(task.id)
      throw new Error('Mock upload interrupted — retry succeeds')
    }
  }

  checkCancelled(controls)
  controls.setStatus('committing')
  await mockDelay(100, 180)

  const entry: FileEntry = {
    id: `upload-${task.candidate.localId}`,
    name: task.candidate.name,
    kind: classifyFileKind(task.candidate.name, task.candidate.mimeType),
    path: parentPath === '/' ? `/${task.candidate.name}` : `${parentPath}/${task.candidate.name}`,
    sizeBytes: task.candidate.sizeBytes,
    modifiedAt: new Date().toISOString(),
    source: { type: 'local', label: 'Uploaded (mock)' },
  }
  mockAddEntry(entry)
  invalidateMockPath(parentPath)
  return entry
}

export function registerMockTransferExecutor() {
  const executor: TransferExecutor = { run: runMockTransfer }
  return registerTransferExecutor(executor)
}
