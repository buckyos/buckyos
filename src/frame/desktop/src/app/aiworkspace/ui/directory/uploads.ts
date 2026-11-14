/* Uploads into directories (框架 PRD §7.4 "上传"): each file is a job with its own progress and cancel; a finished
 * upload becomes a member in its own Commit, so one large video does not hold back the rest. The queue belongs to the
 * workspace store, not to an editor: leaving the editor (a canvas Block deselected, a tab closed) does not stop it. */

import { assetLimitOf, describeError, MEDIA_LIMIT_BYTES } from '../../api/session'
import { randomId } from '../../api/ids'
import { aiwsT } from '../../i18n'
import { Emitter } from '../../state/emitter'
import type { WorkspaceStore } from '../../state/store'
import { formatBytes } from './format'
import type { Place } from './model'
import { commitFile } from './ops'

export type UploadState = 'waiting' | 'sending' | 'saving' | 'done' | 'failed' | 'cancelled'

export interface UploadJob {
  id: string
  /** The editor root the upload was started from (whose editors list it). */
  rootId: string
  place: Place
  name: string
  mode: 'new' | 'replace'
  state: UploadState
  sent: number
  total: number
  error?: string
  /** The member name it was added under (a "keep both" may change it). */
  added?: string
}

const CONCURRENCY = 2
/** How long a finished upload stays listed. */
const DONE_LINGER_MS = 2500

export class UploadQueue {
  private jobs: UploadJob[] = []
  private readonly files = new Map<string, File>()
  private readonly controllers = new Map<string, AbortController>()
  private readonly emitter = new Emitter()
  private version = 0
  readonly subscribe = this.emitter.subscribe
  snapshot = () => this.version
  private readonly store: WorkspaceStore

  constructor(store: WorkspaceStore) { this.store = store }

  list(rootId: string): UploadJob[] { return this.jobs.filter((job) => job.rootId === rootId) }
  /** Uploads still running anywhere (for a status line). */
  active(): UploadJob[] { return this.jobs.filter((job) => job.state === 'waiting' || job.state === 'sending' || job.state === 'saving') }

  /** Queue files under the names chosen for them; a file over its limit is listed as failed with the limit. */
  add(rootId: string, place: Place, items: { file: File; name: string; mode: 'new' | 'replace' }[]) {
    for (const item of items) {
      const id = randomId('up')
      const limit = assetLimitOf(item.file)
      const job: UploadJob = { id, rootId, place, name: item.name, mode: item.mode, state: 'waiting', sent: 0, total: item.file.size }
      if (item.file.size > limit) {
        job.state = 'failed'
        job.error = aiwsT(limit === MEDIA_LIMIT_BYTES ? 'aiworkspace.directory.upload.tooLargeMedia' : 'aiworkspace.directory.upload.tooLarge', { limit: formatBytes(limit), size: formatBytes(item.file.size) })
      } else this.files.set(id, item.file)
      this.jobs.push(job)
    }
    this.changed()
    this.pump()
  }

  cancel(id: string) {
    const job = this.jobs.find((item) => item.id === id)
    if (!job) return
    if (job.state === 'waiting') { this.files.delete(id); this.update(id, { state: 'cancelled' }); this.later(id); return }
    this.controllers.get(id)?.abort()
  }

  dismiss(id: string) {
    this.jobs = this.jobs.filter((job) => job.id !== id)
    this.files.delete(id)
    this.changed()
  }

  private changed() { this.version += 1; this.emitter.emit() }

  private update(id: string, patch: Partial<UploadJob>) {
    this.jobs = this.jobs.map((job) => (job.id === id ? { ...job, ...patch } : job))
    this.changed()
  }

  private later(id: string) { window.setTimeout(() => this.dismiss(id), DONE_LINGER_MS) }

  private pump() {
    const running = this.jobs.filter((job) => job.state === 'sending' || job.state === 'saving').length
    const next = this.jobs.filter((job) => job.state === 'waiting').slice(0, Math.max(0, CONCURRENCY - running))
    for (const job of next) void this.run(job)
  }

  private async run(job: UploadJob) {
    const file = this.files.get(job.id)
    if (!file) return
    const controller = new AbortController()
    this.controllers.set(job.id, controller)
    this.update(job.id, { state: 'sending' })
    try {
      let shown = 0
      const asset = await this.store.session.uploadAsset(file, job.name, {
        signal: controller.signal,
        onProgress: (sent, total) => {
          // a few updates a second are enough for a bar
          const now = performance.now()
          if (sent < total && now - shown < 200) return
          shown = now
          this.update(job.id, { sent, total: total || file.size })
        },
      })
      // a session that cannot stop the transfer (a replica window) still drops a cancelled upload here
      controller.signal.throwIfAborted()
      this.update(job.id, { state: 'saving', sent: file.size })
      const added = await commitFile(this.store, job.place, job.name, asset.object_id, job.mode)
      this.update(job.id, { state: 'done', added })
      this.later(job.id)
    } catch (error) {
      if (controller.signal.aborted) { this.update(job.id, { state: 'cancelled' }); this.later(job.id) }
      else this.update(job.id, { state: 'failed', error: describeError(error) })
    } finally {
      this.controllers.delete(job.id)
      this.files.delete(job.id)
      this.pump()
    }
  }
}

const queues = new WeakMap<WorkspaceStore, UploadQueue>()

export function uploadQueue(store: WorkspaceStore): UploadQueue {
  let queue = queues.get(store)
  if (!queue) { queue = new UploadQueue(store); queues.set(store, queue) }
  return queue
}
