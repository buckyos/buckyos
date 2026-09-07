import { buckyos } from 'buckyos'
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
