/* Write locks, client side (design §2.11): acquire before editing a `lock_required` entity, renew
 * every 20 s, release on blur/close; a lost lease makes the editor read-only at once. */

import { ServiceFailure } from '../api/client'
import type { WorkspaceSession } from '../api/session'
import { TransportError } from '../api/transport'
import { Emitter } from './emitter'

const RENEW_MS = 20_000

interface Held { lockId: string; expiresAt: number }

export class LockManager {
  private readonly held = new Map<string, Held>()
  private readonly lost = new Map<string, string>()
  private readonly emitter = new Emitter()
  private readonly session: WorkspaceSession
  private timer: number | null = null
  private version = 0
  readonly subscribe = this.emitter.subscribe

  constructor(session: WorkspaceSession) {
    this.session = session
  }

  snapshot = (): number => this.version

  private changed() {
    this.version += 1
    this.emitter.emit()
  }

  isHeld(entityId: string): boolean {
    return this.held.has(entityId)
  }

  /** Why the lock of this entity was lost since it was last acquired, if it was. */
  lostReason(entityId: string): string | null {
    return this.lost.get(entityId) ?? null
  }

  /** Throws ServiceFailure (`LOCK_HELD` carries the holder in `data.locks`). */
  async acquire(entityId: string): Promise<void> {
    if (this.held.has(entityId)) return
    const lock = await this.session.lockAcquire(entityId)
    this.held.set(entityId, { lockId: lock.lock_id, expiresAt: Date.parse(lock.expires_at) })
    this.lost.delete(entityId)
    this.timer ??= window.setInterval(() => { void this.renew() }, RENEW_MS)
    this.changed()
  }

  async release(entityId: string): Promise<void> {
    const lock = this.held.get(entityId)
    if (!lock) return
    this.held.delete(entityId)
    this.stopIfIdle()
    this.changed()
    try { await this.session.lockRelease([lock.lockId]) } catch { /* the lease expires on its own */ }
  }

  /** A commit was refused with LOCK_LOST / LOCK_REQUIRED for this entity. */
  noteLost(entityId: string, reason: string) {
    if (!this.held.delete(entityId) && this.lost.has(entityId)) return
    this.lost.set(entityId, reason)
    this.stopIfIdle()
    this.changed()
  }

  private stopIfIdle() {
    if (this.held.size === 0 && this.timer !== null) { window.clearInterval(this.timer); this.timer = null }
  }

  private async renew() {
    for (const [entityId, lock] of [...this.held]) {
      try {
        await this.session.lockRenew([lock.lockId])
        const current = this.held.get(entityId)
        if (current) current.expiresAt = Date.now() + 60_000
      } catch (error) {
        if (error instanceof ServiceFailure && error.code === 'LOCK_LOST') {
          this.noteLost(entityId, '写锁已失去（租约过期、空闲释放或被管理员解除）')
        } else if (error instanceof TransportError && Date.now() > lock.expiresAt) {
          // Unreachable until the lease ran out by our own clock: assume it is gone.
          this.noteLost(entityId, '后台不可达，写锁租约已过期')
        }
      }
    }
  }

  dispose() {
    if (this.timer !== null) window.clearInterval(this.timer)
    this.timer = null
    const ids = [...this.held.values()].map((lock) => lock.lockId)
    this.held.clear()
    if (ids.length > 0) void this.session.lockRelease(ids).catch(() => undefined)
  }
}
