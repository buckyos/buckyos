import { buckyos, getActiveSessionToken, RuntimeType } from 'buckyos'
import { OpenDanError, type OpenDanDataModel } from './datamodel'
import type { PerceptionCursor } from './types'

const SERVICE_URL = '/kapi/opendan'

// The session token of the zone login (SSO cookie -> /sso_refresh), when there
// is one. A `--dev` backend checks nothing and is not behind SSO, so every
// failure here means "no token"; that answer is kept for a while so polling
// does not retry the refresh on each call.
const NO_TOKEN_TTL_MS = 60_000
let sdkReady: Promise<boolean> | null = null
let noTokenUntil = 0

const sessionToken = async (): Promise<string | null> => {
  if (Date.now() < noTokenUntil) return null
  sdkReady ??= buckyos
    .initBuckyOS('opendan', {
      appId: 'opendan',
      zoneHost: window.location.host,
      defaultProtocol: `${window.location.protocol}//`,
      runtimeType: RuntimeType.Browser,
    })
    .then(() => true, () => false)
  const token = (await sdkReady) ? await getActiveSessionToken().catch(() => null) : null
  if (!token) noTokenUntil = Date.now() + NO_TOKEN_TTL_MS
  return token || null
}

const toError = (error: unknown): OpenDanError => {
  const text = error instanceof Error ? error.message : String(error)
  const start = text.indexOf('{')
  if (start >= 0) {
    try {
      const wire = JSON.parse(text.slice(start)) as { kind?: unknown; message?: unknown }
      if (typeof wire.message === 'string') {
        return new OpenDanError(typeof wire.kind === 'string' ? wire.kind : 'other', wire.message)
      }
    } catch {
      // not the service's error object
    }
  }
  return new OpenDanError('transport', text)
}

export const createKrpcDataModel = (): OpenDanDataModel => {
  const client = new buckyos.kRPCClient(SERVICE_URL, null, null, { sessionTokenProvider: sessionToken })
  const call = async <T>(method: string, params: Record<string, unknown> = {}): Promise<T> => {
    try {
      return await client.call<T, Record<string, unknown>>(method, params)
    } catch (error) {
      throw toError(error)
    }
  }

  return {
    source: 'krpc',
    loaderStatus: () => call('loader.status'),
    sessions: () => call('sessions.query'),
    activeSessions: () => call('activity.active', { limit: 50 }),
    session: (sid, worklog = 40) => call('session.read', { sid, worklog }),
    stopSession: (sid, reason) => call('session.stop', { sid, ...(reason ? { reason } : {}) }),
    decideSession: (sid, decision, note) => call('session.decide', { sid, decision, ...(note ? { note } : {}) }),
    postMessage: (sid, text) => call('session.post', { sid, text }),
    perception: async () => {
      const cursor = await call<PerceptionCursor>('perception.cursor')
      const backlog = await call<{ items: [] }>('perception.backlog', { cursor })
      return { cursor, backlog: backlog.items }
    },
    perceptionRecords: (item) => call('perception.read', { item }),
    artifacts: () => call('artifacts.list'),
    artifactVersions: (aid) => call('artifacts.versions', { aid }),
    behaviors: async () => {
      const [revision, behaviors] = await Promise.all([
        call<string>('behaviors.revision'),
        call<[]>('behaviors.list'),
      ])
      return { revision, behaviors }
    },
    behavior: (name) => call('behaviors.get', { name }),
    identity: () => call('behaviors.identity'),
    recallHints: (tags, maxHints = 8) => call('cognition.recall_hints', { query: { tags, max_hints: maxHints } }),
  }
}
