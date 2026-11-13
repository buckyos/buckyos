import { buckyos } from 'buckyos'
import { fetchSystemTokenUsage } from '../../src/api/system_token_usage.ts'

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message)
}

const hourMs = 3_600_000
const bucketMs = 300_000
const boundary = 1_800_000_000_000
const aggregate = (input: number, output: number) => ({ input_tokens: input, output_tokens: output, total_tokens: input + output })

async function withRpc(
  now: number,
  call: (params: Record<string, unknown>, options?: { sessionToken?: string }) => Promise<unknown>,
  run: () => Promise<void>,
) {
  const sdk = buckyos as unknown as {
    getAccountInfo: () => Promise<unknown>
    getServiceRpcClient: (service: string) => unknown
  }
  const originalAccount = sdk.getAccountInfo
  const originalClient = sdk.getServiceRpcClient
  const originalNow = Date.now
  sdk.getAccountInfo = async () => ({ session_token: 'test-session' })
  sdk.getServiceRpcClient = (service) => {
    assert(service === 'aicc', 'usage must come from AICC')
    return { call: async (method: string, params: Record<string, unknown>, options?: { sessionToken?: string }) => {
      assert(method === 'usage.query', 'status must only query usage, without loading the full AI dashboard')
      return call(params, options)
    } }
  }
  Date.now = () => now
  try { await run() } finally {
    sdk.getAccountInfo = originalAccount
    sdk.getServiceRpcClient = originalClient
    Date.now = originalNow
  }
}

Deno.test('system token usage requests exact rolling windows and preserves boundary buckets and gaps', async () => {
  const now = boundary + 120_000
  const firstBucket = boundary - hourMs
  const requests: Record<string, unknown>[] = []
  await withRpc(now, async (params, options) => {
    requests.push(params)
    assert(options?.sessionToken === 'test-session', 'session authentication must stay in kRPC options')
    assert(!('session_token' in params), 'authentication must not enter business parameters')
    const range = params.time_range as { kind: string; start_time_ms: number; end_time_ms: number }
    assert(range.kind === 'explicit' && range.end_time_ms === now, 'all slices share one exact end time')
    assert(params.zone_wide === true && params.output_mode === 'summary', 'every slice keeps the requested scope and avoids event downloads')
    const hours = (now - range.start_time_ms) / hourMs
    return {
      total: aggregate(hours * 100, hours * 20),
      ...(params.time_bucket ? { buckets: [
        { bucket_start_ms: firstBucket, aggregate: aggregate(3, 4) },
        { bucket_start_ms: boundary, aggregate: aggregate(5, 6) },
        { bucket_start_ms: firstBucket - bucketMs, aggregate: aggregate(999, 999) },
        { bucket_start_ms: boundary + bucketMs, aggregate: aggregate(999, 999) },
      ] } : {}),
    }
  }, async () => {
    const { data, error } = await fetchSystemTokenUsage(true)
    assert(data && !error, 'valid responses must produce a reading')
    assert(data.startTimeMs === now - hourMs && data.endTimeMs === now, 'chart range must be a trailing hour')
    assert(data.buckets.length === 13, 'partial first and current five-minute buckets must both be included')
    assert(data.buckets[0].inputTokens === 3 && data.buckets[0].outputTokens === 4, 'first partial bucket must survive')
    assert(data.buckets[12].inputTokens === 5 && data.buckets[12].outputTokens === 6, 'current partial bucket must survive')
    assert(data.buckets.slice(1, -1).every((bucket) => bucket.inputTokens === 0 && bucket.outputTokens === 0), 'missing buckets must stay zero')
    assert(data.totals.hour === 120 && data.totals.fiveHours === 600 && data.totals.day === 2880, 'each rolling total must use its own aggregate')
    assert(requests.length === 3 && requests.filter((request) => request.time_bucket === 'five_minutes').length === 1, 'one bucket query and two summaries are sufficient')
  })
})

Deno.test('exact five-minute boundaries omit future buckets and empty usage remains a valid zero', async () => {
  await withRpc(boundary, async (params) => {
    assert(params.zone_wide === false, 'personal requests must not opt into zone access')
    return { total: aggregate(0, 0) }
  }, async () => {
    const { data, error } = await fetchSystemTokenUsage(false)
    assert(data && !error && data.buckets.length === 12, 'an aligned trailing hour has exactly twelve buckets')
    assert(data.buckets.at(-1)?.startTimeMs === boundary - bucketMs, 'there must be no zero-width future bucket')
    assert(Object.values(data.totals).every((total) => total === 0), 'successful empty responses must report zero')
  })
})

Deno.test('missing total tokens falls back to input and output while explicit zero is preserved', async () => {
  await withRpc(boundary, async (params) => ({
    total: params.time_bucket ? { input_tokens: 10, output_tokens: 20 } : { ...aggregate(10, 20), total_tokens: 0 },
  }), async () => {
    const { data } = await fetchSystemTokenUsage(false)
    assert(data?.totals.hour === 30 && data.totals.fiveHours === 0 && data.totals.day === 0, 'the total field takes precedence when present')
  })
})

Deno.test('failed or malformed usage never becomes a zero reading', async () => {
  for (const invalid of [
    null,
    {},
    { total: { total_tokens: -1 } },
    { total: { total_tokens: Number.NaN } },
    ...['', {}, null].map((buckets) => ({ total: aggregate(0, 0), buckets })),
    { total: aggregate(0, 0), buckets: [{ bucket_start_ms: boundary - bucketMs }] },
  ]) {
    await withRpc(boundary, async () => invalid, async () => {
      const { data, error } = await fetchSystemTokenUsage(false)
      assert(data === null && error instanceof Error, 'malformed responses must be reported as unavailable')
    })
  }
  await withRpc(boundary, async (params) => {
    const range = params.time_range as { start_time_ms: number }
    if (boundary - range.start_time_ms === 5 * hourMs) throw new Error('usage unavailable')
    return { total: aggregate(0, 0) }
  }, async () => {
    const { data, error } = await fetchSystemTokenUsage(false)
    assert(data === null && error instanceof Error && error.message === 'usage unavailable', 'a failed window must invalidate the reading')
  })
})
