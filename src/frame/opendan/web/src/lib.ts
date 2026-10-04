import useSWR, { type SWRResponse } from 'swr'
import type { AgentProfile, RegistryEntry, UiBinding } from './model'

export const REFRESH_MS = 4000

export const useQuery = <T>(key: unknown[] | null, fetcher: () => Promise<T>, poll = true): SWRResponse<T, Error> =>
  useSWR<T, Error>(key, fetcher, {
    refreshInterval: poll ? REFRESH_MS : 0,
    revalidateOnFocus: false,
    keepPreviousData: true,
    shouldRetryOnError: false,
  })

const pad = (n: number) => String(n).padStart(2, '0')

export const fmtTime = (ms?: number | null): string => {
  if (!ms) return '–'
  const d = new Date(ms)
  const day = `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
  const time = `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
  return new Date().toDateString() === d.toDateString() ? time : `${day} ${time}`
}

export const fmtAgo = (ms?: number | null): string => {
  if (!ms) return '–'
  const s = Math.max(0, Math.round((Date.now() - ms) / 1000))
  if (s < 60) return `${s}s ago`
  if (s < 3600) return `${Math.floor(s / 60)}m ago`
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`
  return `${Math.floor(s / 86400)}d ago`
}

export const toText = (value: unknown): string =>
  typeof value === 'string' ? value : JSON.stringify(value, null, 2) ?? ''

export const sessionHref = (sid: string) => `#/session/${encodeURIComponent(sid)}`

export const fmtTokens = (n: number): string => {
  const scaled = (value: number, unit: string) => `${value >= 100 ? Math.round(value) : value.toFixed(1).replace(/\.0$/, '')}${unit}`
  if (n >= 1e9) return scaled(n / 1e9, 'B')
  if (n >= 1e6) return scaled(n / 1e6, 'M')
  if (n >= 1e3) return scaled(n / 1e3, 'k')
  return String(n)
}

/** `did:bns:devtest` -> `devtest`. */
export const didName = (did: string): string => did.split(':').pop() || did

// A UI session's `route_key` is the agent's mailbox address,
// `<agent did>/<conversation session, percent-encoded>`.
const mailboxSession = (routeKey?: string): string | null => {
  const at = routeKey?.indexOf('/') ?? -1
  if (!routeKey || at < 0) return null
  try {
    return decodeURIComponent(routeKey.slice(at + 1))
  } catch {
    return null
  }
}

/**
 * Where a conversation lives in MessageHub. Without a session: the owner's
 * default session with the agent. The owner's own conversations open in
 * their own view; a conversation the agent has with someone else (or in a
 * group) opens as the agent's, observed. `null`: no message system (outside
 * a zone), or the session is not bound to a conversation.
 */
export const messageHubHref = (profile: AgentProfile, entry?: RegistryEntry, binding?: UiBinding): string | null => {
  if (!profile.desktop_url) return null
  const params = new URLSearchParams()
  if (!entry) {
    params.set('entityId', profile.agent_did)
  } else if (entry.kind !== 'ui' || !binding) {
    return null
  } else if (binding.kind !== 'group_msg' && binding.to === profile.owner_did) {
    params.set('entityId', profile.agent_did)
    params.set('sessionId', binding.to_session ?? `dm:${profile.agent_did}`)
  } else {
    params.set('ownerDid', profile.agent_did)
    params.set('mode', 'observe')
    params.set('entityId', binding.to)
    const session = mailboxSession(entry.route_key) ?? binding.to_session
    if (session) params.set('sessionId', session)
  }
  return `${profile.desktop_url}/messagehub?${params}`
}
