import { creationReason, defaultPreferences, isActionMessage, isMessageActivity, relativeActivity, selectDefaultSession, sessionAccess, sessionKey, sessionTitle, sortSessions, viewerSessionKey } from '../../src/app/messagehub/sessionModel.ts'
import { ensureDefaultSession } from '../../src/app/messagehub/store/defaultSession.ts'
import type { Entity, MessageHubContext, Session } from '../../src/app/messagehub/types.ts'
import { isValidMsgSessionId, MSG_SESSION_ID_MAX_CHARS, randomMsgNonce, type MessageObject } from '../../src/app/messagehub/protocol/msgobj.ts'

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
  equal(isMessageActivity({ ...message, kind: 'deliver', content: {} }), false)
  equal(isMessageActivity({ ...message, kind: 'deliver', content: { refs: [{ role: 'output', target: { type: 'data_obj', obj_id: 'cyfile:photo' } }] } }), true)
  equal(isMessageActivity({ ...message, kind: 'deliver', content: { refs: [{ role: 'input', target: { type: 'data_obj', obj_id: 'cyfile:photo' } }] } }), false)
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
  for (const type of ['person', 'agent', 'group', 'service'] as const) {
    equal(creationReason(context, { ...entity, type }, 'default', session.binding), undefined)
    equal(creationReason(context, { ...entity, type }, 'deny', session.binding), 'creation_disabled')
    equal(creationReason(observe, { ...entity, type }, 'allow', session.binding), 'agent_observer')
  }
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

Deno.test('entity defaults follow recent activity, ignore pinning and keep agent defaults native', () => {
  const older = { ...session, id: 'pinned', lastActiveAt: now - 1000 }
  const newer = { ...session, id: 'recent' }
  const tunnel: Session = { ...session, id: 'tunnel', lastActiveAt: now + 1000, binding: { kind: 'tunnel', tunnelInstanceId: 'tg', endpointDid: 'did:msgtunnel:alice.user.tg', connectionName: 'Telegram', connected: true, canSend: true, supportsMultipleSessions: false, canCreateRemoteSession: false } }
  const ordered = sortSessions([newer, older, tunnel], id => ({ ...defaultPreferences, pinned: id === 'pinned' }))
  equal(ordered[0].id, 'pinned')
  for (const type of ['person', 'agent', 'group', 'service'] as const) {
    equal(selectDefaultSession({ ...entity, type }, ordered)?.id, type === 'agent' ? 'recent' : 'tunnel')
    equal(selectDefaultSession({ ...entity, type }, [{ ...newer, lifecycle: 'archived' }, older])?.id, 'pinned')
    equal(selectDefaultSession({ ...entity, type }, [tunnel])?.id ?? null, type === 'agent' ? null : 'tunnel')
  }
  equal(selectDefaultSession(entity, [{ ...session, entityId: 'another' }, { ...session, binding: { kind: 'unknown' } }]), null)
})

function defaultSessionFixture() {
  const state = { sessions: [] as Session[], pages: [] as Session[][], creates: 0, failNext: false }
  const store = {
    ensureOwner: () => Promise.resolve(),
    canView: () => true,
    ownerStatus: () => ({ phase: 'ready' as const }),
    defaultSession: (ctx: MessageHubContext) => selectDefaultSession(entity, state.sessions.filter(item => item.ownerDid === ctx.ownerDid)),
    hasMoreEntities: () => state.pages.length > 0,
    loadMoreEntities: async () => { state.sessions.push(...state.pages.shift()!) },
    findEntity: () => entity,
    connections: () => [{ id: 'native', label: 'BuckyOS', binding: session.binding }],
    policy: () => 'default' as const,
    create: async (ctx: MessageHubContext) => {
      state.creates++
      await Promise.resolve()
      if (state.failNext) { state.failNext = false; throw new Error('unavailable') }
      const created = { ...session, ownerDid: ctx.ownerDid, id: `created-${state.creates}` }
      state.sessions.push(created)
      return created
    },
  }
  return { state, store }
}

Deno.test('default lookup searches later pages before creating and leaves archived sessions archived', async () => {
  const { state, store } = defaultSessionFixture()
  state.sessions.push({ ...session, lifecycle: 'archived' })
  state.pages.push([{ ...session, id: 'later-page' }])
  equal((await ensureDefaultSession(store, context, entity.id))?.id, 'later-page')
  equal(state.creates, 0)
  state.sessions.pop()
  equal((await ensureDefaultSession(store, context, entity.id))?.id, 'created-1')
  equal(state.sessions[0].lifecycle, 'archived')
})

Deno.test('concurrent default creation is deduplicated, failed requests can retry and owners stay isolated', async () => {
  const { state, store } = defaultSessionFixture()
  state.failNext = true
  const failures = await Promise.allSettled([ensureDefaultSession(store, context, entity.id), ensureDefaultSession(store, context, entity.id)])
  equal(failures.map(result => result.status), ['rejected', 'rejected'])
  equal(state.creates, 1)
  const sessions = await Promise.all([ensureDefaultSession(store, context, entity.id), ensureDefaultSession(store, context, entity.id)])
  equal(sessions.map(item => item?.id), ['created-2', 'created-2'])
  equal(state.creates, 2)
  const other: MessageHubContext = { viewerDid: 'did:user:other', ownerDid: 'did:user:other', mode: 'self' }
  equal((await ensureDefaultSession(store, other, entity.id))?.ownerDid, other.ownerDid)
  equal(state.creates, 3)
  equal(await ensureDefaultSession(store, { ...context, ownerDid: 'did:agent:observed', mode: 'observe' }, entity.id), null)
  equal(state.creates, 3)
})

Deno.test('to_session values follow the MsgObject v2 rule and nonces stay safe integers', () => {
  for (const ok of ['release', 'dm:did:bns:alice', '3f2b6c1e-9a4d-4b7e-8c21-0d5e6f7a8b9c', '会'.repeat(MSG_SESSION_ID_MAX_CHARS)]) equal([ok.slice(0, 16), isValidMsgSessionId(ok)], [ok.slice(0, 16), true])
  for (const bad of ['', '.', '..', ' release', 'release ', 'a\u0000b', 'a\u0085b', '会'.repeat(MSG_SESSION_ID_MAX_CHARS + 1)]) equal([bad.slice(0, 16), isValidMsgSessionId(bad)], [bad.slice(0, 16), false])
  const nonces = Array.from({ length: 64 }, () => randomMsgNonce())
  equal(nonces.every(nonce => Number.isSafeInteger(nonce) && nonce >= 0), true)
  equal(new Set(nonces).size > 1, true)
})
