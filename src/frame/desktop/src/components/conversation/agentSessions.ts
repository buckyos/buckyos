/* Conversations with an Agent that an App started (§2.2): the App's id is kept as the source mark, an entry of the
 * owner's own UI state of the session (`ui_session.update_state`, key `ui.source`, UI_DATAMODEL §ui.source). msg-center
 * keeps any key there, so its protocol stays as it is. Knowing which sessions carry a mark takes one read per
 * session (Q5): at most six at a time, the answers kept in memory. The mock runtime keeps the marks in this browser. */

import type { DID } from 'buckyos'
import { conversationBackend, loadConversationBackend } from './backend'
import type { ConversationBackend, ConversationSessionInfo, ConversationSessionOptions } from './backendTypes'
import { announceSources, knownSource, mockSource, rememberSource, sourceKey, storeMockSource } from './sessionSources'

export { subscribeSources } from './sessionSources'

const KEY = 'ui.source'
const RETRY_MS = 30_000
const PARALLEL = 6

interface Read { key: string; ownerDid: string; sessionId: string; done: Array<() => void> }

const failedAt = new Map<string, number>()
const reads = new Map<string, Read>()
const waiting: Read[] = []
let running = 0

function pump(backend: ConversationBackend) {
  while (running < PARALLEL && waiting.length > 0) {
    const read = waiting.shift()!
    running += 1
    backend.listUiState(read.sessionId).then(
      (entries) => {
        const value = entries.find((entry) => entry.key === KEY)?.value
        // A mark written meanwhile (a session just started by an App) wins over what the read found before it.
        if (!knownSource(read.ownerDid, read.sessionId)) rememberSource(read.ownerDid, read.sessionId, typeof value === 'string' ? value : null)
        failedAt.delete(read.key)
      },
      () => { failedAt.set(read.key, Date.now()) },
    ).then(() => {
      running -= 1
      reads.delete(read.key)
      for (const done of read.done) done()
      announceSources()
      pump(backend)
    })
  }
}

/** The read of a session's mark, started unless the mark is known or a read failed a moment ago (then null). */
function read(backend: ConversationBackend, sessionId: string): Read | null {
  const ownerDid = backend.viewerDid()
  const key = sourceKey(ownerDid, sessionId)
  if (knownSource(ownerDid, sessionId) !== undefined || Date.now() - (failedAt.get(key) ?? -RETRY_MS) < RETRY_MS) return null
  let pending = reads.get(key)
  if (!pending) {
    pending = { key, ownerDid, sessionId, done: [] }
    reads.set(key, pending)
    waiting.push(pending)
    pump(backend)
  }
  return pending
}

/** The App that started the session, null when none did; undefined while not known (it is then read in the background). */
export function sessionSource(sessionId: string): string | null | undefined {
  const backend = conversationBackend()
  if (!backend) { void loadConversationBackend(); return undefined }
  const ownerDid = backend.viewerDid()
  if (!ownerDid) return undefined
  if (backend.isMock) return mockSource(ownerDid, sessionId)
  read(backend, sessionId)
  return knownSource(ownerDid, sessionId)
}

/** Whether the session was started by `source`. A session not looked at yet answers false and is read in the background. */
export function isSourced(sessionId: string, source: string): boolean {
  return sessionSource(sessionId) === source
}

/** Whether the session's mark is known: read, or kept in this browser (mock). A read that failed leaves it unknown. */
export function isSourceKnown(sessionId: string): boolean {
  const backend = conversationBackend()
  if (!backend) return false
  return backend.isMock || knownSource(backend.viewerDid(), sessionId) !== undefined
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
  if (backend.isMock) storeMockSource(backend.viewerDid(), sessionId, source)
  else await backend.updateUiState(sessionId, KEY, source)
  rememberSource(backend.viewerDid(), sessionId, source)
  announceSources()
}

/**
 * The owner's main conversation with the Agent the viewer can write in (the default one unless that is on a channel
 * of another platform), created when there is none; `name` lists the Agent until the directory does.
 */
export async function ensureDefaultSession(agentDid: DID, name?: string): Promise<ConversationSessionInfo> {
  const backend = await loadConversationBackend()
  await backend.ensureReady()
  backend.addAgent(agentDid, name)
  const session = await backend.ensureDefaultSession(agentDid)
  if (!session) throw Error('session_missing')
  return session
}

/** A new conversation with the Agent that carries `source` as its mark; the session goes again when the mark cannot be written. */
export async function createSourcedSession(agentDid: DID, source: string, title: string, name?: string, options?: ConversationSessionOptions): Promise<ConversationSessionInfo> {
  const backend = await loadConversationBackend()
  await backend.ensureReady()
  backend.addAgent(agentDid, name)
  const session = await backend.createSession(agentDid, title, options)
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
