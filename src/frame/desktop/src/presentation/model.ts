/* ── Object presentation: DID → object → icon + title + description ──
 *
 * A UI projection only: `key`/`did` stay the identity used for routing, auth and caches;
 * `title`, `description` and `icons` are display text that may change at any time.
 * Candidate order is fixed per object kind: explicit display settings → the DID document /
 * typed profile fields → the readable part of the DID → the kind's default text.
 */

import { namelib } from 'buckyos'

export type PresentationKind = 'zone' | 'user' | 'agent' | 'app'
export type PresentationStatus = 'loading' | 'resolved' | 'failed'

export interface ObjectPresentation {
  key: string
  did?: string
  kind: PresentationKind
  title: string
  description?: string
  /** Loadable image candidates, best first; empty means the kind's default icon. */
  icons: string[]
  status: PresentationStatus
}

export type LocalizedText = string | Readonly<Record<string, unknown>> | null | undefined

export const DEFAULT_PRESENTATION_LOCALE = 'en'

const defaultTitles: Record<PresentationKind, string> = {
  zone: 'BuckyOS',
  user: 'User',
  agent: 'Agent',
  app: 'App',
}

export function textValue(value: unknown): string | undefined {
  if (typeof value !== 'string') return undefined
  const text = value.trim()
  return text || undefined
}

export function firstText(...values: unknown[]): string | undefined {
  for (const value of values) {
    const text = textValue(value)
    if (text) return text
  }
  return undefined
}

export function initialOf(title: string): string {
  return [...title.trim()][0]?.toUpperCase() ?? '?'
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {}
}

/** Current language → its base language → the default language → the first non-empty value by key order. */
export function pickLocalized(value: LocalizedText, locale: string): string | undefined {
  if (value == null || typeof value === 'string') return textValue(value)
  const entries = Object.entries(value)
    .map(([lang, text]) => [lang.replace('_', '-').toLowerCase(), textValue(text)] as const)
    .filter((entry): entry is readonly [string, string] => Boolean(entry[1]))
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
  const lookup = (lang: string) => entries.find(([key]) => key === lang.toLowerCase())?.[1]
  return lookup(locale) ?? lookup(locale.split('-')[0]) ?? lookup(DEFAULT_PRESENTATION_LOCALE) ?? entries[0]?.[1]
}

/** Images a browser may load directly: raster/vector data URLs, http(s) and same-origin paths. */
export function usableImageUrl(value: unknown): string | undefined {
  const url = textValue(value)
  if (!url) return undefined
  return /^(data:image\/|https?:\/\/|\/(?!\/))/i.test(url) ? url : undefined
}

/** An icon reference; a relative one resolves against `base` (e.g. the app's own web host). */
export function resolveImageReference(value: unknown, base?: string): string | undefined {
  const usable = usableImageUrl(value)
  if (usable || !base) return usable
  const text = textValue(value)
  if (!text || /^[a-z][a-z0-9+.-]*:/i.test(text)) return undefined
  try {
    return new URL(text, base).toString()
  } catch {
    return undefined
  }
}

export function iconCandidates(...values: unknown[]): string[] {
  const icons: string[] = []
  for (const value of values) {
    const url = usableImageUrl(value)
    if (url && !icons.includes(url)) icons.push(url)
  }
  return icons
}

function safeDecode(segment: string): string {
  try {
    return decodeURIComponent(segment)
  } catch {
    return segment
  }
}

/** The readable part of a DID (`did:bns:myapp` → `myapp`); the full DID stays the identity. */
export function didDisplayName(did: string): string {
  const value = did.trim()
  if (!value.startsWith('did:')) return value
  const bare = value.split(/[/?#]/, 1)[0]
  let parsed: { method: string; id: string }
  try {
    parsed = namelib.DID.fromStr(bare)
  } catch {
    return value
  }
  if (!parsed.method || !parsed.id) return value
  switch (parsed.method) {
    case 'web':
      return parsed.id.split(':').map(safeDecode).join('/')
    case 'key':
      return parsed.id.length > 16 ? `${parsed.id.slice(0, 8)}…${parsed.id.slice(-6)}` : parsed.id
    default:
      return parsed.id
  }
}

function presentation(
  kind: PresentationKind,
  key: string,
  did: string | undefined,
  title: string | undefined,
  description: string | undefined,
  icons: string[],
  status: PresentationStatus,
): ObjectPresentation {
  const resolvedTitle = title ?? defaultTitles[kind]
  return {
    key,
    ...(did ? { did } : {}),
    kind,
    title: resolvedTitle,
    ...(description && description !== resolvedTitle ? { description } : {}),
    icons,
    status,
  }
}

// ── Zone ──

export interface ZonePresentationFields {
  display_name?: string | null
  description?: string | null
  icon?: string | null
}

/** `zone.presentation.get`: the administrator's settings and the ZoneDocument candidates, kept apart. */
export interface ZonePresentationSource {
  zone_did?: string | null
  hostname?: string | null
  settings?: ZonePresentationFields | null
  document?: ZonePresentationFields | null
}

export const ZONE_PRESENTATION_KEY = 'zone:self'

export function presentZone(
  source: ZonePresentationSource | null | undefined,
  options: { fallbackHost?: string; status?: PresentationStatus } = {},
): ObjectPresentation {
  const settings = asRecord(source?.settings)
  const document = asRecord(source?.document)
  const did = textValue(source?.zone_did)
  return presentation(
    'zone',
    ZONE_PRESENTATION_KEY,
    did,
    firstText(settings.display_name, document.display_name, did && didDisplayName(did), source?.hostname, options.fallbackHost),
    firstText(settings.description, document.description),
    iconCandidates(settings.icon, document.icon),
    options.status ?? 'resolved',
  )
}

// ── User / Owner ──

export interface UserPresentationSource {
  did?: string | null
  userId?: string | null
  /** The effective profile (explicit local settings already over the DID document's profile). */
  profile?: unknown
  /** The OwnerDocument. */
  document?: unknown
  status?: PresentationStatus
}

function displayNameOf(record: Record<string, unknown>) {
  return firstText(record.display_name, record.displayName, record.full_name)
}

export function presentUser(source: UserPresentationSource): ObjectPresentation {
  const profile = asRecord(source.profile)
  const document = asRecord(source.document)
  const did = firstText(source.did, profile.did, document.id)
  const userId = textValue(source.userId)
  return presentation(
    'user',
    did ?? `user:${userId ?? ''}`,
    did,
    firstText(displayNameOf(profile), displayNameOf(document), profile.name, document.name, did && didDisplayName(did), userId),
    firstText(profile.bio, profile.headline),
    iconCandidates(profile.avatar, profile.avatar_url, document.avatar),
    source.status ?? 'resolved',
  )
}

// ── Agent ──

export interface AgentPresentationSource {
  did?: string | null
  agentId?: string | null
  /** The registered Agent name. */
  name?: string | null
  profile?: { display_name?: string | null; avatar?: string | null; bio?: string | null } | null
  /** The Agent document's public description. */
  publicDescription?: string | null
  templateIcon?: string | null
  status?: PresentationStatus
}

export function presentAgent(source: AgentPresentationSource): ObjectPresentation {
  const did = textValue(source.did)
  const agentId = textValue(source.agentId)
  return presentation(
    'agent',
    did ?? `agent:${agentId ?? ''}`,
    did,
    firstText(source.profile?.display_name, source.name, did && didDisplayName(did), agentId),
    firstText(source.profile?.bio, source.publicDescription),
    iconCandidates(source.profile?.avatar, source.templateIcon),
    source.status ?? 'resolved',
  )
}

// ── App ──

export interface AppPresentationText {
  title?: LocalizedText
  summary?: LocalizedText
  description?: LocalizedText
}

export interface AppPresentationSource {
  /** The stable app identity used as the key (AppInstanceId, or the AppId of a catalog app). */
  key: string
  appId?: string | null
  appDid?: string | null
  showName?: string | null
  /** The AppDoc package name. */
  name?: string | null
  presentation?: AppPresentationText | null
  /** `presentation.links.app_icon_url`. */
  iconUrl?: string | null
  /** Base URL for a relative icon reference: the app's own web host. */
  iconBase?: string
  status?: PresentationStatus
}

export function presentApp(source: AppPresentationSource, locale: string): ObjectPresentation {
  const did = textValue(source.appDid)
  const text = source.presentation ?? {}
  const icon = resolveImageReference(source.iconUrl, source.iconBase)
  return presentation(
    'app',
    source.key,
    did,
    firstText(pickLocalized(text.title, locale), source.showName, source.name, did && didDisplayName(did), source.appId, source.key),
    firstText(pickLocalized(text.summary, locale), pickLocalized(text.description, locale)),
    icon ? [icon] : [],
    source.status ?? 'resolved',
  )
}

// ── Any DID document ──

/** Projects a resolved DID document by its own shape (OwnerDocument, AgentDocument, ZoneDocument, AppDoc). */
export function presentDidDocument(did: string, document: unknown, locale: string): ObjectPresentation {
  const doc = asRecord(document)
  if (doc.app_type !== undefined || doc.pkg_list !== undefined) {
    return presentApp({
      key: did,
      appDid: did,
      showName: textValue(doc.show_name),
      name: textValue(doc.name),
      presentation: asRecord(doc.presentation) as AppPresentationText,
      iconUrl: textValue(asRecord(asRecord(doc.presentation).links).app_icon_url),
    }, locale)
  }
  if (Array.isArray(doc.oods) || doc.boot_jwt !== undefined) {
    return presentZone({
      zone_did: did,
      hostname: textValue(doc.hostname),
      document: doc as ZonePresentationFields,
    })
  }
  if (doc.owner !== undefined && (doc.contact !== undefined || doc.support_public_access !== undefined)) {
    return presentAgent({
      did,
      name: firstText(doc.display_name, doc.name),
      publicDescription: textValue(doc.public_description),
      templateIcon: textValue(doc.avatar),
    })
  }
  return presentUser({ did, profile: asRecord(doc.profile), document: doc })
}

/** Stand-in shown while a DID is being resolved or after resolution failed. */
export function presentUnresolvedDid(did: string, kind: PresentationKind, status: PresentationStatus): ObjectPresentation {
  return presentation(kind, did, did, didDisplayName(did), undefined, [], status)
}
