import { buckyos, getActiveSessionToken } from 'buckyos'
import { isMockRuntime } from '../runtime'
import { toAiccRpcCallOptions } from './aicc_rpc_options'

export interface SystemTokenUsage {
  startTimeMs: number
  endTimeMs: number
  buckets: Array<{ startTimeMs: number; inputTokens: number; outputTokens: number }>
  totals: { hour: number; fiveHours: number; day: number }
}

const bucketMs = 5 * 60_000
const hourMs = 60 * 60_000

interface UsageAggregate {
  input_tokens?: unknown
  output_tokens?: unknown
  total_tokens?: unknown
}

interface UsageResponse {
  total: UsageAggregate
  buckets?: Array<{ bucket_start_ms: number; aggregate: UsageAggregate }>
}

function tokenCount(value: unknown): number {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0) {
    throw new Error('Invalid token usage count')
  }
  return value
}

function totalTokens(aggregate: UsageAggregate): number {
  return aggregate.total_tokens === undefined
    ? tokenCount(aggregate.input_tokens) + tokenCount(aggregate.output_tokens)
    : tokenCount(aggregate.total_tokens)
}

function emptyUsage(endTimeMs: number): SystemTokenUsage {
  const startTimeMs = endTimeMs - hourMs
  const firstBucket = Math.floor(startTimeMs / bucketMs) * bucketMs
  return {
    startTimeMs,
    endTimeMs,
    buckets: Array.from({ length: Math.ceil((endTimeMs - firstBucket) / bucketMs) }, (_, index) => ({
      startTimeMs: firstBucket + index * bucketMs,
      inputTokens: 0,
      outputTokens: 0,
    })),
    totals: { hour: 0, fiveHours: 0, day: 0 },
  }
}

function mockUsage(endTimeMs: number, zoneWide: boolean): SystemTokenUsage {
  const usage = emptyUsage(endTimeMs)
  const params = new URLSearchParams(globalThis.location?.search ?? '')
  if ((params.get('aiccScenario') ?? params.get('scenario')) === 'empty') return usage
  const scale = zoneWide ? 1 : 0.6
  for (let start = Math.floor((endTimeMs - 24 * hourMs) / bucketMs) * bucketMs; start < endTimeMs; start += bucketMs) {
    const timestamp = start + bucketMs / 2
    if (timestamp < endTimeMs - 24 * hourMs || timestamp >= endTimeMs) continue
    const phase = start / bucketMs
    const inputTokens = Math.round(scale * (2_100 + 1_500 * Math.sin(phase * 1.7)))
    const outputTokens = Math.round(scale * (800 + 600 * Math.sin(phase * 2.3)))
    const total = inputTokens + outputTokens
    usage.totals.day += total
    if (timestamp >= endTimeMs - 5 * hourMs) usage.totals.fiveHours += total
    if (timestamp < usage.startTimeMs) continue
    usage.totals.hour += total
    const bucket = usage.buckets.find((item) => item.startTimeMs === start)
    if (bucket) Object.assign(bucket, { inputTokens, outputTokens })
  }
  return usage
}

export async function fetchSystemTokenUsage(zoneWide: boolean): Promise<{ data: SystemTokenUsage | null; error: unknown }> {
  try {
    const endTimeMs = Date.now()
    if (isMockRuntime()) return { data: mockUsage(endTimeMs, zoneWide), error: null }
    const account = await buckyos.getAccountInfo()
    const sessionToken = account?.session_token?.trim() || await getActiveSessionToken()
    const client = buckyos.getServiceRpcClient('aicc')
    const query = async (hours: number): Promise<UsageResponse> => {
      const response = await client.call('usage.query', {
        time_range: { kind: 'explicit', start_time_ms: endTimeMs - hours * hourMs, end_time_ms: endTimeMs },
        zone_wide: zoneWide,
        output_mode: 'summary',
        ...(hours === 1 ? { time_bucket: 'five_minutes' } : {}),
      }, toAiccRpcCallOptions(sessionToken))
      if (!response || typeof response !== 'object' || !('total' in response) || !response.total || typeof response.total !== 'object') {
        throw new Error('Invalid token usage response')
      }
      if ('buckets' in response && !Array.isArray(response.buckets)) {
        throw new Error('Invalid token usage buckets')
      }
      return response as UsageResponse
    }
    const [hour, fiveHours, day] = await Promise.all([query(1), query(5), query(24)])
    const usage = emptyUsage(endTimeMs)
    usage.totals = { hour: totalTokens(hour.total), fiveHours: totalTokens(fiveHours.total), day: totalTokens(day.total) }
    for (const row of hour.buckets ?? []) {
      if (!Number.isInteger(row.bucket_start_ms) || row.bucket_start_ms % bucketMs !== 0) {
        throw new Error('Invalid token usage bucket')
      }
      const bucket = usage.buckets.find((item) => item.startTimeMs === row.bucket_start_ms)
      if (!bucket) continue
      bucket.inputTokens += tokenCount(row.aggregate.input_tokens)
      bucket.outputTokens += tokenCount(row.aggregate.output_tokens)
    }
    return { data: usage, error: null }
  } catch (error) {
    return { data: null, error }
  }
}
