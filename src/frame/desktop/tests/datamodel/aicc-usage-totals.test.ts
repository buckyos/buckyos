import { aggregateUsageEvents, eventUsageDimensions, groupUsageEvents, mergeUsageTotals, toUsageTotals } from '../../src/app/ai-center/datamodel/usage-units.ts'
import type { UsageEvent } from '../../src/app/ai-center/mock/types.ts'

function assert(value: unknown, message: string): asserts value { if (!value) throw new Error(message) }
const event = (id: string, usage: Record<string, unknown>, currency?: string): UsageEvent => ({ id, timestamp: '2026-11-21T00:00:00Z', provider_instance_name: 'p', exact_model: 'm@p', requested_model: 'm', api_type: 'image.txt2img', tenant_id: 't', usage, status: 'success', finance_snapshot: currency ? { currency, amount: 0.1 } : undefined })

Deno.test('media quantities preserve missing versus zero and keep currencies separate', () => {
  const events = [event('one', { image_units: 2, audio_seconds: 1.25, input_tokens: 0 }, 'USD'), event('two', { image_units: 0 }, 'EUR'), event('three', {}, undefined)]
  const total = aggregateUsageEvents(events)
  assert(total.total_tokens === 0, 'media requests are not converted to tokens')
  assert(total.dimensions.image_units.amount === 2 && total.dimensions.image_units.reported_events === 2, 'zero is present, missing is not')
  assert(total.dimensions.audio_seconds.reported_events === 1, 'fractional duration keeps coverage')
  assert(!('video_seconds' in total.dimensions), 'absent dimension remains hidden')
  assert(total.finance_totals.length === 2 && total.unpriced_requests === 1 && !total.finance_complete, 'partial multi-currency costs are explicit')
  const grouped = groupUsageEvents(events)
  assert(JSON.stringify(grouped[0].aggregate) === JSON.stringify(total), 'group aggregate equals full filtered set')
  assert(mergeUsageTotals(grouped.map((row) => row.aggregate)).total_requests === 3, 'grouping does not lose records')
})

Deno.test('invalid quantities never become zero or billable usage', () => {
  const dimensions = eventUsageDimensions(event('invalid', { image_units: -1, audio_seconds: NaN, video_seconds: Infinity, compute_seconds: 0 }))
  assert(Object.keys(dimensions).join() === 'compute_seconds', 'only the explicit finite zero remains')
  const total = toUsageTotals({ total_requests: 1, dimensions: { image_units: { amount: 0, reported_events: 0 } }, finance_complete: false, unpriced_requests: 1 })
  assert(Object.keys(total.dimensions).length === 0, 'no observations are hidden')
})

Deno.test('token subdimensions remain independent and are never summed into total tokens', () => {
  const sample = event('tokens', { input_tokens: 100, output_tokens: 20, total_tokens: 120, cache_read_input_tokens: 80, reasoning_tokens: 10, video_input_tokens: 5 }, 'USD')
  sample.token_equivalent = 120
  const total = aggregateUsageEvents([sample])
  assert(total.total_tokens === 120, 'cache, reasoning and video are not counted twice')
  assert(total.dimensions.reasoning_tokens.amount === 10 && total.dimensions.video_input_tokens.amount === 5, 'subsets remain inspectable')
})
