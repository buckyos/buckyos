/* The service's clock on this device (控件机制 TODO G16, Q10): a few `ws.time` round trips, the offset of the one with
 * the shortest round trip (its half is the error bound), measured again every few minutes. Controls that play or count
 * from a shared moment (a broadcast's position, the time left of a time-limited Block) use it, so a device whose clock
 * is wrong still agrees with the others. Without the service it is the device's clock. */

import type { WorkspaceSession } from '../api/session'
import { Emitter } from './emitter'

const RESAMPLE_MS = 5 * 60_000

export class ServerClock {
  private offset = 0
  private bound = Infinity
  private sampledAt = 0
  private sampling: Promise<void> | null = null
  private readonly emitter = new Emitter()
  readonly subscribe = this.emitter.subscribe
  private version = 0
  snapshot = () => this.version
  private readonly session: WorkspaceSession

  constructor(session: WorkspaceSession) {
    this.session = session
  }

  /** The service's time now (ms), as well as this device knows it. */
  now(): number {
    if (Date.now() - this.sampledAt > RESAMPLE_MS) void this.sync()
    return Date.now() + this.offset
  }

  /** True once at least one round trip measured the offset. */
  get synced(): boolean { return this.bound !== Infinity }

  /** Measure the offset (three round trips, the shortest wins). */
  sync(): Promise<void> {
    this.sampling ??= (async () => {
      let best: { offset: number; rtt: number } | null = null
      for (let i = 0; i < 3; i++) {
        const before = Date.now()
        const sample = await this.session.serverTime()
        if (!sample) break
        const offset = sample.server_ms - (before + sample.rtt_ms / 2)
        if (!best || sample.rtt_ms < best.rtt) best = { offset, rtt: sample.rtt_ms }
      }
      this.sampledAt = Date.now()
      if (best) {
        this.offset = Math.round(best.offset)
        this.bound = best.rtt / 2
        this.version += 1
        this.emitter.emit()
      }
    })().finally(() => { this.sampling = null })
    return this.sampling
  }
}

const clocks = new WeakMap<WorkspaceSession, ServerClock>()

export function serverClock(session: WorkspaceSession): ServerClock {
  let clock = clocks.get(session)
  if (!clock) {
    clock = new ServerClock(session)
    clocks.set(session, clock)
    void clock.sync()
  }
  return clock
}
