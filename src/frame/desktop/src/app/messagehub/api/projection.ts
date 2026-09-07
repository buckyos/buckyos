/**
 * Pure projection from msg-center wire data to the UI DataModel
 * (`UI_DATAMODEL.md` §3). No I/O: everything the projection needs is passed
 * in, so the rules are unit-testable with Deno fixtures.
 */
import type { CreationPolicy, Entity, EntityDetail, MessagePreview, Session, SessionBinding } from '../types'
import type { Contact, GroupSummary, MailboxRecordWithObject, SessionSummary } from '../datamodel/sessionApi'
import type { MessageObject } from '../protocol/msgobj'
import { isActionMessage, isMessageActivity } from '../sessionModel'

/** UI container for sessions without a determinable peer. Not a DID, never a send target. */
export const UNASSIGNED_ENTITY_ID = 'messagehub:unassigned'

export interface ProjectionLabels {
  direct: string
  untitled: string
  unassigned: string
  unassignedDescription: string
  you: string
  previewImage: string
  previewAttachment: string
  previewUnavailable: string
}

export interface ProjectionInput {
  ownerDid: string
  summaries: SessionSummary[]
  contacts: Contact[]
  groups: GroupSummary[]
  /** Zone agent DIDs from the control panel (may be empty). */
  agentDids: string[]
  zoneHost?: string
  /** Owner-scoped `ui.title` overrides by session id. */
  personalTitles: Record<string, string>
  policies: Record<string, CreationPolicy>
  labels: ProjectionLabels
}

export interface ProjectedOwner {
  entities: Entity[]
  sessions: Session[]
  details: Record<string, EntityDetail>
  names: Record<string, string>
}

export interface SessionAttribution {
  peerDid: string | null
  isGroup: boolean
  evidence: 'registered' | 'group_tag' | 'group_message' | 'direct' | 'message' | 'record' | 'none'
}

export interface TunnelDidParts {
  accountId: string
  accountType: string
  tunnelInstanceId: string
}

export function parseTunnelDid(did: string): TunnelDidParts | null {
  if (!did.startsWith('did:msgtunnel:')) return null
  const parts = did.slice('did:msgtunnel:'.length).split('.')
  if (parts.length < 3) return null
  const tunnelInstanceId = parts[parts.length - 1]
  const accountType = parts[parts.length - 2]
  const accountId = parts.slice(0, -2).join('.')
  if (!tunnelInstanceId || !accountType || !accountId) return null
  return { accountId, accountType, tunnelInstanceId }
}

export function shortDid(did: string): string {
  const tunnel = parseTunnelDid(did)
  if (tunnel) return `${tunnel.accountId} · ${platformOfInstance(tunnel.tunnelInstanceId)}`
  const parts = did.split(':')
  return parts.length >= 3 ? parts.slice(2).join(':') : did
}

export function platformOfInstance(tunnelInstanceId: string): string {
  const lower = tunnelInstanceId.toLowerCase()
  if (lower.startsWith('tg') || lower.includes('telegram')) return 'telegram'
  if (lower.includes('mail')) return 'email'
  return tunnelInstanceId
}

/** Map an endpoint / alias DID to the contact that owns it. */
export function canonicalizeDid(did: string, contacts: readonly Contact[]): string {
  for (const contact of contacts) {
    if (contact.did === did) return contact.did
    if (contact.bindings?.some(binding => binding.endpoint_did === did)) return contact.did
  }
  return did
}

function groupTag(record: MailboxRecordWithObject | undefined): string | null {
  const tag = record?.record.tags?.find(item => item.startsWith('group:'))
  return tag ? tag.slice('group:'.length) : null
}

/**
 * §3.2.1: registered peer → group tag / group message target → `dm:` peer →
 * original message endpoints relative to the owner → record fields (weak
 * evidence) → unassigned.
 */
export function attributeSession(summary: SessionSummary, ownerDid: string): SessionAttribution {
  const registeredPeer = summary.state?.peer_did
  if (summary.state?.registered && registeredPeer) return { peerDid: registeredPeer, isGroup: false, evidence: 'registered' }
  const record = summary.last_record
  const tagged = groupTag(record)
  if (tagged) return { peerDid: tagged, isGroup: true, evidence: 'group_tag' }
  const msg = record?.msg ?? undefined
  if (msg?.kind === 'group_msg') {
    const target = msg.to[0]
    return target ? { peerDid: target, isGroup: true, evidence: 'group_message' } : { peerDid: null, isGroup: true, evidence: 'none' }
  }
  if (summary.session_id.startsWith('dm:')) {
    const peer = summary.session_id.slice('dm:'.length)
    return peer ? { peerDid: peer, isGroup: false, evidence: 'direct' } : { peerDid: null, isGroup: false, evidence: 'none' }
  }
  if (msg && record) {
    if (record.record.box_kind === 'SENT') {
      const targets = [...new Set(msg.to)]
      if (targets.length === 1) return { peerDid: targets[0], isGroup: false, evidence: 'message' }
      return { peerDid: null, isGroup: false, evidence: 'none' }
    }
    if (msg.from && msg.from !== ownerDid) return { peerDid: msg.from, isGroup: false, evidence: 'message' }
    return { peerDid: null, isGroup: false, evidence: 'none' }
  }
  if (registeredPeer) return { peerDid: registeredPeer, isGroup: false, evidence: 'registered' }
  if (record) {
    // Without the object only the record projection is available; SENT
    // keeps just the first target and inbound `to` is the owner itself.
    if (record.record.box_kind === 'SENT') return { peerDid: record.record.to, isGroup: false, evidence: 'record' }
    if (record.record.from !== ownerDid) return { peerDid: record.record.from, isGroup: false, evidence: 'record' }
  }
  return { peerDid: null, isGroup: false, evidence: 'none' }
}

export function summarizeMessage(msg: MessageObject | null | undefined, labels: ProjectionLabels): string {
  if (!msg) return labels.previewUnavailable
  if (isActionMessage(msg)) return msg.content.content || labels.previewUnavailable
  const refs = msg.content.refs ?? []
  const dataRef = refs.find(ref => ref.target.type === 'data_obj')
  const format = msg.content.format ?? 'text/plain'
  const isText = format.startsWith('text/')
  if (dataRef && !msg.content.content.trim()) {
    const label = dataRef.label ? ` ${dataRef.label}` : ''
    return `${format.startsWith('image/') ? labels.previewImage : labels.previewAttachment}${label}`
  }
  if (isText || msg.content.content.trim()) {
    const line = msg.content.content.split('\n').find(item => item.trim())?.trim() ?? ''
    return line.length > 120 ? `${line.slice(0, 120)}…` : line
  }
  return `${labels.previewAttachment} ${format}`
}

export function isZoneAgentDid(did: string, zoneHost: string | undefined): boolean {
  if (!zoneHost || !did.startsWith('did:web:')) return false
  const host = did.slice('did:web:'.length)
  return host !== zoneHost && host.endsWith(`.${zoneHost}`) && !host.startsWith('msg-hub.')
}

interface EntitySeed {
  id: string
  type: Entity['type']
  name: string
  domain: 'managed' | 'external'
  contact?: Contact
  group?: GroupSummary
  sources: Set<string>
}

function bindingFor(peerDid: string, contacts: readonly Contact[], summary: SessionSummary | null): SessionBinding {
  const registered = summary?.state?.registered ? summary.state.binding : undefined
  if (registered && typeof registered === 'object' && (registered as { kind?: string }).kind === 'native') {
    const target = (registered as { targetDid?: string }).targetDid ?? peerDid
    return { kind: 'native', targetDid: target }
  }
  const tunnel = parseTunnelDid(peerDid)
  if (tunnel) {
    const contact = contacts.find(item => item.did === peerDid || item.bindings?.some(binding => binding.endpoint_did === peerDid))
    const binding = contact?.bindings?.find(item => item.endpoint_did === peerDid)
    const platform = binding?.platform ?? platformOfInstance(tunnel.tunnelInstanceId)
    return {
      kind: 'tunnel',
      tunnelInstanceId: tunnel.tunnelInstanceId,
      endpointDid: peerDid,
      connectionName: `${platform} · ${tunnel.tunnelInstanceId}`,
      remoteContextId: summary && !summary.session_id.startsWith('dm:') ? summary.session_id : undefined,
      supportsMultipleSessions: false,
      canCreateRemoteSession: false,
      canSend: true,
      connected: true,
    }
  }
  return { kind: 'native', targetDid: peerDid }
}

export function projectOwner(input: ProjectionInput): ProjectedOwner {
  const { ownerDid, contacts, groups, labels } = input
  const groupByDid = new Map(groups.map(group => [group.group_did, group]))
  const contactByDid = new Map(contacts.map(contact => [contact.did, contact]))
  const agentSet = new Set(input.agentDids)
  const seeds = new Map<string, EntitySeed>()
  const names: Record<string, string> = {}

  const ensureSeed = (did: string, hint?: { isGroup?: boolean; fromName?: string }): EntitySeed => {
    const existing = seeds.get(did)
    if (existing) return existing
    const contact = contactByDid.get(did)
    const group = groupByDid.get(did)
    const tunnel = parseTunnelDid(did)
    const isGroup = Boolean(group) || hint?.isGroup === true || tunnel?.accountType === 'group' || tunnel?.accountType === 'channel'
    const isAgent = !isGroup && (agentSet.has(did) || contact?.tags?.includes('agent') === true || isZoneAgentDid(did, input.zoneHost))
    const type: Entity['type'] = isGroup ? 'group' : isAgent ? 'agent' : 'person'
    const domain: EntitySeed['domain'] = tunnel ? 'external' : group ? (group.is_hosted_by_self ? 'managed' : 'external') : did.startsWith('did:bns:') || isAgent ? 'managed' : 'external'
    const name = contact?.name?.trim() || group?.name?.trim() || hint?.fromName?.trim() || shortDid(did)
    const seed: EntitySeed = { id: did, type, name, domain, contact, group, sources: new Set() }
    if (tunnel) seed.sources.add(platformOfInstance(tunnel.tunnelInstanceId))
    contact?.bindings?.forEach(binding => seed.sources.add(binding.platform))
    if (!tunnel) seed.sources.add('buckyos')
    seeds.set(did, seed)
    names[did] = name
    return seed
  }

  for (const contact of contacts) {
    if (contact.did === ownerDid) { names[ownerDid] = contact.name || labels.you; continue }
    ensureSeed(contact.did)
  }
  for (const group of groups) ensureSeed(group.group_did, { isGroup: true })
  for (const did of input.agentDids) ensureSeed(did)
  names[ownerDid] ??= labels.you

  const sessions: Session[] = []
  const sessionsByEntity = new Map<string, Session[]>()
  for (const summary of input.summaries) {
    const attribution = attributeSession(summary, ownerDid)
    const rawPeer = attribution.peerDid
    const peerDid = rawPeer ? canonicalizeDid(rawPeer, contacts) : null
    const entityId = peerDid ?? UNASSIGNED_ENTITY_ID
    if (peerDid) ensureSeed(peerDid, { isGroup: attribution.isGroup, fromName: summary.last_record?.record.from_name })
    const binding: SessionBinding = peerDid ? bindingFor(rawPeer ?? peerDid, contacts, summary) : { kind: 'unknown' }
    const msg = summary.last_record?.msg ?? undefined
    const lastMessage: MessagePreview | undefined = summary.last_record
      ? { senderName: attribution.isGroup && msg ? (msg.from === ownerDid ? labels.you : names[canonicalizeDid(msg.from, contacts)] ?? shortDid(msg.from)) : undefined, text: summarizeMessage(msg, labels), timestamp: summary.last_record.record.sort_key }
      : undefined
    const registeredTitle = summary.state?.registered ? summary.state.title?.trim() ?? '' : ''
    const topic = msg?.thread?.topic?.trim()
    const derivedTitle = topic && topic !== summary.session_id && !summary.session_id.startsWith('dm:') ? topic
      : summary.session_id.startsWith('dm:') ? labels.direct
      : attribution.isGroup && peerDid ? names[peerDid] ?? shortDid(peerDid)
      : binding.kind === 'tunnel' ? `${binding.connectionName}`
      : peerDid ? labels.direct : labels.untitled
    const session: Session = {
      id: summary.session_id,
      ownerDid,
      entityId,
      binding,
      origin: summary.state?.registered ? (summary.state.origin === 'connection' || summary.state.origin === 'remote_context' ? summary.state.origin : 'manual') : binding.kind === 'tunnel' ? 'connection' : peerDid ? 'remote_context' : 'unknown',
      lifecycle: summary.lifecycle ?? 'active',
      createdAt: summary.state?.created_at_ms ?? 0,
      shared: { title: registeredTitle, description: '', updatedAt: summary.state?.updated_at_ms ?? 0 },
      members: {},
      lastMessage: lastMessage && isMessageActivity(msg ?? { kind: 'chat', from: '', to: [], created_at_ms: 0, content: { content: lastMessage.text } }) ? lastMessage : lastMessage,
      title: derivedTitle,
      type: 'chat',
      source: binding.kind === 'tunnel' ? platformOfInstance(binding.tunnelInstanceId) : 'buckyos',
      lastActiveAt: summary.last_activity_ms || summary.state?.created_at_ms || 0,
      unreadCount: summary.unread_count,
      requestCount: summary.request_count,
      lastDelivery: summary.last_record?.record.box_kind === 'SENT' ? undefined : undefined,
      attributionEvidence: attribution.evidence,
    }
    sessions.push(session)
    const list = sessionsByEntity.get(entityId) ?? []
    list.push(session)
    sessionsByEntity.set(entityId, list)
  }

  const entities: Entity[] = []
  const details: Record<string, EntityDetail> = {}
  const toEntity = (seed: EntitySeed): Entity => {
    const own = sessionsByEntity.get(seed.id) ?? []
    const latest = [...own].sort((a, b) => b.lastActiveAt - a.lastActiveAt || a.id.localeCompare(b.id))[0]
    const entity: Entity = {
      id: seed.id,
      type: seed.type,
      name: seed.name,
      avatar: seed.contact?.avatar ?? seed.group?.avatar,
      statusText: seed.group ? `${seed.group.member_count} members` : seed.contact?.access_level === 'stranger' ? 'stranger' : undefined,
      isOnline: undefined,
      isPinned: false,
      isMuted: false,
      unreadCount: own.reduce((sum, session) => sum + session.unreadCount, 0),
      requestCount: own.reduce((sum, session) => sum + (session.requestCount ?? 0), 0),
      tags: [...new Set([...(seed.contact?.tags ?? []), ...(seed.contact?.groups ?? [])])].sort(),
      lastMessage: latest?.lastMessage,
      lastActiveAt: latest?.lastActiveAt ?? 0,
      source: [...seed.sources][0],
      sources: [...seed.sources],
      domain: seed.domain,
      sessionCount: own.length,
    }
    details[seed.id] = {
      ...entity,
      bio: seed.contact?.note ?? undefined,
      note: seed.contact?.note ?? undefined,
      memberCount: seed.group?.member_count,
      bindings: (seed.contact?.bindings ?? []).map(binding => ({ platform: binding.platform, accountId: binding.account_id, displayId: binding.display_id })),
      createdAt: seed.contact?.created_at ?? seed.group?.updated_at_ms ?? 0,
      accessLevel: seed.contact?.access_level,
      isVerified: seed.contact?.is_verified,
      contactSource: seed.contact?.source,
    }
    return entity
  }
  for (const seed of seeds.values()) entities.push(toEntity(seed))
  const unassigned = sessionsByEntity.get(UNASSIGNED_ENTITY_ID) ?? []
  if (unassigned.length > 0) {
    const latest = [...unassigned].sort((a, b) => b.lastActiveAt - a.lastActiveAt)[0]
    const container: Entity = {
      id: UNASSIGNED_ENTITY_ID,
      type: 'service',
      name: labels.unassigned,
      drilldownDescription: labels.unassignedDescription,
      unreadCount: unassigned.reduce((sum, session) => sum + session.unreadCount, 0),
      requestCount: unassigned.reduce((sum, session) => sum + (session.requestCount ?? 0), 0),
      tags: [],
      lastMessage: latest?.lastMessage,
      lastActiveAt: latest?.lastActiveAt ?? 0,
      sources: [],
      domain: 'external',
      sessionCount: unassigned.length,
      sessionCreation: { policy: 'deny', canCreate: false, unavailableReason: 'binding_unknown' },
    }
    entities.push(container)
    details[container.id] = { ...container, bindings: [], createdAt: 0 }
    names[UNASSIGNED_ENTITY_ID] = container.name
  }
  entities.sort((a, b) => Number(!!b.isPinned) - Number(!!a.isPinned) || b.lastActiveAt - a.lastActiveAt || a.name.localeCompare(b.name))
  return { entities, sessions, details, names }
}
