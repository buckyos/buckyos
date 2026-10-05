import { useEffect, useMemo, useRef, useState } from 'react'
import { isMessageHubMock } from '../../store'
import { mockWorklogSource } from '../../mock/worklog'
import type { WorklogEntry, WorklogPage, WorklogTarget } from './model'
import { createWorklogSource } from './source'

export function useWorklog(target: WorklogTarget) {
  const { agentDid, sessionId, turn } = target
  const source = useMemo(() => {
    const binding = { agentDid, sessionId, turn }
    return isMessageHubMock() ? mockWorklogSource(binding) : createWorklogSource(binding)
  }, [agentDid, sessionId, turn])
  const [view, setView] = useState<{ page?: WorklogPage; loading: boolean; error: boolean; older: boolean }>({ loading: true, error: false, older: false })
  const commands = useRef<{ older: () => void; retry: () => void } | null>(null)
  useEffect(() => {
    let disposed = false
    let busy = false
    let page: WorklogPage | undefined
    let timer: ReturnType<typeof setTimeout> | undefined
    const entries = new Map<number, WorklogEntry>()
    const publish = (loading: boolean, error: boolean, older: boolean) => {
      if (!disposed) setView({ page: page ? { ...page, entries: [...entries.values()].sort((a, b) => a.seq - b.seq) } : undefined, loading, error, older })
    }
    const read = async (older = false) => {
      if (disposed || busy || (older && page?.next_before == null)) return
      busy = true
      clearTimeout(timer)
      publish(!page, false, older)
      try {
        let cursor = older ? { before: page!.next_before! } : page ? { after: page.next_after } : {}
        do {
          const next = await source(cursor)
          if (disposed) return
          for (const entry of next.entries) entries.set(entry.seq, entry)
          const previous = page
          page = {
            ...next,
            next_before: 'before' in cursor || !previous ? next.next_before : previous.next_before,
            next_after: 'before' in cursor && previous ? previous.next_after : next.next_after,
          }
          publish(false, false, older)
          if (!('after' in cursor) && next.entries.length === 0 && next.next_before !== null) {
            cursor = { before: next.next_before }
            continue
          }
          if (!older && 'after' in cursor && next.next_after < next.committed) {
            cursor = { after: next.next_after }
            continue
          }
          break
        } while (!disposed)
        publish(false, false, false)
      } catch {
        publish(false, true, false)
      } finally {
        busy = false
        if (!disposed && (!page?.complete || page.next_after < page.committed)) timer = setTimeout(() => { void read() }, 2500)
      }
    }
    commands.current = { older: () => { void read(true) }, retry: () => { void read() } }
    void read()
    const resume = () => { if (document.visibilityState === 'visible') void read() }
    window.addEventListener('online', resume)
    document.addEventListener('visibilitychange', resume)
    return () => { disposed = true; clearTimeout(timer); window.removeEventListener('online', resume); document.removeEventListener('visibilitychange', resume) }
  }, [source])
  return { ...view, loadOlder: () => commands.current?.older(), retry: () => commands.current?.retry() }
}
