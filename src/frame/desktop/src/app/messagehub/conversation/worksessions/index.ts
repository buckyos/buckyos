import { useEffect, useState, useSyncExternalStore } from 'react'
import { mockWorkCardsSource, mockWorkCards } from '../../mock/worksessions'
import { isMessageHubMock } from '../../store'
import { ChoiceMemory, getTaskWatchSource, useRememberedChoice } from '../tasks'
import { workCardsSource, type WorkCardsRequest } from './source'
import { WorkCardsWatch, workCardsKey, type WorkCardsSnapshot } from './watch'

let instance: WorkCardsWatch | null = null

/** One work-card cache for every MessageHub view; tree hints come from the shared TaskMgr watch source. */
export function getWorkCardsWatch(): WorkCardsWatch {
  if (instance) return instance
  const mock = isMessageHubMock()
  const watch = new WorkCardsWatch(mock ? mockWorkCardsSource : workCardsSource, getTaskWatchSource())
  instance = watch
  if (typeof window !== 'undefined') {
    window.addEventListener('online', () => watch.resync())
    document.addEventListener('visibilitychange', () => { if (document.visibilityState === 'visible') watch.resync() })
    if (import.meta.env.DEV) Object.assign(window, { __messageHubWorkCards: watch, ...(mock ? { __messageHubMockWorkCards: mockWorkCards } : {}) })
  }
  return watch
}

/**
 * The work session cards a bubble's UI Turn created, followed while mounted.
 * `request` is undefined until the bubble knows it has work to show (the
 * Turn's task has sub-tasks): nothing is read for quick replies.
 */
export function useWorkCards(request: WorkCardsRequest | undefined, rootTerminal: boolean): WorkCardsSnapshot | undefined {
  const watch = getWorkCardsWatch()
  const key = request ? workCardsKey(request) : ''
  useEffect(() => request ? watch.watch(request, rootTerminal) : undefined, [watch, key]) // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => { if (request) watch.setRootTerminal(request, rootTerminal) }, [watch, key, rootTerminal]) // eslint-disable-line react-hooks/exhaustive-deps
  return useSyncExternalStore(watch.subscribe, () => request ? watch.snapshot(request) : undefined)
}

/** A clock that ticks every second while `active` (a running card shows its time to the second). */
export function useSecondClock(active: boolean): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (!active) return
    const timer = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(timer)
  }, [active])
  return now
}

const todoExpanded = new ChoiceMemory('buckyos.messagehub.todoExpanded.v1')

/**
 * Whether the TodoList of a plan card is open: `fallback` (open while the
 * plan runs, closed once it finished) until the viewer opens or closes it by
 * hand, which is remembered per viewer, bubble anchor and plan session.
 */
export function useTodoExpanded(viewerDid: string, anchorId: string, sessionId: string, fallback: boolean): [boolean, (value: boolean) => void] {
  return useRememberedChoice(todoExpanded, viewerDid, `${anchorId}/${sessionId}`, fallback)
}

const openBranches = new Set<string>()

/** Whether the sub sessions under a card are unfolded; kept per list and session for this page, so a list rendered again (a panel that moved, a refreshed tree) stays as it was. */
export function useBranchExpanded(anchorId: string, sessionId: string): [boolean, (value: boolean) => void] {
  const key = `${anchorId}/${sessionId}`
  const [expanded, setExpanded] = useState(() => openBranches.has(key))
  return [expanded, value => {
    if (value) openBranches.add(key); else openBranches.delete(key)
    setExpanded(value)
  }]
}
