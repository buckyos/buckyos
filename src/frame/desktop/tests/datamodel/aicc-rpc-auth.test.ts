import { buckyos } from 'buckyos'
import { toAiccRpcCallOptions } from '../../src/api/aicc_rpc_options.ts'

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message)
}

Deno.test('AICC authentication is encoded in kRPC sys instead of business params', async () => {
  let request: Record<string, unknown> | undefined
  const client = new buckyos.kRPCClient('https://example.com/kapi/aicc', null, null, {
    fetcher: async (_input, init) => {
      request = JSON.parse(String(init?.body)) as Record<string, unknown>
      const sys = Array.isArray(request.sys) ? [request.sys[0]] : undefined
      return new Response(JSON.stringify({ result: {}, sys }), {
        headers: { 'content-type': 'application/json' },
      })
    },
  })

  await client.call('provider.catalog', {}, toAiccRpcCallOptions('session-token'))

  assert(request !== undefined, 'kRPC request must be captured')
  assert(Array.isArray(request.sys) && request.sys[1] === 'session-token', 'session token must use kRPC sys')
  assert(
    typeof request.params === 'object'
      && request.params !== null
      && !('session_token' in request.params),
    'session token must not use a business params field',
  )
})

Deno.test('AICC calls preserve the SDK token provider when no token is resolved', () => {
  assert(toAiccRpcCallOptions(null) === undefined, 'missing token must not override SDK call options')
})
