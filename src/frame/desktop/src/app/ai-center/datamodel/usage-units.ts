import type { UsageBreakdown, UsageDimension, UsageEvent, UsageTotals } from '../mock/types'
import { normalizeFinanceTotals } from './transforms'

export const usageUnits = {
  total_tokens: ['Total tokens', '总 token', 'tokens'],
  input_tokens: ['Input tokens', '输入 token', 'tokens'],
  output_tokens: ['Output tokens', '输出 token', 'tokens'],
  cache_read_input_tokens: ['Cache read', '缓存命中', 'tokens'],
  cache_write_input_tokens: ['Cache write (all TTLs)', '缓存写入（全部时长）', 'tokens'],
  cache_write_1h_input_tokens: ['Cache write (1h subset)', '缓存写入（1h 子集）', 'tokens'],
  reasoning_tokens: ['Reasoning (output subset)', '推理（输出子集）', 'tokens'],
  image_input_tokens: ['Image input', '图片输入', 'tokens'],
  image_output_tokens: ['Image output', '图片输出', 'tokens'],
  video_input_tokens: ['Video input', '视频输入', 'tokens'],
  video_output_tokens: ['Video output', '视频输出', 'tokens'],
  audio_input_tokens: ['Audio input', '音频输入', 'tokens'],
  audio_output_tokens: ['Audio output', '音频输出', 'tokens'],
  image_units: ['Generated images', '生成图片张数', 'images'],
  reference_images: ['Reference images', '参考图片张数', 'images'],
  video_seconds: ['Video duration', '视频时长', 's'],
  audio_seconds: ['Audio duration', '音频时长', 's'],
  characters: ['Characters', '字符数', 'chars'],
  request_units: ['Billable requests', '请求计费单位', 'requests'],
  compute_seconds: ['Compute duration', '计算时长', 's'],
  megapixels: ['Billable megapixels', '计费百万像素', 'MP'],
} as const

const number = (value: unknown) => typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : undefined
const record = (value: unknown): Record<string, unknown> => value !== null && typeof value === 'object' ? value as Record<string, unknown> : {}

export function toUsageTotals(value: unknown): UsageTotals {
  const raw = record(value)
  const dimensions: Record<string, UsageDimension> = {}
  for (const [key, value] of Object.entries(record(raw.dimensions))) {
    const dimension = record(value)
    const amount = number(dimension.amount)
    const count = number(dimension.reported_events)
    if (key in usageUnits && amount !== undefined && count !== undefined && count > 0) dimensions[key] = { amount, reported_events: count }
  }
  const count = number(raw.total_requests) ?? 0
  return {
    total_requests: count,
    total_tokens: number(raw.total_tokens) ?? 0,
    dimensions,
    finance_totals: normalizeFinanceTotals(Array.isArray(raw.finance_totals) ? raw.finance_totals : []),
    finance_complete: raw.finance_complete === true || count === 0,
    unpriced_requests: number(raw.unpriced_requests) ?? (raw.finance_complete === true ? 0 : count),
  }
}

export function eventUsageDimensions(event: UsageEvent): Record<string, UsageDimension> {
  const usage = { input_tokens: event.tokens_in, output_tokens: event.tokens_out, total_tokens: event.token_equivalent, ...event.usage }
  return Object.fromEntries(Object.entries(usage).flatMap(([key, value]) => key in usageUnits && number(value) !== undefined ? [[key, { amount: value as number, reported_events: 1 }]] : []))
}

export function mergeUsageTotals(totals: UsageTotals[]): UsageTotals {
  const result = toUsageTotals({ total_requests: 0, finance_complete: true })
  const money = new Map<string, number>()
  for (const total of totals) {
    result.total_requests += total.total_requests
    result.total_tokens += total.total_tokens
    result.unpriced_requests += total.unpriced_requests
    result.finance_complete &&= total.finance_complete
    for (const [key, dimension] of Object.entries(total.dimensions)) {
      const current = result.dimensions[key] ?? { amount: 0, reported_events: 0 }
      result.dimensions[key] = { amount: current.amount + dimension.amount, reported_events: current.reported_events + dimension.reported_events }
    }
    for (const cost of total.finance_totals) money.set(cost.currency, (money.get(cost.currency) ?? 0) + cost.amount)
  }
  result.finance_totals = Array.from(money, ([currency, amount]) => ({ currency, amount })).sort((left, right) => left.currency.localeCompare(right.currency))
  return result
}

export function aggregateUsageEvents(events: UsageEvent[]): UsageTotals {
  return mergeUsageTotals(events.map((event) => {
    const finance = event.finance_snapshot
    const known = number(finance?.amount) !== undefined && Boolean(finance?.currency?.trim())
    return toUsageTotals({ total_requests: 1, total_tokens: event.token_equivalent ?? (event.tokens_in ?? 0) + (event.tokens_out ?? 0), dimensions: eventUsageDimensions(event), finance_complete: known, unpriced_requests: known ? 0 : 1, finance_totals: known ? [finance] : [] })
  }))
}

export function groupUsageEvents(events: UsageEvent[]): UsageBreakdown[] {
  const grouped = new Map<string, UsageEvent[]>()
  for (const event of events) {
    const key = JSON.stringify({ provider_instance_name: event.provider_instance_name, provider_model: event.exact_model, caller_app_id: event.app_id ?? 'system', capability: event.api_type })
    grouped.set(key, [...(grouped.get(key) ?? []), event])
  }
  return Array.from(grouped, ([key, events]) => ({ group: JSON.parse(key), aggregate: aggregateUsageEvents(events) }))
}
