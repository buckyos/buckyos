/// <reference types="vite/client" />
import { ANONYMOUS_IDENTITY, buckyos, type CurrentIdentity } from 'buckyos'
import { MessageHubApiStore } from '../../src/app/messagehub/api/store.ts'
import { fetchOwnerDid, MessageHubApiError } from '../../src/app/messagehub/datamodel/sessionApi.ts'

function equal(actual: unknown, expected: unknown) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw Error(`Expected ${JSON.stringify(expected)}, received ${JSON.stringify(actual)}`)
}

Deno.test('MessageHub resolves the signed-in user from the profile and never invents a DID', async () => {
  const getAccountInfo = buckyos.getAccountInfo
  const getServiceRpcClient = buckyos.getServiceRpcClient
  let username: string | null = 'lucy'
  let detail: Record<string, unknown> = {}
  let requests = 0
  buckyos.getAccountInfo = async () => username ? { user_id: username, user_name: username, session_token: 'test-only' } : null
  buckyos.getServiceRpcClient = ((service: string) => ({
    call: async (method: string, params: Record<string, unknown>) => {
      equal(service, 'control-panel')
      equal(method, 'user.get')
      equal(params, { user_id: username })
      requests++
      return detail
    },
  })) as typeof buckyos.getServiceRpcClient
  try {
    detail = { local_profile: { did: 'did:web:lucy.test.buckyos.io' }, profile: { did: 'did:web:lucy.test.buckyos.io' } }
    equal(await fetchOwnerDid(), 'did:web:lucy.test.buckyos.io')
    username = 'devtest'
    detail = { local_profile: { did: 'did:bns:devtest' } }
    equal(await fetchOwnerDid(), 'did:bns:devtest')
    username = 'invited-user'
    detail = { profile: { did: 'did:bns:independent-identity' } }
    equal(await fetchOwnerDid(), 'did:bns:independent-identity')
    for (const invalid of [{}, { local_profile: { did: 'lucy' } }]) {
      detail = invalid
      let denied = false
      try { await fetchOwnerDid() } catch (error) { denied = error instanceof MessageHubApiError && error.kind === 'permission_denied' }
      equal(denied, true)
    }
    username = null
    equal(await fetchOwnerDid(), null)
    equal(requests, 5)
  } finally {
    buckyos.getAccountInfo = getAccountInfo
    buckyos.getServiceRpcClient = getServiceRpcClient
  }
})

Deno.test('MessageHub reloads for another subject or a sign-out, not while the Zone does not answer', async () => {
  const account = (user: string): CurrentIdentity => ({ kind: 'account', user_id: user, did: `did:bns:${user}`, user_type: 'admin', show_name: user, zone_host: 'alice.test', restricted: false } as unknown as CurrentIdentity)
  const saved = { getCurrentIdentity: buckyos.getCurrentIdentity, getAccountInfo: buckyos.getAccountInfo, getServiceRpcClient: buckyos.getServiceRpcClient }
  let identity: CurrentIdentity = account('alice')
  let stored: CurrentIdentity | null = identity
  let reloads = 0
  buckyos.getCurrentIdentity = async () => identity
  buckyos.getAccountInfo = async () => ({ user_id: 'alice', user_name: 'Alice', session_token: 'test-only' })
  buckyos.getServiceRpcClient = (() => ({
    call: async (method: string) => {
      if (method === 'user.get') return { local_profile: { did: 'did:bns:alice' } }
      if (method === 'agent.list') return { agents: [] }
      if (method === 'msg.list_sessions') return { items: [] }
      return []
    },
  })) as unknown as typeof buckyos.getServiceRpcClient
  Object.assign(globalThis, {
    window: {
      localStorage: { getItem: (key: string) => key === 'buckyos.identity' && stored ? JSON.stringify(stored) : null },
      location: { reload: () => { reloads++ } },
      addEventListener() {}, removeEventListener() {},
    },
  })
  const check = async (next: CurrentIdentity, kept: CurrentIdentity | null) => {
    identity = account('alice')
    stored = identity
    reloads = 0
    const store = new MessageHubApiStore()
    await store.initialize()
    identity = next
    stored = kept
    await (store as unknown as { checkIdentity(): Promise<void> }).checkIdentity()
    return reloads
  }
  try {
    equal(await check(account('alice'), account('alice')), 0)
    equal(await check(ANONYMOUS_IDENTITY, account('alice')), 0)
    equal(await check(ANONYMOUS_IDENTITY, null), 1)
    equal(await check(ANONYMOUS_IDENTITY, account('bob')), 1)
    equal(await check(account('bob'), account('bob')), 1)
  } finally {
    Object.assign(buckyos, saved)
    delete (globalThis as { window?: unknown }).window
  }
})
