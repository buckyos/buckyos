/** Window-side hooks for the transfer store (panel visibility, status-bar totals). */

import { useEffect, useState, useSyncExternalStore } from 'react'
import { transferStore, type TransferSummary } from './data/transfers'

/** Per-window visibility: hidden until a task newer than the hide moment arrives. */
export function useTransfersPanelState() {
  const [hiddenAt, setHiddenAt] = useState<number | null>(null)
  return {
    hiddenAt,
    hide: () => setHiddenAt(transferStore.lastTaskSeq()),
    show: () => setHiddenAt(null),
  }
}

export type TransfersPanelState = ReturnType<typeof useTransfersPanelState>

/** Live totals for the status bar; re-reads once a second while busy so the speed decays when progress stalls. */
export function useTransferSummary(): TransferSummary {
  useSyncExternalStore(transferStore.subscribe, transferStore.snapshot)
  const [, tick] = useState(0)
  const summary = transferStore.summary()
  const busy = summary.active > 0
  useEffect(() => {
    if (!busy) return
    const timer = window.setInterval(() => tick((n) => n + 1), 1000)
    return () => window.clearInterval(timer)
  }, [busy])
  return summary
}
