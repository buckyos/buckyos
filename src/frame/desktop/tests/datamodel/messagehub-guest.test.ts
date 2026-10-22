import { buckyos, RPCError, type CurrentIdentity } from 'buckyos'
import { objectUrl, ObjectRouteError } from '../../src/app/messagehub/api/objects.ts'
import { objectContextOf } from '../../src/app/messagehub/conversation/media/source.ts'
import { foldMessageRelations } from '../../src/app/messagehub/conversation/history/relations.ts'
import { GuestHub } from '../../src/app/messagehub/guest/hub.ts'
import type { GuestApi, GuestMessagePage } from '../../src/app/messagehub/guest/api.ts'
import { decodeNamedObject, guestEntryAllows, guestEntryPath, guestPagePath, guestSessions, guestTimeline, guestTimelineMessage, isCustomerUserId, isGroupDid, suggestCustomerUserId } from '../../src/app/messagehub/guest/model.ts'
import type { MessageObject, MsgObject } from '../../src/app/messagehub/protocol/msgobj.ts'

function equal(actual: unknown, expected: unknown) { if (JSON.stringify(actual) !== JSON.stringify(expected)) throw Error(`Expected ${JSON.stringify(expected)}, received ${JSON.stringify(actual)}`) }

const GROUP = 'did:bns:support.test'
const ADDRESS = `${GROUP}/t-123`
const ME = 'did:bns:acme-c1', STAFF = 'did:bns:bob'
const denial = (method: string) => new RPCError('access denied: resource_grant_missing', 'access_denied', { reason: 'resource_grant_missing', adjust: { party: 'sharer', location: 'share_panel' }, service: 'msg-center', method })
const message = (from: string, content: string, extra: Partial<MsgObject> = {}): MsgObject => ({ from, to: [GROUP], kind: 'group_msg', to_session: 't-123', created_at_ms: 1, content: { format: 'text/plain', content }, ...extra })

Deno.test('attachments of a hosted group session are read with the context_path of the message that carries them', () => {
  const getZoneServiceURL = buckyos.getZoneServiceURL
  buckyos.getZoneServiceURL = () => 'https://test.buckyos.io/kapi/msg-center/'
  try {
    equal(objectUrl('cyfile:abc', true, `${GROUP}/t%20123/cymsg:1`), 'https://test.buckyos.io/kapi/msg-center/objects/cyfile%3Aabc/content?context_path=did%3Abns%3Asupport.test%2Ft%2520123%2Fcymsg%3A1')
    equal(objectUrl('cyfile:abc'), 'https://test.buckyos.io/kapi/msg-center/objects/cyfile%3Aabc')
  } finally {
    buckyos.getZoneServiceURL = getZoneServiceURL
  }
  const ref = { role: 'input' as const, target: { type: 'data_obj' as const, obj_id: 'cyfile:new' } }
  const original: MessageObject = { ...message(STAFF, 'v1'), ui_message_id: 'cymsg:orig', ui_group_session: ADDRESS }
  equal(objectContextOf(original), `${ADDRESS}/cymsg:orig`)
  // Once edited, the refs shown are the latest edit's, so its ObjId is the context.
  const edit1: MessageObject = { ...message(STAFF, 'v2', { relates_to: { rel: 'edit', target: 'cymsg:orig' }, created_at_ms: 2 }), ui_message_id: 'cymsg:e1' }
  const edit2: MessageObject = { ...message(STAFF, 'v3', { relates_to: { rel: 'edit', target: 'cymsg:orig' }, created_at_ms: 3 }), content: { format: 'text/plain', content: 'v3', refs: [ref] }, ui_message_id: 'cymsg:e2' }
  const [folded] = foldMessageRelations([original, edit2, edit1])
  equal(objectContextOf(folded), `${ADDRESS}/cymsg:e2`)
  // A message of anything but a hosted group session is read without one.
  equal(objectContextOf({ ...message(STAFF, 'x'), ui_message_id: 'cymsg:dm' }), undefined)
})

Deno.test('the guest page model: paths, sessions, named objects and the timeline', () => {
  equal(guestPagePath(GROUP, 't 1'), '/messagehub/guest?group=did%3Abns%3Asupport.test&session=t+1')
  equal(guestEntryPath(GROUP), '/sso/entry?service=msg-center&entry=did%3Abns%3Asupport.test')
  equal([isGroupDid(GROUP), isGroupDid('support'), isGroupDid('did:bns:a/b'), isGroupDid(null)], [true, false, false, false])
  equal(guestSessions(GROUP, [
    { session_id: null, session: GROUP, lifecycle: 'active', revision: 'r0', has_guests: false },
    { session_id: 't 1', session: `${GROUP}/t%201`, shared_state: { title: 'Ticket' }, lifecycle: 'active', revision: 'r1', has_guests: true },
  ]), [{ sessionId: 't 1', address: `${GROUP}/t%201`, title: 'Ticket', lifecycle: 'active' }])

  const body = message(STAFF, 'hi', { agent_task: { task_id: 'task-1' } })
  const payload = btoa(JSON.stringify(body)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
  equal(decodeNamedObject('application/cyfs-named-object+jwt', `eyJhbGciOiJFZERTQSJ9.${payload}.sig`), body)
  equal(decodeNamedObject('application/cyfs-named-object+json', JSON.stringify(body)), body)

  const stored = guestTimelineMessage(body, { seq: 2, objId: 'cymsg:2' }, ADDRESS)
  equal([stored.agent_task, stored.ui_message_id, stored.ui_group_session, stored.ui_seq], [undefined, 'cymsg:2', ADDRESS, 2])
  const first = guestTimelineMessage(message(ME, 'first'), { seq: 1, objId: 'cymsg:1' }, ADDRESS)
  const sent: MessageObject = { ...message(ME, 'mine', { created_at_ms: 5 }), ui_message_id: 'local:k1', ui_sent_msg_id: 'cymsg:2' }
  const waiting: MessageObject = { ...message(ME, 'later', { created_at_ms: 9 }), ui_message_id: 'local:k2' }
  const timeline = guestTimeline([stored, first], [waiting, sent], did => did === ME ? 'You' : undefined)
  equal(timeline.map(item => [item.ui_message_id, item.ui_sender_name ?? null]), [['cymsg:1', 'You'], ['cymsg:2', null], ['local:k2', 'You']])

  const methods = { guest: false, password: true, self_register: false, did_login: false }
  equal([guestEntryAllows(methods, 'exclusive_invite'), guestEntryAllows(methods, 'password'), guestEntryAllows(methods, 'did_federation'), guestEntryAllows({ ...methods, did_login: true }, 'did_federation'), guestEntryAllows(null, 'password')], [true, true, false, true, false])
  equal([suggestCustomerUserId('Dana Rivera'), suggestCustomerUserId('  Zoë_O\'Neil! '), suggestCustomerUserId('王小明')], ['dana-rivera', 'zoe-o-neil', ''])
  equal([isCustomerUserId('dana.r_1'), isCustomerUserId('-dana'), isCustomerUserId('Dana'), isCustomerUserId('')], [true, false, false, false])
})

interface FakeHost {
  state: 'invited' | 'included' | 'removed' | 'none'
  stored: Array<{ seq: number; objId: string; msg: MsgObject }>
  accepts: string[]
  sends: Array<{ key: string; message: MsgObject }>
  failFirstAcceptAnswer?: boolean
  objectErrors?: Record<string, unknown>
}

function fakeApi(host: FakeHost, identity: CurrentIdentity): GuestApi {
  // Like msg-center: a restricted caller who is not in the Session gets a structured denial.
  const requireIncluded = (method: string) => {
    if (host.state !== 'included') throw denial(method)
  }
  return {
    identity: () => Promise.resolve(identity),
    groupName: () => Promise.resolve('Acme Support'),
    entry: () => Promise.resolve(null),
    sessions: async () => { requireIncluded('group.list_sessions'); return [{ sessionId: 't-123', address: ADDRESS, title: 'Order', lifecycle: 'active' }] },
    invited: async () => host.state === 'invited',
    members: async () => { requireIncluded('group.list_session_members'); return [{ member_did: STAFF, kind: 'group_member', role: 'admin', entity_kind: 'user', state: 'included' }] },
    messages: async (_group, _session, afterSeq, limit): Promise<GuestMessagePage> => {
      requireIncluded('group.list_messages')
      const after = host.stored.filter(item => item.seq > afterSeq).slice(0, limit)
      return { items: after.map(item => ({ seq: item.seq, objId: item.objId })), nextAfterSeq: after.at(-1)?.seq ?? afterSeq, limited: false }
    },
    message: async (address, objId) => {
      if (address !== ADDRESS) throw new ObjectRouteError(404, 'not-found')
      const error = host.objectErrors?.[objId]
      if (error) throw error
      return structuredClone(host.stored.find(item => item.objId === objId)!.msg)
    },
    accept: async (_group, _session, key) => {
      host.accepts.push(key)
      if (host.state !== 'invited') throw new RPCError('not-found')
      host.state = 'included'
      if (host.failFirstAcceptAnswer) { host.failFirstAcceptAnswer = false; throw new RPCError('RPC call failed: timeout') }
    },
    request: () => Promise.reject(new RPCError('guest-entry-disabled')),
    send: async (msg, key) => {
      requireIncluded('msg.post_send')
      host.sends.push({ key, message: msg })
      const objId = `cymsg:sent-${host.sends.length}`
      host.stored.push({ seq: host.stored.length + 1, objId, msg })
      return { ok: true, msgId: objId }
    },
    markRead: () => Promise.resolve(),
    objects: { describe: () => Promise.reject(new Error('unused')), contentUrl: () => Promise.reject(new Error('unused')) },
    signOut: () => Promise.resolve(),
  }
}

const limited: CurrentIdentity = { kind: 'account', user_id: 'acme-c1', did: ME, user_type: 'limited', show_name: 'Alice', zone_host: 'test.buckyos.io', auth_method: 'exclusive_invite', restricted: true, scopes: ['account_self', 'msg_group', 'aiworkspace', 'homestation'], password_set: false }

async function settle(hub: GuestHub, done: () => boolean) {
  for (let i = 0; i < 100 && !done(); i++) await new Promise(resolve => setTimeout(resolve, 5))
  if (!done()) throw Error(`hub did not settle: ${JSON.stringify(hub.getSnapshot().step)}`)
}

Deno.test('an invited guest accepts once; a retry after a lost answer finds the Session instead of failing', async () => {
  const host: FakeHost = { state: 'invited', stored: [{ seq: 1, objId: 'cymsg:1', msg: message(STAFF, 'hello') }], accepts: [], sends: [], failFirstAcceptAnswer: true }
  const hub = new GuestHub(fakeApi(host, limited), GROUP, 't-123', 'You')
  try {
    await hub.start()
    equal(hub.getSnapshot().step.kind, 'invited')
    equal(hub.getSnapshot().reader.totalCount, 0)
    await hub.accept()
    await settle(hub, () => hub.getSnapshot().historyLoaded)
    equal(hub.getSnapshot().step.kind, 'session')
    equal((await hub.getSnapshot().reader.readRange(0, 10)).map(item => [item.content.content, item.ui_sender_name]), [['hello', 'bob']])
    equal(host.accepts.length, 1)
  } finally {
    hub.stop()
  }
})

Deno.test('someone not invited is refused like a missing Session, without content', async () => {
  const host: FakeHost = { state: 'none', stored: [{ seq: 1, objId: 'cymsg:1', msg: message(STAFF, 'secret') }], accepts: [], sends: [] }
  const hub = new GuestHub(fakeApi(host, limited), GROUP, 't-123', 'You')
  try {
    await hub.start()
    const step = hub.getSnapshot().step
    equal([step.kind, step.kind === 'denied' && step.denial?.reason], ['denied', 'resource_grant_missing'])
    equal(hub.getSnapshot().reader.totalCount, 0)
    equal(host.accepts.length, 0)
  } finally {
    hub.stop()
  }
})

Deno.test('a removed guest is refused with the denial and the Session content leaves the page', async () => {
  const host: FakeHost = { state: 'included', stored: [{ seq: 1, objId: 'cymsg:1', msg: message(STAFF, 'hello') }], accepts: [], sends: [] }
  const hub = new GuestHub(fakeApi(host, limited), GROUP, 't-123', 'You')
  try {
    await hub.start()
    equal(hub.getSnapshot().reader.totalCount, 1)
    host.state = 'removed'
    await hub.sync()
    const step = hub.getSnapshot().step
    equal(step.kind === 'denied' ? step.denial?.reason : null, 'resource_grant_missing')
    equal(hub.getSnapshot().reader.totalCount, 0)
  } finally {
    hub.stop()
  }
})

Deno.test('a guest posts group messages from its own DID; the same text retried reuses its idempotency key', async () => {
  const host: FakeHost = { state: 'included', stored: [], accepts: [], sends: [] }
  const api = fakeApi(host, limited)
  const send = api.send
  const keys: string[] = []
  let failures = 1
  api.send = (msg, key) => { keys.push(key); return failures-- > 0 ? Promise.reject(new RPCError('RPC call failed: timeout')) : send(msg, key) }
  const hub = new GuestHub(api, GROUP, 't-123', 'You')
  try {
    await hub.start()
    let failure = ''
    await hub.send({ content: ' Hi there ', attachments: [] }).catch(error => { failure = error.message })
    equal(failure.startsWith('result_unknown'), true)
    await hub.send({ content: 'Hi there', attachments: [] })
    equal([host.sends.length, keys.length, keys[0] === keys[1]], [1, 2, true])
    equal(host.sends[0].message, { ...host.sends[0].message, from: ME, to: [GROUP], kind: 'group_msg', to_session: 't-123', content: { format: 'text/plain', content: 'Hi there' } })
    const rows = await hub.getSnapshot().reader.readRange(0, 10)
    equal(rows.map(item => [item.ui_message_id, item.ui_sender_name]), [['cymsg:sent-1', 'You']])
    let rejected = ''
    await hub.send({ content: 'x', attachments: [{ file: new File(['a'], 'a.txt') }] }).catch(error => { rejected = error.message })
    equal(rejected, 'rejected: attachments-not-supported')
  } finally {
    hub.stop()
  }
})

Deno.test('an unreadable message object holds the timeline at it; one the host refuses is skipped', async () => {
  const host: FakeHost = {
    state: 'included',
    stored: [{ seq: 1, objId: 'cymsg:1', msg: message(STAFF, 'one') }, { seq: 2, objId: 'cymsg:2', msg: message(STAFF, 'gone') }, { seq: 3, objId: 'cymsg:3', msg: message(STAFF, 'three') }],
    accepts: [], sends: [],
    objectErrors: { 'cymsg:2': new ObjectRouteError(404, 'not-found'), 'cymsg:3': new TypeError('Failed to fetch') },
  }
  const hub = new GuestHub(fakeApi(host, limited), GROUP, 't-123', 'You')
  try {
    await hub.start()
    equal((await hub.getSnapshot().reader.readRange(0, 10)).map(item => item.content.content), ['one'])
    equal(hub.getSnapshot().offline, true)
    delete host.objectErrors!['cymsg:3']
    await hub.sync()
    equal(hub.getSnapshot().offline, false)
    equal((await hub.getSnapshot().reader.readRange(0, 10)).map(item => item.content.content), ['one', 'three'])
  } finally {
    hub.stop()
  }
})

Deno.test('another subject signing in behind the page drops what was read and starts the page over', async () => {
  const host: FakeHost = { state: 'included', stored: [{ seq: 1, objId: 'cymsg:1', msg: message(STAFF, 'hello') }], accepts: [], sends: [] }
  const api = fakeApi(host, limited)
  const hub = new GuestHub(api, GROUP, 't-123', 'You')
  try {
    await hub.start()
    equal(hub.getSnapshot().reader.totalCount, 1)
    api.identity = () => Promise.resolve({ ...limited, user_id: 'acme-c2', did: 'did:bns:acme-c2' })
    await hub.sync()
    equal([hub.getSnapshot().step.kind, hub.getSnapshot().reader.totalCount, hub.getSnapshot().members.length], ['identity_changed', 0, 0])
  } finally {
    hub.stop()
  }
})

Deno.test('without a browser session the hub stops at anonymous and reads nothing', async () => {
  const host: FakeHost = { state: 'included', stored: [], accepts: [], sends: [] }
  const api = fakeApi(host, { kind: 'anonymous' })
  let reads = 0
  api.sessions = () => { reads++; return Promise.resolve([]) }
  const hub = new GuestHub(api, GROUP, 't-123', 'You')
  await hub.start()
  hub.stop()
  equal([hub.getSnapshot().step.kind, reads], ['anonymous', 0])
})
