/**
 * Self-host Group application extensions as the clients see them (扩展架构
 * 设计 L1–L3, self-host-group-extension-todo EXT-01..06): the embedded panels a
 * Session shows, the group's / Session's display configuration and the
 * business Hooks bound to it. Message Hub only renders; the host validated
 * every declaration when it was written.
 */
import type { GroupDisplay, GroupPanelDecl, GroupSessionItem, GroupSessionRules } from '../datamodel/sessionApi'

/** One panel in effect for a Session (`group.list_sessions` `panels[]`). */
export interface GroupPanelRef {
  id: string
  title: string
  entryUrl: string
}

export interface GroupDisplayInfo {
  announcement?: string
  links: Array<{ title: string; url: string }>
  files: Array<{ title: string; objId: string }>
  featuredMembers: string[]
}

export interface GroupHookInfo {
  point: string
  service: string
  mayGrant: boolean
}

export const emptyDisplay: GroupDisplayInfo = { links: [], files: [], featuredMembers: [] }

/** What the panel page learns from its URL (D-1): never a token or the viewer. */
export interface PanelContext {
  groupDid: string
  /** Raw Session id; null for the default session. */
  sessionId: string | null
  /** Canonical MailboxAddress of the Session. */
  session: string
  locale: string
}

const text = (value: unknown) => typeof value === 'string' ? value.trim() : ''

export function panelsOf(items: readonly GroupPanelDecl[] | null | undefined): GroupPanelRef[] {
  return (items ?? []).flatMap(item => {
    const id = text(item?.id), entryUrl = text(item?.entry_url)
    return id && entryUrl ? [{ id, title: text(item.title) || id, entryUrl }] : []
  })
}

export function displayOf(value: GroupDisplay | null | undefined): GroupDisplayInfo {
  const announcement = text(value?.announcement)
  return {
    ...(announcement ? { announcement } : {}),
    links: (value?.links ?? []).flatMap(link => { const url = text(link?.url); return url ? [{ title: text(link.title) || url, url }] : [] }),
    files: (value?.files ?? []).flatMap(file => { const objId = text(file?.obj_id); return objId ? [{ title: text(file.title) || objId, objId }] : [] }),
    featuredMembers: (value?.featured_members ?? []).map(text).filter(Boolean),
  }
}

export function hooksOf(rules: GroupSessionRules | null | undefined): GroupHookInfo[] {
  return (rules?.hooks ?? []).flatMap(hook => {
    const point = text(hook?.point), service = text(hook?.service)
    return point && service ? [{ point, service, mayGrant: hook.may_grant === true }] : []
  })
}

/** Extension fields of one `group.list_sessions` item. */
export function sessionExtensions(item: GroupSessionItem) {
  return { panels: panelsOf(item.panels), display: displayOf(item.display), hooks: hooksOf(item.rules) }
}

/** The Hooks bound anywhere in the group, one row per (point, service). */
export function groupHooks(sessions: ReadonlyArray<{ hooks: GroupHookInfo[] }>): GroupHookInfo[] {
  const seen = new Map<string, GroupHookInfo>()
  for (const session of sessions) for (const hook of session.hooks) {
    const key = `${hook.point}\n${hook.service}`
    const known = seen.get(key)
    if (!known) seen.set(key, hook)
    else if (hook.mayGrant && !known.mayGrant) seen.set(key, hook)
  }
  return [...seen.values()]
}

export function hookServiceHost(service: string): string {
  try { return new URL(service).host } catch { return service }
}

/** Host name of a Zone DID as the gateway serves it (`did:web:x` → `x`; others `<id>.<method>.did`). */
export function zoneHostOfDid(did: string): string {
  const parts = did.split(':')
  const method = parts[1] ?? ''
  const id = parts.slice(2).join(':').split(':')[0] ?? ''
  return (method === 'web' ? id : `${id}.${method}.did`).replace(/\.$/, '').toLowerCase()
}

/**
 * D-2: a panel is trusted only when an App of the group's host Zone serves it
 * over https, from `<app>.<zone>` or `<app>-<zone>`. `allowOrigins` is the
 * mock host's own origin (dev pages), never used with a real backend.
 */
export function panelOriginTrusted(entryUrl: string, hostDid: string, allowOrigins: readonly string[] = []): boolean {
  let url: URL
  try { url = new URL(entryUrl) } catch { return false }
  if (allowOrigins.includes(url.origin)) return true
  if (url.protocol !== 'https:' || url.username || url.password) return false
  const zone = zoneHostOfDid(hostDid)
  if (!zone) return false
  const host = url.hostname.toLowerCase().replace(/\.$/, '')
  const suffix = host.endsWith(`.${zone}`) ? '.' : host.endsWith(`-${zone}`) ? '-' : null
  if (!suffix) return false
  const label = host.slice(0, host.length - zone.length - 1)
  return /^[a-z0-9_-]+$/.test(label)
}

/** Mock mode only: the dev server's own origin is an accepted panel source (`public/messagehub/mock-panel.html`). */
export function mockPanelOrigins(): readonly string[] {
  return typeof window !== 'undefined' ? [window.location.origin] : []
}

/** `entry_url` with the Session context appended (D-1); existing query keys of the entry stay. */
export function panelSrc(entryUrl: string, context: PanelContext): string {
  const url = new URL(entryUrl)
  url.searchParams.set('group_did', context.groupDid)
  if (context.sessionId !== null) url.searchParams.set('session_id', context.sessionId)
  else url.searchParams.delete('session_id')
  url.searchParams.set('session', context.session)
  url.searchParams.set('locale', context.locale)
  return url.toString()
}
