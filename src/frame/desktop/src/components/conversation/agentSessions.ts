/* Conversations with an Agent that an App started (§2.2): the App's id is kept as the source mark, an entry of the
 * owner's own UI state of the session (`ui_session.update_state`, key `ui.source`, UI_DATAMODEL §ui.source). msg-center
 * keeps any key there, so its protocol stays as it is. Knowing which sessions carry a mark takes one read per
 * session (Q5): at most six at a time, the answers kept in memory. The mock runtime keeps the marks in this browser. */

import type { DID } from 'buckyos'
import { conversationBackend, loadConversationBackend } from './backend'
import type { ConversationBackend, ConversationSessionInfo } from './backendTypes'

const KEY = 'ui.source'
const MOCK_KEY = 'buckyos.conversation.mock.sources.v1'
const RETRY_MS = 30_000
const PARALLEL = 6

interface Read { key: string; sessionId: string; done: Array<() => void> }

const sources = new Map<string, string | null>()
const failedAt = new Map<string, number>()
const reads = new Map<string, Read>()
const waiting: Read[] = []
const listeners = new Set<() => void>()
let running = 0

const keyOf = (ownerDid: string, sessionId: string) => `${ownerDid}\n${sessionId}`

function mockSources(): Record<string, string> {
  try {
    const stored: unknown = JSON.parse(window.localStorage.getItem(MOCK_KEY) ?? '{}')
    return stored && typeof stored === 'object' && !Array.isArray(stored) ? Object.fromEntries(Object.entries(stored).filter((entry): entry is [string, string] => typeof entry[1] === 'string')) : {}
  } catch {
    return {}
  }
}

function pump(backend: ConversationBackend) {
  while (running < PARALLEL && waiting.length > 0) {
    const read = waiting.shift()!
    running += 1
    backend.listUiState(read.sessionId).then(
      (entries) => {
        const value = entries.find((entry) => entry.key === KEY)?.value
        const source = typeof value === 'string' ? value : null
        sources.set(read.key, source)
        failedAt.delete(read.key)
        return source
      },
      () => { failedAt.set(read.key, Date.now()); return null },
    ).then((source) => {
      running -= 1
      reads.delete(read.key)
      for (const done of read.done) done()
      if (source) for (const listener of [...listeners]) listener()
      pump(backend)
    })
  }
}

/** The read of a session's mark, started unless the mark is known or a read failed a moment ago (then null). */
function read(backend: ConversationBackend, sessionId: string): Read | null {
  const key = keyOf(backend.viewerDid(), sessionId)
  if (sources.has(key) || Date.now() - (failedAt.get(key) ?? -RETRY_MS) < RETRY_MS) return null
  let pending = reads.get(key)
  if (!pending) {
    pending = { key, sessionId, done: [] }
    reads.set(key, pending)
    waiting.push(pending)
    pump(backend)
  }
  return pending
}

/** A mark that was not known became known. */
export function subscribeSources(listener: () => void): () => void {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}

/** Whether the session was started by `source`. A session not looked at yet answers false and is read in the background. */
export function isSourced(sessionId: string, source: string): boolean {
  const backend = conversationBackend()
  if (!backend) { void loadConversationBackend(); return false }
  const ownerDid = backend.viewerDid()
  if (!ownerDid) return false
  const key = keyOf(ownerDid, sessionId)
  if (backend.isMock) return (sources.get(key) ?? mockSources()[key] ?? null) === source
  read(backend, sessionId)
  return sources.get(key) === source
}

/** Resolves once the marks of these sessions are known (or could not be read). */
export async function loadSources(sessionIds: string[]): Promise<void> {
  const backend = await loadConversationBackend()
  await backend.ensureReady()
  if (backend.isMock) return
  await Promise.all(sessionIds.map((sessionId) => {
    const pending = read(backend, sessionId)
    return pending ? new Promise<void>((resolve) => { pending.done.push(resolve) }) : undefined
  }))
}

async function markSourced(backend: ConversationBackend, sessionId: string, source: string): Promise<void> {
  const key = keyOf(backend.viewerDid(), sessionId)
  if (backend.isMock) {
    try { window.localStorage.setItem(MOCK_KEY, JSON.stringify({ ...mockSources(), [key]: source })) } catch { /* no storage: the mark lasts until the page is reloaded */ }
  } else {
    await backend.updateUiState(sessionId, KEY, source)
  }
  sources.set(key, source)
}

/** The owner's default conversation with the Agent, created when there is none; `name` lists the Agent until the directory does. */
export async function ensureDefaultSession(agentDid: DID, name?: string): Promise<ConversationSessionInfo> {
  const backend = await loadConversationBackend()
  await backend.ensureReady()
  backend.addAgent(agentDid, name)
  const session = await backend.ensureDefaultSession(agentDid)
  if (!session) throw Error('session_missing')
  return session
}

/** A new conversation with the Agent that carries `source` as its mark; the session goes again when the mark cannot be written. */
export async function createSourcedSession(agentDid: DID, source: string, title: string, name?: string): Promise<ConversationSessionInfo> {
  const backend = await loadConversationBackend()
  await backend.ensureReady()
  backend.addAgent(agentDid, name)
  const session = await backend.createSession(agentDid, title)
  try {
    await markSourced(backend, session.id, source)
  } catch (error) {
    await backend.deleteSession(session.id).catch(() => undefined)
    throw error
  }
  return session
}

/** The owner's active conversations with the Agent that `source` started, newest first. */
export async function sourcedSessions(agentDid: DID, source: string): Promise<ConversationSessionInfo[]> {
  const backend = await loadConversationBackend()
  await backend.ensureReady()
  const sessions = backend.sessions(agentDid)
  await loadSources(sessions.map((session) => session.id))
  return sessions.filter((session) => isSourced(session.id, source))
}
