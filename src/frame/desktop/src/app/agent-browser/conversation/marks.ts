/* DG1: which sessions with the Agent were started in the Agent browser. The mark is an entry of the owner's own UI
 * state of the session (`ui_session.update_state`, key `ui.source`), written when the thing is created: msg-center
 * keeps any key there, so its protocol stays as it is. The mock runtime keeps the marks in this browser. */

import { listUiSessionState, updateUiSessionState } from '../../messagehub/datamodel/sessionApi'
import type { ThingContext } from '../extensions'

const KEY = 'ui.source'
const SOURCE = 'agent-browser'
const MOCK_KEY = 'buckyos.agent-browser.mock.marks'
const RETRY_MS = 30_000
const PARALLEL = 6

interface Read { key: string; ownerDid: string; sessionId: string; done: Array<() => void> }

const marked = new Map<string, boolean>()
const failedAt = new Map<string, number>()
const reads = new Map<string, Read>()
const waiting: Read[] = []
const listeners = new Set<() => void>()
let running = 0

const keyOf = (ownerDid: string, sessionId: string) => `${ownerDid}\n${sessionId}`

function mockMarks(): string[] {
  try {
    const stored: unknown = JSON.parse(window.localStorage.getItem(MOCK_KEY) ?? '[]')
    return Array.isArray(stored) ? stored.filter((item): item is string => typeof item === 'string') : []
  } catch {
    return []
  }
}

function pump() {
  while (running < PARALLEL && waiting.length > 0) {
    const read = waiting.shift()!
    running += 1
    listUiSessionState(read.sessionId, read.ownerDid).then(
      (entries) => {
        const own = entries.some((entry) => entry.key === KEY && entry.value === SOURCE)
        marked.set(read.key, own)
        failedAt.delete(read.key)
        return own
      },
      () => { failedAt.set(read.key, Date.now()); return false },
    ).then((own) => {
      running -= 1
      reads.delete(read.key)
      for (const done of read.done) done()
      if (own) for (const listener of [...listeners]) listener()
      pump()
    })
  }
}

/** The read of a session's mark, started unless the mark is known or a read failed a moment ago (then null). */
function read(ownerDid: string, sessionId: string): Read | null {
  const key = keyOf(ownerDid, sessionId)
  if (marked.has(key) || Date.now() - (failedAt.get(key) ?? -RETRY_MS) < RETRY_MS) return null
  let pending = reads.get(key)
  if (!pending) {
    pending = { key, ownerDid, sessionId, done: [] }
    reads.set(key, pending)
    waiting.push(pending)
    pump()
  }
  return pending
}

/** A mark that was not known became known as set. */
export function subscribeMarks(listener: () => void): () => void {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}

/** Whether the session was started in the Agent browser. A session not looked at yet answers false and is read in the background. */
export function isMarked({ hub, context }: ThingContext, sessionId: string): boolean {
  const key = keyOf(context.ownerDid, sessionId)
  if (hub.isMock) return marked.get(key) ?? mockMarks().includes(key)
  read(context.ownerDid, sessionId)
  return marked.get(key) ?? false
}

/** Resolves once the marks of these sessions are known (or could not be read). */
export async function loadMarks({ hub, context }: ThingContext, sessionIds: string[]): Promise<void> {
  if (hub.isMock) return
  await Promise.all(sessionIds.map((sessionId) => {
    const pending = read(context.ownerDid, sessionId)
    return pending ? new Promise<void>((resolve) => { pending.done.push(resolve) }) : undefined
  }))
}

export async function markSession({ hub, context }: ThingContext, sessionId: string): Promise<void> {
  const key = keyOf(context.ownerDid, sessionId)
  if (hub.isMock) {
    try { window.localStorage.setItem(MOCK_KEY, JSON.stringify([...new Set([...mockMarks(), key])])) } catch { /* no storage: the mark lasts until the page is reloaded */ }
  } else {
    await updateUiSessionState(sessionId, KEY, SOURCE, context.ownerDid)
  }
  marked.set(key, true)
}
