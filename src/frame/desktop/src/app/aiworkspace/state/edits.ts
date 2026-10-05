/* Save state of every edit (design §6.5).
 *
 *   未保存          the input exists only in this window's memory (being sent or stored, the result is
 *                   unknown, or the replica database could not be written)
 *   已保存到本设备  replica session only: the pending row is committed in the replica database (OPFS)
 *   已提交          the backend accepted it
 *   需要处理        conflict / rejected / blocked — my input is kept until I decide */

import type { Json } from '../api/types'
import { Emitter } from './emitter'

export type EditState = 'unsaved' | 'saved_locally' | 'committed' | 'needs_attention'

export const EDIT_STATE_LABEL: Record<EditState, string> = {
  unsaved: '未保存',
  saved_locally: '已保存到本设备',
  committed: '已提交',
  needs_attention: '需要处理',
}

export interface EditEntry {
  id: string
  label: string
  state: EditState
  /** Why it needs attention / why it is still unsaved. */
  detail?: string
  code?: string
  /** My input, kept verbatim until the edit is committed or I discard it. */
  mine?: Json
  hasMine?: boolean
  /** Current value on the backend when a conflict was reported. */
  theirs?: Json
  hasTheirs?: boolean
  currentRev?: number
  /** Set when the result is unknown: the same request can be resent with this key. */
  unknownKey?: string
  /** Replica session: idempotency key of the pending submission this entry shows. */
  pendingKey?: string
  /** The replica database could not be written: the input lives in memory only and can be exported. */
  storageFailed?: boolean
  at: number
}

export class EditStore {
  private entries = new Map<string, EditEntry>()
  private readonly emitter = new Emitter()
  private readonly timers = new Map<string, number>()
  readonly subscribe = this.emitter.subscribe

  snapshot = (): ReadonlyMap<string, EditEntry> => this.entries

  get(id: string): EditEntry | undefined {
    return this.entries.get(id)
  }

  set(entry: Omit<EditEntry, 'at'>) {
    const timer = this.timers.get(entry.id)
    if (timer !== undefined) { window.clearTimeout(timer); this.timers.delete(entry.id) }
    const next = new Map(this.entries)
    next.set(entry.id, { ...entry, at: Date.now() })
    this.entries = next
    this.emitter.emit()
    // A committed edit has nothing left to decide; its badge fades after a moment.
    if (entry.state === 'committed') {
      this.timers.set(entry.id, window.setTimeout(() => { if (this.entries.get(entry.id)?.state === 'committed') this.remove(entry.id) }, 4000))
    }
  }

  remove(id: string) {
    if (!this.entries.has(id)) return
    const next = new Map(this.entries)
    next.delete(id)
    this.entries = next
    this.emitter.emit()
  }

  dispose() {
    for (const timer of this.timers.values()) window.clearTimeout(timer)
    this.timers.clear()
  }
}
