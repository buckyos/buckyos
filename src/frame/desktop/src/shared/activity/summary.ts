/* ── What the badges count (`notify.summary`) ── */

import type { MsgCenterClient, NotifySummary } from 'buckyos'
import { msgCenterOf, type ZoneChannel } from '../channel.ts'

export type { NotifySummary as ActivitySummary }

/** Reads the counts; `null` from a Zone whose msg-center is too old to have them, which is then not asked again. */
export class SummaryReader {
  private readonly client: Pick<MsgCenterClient, 'notifySummary'>
  private unsupported = false

  constructor(channel: ZoneChannel) {
    this.client = msgCenterOf(channel)
  }

  async read(): Promise<NotifySummary | null> {
    if (this.unsupported) return null
    try {
      return await this.client.notifySummary()
    } catch (error) {
      if (!/unknown method/i.test(error instanceof Error ? error.message : String(error))) throw error
      this.unsupported = true
      return null
    }
  }
}

export interface SummaryWatch {
  /** The last counts; null until read, and on a Zone that does not have them (count what is listed then). */
  current(): NotifySummary | null
  /** Something changed: reads the counts again soon. */
  kick(): void
  stop(): void
}

/** Keeps the counts between changes the caller learns of; `onChange` runs when they are different. A failed read leaves them as they were. */
export function watchSummary(channel: ZoneChannel, onChange: () => void, delayMs = 300): SummaryWatch {
  const reader = new SummaryReader(channel)
  let summary: NotifySummary | null = null
  let pending: ReturnType<typeof setTimeout> | null = null
  let stopped = false
  const read = async () => {
    pending = null
    const next = await reader.read().catch(() => summary)
    if (stopped || JSON.stringify(next) === JSON.stringify(summary)) return
    summary = next
    onChange()
  }
  return {
    current: () => summary,
    kick: () => { if (!pending && !stopped) pending = setTimeout(() => { void read() }, delayMs) },
    stop: () => { stopped = true; if (pending) clearTimeout(pending) },
  }
}
