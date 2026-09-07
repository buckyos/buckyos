import { attributeSession, canonicalizeDid, parseTunnelDid, projectOwner, summarizeMessage, UNASSIGNED_ENTITY_ID, type ProjectionLabels } from '../../src/app/messagehub/api/projection.ts'
import { itemToMessage, recordMeta, removeMessage, upsertMessages, emptyHistory } from '../../src/app/messagehub/api/reader.ts'
import type { Contact, SessionMessageItem, SessionSummary } from '../../src/app/messagehub/datamodel/sessionApi.ts'
import type { MessageObject } from '../../src/app/messagehub/protocol/msgobj.ts'

const owner = 'did:bns:devtest'
const peerEndpoint = 'did:msgtunnel:5397330802.user.tg-main-tunnel'
const labels: ProjectionLabels = { direct: 'Direct', untitled: 'Untitled', unassigned: 'Unassigned', unassignedDescription: '', you: 'You', previewImage: '[Image]', previewAttachment: '[Attachment]', previewUnavailable: 'Unavailable' }
const contacts: Contact[] = [
  { did: peerEndpoint, name: 'zzcc ll', source: 'auto_inferred', is_verified: false, access_level: 'stranger', created_at: 1, updated_at: 1, bindings: [{ platform: 'telegram', account_id: '5397330802', display_id: '@wacer2026', tunnel_instance_id: 'tg-main-tunnel', account_type: 'user', endpoint_did: peerEndpoint, last_active_at: 1 }] },
  { did: 'did:bns:alice', name: 'Alice', source: 'shared', is_verified: true, access_level: 'friend', created_at: 1, updated_at: 1, tags: ['zone_user'] },
]
function equal(actual: unknown, expected: unknown) { if (JSON.stringify(actual) !== JSON.stringify(expected)) throw Error(`Expected ${JSON.stringify(expected)}, received ${JSON.stringify(actual)}`) }
function chat(from: string, to: string[], text: string, created: number, extra: Partial<MessageObject> = {}): MessageObject { return { from, to, kind: 'chat', created_at_ms: created, content: { format: 'text/plain', content: text }, ...extra } }
function summary(session_id: string, record: { box_kind: SessionSummary['last_record'] extends undefined ? never : 'INBOX' | 'SENT' | 'GROUP_INBOX' | 'REQUEST_BOX'; from: string; to: string; msg?: MessageObject | null; tags?: string[]; sort_key?: number } | null, extra: Partial<SessionSummary> = {}): SessionSummary {
  return { session_id, unread_count: 0, updated_at_ms: 10, last_activity_ms: 10, request_count: 0, lifecycle: 'active', last_record: record ? { record: { record_id: `${owner}|${record.box_kind}|m|inbox`, owner, box_kind: record.box_kind, msg_id: 'm', state: 'UNREAD', from: record.from, to: record.to, session_id, sort_key: record.sort_key ?? 10, tags: record.tags, created_at_ms: 10, updated_at_ms: 10 }, msg: record.msg } : undefined, ...extra }
}

Deno.test('group member INBOX copies attribute to the group, not the owner, in both directions', () => {
  const group = 'did:bns:team'
  const inbound = summary('topic-a', { box_kind: 'INBOX', from: 'did:bns:alice', to: owner, tags: [`group:${group}`], msg: chat('did:bns:alice', [group], 'hi', 10, { kind: 'group_msg' }) })
  equal(attributeSession(inbound, owner), { peerDid: group, isGroup: true, evidence: 'group_tag' })
  const outbound = summary('topic-a', { box_kind: 'SENT', from: owner, to: group, msg: chat(owner, [group], 'reply', 11, { kind: 'group_msg' }) })
  equal(attributeSession(outbound, owner), { peerDid: group, isGroup: true, evidence: 'group_message' })
  const multi = summary('topic-b', { box_kind: 'SENT', from: owner, to: 'did:bns:a', msg: chat(owner, ['did:bns:a', 'did:bns:b'], 'x', 12) })
  equal(attributeSession(multi, owner).peerDid, null)
  const noObject = summary('topic-c', { box_kind: 'SENT', from: owner, to: 'did:bns:a', msg: null })
  equal(attributeSession(noObject, owner).evidence, 'record')
  const registered = summary('uuid-1', null, { state: { owner, session_id: 'uuid-1', lifecycle: 'active', registered: true, peer_did: 'did:bns:agent', created_at_ms: 5, updated_at_ms: 5 } })
  equal(attributeSession(registered, owner), { peerDid: 'did:bns:agent', isGroup: false, evidence: 'registered' })
})

Deno.test('tunnel endpoints canonicalize through contact bindings and keep a tunnel binding', () => {
  equal(parseTunnelDid(peerEndpoint), { accountId: '5397330802', accountType: 'user', tunnelInstanceId: 'tg-main-tunnel' })
  equal(canonicalizeDid(peerEndpoint, contacts), peerEndpoint)
  const tg = summary('tg:lzc_jarvis:5397330802', { box_kind: 'REQUEST_BOX', from: peerEndpoint, to: owner, msg: chat(peerEndpoint, [owner], 'hello', 20) }, { request_count: 1, last_activity_ms: 20 })
  const projected = projectOwner({ ownerDid: owner, summaries: [tg], contacts, groups: [], agentDids: [], personalTitles: {}, policies: {}, labels })
  const session = projected.sessions[0]
  equal(session.entityId, peerEndpoint)
  equal(session.binding.kind, 'tunnel')
  equal((session.binding as { tunnelInstanceId: string }).tunnelInstanceId, 'tg-main-tunnel')
  equal(session.requestCount, 1)
  equal(session.lastActiveAt, 20)
  equal(session.title, 'telegram · tg-main-tunnel')
  const entity = projected.entities.find(item => item.id === peerEndpoint)!
  equal(entity.type, 'person')
  equal(entity.domain, 'external')
  equal(entity.name, 'zzcc ll')
  equal(entity.requestCount, 1)
  equal(entity.sources, ['telegram'])
})

Deno.test('zone agents, unassigned sessions and previews follow the data model rules', () => {
  const agent = 'did:web:jarvis.test.buckyos.io'
  const dm = summary(`dm:${agent}`, { box_kind: 'INBOX', from: agent, to: owner, msg: chat(agent, [owner], 'line one\nline two', 30) }, { last_activity_ms: 30, unread_count: 2 })
  const orphan = summary('mystery', { box_kind: 'SENT', from: owner, to: 'did:bns:a', msg: chat(owner, ['did:bns:a', 'did:bns:b'], 'x', 12) })
  const empty = summary('uuid-2', null, { last_activity_ms: 0, state: { owner, session_id: 'uuid-2', lifecycle: 'active', registered: true, peer_did: agent, title: 'Plan', created_at_ms: 40, updated_at_ms: 40 } })
  const projected = projectOwner({ ownerDid: owner, summaries: [dm, orphan, empty], contacts, groups: [], agentDids: [agent], personalTitles: {}, policies: {}, labels })
  const agentEntity = projected.entities.find(item => item.id === agent)!
  equal(agentEntity.type, 'agent')
  equal(agentEntity.unreadCount, 2)
  equal(agentEntity.sessionCount, 2)
  equal(agentEntity.lastActiveAt, 40)
  equal(projected.sessions.find(item => item.id === 'uuid-2')?.shared.title, 'Plan')
  equal(projected.sessions.find(item => item.id === 'uuid-2')?.lastActiveAt, 40)
  equal(projected.sessions.find(item => item.id === 'uuid-2')?.origin, 'manual')
  equal(projected.sessions.find(item => item.id === `dm:${agent}`)?.title, 'Direct')
  equal(projected.sessions.find(item => item.id === `dm:${agent}`)?.lastMessage?.text, 'line one')
  const unassigned = projected.entities.find(item => item.id === UNASSIGNED_ENTITY_ID)!
  equal(unassigned.sessionCreation?.canCreate, false)
  equal(projected.sessions.find(item => item.id === 'mystery')?.binding.kind, 'unknown')
  equal(summarizeMessage({ ...chat(agent, [owner], '', 1), content: { format: 'image/png', content: '', refs: [{ role: 'input', label: 'photo.png', target: { type: 'data_obj', obj_id: 'cyfile:1' } }] } }, labels), '[Image] photo.png')
  equal(summarizeMessage(null, labels), 'Unavailable')
  equal(summarizeMessage({ ...chat(agent, [owner], 'Title changed', 1), kind: 'event', content: { content: 'Title changed', machine: { intent: 'buckyos.action_log' } } }, labels), 'Title changed')
})

Deno.test('timeline items keep record context, placeholders and upsert / remove semantics', () => {
  const item: SessionMessageItem = { record_id: 'r1', msg_id: 'm1', direction: 'in', box_kind: 'REQUEST_BOX', sort_key: 5, from: peerEndpoint, to: owner, recipient_state: 'UNREAD', msg: chat(peerEndpoint, [owner], 'hi', 5) }
  const message = itemToMessage(item, owner, 'tg', 'zzcc ll', 'Unavailable')
  equal(recordMeta(message)?.boxKind, 'REQUEST_BOX')
  equal(recordMeta(message)?.recipientState, 'UNREAD')
  equal(message.ui_sender_name, 'zzcc ll')
  equal(message.ui_message_id, 'r1')
  const missing = itemToMessage({ ...item, record_id: 'r0', sort_key: 1, msg: null }, owner, 'tg', undefined, 'Unavailable')
  equal(missing.ui_unavailable, true)
  equal(missing.content.content, 'Unavailable')
  const out = itemToMessage({ record_id: 'r2', msg_id: 'm2', direction: 'out', box_kind: 'SENT', sort_key: 9, from: owner, to: peerEndpoint, delivery: { overall: 'partial_failed', per_target: [{ target_did: peerEndpoint, state: 'DEAD', attempts: 3, last_error: { message: 'boom', retryable: false, duplicate_risk: true } }] }, msg: chat(owner, [peerEndpoint], 'x', 9) }, owner, 'tg', 'You', 'Unavailable')
  equal(out.ui_delivery_status, 'failed')
  equal(recordMeta(out)?.delivery?.per_target?.[0].last_error?.duplicate_risk, true)
  const history = upsertMessages(emptyHistory, [message, out], { loaded: true })
  equal(history.messages.map(item => item.ui_message_id), ['r1', 'r2'])
  equal(history.revision, 1)
  const older = upsertMessages(history, [missing], { hasOlder: false })
  equal(older.messages.map(item => item.ui_message_id), ['r0', 'r1', 'r2'])
  equal(upsertMessages(older, [message]) === older, true)
  const read = upsertMessages(older, [{ ...message, ui_record: { ...recordMeta(message)!, recipientState: 'READ' } }])
  equal(read.revision, older.revision + 1)
  equal(read.messages.length, 3)
  equal(recordMeta(read.messages[1])?.recipientState, 'READ')
  equal(removeMessage(read, 'r0').messages.length, 2)
  equal(removeMessage(read, 'nope') === read, true)
})

Deno.test('attachment-only protocol objects survive timeline loading and session previews', () => {
  const message = chat(owner, [peerEndpoint], '', 10, {
    content: { format: 'text/plain', refs: [{ role: 'input', label: 'photo.png', target: { type: 'data_obj', obj_id: 'cyfile:photo' } }] },
  })
  const restored: MessageObject = JSON.parse(JSON.stringify(message))
  const item: SessionMessageItem = { record_id: 'photo-record', msg_id: 'photo-message', direction: 'out', box_kind: 'SENT', sort_key: 10, from: owner, to: peerEndpoint, msg: restored }
  const projectedMessage = itemToMessage(item, owner, 'photo-session', 'You', 'Unavailable')
  equal(projectedMessage.content.content, undefined)
  equal(projectedMessage.content.refs, message.content.refs)
  equal(summarizeMessage(projectedMessage, labels), '[Attachment] photo.png')
  equal(summarizeMessage({ ...restored, content: { ...restored.content, format: 'image/png' } }, labels), '[Image] photo.png')
  equal(summarizeMessage({ ...restored, content: { ...restored.content, content: '  Caption\nmore text' } }, labels), 'Caption')
  equal(summarizeMessage({ ...restored, content: {} }, labels), '')
  const projected = projectOwner({ ownerDid: owner, summaries: [summary('photo-session', { box_kind: 'SENT', from: owner, to: peerEndpoint, msg: restored })], contacts, groups: [], agentDids: [], personalTitles: {}, policies: {}, labels })
  equal(projected.sessions[0].lastMessage?.text, '[Attachment] photo.png')
})

Deno.test('web DID zone users are people and unknown local DIDs do not become agents', () => {
  const lucy = 'did:web:lucy.test.buckyos.io'
  const unknown = 'did:web:unregistered.test.buckyos.io'
  const agent = 'did:web:jarvis.test.buckyos.io'
  const userContact: Contact = { ...contacts[1], did: lucy, name: 'Lucy' }
  const projected = projectOwner({ ownerDid: owner, summaries: [summary('unknown-session', { box_kind: 'SENT', from: owner, to: unknown, msg: chat(owner, [unknown], 'hi', 1) })], contacts: [...contacts, userContact], groups: [], agentDids: [agent], personalTitles: {}, policies: {}, labels })
  const entity = projected.entities.find(item => item.id === lucy)!
  equal(entity.type, 'person')
  equal(entity.domain, 'managed')
  equal(projected.entities.find(item => item.id === agent)?.type, 'agent')
  equal(projected.entities.find(item => item.id === unknown)?.type, 'person')
  const own = projectOwner({ ownerDid: lucy, summaries: [], contacts: [userContact], groups: [], agentDids: [], personalTitles: {}, policies: {}, labels })
  equal(own.entities.some(item => item.id === lucy), false)
})
