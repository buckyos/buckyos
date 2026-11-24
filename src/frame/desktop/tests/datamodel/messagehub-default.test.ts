/// <reference types="vite/client" />
import { buckyos } from 'buckyos'
import { MessageHubApiStore } from '../../src/app/messagehub/api/store.ts'
import type { DefaultSession, PeerConversations, SessionSummary } from '../../src/app/messagehub/datamodel/sessionApi.ts'
import { readingContext } from '../../src/app/messagehub/sessionModel.ts'

/** The default session of an Agent over the msg-center surface (`msg.get_default_session`, `msg.list_conversations`, `msg.set_default_session`). */

const owner = 'did:bns:me'
const agent = 'did:bns:jarvis'
const telegram = { session_id: 'tg:bot:4201', platform: 'telegram', to: 'did:msgtunnel:4201.user.tg', to_session: 'tg:bot:4201', last_active_at_ms: 5 }
function equal(actual: unknown, expected: unknown) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw Error(`Expected ${JSON.stringify(expected)}, received ${JSON.stringify(actual)}`)
}

function summary(session_id: string, created_at_ms: number, last_activity_ms: number, lifecycle: SessionSummary['lifecycle'] = 'active'): SessionSummary {
  return { session_id, unread_count: 0, updated_at_ms: last_activity_ms, last_activity_ms, request_count: 0, lifecycle, state: { owner, session_id, lifecycle, registered: true, origin: 'manual', peer_did: agent, created_at_ms, updated_at_ms: created_at_ms } }
}
const native = (session_id: string, extra: Partial<DefaultSession> = {}): DefaultSession => ({ session_id, platform: 'native', to: owner, to_session: session_id, set_at_ms: 1, ...extra })

interface Backend {
  summaries: SessionSummary[]
  conversations: PeerConversations
  /** What `msg.get_default_session` answers, for a native chat and otherwise. */
  answer: { default: DefaultSession; native?: DefaultSession }
  uiState: Record<string, Array<{ key: string; value: unknown }>>
  calls: Array<[string, Record<string, unknown>]>
  failDefault: boolean
  beforeConversations?: () => Promise<void>
}

async function fixture(run: (store: MessageHubApiStore, backend: Backend) => Promise<void>) {
  const backend: Backend = { summaries: [summary('A', 10, 100), summary('B', 20, 900), summary('C', 5, 800)], conversations: { conversations: [] }, answer: { default: native('A', { automatic: true }) }, uiState: {}, calls: [], failDefault: false }
  const saved = { getCurrentIdentity: buckyos.getCurrentIdentity, getAccountInfo: buckyos.getAccountInfo, getServiceRpcClient: buckyos.getServiceRpcClient }
  const stateOf = (id: unknown) => backend.summaries.find(item => item.session_id === id)!.state!
  buckyos.getCurrentIdentity = async () => ({ kind: 'anonymous' })
  buckyos.getAccountInfo = async () => ({ user_id: 'me', user_name: 'Me', session_token: 'test-only' })
  buckyos.getServiceRpcClient = (() => ({
    call: async (method: string, input: Record<string, unknown>) => {
      if (method === 'user.get') return { local_profile: { did: owner } }
      if (method === 'agent.list') return { agents: [{ agent_did: agent }] }
      if (method === 'contact.list_contacts' || method === 'group.list_by_member') return []
      if (method === 'msg.list_sessions') {
        const sorted = [...backend.summaries].sort((a, b) => b.last_activity_ms - a.last_activity_ms)
        const rest = input.cursor_updated_at_ms === undefined ? sorted : sorted.filter(item => item.last_activity_ms < Number(input.cursor_updated_at_ms))
        // Pages of two: the least active session is on the last one.
        const items = rest.slice(0, 2), last = items.at(-1)
        return { items: structuredClone(items), ...(rest.length > 2 && last ? { next_cursor_updated_at_ms: last.last_activity_ms, next_cursor_session_id: last.session_id } : {}) }
      }
      if (method === 'ui_session.list_state') return backend.uiState[String(input.session_id)] ?? []
      backend.calls.push([method, input])
      if (method === 'ui_session.update_state') { backend.uiState[String(input.session_id)] = [...(backend.uiState[String(input.session_id)] ?? []), { key: String(input.key), value: input.value }]; return {} }
      if (method === 'msg.get_default_session') {
        if (backend.failDefault) throw Error('method not found')
        return structuredClone(input.native ? backend.answer.native ?? backend.answer.default : backend.answer.default)
      }
      if (method === 'msg.list_conversations') { const value = structuredClone(backend.conversations); await backend.beforeConversations?.(); return value }
      if (method === 'msg.get_session_state') return structuredClone(backend.summaries.find(item => item.session_id === input.session_id)?.state ?? null)
      if (method === 'msg.set_default_session') return input.to === owner ? native(String(input.to_session)) : { ...telegram, set_at_ms: 1 }
      if (method === 'msg.create_session') { backend.summaries.push(summary('new', 2000, 2000)); return { ...stateOf('new'), ...(input.memory_mode ? { memory_mode: input.memory_mode } : {}) } }
      if (method === 'msg.restore_session') return { ...stateOf(input.session_id), lifecycle: 'active' }
      if (method === 'msg.delete_session') return { ...stateOf(input.session_id), registered: false }
      throw Error(`Unexpected RPC ${method}`)
    },
  })) as unknown as typeof buckyos.getServiceRpcClient
  try {
    const store = new MessageHubApiStore()
    await store.initialize()
    await run(store, backend)
  } finally {
    Object.assign(buckyos, saved)
  }
}
const methods = (backend: Backend, ...names: string[]) => backend.calls.filter(([method]) => names.includes(method))

Deno.test('the main entry opens what msg-center answers, listed here even from a page that is not loaded', () => fixture(async (store, backend) => {
  const context = store.defaultContext()
  equal([store.defaultSession(context, agent), store.sessions(context, agent).map(session => session.id)], [null, ['B', 'C']])
  equal((await store.ensureDefaultSession(context, agent))?.id, 'A')
  equal(store.defaultSession(context, agent)?.id, 'A')
  equal(store.sessions(context, agent).map(session => session.id), ['B', 'C', 'A'])
  // One state read instead of the rest of the list; nothing is chosen or created here.
  equal(store.hasMoreEntities(context), true)
  equal(backend.calls.map(([method]) => method), ['msg.get_default_session', 'msg.get_session_state'])
  backend.answer.default = native('B')
  equal((await store.ensureDefaultSession(context, agent))?.id, 'B')
  equal(store.defaultSession(context, agent)?.id, 'B')
}))

Deno.test('an archived default is restored; without an answer the entry fails and can be tried again', () => fixture(async (store, backend) => {
  const context = store.defaultContext()
  backend.summaries[1] = summary('B', 20, 900, 'archived')
  backend.answer.default = native('B')
  await store.ensureOwner(context, true)
  backend.failDefault = true
  const failed = await store.ensureDefaultSession(context, agent).then(() => false, () => true)
  equal([failed, store.defaultSession(context, agent)], [true, null])
  backend.failDefault = false
  const restored = await store.ensureDefaultSession(context, agent)
  equal([restored?.id, restored?.lifecycle], ['B', 'active'])
  equal(methods(backend, 'msg.restore_session', 'msg.create_session').map(([method, input]) => [method, input.session_id]), [['msg.restore_session', 'B']])
}))

Deno.test('a default on a channel opens read-only from the Agent mailbox; what has to be written asks for a native chat', () => fixture(async (store, backend) => {
  const context = store.defaultContext()
  backend.conversations = { default: { ...telegram, set_at_ms: 1 }, conversations: [telegram, { session_id: 'A', platform: 'native', to: owner, to_session: 'A', last_active_at_ms: 9 }] }
  backend.answer = { default: { ...telegram, set_at_ms: 1 }, native: native('A') }
  const opened = (await store.ensureDefaultSession(context, agent))!
  equal([opened.id, opened.ownerDid, opened.entityId, opened.title], [telegram.session_id, agent, agent, 'Telegram'])
  equal(readingContext(context, opened), { viewerDid: owner, ownerDid: agent, mode: 'observe' })
  await store.loadConversations(context, agent)
  equal(store.channelSessions(context, agent).map(session => session.id), [telegram.session_id])
  equal(store.sessions(context, agent).some(session => session.id === telegram.session_id), false)
  equal((await store.ensureDefaultSession(context, agent, { writable: true }))?.id, 'A')
  equal(store.defaultSession(context, agent)?.id, telegram.session_id)
  equal(methods(backend, 'msg.get_default_session').map(([, input]) => input.native ?? false), [false, true])
  equal(store.channelSessions(readingContext(context, opened), agent), [])
}))

Deno.test('a default learned while the conversations are read is not overwritten by them', () => fixture(async (store, backend) => {
  const context = store.defaultContext()
  backend.conversations = { conversations: [telegram] }
  let answer = () => {}
  backend.beforeConversations = () => new Promise<void>(resolve => { answer = resolve })
  const loading = store.loadConversations(context, agent)
  await store.ensureDefaultSession(context, agent)
  answer()
  await loading
  equal([store.defaultSession(context, agent)?.id, store.channelSessions(context, agent).length], ['A', 1])
}))

Deno.test('setting the default names the conversation; creating carries the default and the memory mode', () => fixture(async (store, backend) => {
  const context = store.defaultContext()
  backend.conversations = { conversations: [telegram] }
  await store.loadConversations(context, agent)
  await store.setDefaultSession(context, store.sessions(context, agent).find(session => session.id === 'B')!)
  equal(store.defaultSession(context, agent)?.id, 'B')
  await store.setDefaultSession(context, store.channelSessions(context, agent)[0])
  equal(store.defaultSession(context, agent)?.id, telegram.session_id)
  const created = await store.create(context, { entityId: agent, title: 'Next', connection: 'native', setDefault: true, memoryMode: 'no_write' })
  equal([created.id, created.memoryMode, store.defaultSession(context, agent)?.id], ['new', 'no_write', 'new'])
  equal(methods(backend, 'msg.set_default_session', 'msg.create_session').map(([method, input]) => [method, input.to ?? input.set_default, input.to_session ?? input.memory_mode]), [
    ['msg.set_default_session', owner, 'B'],
    ['msg.set_default_session', telegram.to, telegram.to_session],
    ['msg.create_session', true, 'no_write'],
  ])
}))

Deno.test('a deleted default is replaced by the next entry, not by another session; the new one inherits the mute of the Agent', () => fixture(async (store, backend) => {
  const context = store.defaultContext()
  equal((await store.ensureDefaultSession(context, agent))?.id, 'A')
  await store.manage(context, 'A', 'delete')
  equal([store.defaultSession(context, agent), store.sessions(context, agent).map(session => session.id)], [null, ['B', 'C']])
  for (const id of ['B', 'C']) await store.updatePreferences(context, id, { muted: true })
  backend.summaries.push(summary('D', 3000, 3000))
  backend.answer.default = native('D', { set_at_ms: 3001 })
  const replaced = await store.ensureDefaultSession(context, agent)
  equal([replaced?.id, store.defaultSession(context, agent)?.id, store.preferences(context, 'D').muted], ['D', 'D', true])
  equal(store.sessions(context, agent).map(session => session.id).sort(), ['B', 'C', 'D'])
}))
