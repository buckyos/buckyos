/// <reference types="vite/client" />
import { buckyos } from 'buckyos'
import { MessageHubApiStore } from '../../src/app/messagehub/api/store.ts'
import { recordMeta } from '../../src/app/messagehub/api/reader.ts'
import type { MailboxRecord, SessionSummary } from '../../src/app/messagehub/datamodel/sessionApi.ts'

const owner = 'did:bns:me'
const peer = 'did:bns:alice'
const active = 'dm:alice'
const archived = 'archived-session'
const notification = 'sysnotify:app'
function equal(actual: unknown, expected: unknown) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw Error(`Expected ${JSON.stringify(expected)}, received ${JSON.stringify(actual)}`)
}

function record(session: string, index: number, state: MailboxRecord['state'] = 'UNREAD', box: MailboxRecord['box_kind'] = 'INBOX'): MailboxRecord {
  return { record_id: `${session}:${index}`, mailbox: `${owner}/${session}`, owner, box_kind: box, msg_id: `msg:${session}:${index}`, state, from: box === 'SENT' ? owner : peer, to: box === 'SENT' ? peer : owner, session_id: session, sort_key: index, created_at_ms: index, updated_at_ms: index }
}

async function fixture(run: (store: MessageHubApiStore, backend: {
  records: Map<string, MailboxRecord[]>
  writes: string[]
  failures: Set<string>
  messagePages: Map<string, number>
  beforeFirstWrite?: () => void
  beforeSessionList?: () => Promise<void>
  maxConcurrent: number
}) => Promise<void>) {
  const records = new Map<string, MailboxRecord[]>()
  records.set(active, [...Array.from({ length: 66 }, (_, index) => record(active, 1000 + index)), record(active, 1066, 'READ'), record(active, 1067, 'ARCHIVED'), record(active, 1068, 'READ', 'SENT')])
  for (let index = 0; index < 48; index++) records.set(`empty-${index}`, [])
  records.set(notification, [record(notification, 10)])
  records.set(archived, [record(archived, 1, 'UNREAD', 'REQUEST_BOX')])
  const backend = { records, writes: [] as string[], failures: new Set<string>(), messagePages: new Map<string, number>(), beforeFirstWrite: undefined as (() => void) | undefined, beforeSessionList: undefined as (() => Promise<void>) | undefined, maxConcurrent: 0 }
  const saved = { getCurrentIdentity: buckyos.getCurrentIdentity, getAccountInfo: buckyos.getAccountInfo, getServiceRpcClient: buckyos.getServiceRpcClient }
  let concurrent = 0
  buckyos.getCurrentIdentity = async () => ({ kind: 'anonymous' })
  buckyos.getAccountInfo = async () => ({ user_id: 'me', user_name: 'Me', session_token: 'test-only' })
  buckyos.getServiceRpcClient = (() => ({
    call: async (method: string, input: Record<string, unknown>) => {
      if (method === 'user.get') return { local_profile: { did: owner } }
      if (method === 'agent.list') return { agents: [] }
      if (method === 'contact.list_contacts' || method === 'group.list_by_member') return []
      if (method === 'msg.list_sessions') {
        await backend.beforeSessionList?.()
        equal([input.owner, input.lifecycle, input.order_by], [owner, 'all', 'activity'])
        let sessions = [...records].map(([session_id, values], index): SessionSummary => {
          const sorted = [...values].sort((a, b) => b.sort_key - a.sort_key || b.record_id.localeCompare(a.record_id))
          const last = sorted[0]
          return { session_id, unread_count: values.filter(value => value.state === 'UNREAD').length, updated_at_ms: Math.max(0, ...values.map(value => value.updated_at_ms)), last_activity_ms: last?.sort_key ?? 500 - index, request_count: values.filter(value => value.box_kind === 'REQUEST_BOX').length, lifecycle: session_id === archived ? 'archived' : 'active', ...(last ? { last_record: { record: { ...last }, msg: { from: last.from, to: [last.to], kind: 'chat', created_at_ms: last.sort_key, content: { content: last.record_id } } } } : {}) }
        }).sort((a, b) => b.last_activity_ms - a.last_activity_ms || b.session_id.localeCompare(a.session_id))
        if (input.cursor_updated_at_ms !== undefined) sessions = sessions.filter(item => item.last_activity_ms < Number(input.cursor_updated_at_ms) || (item.last_activity_ms === input.cursor_updated_at_ms && item.session_id < String(input.cursor_session_id)))
        const items = sessions.slice(0, Number(input.limit))
        const last = items.at(-1)
        return { items, ...(sessions.length > items.length && last ? { next_cursor_updated_at_ms: last.last_activity_ms, next_cursor_session_id: last.session_id } : {}) }
      }
      if (method === 'msg.list_session') {
        const session = String(input.session_id)
        backend.messagePages.set(session, (backend.messagePages.get(session) ?? 0) + 1)
        let values = [...records.get(session)!].sort((a, b) => b.sort_key - a.sort_key || b.record_id.localeCompare(a.record_id))
        if (input.cursor_sort_key !== undefined) values = values.filter(item => item.sort_key < Number(input.cursor_sort_key) || (item.sort_key === input.cursor_sort_key && item.record_id < String(input.cursor_record_id)))
        const items = values.slice(0, Number(input.limit)).map(value => ({ record_id: value.record_id, msg_id: value.msg_id, direction: value.box_kind === 'SENT' ? 'out' : 'in', box_kind: value.box_kind, sort_key: value.sort_key, from: value.from, to: value.to, recipient_state: value.box_kind === 'SENT' ? undefined : value.state }))
        const last = items.at(-1)
        return { items, ...(values.length > items.length && last ? { next_cursor_sort_key: last.sort_key, next_cursor_record_id: last.record_id } : {}) }
      }
      if (method === 'msg.update_record_state') {
        equal(input.new_state, 'READ')
        if (backend.writes.length === 0) backend.beforeFirstWrite?.()
        const id = String(input.record_id)
        backend.writes.push(id)
        backend.maxConcurrent = Math.max(backend.maxConcurrent, ++concurrent)
        await Promise.resolve()
        concurrent--
        if (backend.failures.has(id)) throw Error('read update failed')
        const value = [...records.values()].flat().find(item => item.record_id === id)!
        value.state = 'READ'
        value.updated_at_ms += 100_000
        return { ...value }
      }
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

Deno.test('mark all read persists every session and message page, includes archived requests and notifications, and preserves new arrivals', async () => {
  await fixture(async (store, backend) => {
    const context = store.defaultContext()
    equal(store.hasMoreEntities(context), true)
    equal(store.sessions(context).some(session => session.id === archived), false)
    store.reader(context, active)
    for (let attempt = 0; attempt < 20 && store.historyStatus(context, active) === 'loading'; attempt++) await Promise.resolve()
    equal(store.historyStatus(context, active), 'ready')
    backend.beforeFirstWrite = () => backend.records.get(notification)!.push(record(notification, 2000), { ...record(notification, 5), created_at_ms: 2000 })
    await store.markAllRead(context)
    equal(backend.writes.length, 68)
    equal(new Set(backend.writes).size, 68)
    equal(backend.messagePages.get(active), 3)
    equal(backend.maxConcurrent, 8)
    equal(backend.records.get(active)!.filter(item => item.state === 'UNREAD').length, 0)
    equal(backend.records.get(active)!.find(item => item.sort_key === 1067)?.state, 'ARCHIVED')
    equal(backend.records.get(archived)![0].state, 'READ')
    equal(backend.records.get(notification)!.map(item => item.state), ['READ', 'UNREAD', 'UNREAD'])
    equal(store.sessions(context).reduce((sum, session) => sum + session.unreadCount, 0), 2)
    equal(store.sessions(context).find(session => session.id === archived)?.requestCount, 1)
    equal(store.hasMoreEntities(context), false)
    const cached = await store.reader(context, active).readRange(0, 100)
    equal(cached.some(message => recordMeta(message)?.recipientState === 'UNREAD'), false)
    await store.ensureOwner(context, true)
    equal(store.sessions(context).reduce((sum, session) => sum + session.unreadCount, 0), 2)
  })
})

Deno.test('mark all read reports partial failures, keeps successful writes, and rejects observer contexts', async () => {
  await fixture(async (store, backend) => {
    const context = store.defaultContext()
    for (const foreign of [{ ...context, mode: 'observe' as const }, { ...context, ownerDid: peer }, { ...context, viewerDid: peer }]) {
      let rejected = false
      try { await store.markAllRead(foreign) } catch { rejected = true }
      equal(rejected, true)
    }
    equal(backend.writes.length, 0)
    backend.failures.add(`${active}:1000`)
    let rejected = false
    try { await store.markAllRead(context) } catch { rejected = true }
    equal(rejected, true)
    equal(backend.writes.length, 68)
    equal(backend.records.get(active)![0].state, 'UNREAD')
    equal(backend.records.get(archived)![0].state, 'READ')
    equal(store.sessions(context).reduce((sum, session) => sum + session.unreadCount, 0), 1)
    backend.failures.clear()
    await store.markAllRead(context)
    equal(backend.writes.length, 69)
    equal(store.sessions(context).reduce((sum, session) => sum + session.unreadCount, 0), 0)
  })
})

Deno.test('owner refresh after a relationship action keeps the hub ready while updating summaries', async () => {
  await fixture(async (store, backend) => {
    const context = store.defaultContext()
    equal(store.ownerStatus(context).phase, 'ready')
    const before = store.sessions(context).find(session => session.id === active)!.unreadCount
    let release!: () => void
    backend.beforeSessionList = () => new Promise<void>(resolve => { release = resolve })
    const phases: string[] = []
    const unsubscribe = store.subscribe(() => phases.push(store.ownerStatus(context).phase))
    const refresh = store.ensureOwner(context, true)
    try {
      equal(store.ownerStatus(context).phase, 'ready')
      if (store.ensureOwner(context, true) !== refresh) throw Error('Refresh was not shared')
      equal(store.sessions(context).find(session => session.id === active)!.unreadCount, before)
      backend.records.get(active)!.forEach(record => { record.state = 'READ' })
    } finally {
      release()
      await refresh
      unsubscribe()
    }
    equal(store.ownerStatus(context).phase, 'ready')
    equal(phases, ['ready'])
    equal(store.sessions(context).find(session => session.id === active)!.unreadCount, 0)
  })
})
