/// <reference types="vite/client" />
import { buckyos } from 'buckyos'
import { MessageHubApiStore } from '../../src/app/messagehub/api/store.ts'
import { recordMeta } from '../../src/app/messagehub/api/reader.ts'
import type { MailboxRecord, SessionSummary } from '../../src/app/messagehub/datamodel/sessionApi.ts'

const owner = 'did:bns:me'
const peer = 'did:bns:alice'
const session = 'dm:alice'

function equal(actual: unknown, expected: unknown) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw Error(`Expected ${JSON.stringify(expected)}, received ${JSON.stringify(actual)}`)
}

function record(index: number): MailboxRecord {
  return { record_id: `${session}:${index}`, mailbox: `${owner}/${session}`, owner, box_kind: 'INBOX', msg_id: `msg:${index}`, state: 'UNREAD', from: peer, to: owner, session_id: session, sort_key: index, created_at_ms: index, updated_at_ms: index }
}

function range(start: number, count: number) {
  return Array.from({ length: count }, (_, offset) => record(start + offset))
}

async function fixture(run: (store: MessageHubApiStore, backend: { records: MailboxRecord[]; pages: number }) => Promise<void>) {
  const backend = { records: range(1000, 10), pages: 0 }
  const saved = { getCurrentIdentity: buckyos.getCurrentIdentity, getAccountInfo: buckyos.getAccountInfo, getServiceRpcClient: buckyos.getServiceRpcClient, subscribeKEvent: buckyos.subscribeKEvent }
  buckyos.getCurrentIdentity = async () => ({ kind: 'anonymous' })
  buckyos.getAccountInfo = async () => ({ user_id: 'me', user_name: 'Me', session_token: 'test-only' })
  buckyos.subscribeKEvent = (async () => ({ close: async () => {} })) as unknown as typeof buckyos.subscribeKEvent
  Object.assign(globalThis, { window: globalThis })
  buckyos.getServiceRpcClient = (() => ({
    call: async (method: string, input: Record<string, unknown>) => {
      if (method === 'user.get') return { local_profile: { did: owner } }
      if (method === 'agent.list') return { agents: [] }
      if (method === 'contact.list_contacts' || method === 'group.list_by_member' || method === 'ui_session.list_state') return []
      const sorted = [...backend.records].sort((a, b) => b.sort_key - a.sort_key)
      if (method === 'msg.list_sessions') {
        const last = sorted[0]
        const summary: SessionSummary = { session_id: session, unread_count: sorted.length, updated_at_ms: last.updated_at_ms, last_activity_ms: last.sort_key, request_count: 0, lifecycle: 'active', last_record: { record: { ...last }, msg: { from: peer, to: [owner], kind: 'chat', created_at_ms: last.sort_key, content: { content: last.record_id } } } }
        return { items: [summary] }
      }
      if (method === 'msg.list_session') {
        backend.pages++
        const values = input.cursor_sort_key === undefined ? sorted : sorted.filter(item => item.sort_key < Number(input.cursor_sort_key))
        const items = values.slice(0, Number(input.limit)).map(value => ({ record_id: value.record_id, msg_id: value.msg_id, direction: 'in', box_kind: value.box_kind, sort_key: value.sort_key, from: value.from, to: value.to, recipient_state: value.state }))
        const last = items.at(-1)
        return { items, ...(values.length > items.length && last ? { next_cursor_sort_key: last.sort_key, next_cursor_record_id: last.record_id } : {}) }
      }
      throw Error(`Unexpected RPC ${method}`)
    },
  })) as unknown as typeof buckyos.getServiceRpcClient
  try {
    const store = new MessageHubApiStore()
    await store.initialize()
    const context = store.defaultContext()
    store.reader(context, session)
    await settle(() => store.historyStatus(context, session) === 'ready')
    backend.pages = 0
    await run(store, backend)
  } finally {
    Object.assign(buckyos, saved)
    delete (globalThis as { window?: unknown }).window
  }
}

async function settle(done: () => boolean) {
  for (let attempt = 0; attempt < 200 && !done(); attempt++) await new Promise(resolve => setTimeout(resolve, 0))
  if (!done()) throw Error('Timed out')
}

async function timelineIds(store: MessageHubApiStore) {
  const reader = store.reader(store.defaultContext(), session)
  return (await reader.readRange(0, reader.totalCount)).map(message => recordMeta(message)?.sortKey)
}

async function activate(store: MessageHubApiStore, backend: { pages: number }, pages: number) {
  const stop = store.startSync(store.defaultContext(), session)
  try { await settle(() => backend.pages >= pages) } finally { stop() }
  await new Promise(resolve => setTimeout(resolve, 0))
}

Deno.test('opening a cached session reads what arrived while another session was open', async () => {
  await fixture(async (store, backend) => {
    backend.records.push(record(1010))
    await activate(store, backend, 1)
    equal(await timelineIds(store), range(1000, 11).map(item => item.sort_key))
    equal(backend.pages, 1)
  })
})

Deno.test('a tail reconcile pages back until it reaches the cached timeline', async () => {
  await fixture(async (store, backend) => {
    backend.records.push(...range(1010, 100))
    await activate(store, backend, 4)
    equal(await timelineIds(store), range(1000, 110).map(item => item.sort_key))
    equal(backend.pages, 4)
    equal(store.hasOlder(store.defaultContext(), session), false)
  })
})

Deno.test('a gap past the page limit restarts the timeline from the latest pages', async () => {
  await fixture(async (store, backend) => {
    backend.records.push(...range(1010, 300))
    await activate(store, backend, 8)
    equal(await timelineIds(store), range(1054, 256).map(item => item.sort_key))
    equal(store.hasOlder(store.defaultContext(), session), true)
    await store.loadOlder(store.defaultContext(), session)
    equal(await timelineIds(store), range(1000, 310).map(item => item.sort_key))
    equal(store.hasOlder(store.defaultContext(), session), false)
  })
})
