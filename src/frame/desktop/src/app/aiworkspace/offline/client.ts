/* ReplicaClient: the page's thin layer over the Replica Worker (design §6.1). One request at a time
 * is answered by the Worker in arrival order; a Worker that stops answering is treated as dead and
 * every later call fails with a StorageFailure — nothing is ever reported saved by this side. */

import { ServiceFailure } from '../api/client'
import { aiwsT, currentAiwsLocale } from '../i18n'
import type { InitResult, ReplicaApi, ReplicaOp, WorkerRequest, WorkerResponse } from './protocol'

/** The replica database could not be written or reached: the edit is NOT saved on this device. */
export class StorageFailure extends Error {
  readonly kind: 'quota' | 'transaction' | 'worker'
  constructor(kind: 'quota' | 'transaction' | 'worker', message: string) {
    super(message)
    this.name = 'StorageFailure'
    this.kind = kind
  }
}

export function describeStorageFailure(failure: StorageFailure): string {
  if (failure.kind === 'quota') return aiwsT('aiworkspace.core.storage.quota', { error: failure.message })
  if (failure.kind === 'worker') return aiwsT('aiworkspace.core.storage.worker', { error: failure.message })
  return aiwsT('aiworkspace.core.storage.transaction', { error: failure.message })
}

const SLOW_OPS = new Set<ReplicaOp>(['init', 'importReplica', 'open', 'destroy', 'applyRemote', 'putAsset', 'exportLocal'])
const TIMEOUT_MS = 10_000
const SLOW_TIMEOUT_MS = 120_000

interface Waiter { resolve: (value: unknown) => void; reject: (error: Error) => void; timer: number }

export class ReplicaClient {
  readonly workspaceId: string
  private readonly worker: Worker
  private readonly waiters = new Map<number, Waiter>()
  private nextId = 1
  private deadReason: string | null = null
  private readonly deathListeners = new Set<() => void>()

  private constructor(workspaceId: string) {
    this.workspaceId = workspaceId
    this.worker = new Worker(new URL('./replica.worker.ts', import.meta.url), { type: 'module', name: `aiworkspace-replica:${workspaceId}` })
    this.worker.onmessage = (event: MessageEvent<WorkerResponse>) => this.onMessage(event.data)
    this.worker.onerror = (event) => { event.preventDefault(); this.die(event.message || aiwsT('aiworkspace.core.worker.loadError')) }
    this.worker.onmessageerror = () => this.die(aiwsT('aiworkspace.core.worker.decodeError'))
  }

  /** Start the Worker and its storage. `ok: false` carries the reason offline is unavailable here. */
  static async start(workspaceId: string, testHooks: boolean): Promise<{ client: ReplicaClient; init: InitResult }> {
    if (typeof Worker === 'undefined') throw new StorageFailure('worker', aiwsT('aiworkspace.core.worker.unsupported'))
    const client = new ReplicaClient(workspaceId)
    try {
      const init = await client.call('init', { workspaceId, testHooks })
      if (!init.ok) client.terminate()
      return { client, init }
    } catch (error) {
      client.terminate()
      throw error
    }
  }

  get dead(): string | null { return this.deadReason }

  onDeath(listener: () => void): () => void {
    this.deathListeners.add(listener)
    return () => { this.deathListeners.delete(listener) }
  }

  call<K extends ReplicaOp>(op: K, ...args: Parameters<ReplicaApi[K]>): Promise<ReturnType<ReplicaApi[K]>> {
    if (this.deadReason !== null) return Promise.reject(new StorageFailure('worker', this.deadReason))
    const id = this.nextId++
    return new Promise<ReturnType<ReplicaApi[K]>>((resolve, reject) => {
      const timer = window.setTimeout(() => {
        // No answer: a crashed or terminated Worker raises no event, silence is the only signal.
        this.die(aiwsT('aiworkspace.core.worker.timeout', { op, seconds: (SLOW_OPS.has(op) ? SLOW_TIMEOUT_MS : TIMEOUT_MS) / 1000 }))
      }, SLOW_OPS.has(op) ? SLOW_TIMEOUT_MS : TIMEOUT_MS)
      this.waiters.set(id, { resolve: resolve as (value: unknown) => void, reject, timer })
      // the Worker has no page storage: each request tells it the language for the reasons it returns
      const request: WorkerRequest = { id, op, args, locale: currentAiwsLocale() }
      const transfer = args.filter((value): value is ArrayBuffer => value instanceof ArrayBuffer)
      try {
        this.worker.postMessage(request, transfer)
      } catch (error) {
        this.waiters.delete(id)
        window.clearTimeout(timer)
        reject(new StorageFailure('worker', error instanceof Error ? error.message : String(error)))
      }
    })
  }

  private onMessage(response: WorkerResponse) {
    const waiter = this.waiters.get(response.id)
    if (!waiter) return
    this.waiters.delete(response.id)
    window.clearTimeout(waiter.timer)
    if (response.ok) { waiter.resolve(response.value); return }
    const failure = response.error
    if (failure.kind === 'storage') waiter.reject(new StorageFailure(failure.storageKind ?? 'transaction', failure.message))
    else if (failure.kind === 'service' && failure.service) waiter.reject(new ServiceFailure(failure.service))
    else waiter.reject(new Error(failure.message))
  }

  private die(reason: string) {
    if (this.deadReason !== null) return
    this.deadReason = reason
    try { this.worker.terminate() } catch { /* already gone */ }
    for (const waiter of this.waiters.values()) {
      window.clearTimeout(waiter.timer)
      waiter.reject(new StorageFailure('worker', reason))
    }
    this.waiters.clear()
    for (const listener of [...this.deathListeners]) listener()
  }

  /** Orderly end: close the database (releases the OPFS handles), then stop the Worker. */
  async close(): Promise<void> {
    if (this.deadReason === null) {
      try { await this.call('close') } catch { /* it is being stopped anyway */ }
    }
    this.terminate()
  }

  terminate() {
    if (this.deadReason === null) this.deadReason = aiwsT('aiworkspace.core.worker.closed')
    try { this.worker.terminate() } catch { /* already gone */ }
    for (const waiter of this.waiters.values()) {
      window.clearTimeout(waiter.timer)
      waiter.reject(new StorageFailure('worker', this.deadReason))
    }
    this.waiters.clear()
  }

  /** e2e only: the Worker process disappears without a word, as in a crash. */
  testKillWorker() {
    this.worker.terminate()
  }
}
