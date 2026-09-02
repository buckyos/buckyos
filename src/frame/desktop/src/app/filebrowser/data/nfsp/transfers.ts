/**
 * NFSP transfer executor (UI_DATAMODEL.md §4.7, §8.3): drives the real
 * probe → open_write/tus → commit_file pipeline behind the transfer store's
 * stage model. A probe hit commits by hash (秒传) and skips the upload;
 * NEED_PULL falls through to the byte upload. Name collisions surface as a
 * conflict before any bytes move. Cancellation is honored between chunks.
 */

import type { CommitResult } from '../../../../api/nfsp_client'
import type { FileEntry } from '../../types'
import { classifyFileKind } from '../fileKinds'
import type { TransferControls } from '../transfers'
import { registerTransferExecutor, takeLocalFile, TransferCancelledError } from '../transfers'
import type { TransferTask } from '../state'
import { dfsPathOf } from '../urls'
import { ensureSession, nfspClient } from './client'
import { nfspToUiError } from './errors'
import { notifyDfsPath } from './invalidation'

const CHUNK_SIZE = 4 * 1024 * 1024

function checkCancelled(controls: TransferControls) {
  if (controls.isCancelled()) throw new TransferCancelledError()
}

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', bytes as BufferSource)
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, '0')).join('')
}

function committedEntry(task: TransferTask, parentPath: string, result: CommitResult): FileEntry {
  const name = task.candidate.name
  return {
    id: result.ref.type === 'live' ? result.ref.node_id : result.obj.sha256,
    name,
    kind: classifyFileKind(name, task.candidate.mimeType),
    path: parentPath === '/' ? `/${name}` : `${parentPath}/${name}`,
    sizeBytes: result.obj.size,
    modifiedAt: new Date().toISOString(),
    source: { type: 'local', label: 'Uploaded' },
  }
}

async function runNfspTransfer(
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
  const file = takeLocalFile(task.candidate.localId)
  if (!file) {
    throw {
      code: 'INTERNAL',
      messageKey: 'filebrowser.transfer.missingFile',
      fallback: 'The selected file is no longer available — pick it again',
      retryable: false,
    }
  }

  await ensureSession()
  const client = nfspClient()
  const name = task.candidate.name

  try {
    checkCancelled(controls)
    controls.setStatus('hashing')
    const bytes = new Uint8Array(await file.arrayBuffer())
    const hash = `sha256:${await sha256Hex(bytes)}`

    checkCancelled(controls)
    controls.setStatus('probing')
    // Collision requires a user decision (§4.7) before any bytes move.
    const existing = await client.stat(parentPath, { name, cache: 'no-cache' }).catch(() => null)
    if (existing) {
      throw {
        code: 'NAMESPACE_CONFLICT',
        messageKey: 'filebrowser.transfer.conflict',
        fallback: `"${name}" already exists here`,
        retryable: false,
      }
    }
    const parentInfo = await client.resolve(parentPath)
    if (!parentInfo) throw nfspToUiError({ code: 'NOT_FOUND', message: 'destination gone' })
    const probe = await client.raw.probe([{ hash, size: bytes.byteLength }])
    if (probe.missing.length === 0) {
      // Content already known: commit by hash (秒传). NEED_PULL falls back
      // to the byte upload instead of failing the transfer (§8.5).
      controls.setStatus('committing')
      try {
        const result = await client.commitFile(parentPath, name, { hash })
        notifyDfsPath(parentPath)
        return committedEntry(task, parentPath, result)
      } catch (err) {
        if (nfspToUiError(err).code !== 'NEED_PULL') throw err
      }
    }

    checkCancelled(controls)
    controls.setStatus('uploading')
    const openResult = await client.raw.openWrite({
      parentRef: parentInfo.ref,
      name,
      size: bytes.byteLength,
    })
    let offset = await client.raw.uploadOffset(openResult.fb_handle)
    controls.setProgress(offset)
    while (offset < bytes.byteLength) {
      checkCancelled(controls)
      const chunk = bytes.subarray(offset, Math.min(offset + CHUNK_SIZE, bytes.byteLength))
      offset = await client.raw.uploadChunk(openResult.fb_handle, offset, chunk)
      controls.setProgress(offset)
    }

    checkCancelled(controls)
    controls.setStatus('committing')
    const result = await client.commitFile(parentPath, name, {
      fbHandle: openResult.fb_handle,
      leaseId: openResult.lease.lease_id,
    })
    notifyDfsPath(parentPath)
    return committedEntry(task, parentPath, result)
  } catch (err) {
    if (err instanceof TransferCancelledError) throw err
    throw nfspToUiError(err)
  }
}

export function registerNfspTransferExecutor() {
  return registerTransferExecutor({ run: runNfspTransfer })
}
