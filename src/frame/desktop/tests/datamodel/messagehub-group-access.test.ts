/// <reference types="vite/client" />
import { buckyos } from 'buckyos'
import { MessageHubApiStore } from '../../src/app/messagehub/api/store.ts'
import type { GroupMemberRecord, SessionSummary } from '../../src/app/messagehub/datamodel/sessionApi.ts'

const me = 'did:bns:me'
const alice = 'did:bns:alice'
const bob = 'did:bns:bob'
const group = 'did:bns:team'

function equal(actual: unknown, expected: unknown) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw Error(`Expected ${JSON.stringify(expected)}, received ${JSON.stringify(actual)}`)
}

interface Backend {
  members: GroupMemberRecord[]
  summary: SessionSummary
  calls: Record<string, number>
  /** Holds `group.check_access` answers until released. */
  hold: boolean
  release: () => void
  notify: () => void
}

const member = (did: string, role: GroupMemberRecord['role']): GroupMemberRecord => ({ member_did: did, role, state: 'active', entity_kind: 'user' })

function summaryAt(at: number, msg: Record<string, unknown>): SessionSummary {
  return { session_id: group, unread_count: 0, updated_at_ms: at, last_activity_ms: at, request_count: 0, lifecycle: 'active', state: { owner: me, session_id: group, lifecycle: 'active', registered: true, origin: 'group', peer_did: group, created_at_ms: 1, updated_at_ms: at }, last_record: { record: { record_id: `r${at}`, mailbox: `${me}/${group}`, owner: me, box_kind: 'INBOX', msg_id: `m${at}`, state: 'READ', from: alice, to: me, session_id: group, sort_key: at, created_at_ms: at, updated_at_ms: at }, msg: { from: alice, to: [group], created_at_ms: at, ...msg } as never } }
}

const chat = { kind: 'group_msg', content: { content: 'hello' } }
const joined = { kind: 'event', content: { content: 'entity.member_joined', machine: { intent: 'buckyos.action_log', data: { action: 'entity.member_joined', subject_did: bob } } } }

async function fixture(hosted: boolean, run: (store: MessageHubApiStore, backend: Backend) => Promise<void>) {
  const owner = hosted ? me : alice
  const doc = { id: group, owner, controller: owner, host: 'did:web:host.test', lifecycle: 'active', revision: '1', profile: { name: 'Team' } }
  let waiting: Array<() => void> = []
  let subscribed = false
  const backend: Backend = {
    members: [member(owner, 'owner'), member(hosted ? alice : me, 'member')],
    summary: summaryAt(10, chat),
    calls: {},
    hold: false,
    release: () => { const pending = waiting; waiting = []; pending.forEach(resolve => resolve()) },
    notify: () => {},
  }
  const saved = { getCurrentIdentity: buckyos.getCurrentIdentity, getAccountInfo: buckyos.getAccountInfo, getServiceRpcClient: buckyos.getServiceRpcClient, subscribeKEvent: buckyos.subscribeKEvent }
  buckyos.getCurrentIdentity = async () => ({ kind: 'anonymous' })
  buckyos.getAccountInfo = async () => ({ user_id: 'me', user_name: 'Me', session_token: 'test-only' })
  buckyos.subscribeKEvent = (async (_patterns: unknown, callback: () => void) => { backend.notify = callback; subscribed = true; return { close: async () => {} } }) as unknown as typeof buckyos.subscribeKEvent
  Object.assign(globalThis, { window: globalThis })
  buckyos.getServiceRpcClient = (() => ({
    call: async (method: string, input: Record<string, unknown>) => {
      backend.calls[method] = (backend.calls[method] ?? 0) + 1
      if (method === 'user.get') return { local_profile: { did: me } }
      if (method === 'agent.list') return { agents: [] }
      if (method === 'contact.list_contacts' || method === 'ui_session.list_state') return []
      if (method === 'group.list_by_member') return hosted ? { items: [{ doc }] } : { joined: [{ group_did: group, doc_cache: { doc } }] }
      if (method === 'msg.list_sessions') return { items: [backend.summary] }
      if (method === 'msg.list_session') return { items: [] }
      if (method === 'group.check_access') {
        if (!hosted) throw Error('a joined group is checked by its host on submit')
        if (backend.hold && input.action === 'session.post') await new Promise<void>(resolve => waiting.push(resolve))
        return { allowed: true, reason: null }
      }
      if (method === 'group.get_doc') return { doc }
      if (method === 'group.get_config') return { revision: '1', default_session: {} }
      if (method === 'group.list_sessions') return { items: [] }
      if (method === 'group.list_members') return { items: backend.members }
      if (method === 'group.list_session_members') return { items: backend.members.map(item => ({ member_did: item.member_did, kind: 'group_member', role: item.role, state: 'included' })), complete: true }
      throw Error(`Unexpected RPC ${method}`)
    },
  })) as unknown as typeof buckyos.getServiceRpcClient
  const store = new MessageHubApiStore()
  const stop = { sync: () => {} }
  try {
    await store.initialize()
    stop.sync = store.startSync(store.defaultContext(), null)
    await settle(() => subscribed)
    await run(store, backend)
  } finally {
    stop.sync()
    Object.assign(buckyos, saved)
    delete (globalThis as { window?: unknown }).window
  }
}

async function settle(done: () => boolean) {
  for (let attempt = 0; attempt < 200 && !done(); attempt++) await new Promise(resolve => setTimeout(resolve, 0))
  if (!done()) throw Error('Timed out')
}

function groupSession(store: MessageHubApiStore) {
  const session = store.sessions(store.defaultContext()).find(item => item.id === group)
  if (!session) throw Error('group session missing')
  return session
}

const participants = (store: MessageHubApiStore) => (store.groupSessionMembers(store.defaultContext(), group, group)?.items ?? []).map(item => item.did).sort()

Deno.test('a group message rechecks the post permission without taking the composer away', async () => {
  await fixture(true, async (store, backend) => {
    const context = store.defaultContext()
    equal(store.access(context, groupSession(store), false).readOnlyReason, 'permission_pending')
    await settle(() => store.access(context, groupSession(store), false).mode === 'read_write')
    backend.hold = true
    backend.summary = summaryAt(20, chat)
    const internal = (store as unknown as { owner(did: string): { postAccess: Map<string, { stale?: boolean }>; postChecks: Set<string> } }).owner(me)
    backend.notify()
    await settle(() => internal.postAccess.get(group)?.stale === true)
    equal(store.access(context, groupSession(store), false).mode, 'read_write')
    equal(backend.calls['group.check_access'], 2)
    equal(store.access(context, groupSession(store), false).mode, 'read_write')
    equal(backend.calls['group.check_access'], 2)
    backend.release()
    await settle(() => !internal.postChecks.size)
    equal(internal.postAccess.get(group)?.stale, undefined)
    equal(store.access(context, groupSession(store), false).mode, 'read_write')
    equal(backend.calls['group.check_access'], 2)
  })
})

Deno.test('a membership event reloads the members and participants offered for @; a chat message does not', async () => {
  await fixture(true, async (store, backend) => {
    const context = store.defaultContext()
    await store.ensureGroup(context, group)
    await store.ensureGroupSessionMembers(context, group, group)
    equal(participants(store), [alice, me].sort())
    const loads = backend.calls['group.list_members']
    backend.members = [...backend.members, member(bob, 'member')]
    backend.summary = summaryAt(20, chat)
    backend.notify()
    await settle(() => backend.calls['msg.list_sessions'] >= 2)
    await new Promise(resolve => setTimeout(resolve, 0))
    equal(backend.calls['group.list_members'], loads)
    backend.summary = summaryAt(30, joined)
    backend.notify()
    await settle(() => participants(store).includes(bob))
    equal(store.group(context, group)?.members?.map(item => item.did).sort(), [alice, bob, me].sort())
  })
})

Deno.test('a group hosted on another zone lists its members and participants through this zone', async () => {
  await fixture(false, async (store, backend) => {
    const context = store.defaultContext()
    await store.ensureGroup(context, group)
    const info = store.group(context, group)
    equal(info?.hosted, false)
    equal(info?.members?.map(item => item.did).sort(), [alice, me].sort())
    await store.ensureGroupSessionMembers(context, group, group)
    equal(participants(store), [alice, me].sort())
    equal(store.access(context, groupSession(store), false).mode, 'read_write')
    backend.members = [...backend.members, member(bob, 'member')]
    backend.summary = summaryAt(30, joined)
    backend.notify()
    await settle(() => participants(store).includes(bob))
    equal(backend.calls['group.check_access'], undefined)
  })
})
