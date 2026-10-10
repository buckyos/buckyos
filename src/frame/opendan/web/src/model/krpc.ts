import { buckyos, getActiveSessionToken, RuntimeType } from 'buckyos'
import { OpenDanError, type OpenDanDataModel } from './datamodel'
import type { AgentProfile, LoaderStatus, PerceptionCursor } from './types'

const SERVICE_URL = '/kapi/opendan'

// The session token of the zone login (SSO cookie -> /sso_refresh). The token
// is issued to the app that serves this page; the service names it on GET.
// Outside a zone (`--dev`) there is no app and nothing is checked. A failure
// means "no token"; that answer is kept for a while so polling does not retry
// the refresh on each call.
const NO_TOKEN_TTL_MS = 60_000
let sdkReady: Promise<boolean> | null = null
let noTokenUntil = 0

const initSdk = async (): Promise<boolean> => {
  const response = await fetch(SERVICE_URL, { headers: { accept: 'application/json' } })
  const { app_id: appId } = (await response.json()) as { app_id?: string | null }
  if (!appId) return false
  await buckyos.initBuckyOS(appId, {
    appId,
    zoneHost: window.location.host,
    defaultProtocol: `${window.location.protocol}//`,
    runtimeType: RuntimeType.Browser,
  })
  return true
}

const sessionToken = async (): Promise<string | null> => {
  if (Date.now() < noTokenUntil) return null
  sdkReady ??= initSdk().catch(() => false)
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

  // A service older than the home page answers `unknown method`.
  const optional = async <T>(method: string, fallback: () => T | Promise<T>): Promise<T> => {
    try {
      return await call<T>(method)
    } catch (error) {
      if (error instanceof OpenDanError && error.message.includes('unknown method')) return fallback()
      throw error
    }
  }

  return {
    source: 'krpc',
    loaderStatus: () => call('loader.status'),
    profile: () =>
      optional<AgentProfile>('agent.profile', async () => {
        const status = await call<LoaderStatus>('loader.status')
        return {
          agent_did: status.agent_did,
          agent_id: status.agent_id,
          display_name: status.agent_id,
          avatar: null,
          bio: '',
          owner_did: null,
          desktop_url: null,
          editable: false,
        }
      }).then((p) => ({ ...p, editable: p.editable ?? true })),
    setProfile: (patch) => call<AgentProfile>('agent.profile_set', { ...patch }).then((p) => ({ ...p, editable: true })),
    usageModels: () => optional('usage.models', () => null),
    uiBindings: () => optional('ui.bindings', () => []),
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
