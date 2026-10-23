import { getActiveSessionToken, ndm } from 'buckyos'
import type { CommitObjectResult } from '../../../../api/nfsp_client'
import type { FileEntry } from '../../types'
import { classifyFileKind } from '../fileKinds'
import type { TransferControls } from '../transfers'
import { registerTransferExecutor, takeLocalInput, TransferCancelledError } from '../transfers'
import type { TransferTask } from '../state'
import { dfsPathOf } from '../urls'
import { ensureSession, nfspBaseUrl, nfspClient } from './client'
import { nfspToUiError } from './errors'
import { notifyDfsPath } from './invalidation'

const imports = new Map<string, { sessionId: string; objectId: string }>()

function checkCancelled(controls: TransferControls) {
  if (controls.isCancelled() || controls.signal.aborted) throw new TransferCancelledError()
}

function committedEntry(task: TransferTask, parentPath: string, result: CommitObjectResult): FileEntry {
  const name = task.candidate.name
  return {
    id: result.ref.type === 'live' ? result.ref.node_id : result.obj_id,
    name,
    kind: result.kind === 'dir' ? 'folder' : classifyFileKind(name, task.candidate.mimeType),
    path: parentPath === '/' ? `/${name}` : `${parentPath}/${name}`,
    sizeBytes: result.obj.size,
    modifiedAt: new Date().toISOString(),
    source: { type: 'local', label: 'Uploaded' },
  }
}

async function runNfspTransfer(task: TransferTask, controls: TransferControls): Promise<FileEntry> {
  const parentPath = dfsPathOf(task.targetUrl)
  if (parentPath === null) throw { code: 'UNSUPPORTED', fallback: 'This location does not accept uploads', retryable: false }
  const input = takeLocalInput(task.candidate.localId)
  if (!input) throw { code: 'INTERNAL', fallback: 'The selected files are no longer available — pick them again', retryable: false }
  await ensureSession()
  const client = nfspClient()
  const name = task.candidate.name
  try {
    checkCancelled(controls)
    controls.setStatus('probing')
    const existing = await client.stat(parentPath, { name, cache: 'no-cache' }).catch((error: unknown) => {
      if (nfspToUiError(error).code === 'NOT_FOUND') return null
      throw error
    })
    if (existing && !imports.has(task.candidate.localId)) throw { code: 'NAMESPACE_CONFLICT', fallback: `"${name}" already exists here; choose another destination name`, retryable: true }
    const endpoint = nfspBaseUrl()
    const sessionToken = new URL(endpoint).origin === window.location.origin ? await getActiveSessionToken() : null
    const options = { endpoint, sessionToken, signal: controls.signal, autoStartUpload: false }
    let imported = imports.get(task.candidate.localId)
    if (!imported) {
      controls.setStatus('hashing')
      const snapshot = input.kind === 'tree'
        ? await ndm.importDirectory(input.tree, options)
        : input.kind === 'folder'
          ? await ndm.importFiles(input.files, { ...options, mode: 'single_dir' })
          : await ndm.importFiles([input.file], { ...options, mode: 'single_file' })
      const object = snapshot.items[0]
      if (!object) throw new Error('Object import returned no object')
      imported = { sessionId: snapshot.sessionId, objectId: object.objectId }
      imports.set(task.candidate.localId, imported)
    }
    checkCancelled(controls)
    controls.setStatus('uploading')
    const status = await ndm.ensureUploaded(imported.sessionId, {
      ...options,
      onProgress: progress => controls.setProgress(progress.uploadedBytes),
    })
    if (status.objectStatus !== 'ready') throw new Error('The imported object is not ready')
    checkCancelled(controls)
    controls.setStatus('committing')
    const result = await client.commitObject(parentPath, name, imported.objectId)
    notifyDfsPath(parentPath)
    imports.delete(task.candidate.localId)
    ndm.releaseImportSession(imported.sessionId)
    return committedEntry(task, parentPath, result)
  } catch (error) {
    if (error instanceof TransferCancelledError || controls.signal.aborted) throw new TransferCancelledError()
    throw nfspToUiError(error)
  }
}

export function registerNfspTransferExecutor() {
  return registerTransferExecutor({
    run: runNfspTransfer,
    release: localId => {
      const imported = imports.get(localId)
      if (imported) ndm.releaseImportSession(imported.sessionId)
      imports.delete(localId)
    },
  })
}
