/* What is known of the source marks (`ui.source`, read and written by agentSessions.ts): the App that started a
 * session, null when none did. Kept apart from the reading so MessageHub's stores can look at it. */

const MOCK_KEY = 'buckyos.conversation.mock.sources.v1'

const sources = new Map<string, string | null>()
const listeners = new Set<() => void>()

export const sourceKey = (ownerDid: string, sessionId: string) => `${ownerDid}\n${sessionId}`

/** A mark became known. */
export function subscribeSources(listener: () => void): () => void {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}

/** The App that started the session, null when none did; undefined while its mark is not known. */
export function knownSource(ownerDid: string, sessionId: string): string | null | undefined {
  return sources.get(sourceKey(ownerDid, sessionId))
}

export function rememberSource(ownerDid: string, sessionId: string, source: string | null) {
  sources.set(sourceKey(ownerDid, sessionId), source)
}

export function announceSources() {
  for (const listener of [...listeners]) listener()
}

/** The mock runtime's marks, kept in this browser. */
function mockSources(): Record<string, string> {
  try {
    const stored: unknown = JSON.parse(window.localStorage.getItem(MOCK_KEY) ?? '{}')
    return stored && typeof stored === 'object' && !Array.isArray(stored) ? Object.fromEntries(Object.entries(stored).filter((entry): entry is [string, string] => typeof entry[1] === 'string')) : {}
  } catch {
    return {}
  }
}

export function mockSource(ownerDid: string, sessionId: string): string | null {
  return knownSource(ownerDid, sessionId) ?? mockSources()[sourceKey(ownerDid, sessionId)] ?? null
}

export function storeMockSource(ownerDid: string, sessionId: string, source: string) {
  try { window.localStorage.setItem(MOCK_KEY, JSON.stringify({ ...mockSources(), [sourceKey(ownerDid, sessionId)]: source })) } catch { /* no storage: the mark lasts until the page is reloaded */ }
}
