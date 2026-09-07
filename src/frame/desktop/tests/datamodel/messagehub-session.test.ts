import { creationReason, defaultPreferences, isActionMessage, isMessageActivity, relativeActivity, sessionAccess, sessionKey, sessionTitle, sortSessions, viewerSessionKey } from '../../src/app/messagehub/sessionModel.ts'
import type { Entity, MessageHubContext, Session } from '../../src/app/messagehub/types.ts'
import type { MessageObject } from '../../src/app/messagehub/protocol/msgobj.ts'

const context: MessageHubContext = { viewerDid: 'did:user:me', ownerDid: 'did:user:me', mode: 'self' }
const now = 1_780_000_000_000
const entity: Entity = { id: 'did:agent:a', type: 'agent', name: 'Assistant', tags: [], unreadCount: 0, lastActiveAt: now }
const session: Session = { id: 'same-id', ownerDid: context.ownerDid, entityId: entity.id, type: 'chat', title: 'Derived', binding: { kind: 'native', targetDid: entity.id }, origin: 'manual', lifecycle: 'active', lastActiveAt: now, createdAt: now, unreadCount: 0, shared: { title: 'Shared', description: '', updatedAt: now }, members: {} }
const message: MessageObject = { from: entity.id, to: [context.ownerDid], kind: 'chat', created_at_ms: now, content: { content: 'Hello' } }
function equal(actual: unknown, expected: unknown) { if (JSON.stringify(actual) !== JSON.stringify(expected)) throw Error(`Expected ${JSON.stringify(expected)}, received ${JSON.stringify(actual)}`) }

Deno.test('message activity excludes state logs, notify, delivery-only events and status rows', () => {
  equal(isMessageActivity(message), true)
  equal(isMessageActivity({ ...message, kind: 'group_msg' }), true)
  equal(isMessageActivity({ ...message, kind: 'deliver' }), true)
  equal(isMessageActivity({ ...message, kind: 'deliver', content: { content: '' } }), false)
  equal(isMessageActivity({ ...message, kind: 'notify' }), false)
  equal(isMessageActivity({ ...message, ui_item_kind: 'status' }), false)
  equal(isMessageActivity({ ...message, kind: 'event', content: { content: 'Result', machine: { intent: 'buckyos.action_log', data: { schema_version: 99 } } } }), false)
  equal(isActionMessage({ ...message, kind: 'event' }), false)
  equal(isActionMessage({ ...message, content: { content: 'Joined group', machine: { intent: 'buckyos.action_log' } } }), false)
})
Deno.test('relative time uses floor, clamps future times and handles unknown timestamps', () => {
  for (const [age, expected] of [[0, 'now'], [59999, 'now'], [120000, '2m'], [4 * 3600000, '4h'], [3 * 86400000, '3d'], [-5000, 'now']] as const) equal(relativeActivity(now - age, now, 'now'), expected)
  equal(relativeActivity(undefined, now, 'now'), '—')
  equal(relativeActivity(NaN, now, 'now'), '—')
})
Deno.test('sorting is pinned first, then activity, then stable IDs; metadata does not matter', () => {
  const sessions = [{ ...session, id: 'b' }, { ...session, id: 'a', shared: { ...session.shared, updatedAt: now + 10000 } }, { ...session, id: 'old', lastActiveAt: now - 1000 }]
  equal(sortSessions(sessions, () => defaultPreferences).map(item => item.id), ['a', 'b', 'old'])
  equal(sortSessions(sessions, id => ({ ...defaultPreferences, pinned: id === 'old' })).map(item => item.id), ['old', 'a', 'b'])
  equal(sessionTitle(session, { ...defaultPreferences, title: 'Personal' }), 'Personal')
  equal(sessionTitle(session, defaultPreferences), 'Shared')
  equal(sessionTitle({ ...session, shared: { ...session.shared, title: '' } }, defaultPreferences), 'Derived')
})
Deno.test('owner scope, observer capabilities and tunnel creation limits cannot be bypassed', () => {
  const observe = { ...context, ownerDid: entity.id, mode: 'observe' as const }
  equal(sessionKey(context.ownerDid, session.id) === sessionKey(observe.ownerDid, session.id), false)
  equal(viewerSessionKey(context, session.id) === viewerSessionKey({ ...context, viewerDid: entity.id }, session.id), false)
  equal(sessionAccess(observe, { ...session, ownerDid: entity.id }, true).canManage, false)
  equal(sessionAccess(observe, { ...session, ownerDid: entity.id }, true).mode, 'read_only')
  equal(creationReason(context, entity, 'default', session.binding), undefined)
  equal(creationReason(context, { ...entity, type: 'person' }, 'default', session.binding), 'creation_disabled')
  const tunnel = { kind: 'tunnel' as const, tunnelInstanceId: 'work', endpointDid: 'did:telegram:alice', connectionName: 'Telegram · Work', supportsMultipleSessions: false, canCreateRemoteSession: true, canSend: true, connected: true }
  equal(creationReason(context, { ...entity, type: 'person' }, 'allow', tunnel), 'platform_read_only')
  equal(creationReason(context, { ...entity, type: 'person' }, 'allow', { ...tunnel, supportsMultipleSessions: true }), undefined)
  equal(sessionAccess(context, { ...session, binding: tunnel }, false).canManage, true)
  equal(sessionAccess(context, { ...session, binding: tunnel }, false).mode, 'read_only')
  equal(sessionAccess(context, { ...session, binding: tunnel }, true).mode, 'read_write')
  equal(sessionAccess(context, { ...session, binding: tunnel }, true).canEditSharedState, false)
  equal(sessionAccess(context, { ...session, binding: { ...tunnel, connected: false } }, true).mode, 'read_only')
})

Deno.test('Action filtering preserves raw indices and rebuilds dates, including incremental appends', async () => {
  const { InMemoryConversationMessageReader, buildConversationProjection, extendConversationProjection, materializeConversationWindow } = await import('../../src/app/messagehub/conversation/history/data-source.ts')
  const action = { ...message, kind: 'event' as const, created_at_ms: now + 86400000, ui_message_id: 'action', content: { content: 'Unknown version', machine: { intent: 'buckyos.action_log', data: { schema_version: 99 } } } }
  const ordinaryEvent = { ...message, kind: 'event' as const, ui_message_id: 'ordinary-event' }
  const reader = InMemoryConversationMessageReader.fromMessages([{ ...message, ui_message_id: 'chat' }, action, ordinaryEvent])
  const filtered = await buildConversationProjection(reader, [], false)
  equal(filtered.messageCount, 3)
  equal(filtered.entries.filter(entry => entry.kind === 'message').map(entry => entry.messageIndex), [0, 2])
  equal(filtered.entries.filter(entry => entry.kind === 'timestamp').length, 1)
  const window = await materializeConversationWindow(filtered, reader, 0, filtered.totalCount)
  equal(window.items.filter(item => item.kind === 'message').map(item => item.messageIndex), [0, 2])
  const extended = extendConversationProjection(filtered, [{ ...action, ui_message_id: 'second-action' }])
  equal(extended.totalCount, filtered.totalCount)
  equal(extended.messageCount, 4)
  const onlyAction = await buildConversationProjection(InMemoryConversationMessageReader.fromMessages([action]), [], false)
  equal(onlyAction.totalCount, 0)
  equal((await buildConversationProjection(reader)).messageCount, 3)
  equal((await buildConversationProjection(reader)).entries.filter(entry => entry.kind === 'message').length, 3)
})
