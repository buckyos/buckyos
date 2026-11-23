import { buckyos } from 'buckyos'
import { createAICCMgr, mergeUsageOverview, type UsageScope } from '../../src/api/aicc_mgr.ts'

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message)
}

Deno.test('usage overview and event requests apply the same explicit scope across every aggregate', async () => {
  const sdk = buckyos as unknown as {
    getAccountInfo: () => Promise<unknown>
    getServiceRpcClient: (service: string) => unknown
  }
  const originalAccountInfo = sdk.getAccountInfo
  const originalClient = sdk.getServiceRpcClient
  const sent: Record<string, unknown>[] = []
  const aggregate = {
    total_requests: 3,
    total_tokens: 120,
    finance_totals: [{ currency: 'usd', amount: 0.25 }, { currency: 'EUR', amount: 0.5 }],
    finance_complete: false,
    dimensions: { input_tokens: { amount: 100, reported_events: 3 }, image_units: { amount: 1, reported_events: 1 } },
  }
  sdk.getAccountInfo = async () => ({ session_token: 'test-session' })
  sdk.getServiceRpcClient = () => ({
    call: async (method: string, params: Record<string, unknown>) => {
      assert(method === 'usage.query', 'usage reads must not require provider management RPCs')
      sent.push(params)
      const key = (params.group_by as string[] | undefined)?.[0]
      return {
        total: aggregate,
        grouped: key ? [{ group: { [key]: key === 'user_id' ? 'bob' : key === 'caller_app_id' ? null : 'model@provider' }, aggregate }] : [],
        buckets: params.time_bucket ? [{ bucket_start_ms: 1_700_000_000_000, aggregate }] : [],
        events: params.output_mode === 'summary_and_events' ? [{
          event_id: 'event-bob', tenant_id: 'tenant-bob', user_id: 'bob', created_at_ms: 1_700_000_000_000,
          provider_model: 'model@provider', total_tokens: 120,
        }] : [],
      }
    },
  })
  try {
    const manager = createAICCMgr({ useMock: false })
    const initialSummary = manager.getUsageSummary()
    const scopes: UsageScope[] = [{}, { zoneWide: true }, { zoneWide: true, userId: 'bob' }]
    for (const scope of scopes) {
      sent.length = 0
      const overview = await manager.queryUsageOverview(scope)
      assert(overview.summary.total_tokens === 120 && overview.summary.total_requests === 3, 'aggregate totals are preserved')
      assert(overview.summary.today_tokens === 120 && overview.summary.this_month_tokens === 120, 'short windows are queried in the same scope')
      assert(overview.users[0].user_id === 'bob', 'user breakdown uses user_id, not tenant_id')
      assert(overview.users[0].finance_totals.length === 2, 'per-user costs preserve separate currencies')
      assert(!overview.users[0].finance_complete, 'incomplete per-user costs remain marked incomplete')
      assert(overview.trend[0].tokens === 120, 'trend carries scoped aggregates')
      for (const period of Object.values(overview.periods)) assert(period.total_tokens === 120 && period.dimensions.image_units.amount === 1 && period.dimensions.input_tokens.reported_events === 3, 'every home period preserves its own full usage dimensions and coverage')
      assert(overview.breakdowns.some((row) => row.group.caller_app_id === 'system'), 'missing caller app uses the same system group as the home summary')
      assert(overview.breakdowns.length === 4 && overview.breakdowns[0].aggregate.finance_totals.length === 2, 'home breakdowns preserve scoped aggregate currencies instead of estimating from recent rows')
      const page = await manager.queryUsageEvents({
        scope, timeRange: { startTimeMs: 0, endTimeMs: Date.now() }, limit: 10,
        filters: { providerInstanceNames: ['provider'] },
      })
      assert(page.events[0].user_id === 'bob' && page.events[0].tenant_id === 'tenant-bob', 'event ownership and tenant remain distinct')
      assert(sent.length === 9, 'all eight independent overview slices plus events are requested')
      for (const params of sent) {
        assert(params.zone_wide === (scope.zoneWide === true), 'every request carries the selected system/personal boundary')
        const filters = params.filters as Record<string, unknown>
        assert(JSON.stringify(filters.user_ids) === JSON.stringify(scope.userId ? [scope.userId] : undefined), 'every slice carries the selected user')
      }
      assert((sent.at(-1)?.filters as { provider_instance_names: string[] }).provider_instance_names[0] === 'provider', 'scope composes with existing detail filters')
      assert(manager.getUsageSummary() === initialSummary, 'viewing another user does not replace the cached personal summary')
    }
  } finally {
    sdk.getAccountInfo = originalAccountInfo
    sdk.getServiceRpcClient = originalClient
  }
})

Deno.test('usage overview rejects a failed slice instead of displaying an empty user breakdown', async () => {
  const sdk = buckyos as unknown as {
    getAccountInfo: () => Promise<unknown>
    getServiceRpcClient: (service: string) => unknown
  }
  const originalAccountInfo = sdk.getAccountInfo
  const originalClient = sdk.getServiceRpcClient
  sdk.getAccountInfo = async () => ({ session_token: 'test-session' })
  sdk.getServiceRpcClient = () => ({
    call: async (_method: string, params: Record<string, unknown>) => {
      if ((params.group_by as string[] | undefined)?.includes('user_id')) throw new Error('usage unavailable')
      return {}
    },
  })
  try {
    let error: unknown
    const completed: string[] = []
    const failed: string[] = []
    try {
      await createAICCMgr({ useMock: false }).queryUsageOverview({ zoneWide: true }, (progress) => (progress.failed ? failed : completed).push(progress.section))
    } catch (caught) {
      error = caught
    }
    assert(error instanceof Error && error.message === 'usage unavailable', 'failed group query must remain visible to the dashboard')
    assert(failed.join(',') === 'users' && completed.includes('today') && completed.length === 7, 'one failed card does not discard successful slices')
  } finally {
    sdk.getAccountInfo = originalAccountInfo
    sdk.getServiceRpcClient = originalClient
  }
})

Deno.test('mock usage scopes separate personal, system and per-user totals and events', async () => {
  const originalLocation = Object.getOwnPropertyDescriptor(globalThis, 'location')
  Object.defineProperty(globalThis, 'location', { configurable: true, value: { search: '?aiccScenario=populated' } })
  try {
    const manager = createAICCMgr({ useMock: true })
    const all = await manager.queryUsageOverview({ zoneWide: true })
    const own = await manager.queryUsageOverview({})
    const bob = await manager.queryUsageOverview({ zoneWide: true, userId: 'bob' })
    assert(all.users.length === 3, 'the fixture has multiple owners')
    assert(own.users.length === 1 && own.users[0].user_id === 'alice', 'personal usage belongs to the current account')
    assert(bob.users.length === 1 && bob.users[0].user_id === 'bob', 'selecting bob excludes other owners')
    assert(all.summary.total_tokens > own.summary.total_tokens && all.summary.total_tokens > bob.summary.total_tokens, 'system totals exceed each selected user')
    assert(all.users.reduce((total, user) => total + user.total_tokens, 0) === all.summary.total_tokens, 'per-user breakdown reconciles to the system aggregate')
    for (const [scope, owner] of [[{}, 'alice'], [{ zoneWide: true, userId: 'bob' }, 'bob']] as const) {
      const page = await manager.queryUsageEvents({ scope, timeRange: { startTimeMs: 0, endTimeMs: Date.now() }, limit: 200 })
      assert(page.events.length > 0 && page.events.every((event) => event.user_id === owner), 'event owner matches the selected scope')
    }
  } finally {
    if (originalLocation) Object.defineProperty(globalThis, 'location', originalLocation)
    else Reflect.deleteProperty(globalThis, 'location')
  }
})

Deno.test('usage pages omit aggregates while independent breakdowns keep scope and bounded groups', async () => {
  const sdk = buckyos as unknown as { getServiceRpcClient: (service: string) => unknown }
  const original = sdk.getServiceRpcClient
  const sent: Record<string, unknown>[] = []
  sdk.getServiceRpcClient = () => ({ call: (_method: string, params: Record<string, unknown>) => {
    sent.push(params)
    return Promise.resolve({ total: { total_requests: 30 }, events: [], grouped: [], next_cursor: 'next' })
  } })
  try {
    const manager = createAICCMgr({ useMock: false })
    const query = { scope: { zoneWide: true, userId: 'owner' }, timeRange: { startTimeMs: 1, endTimeMs: 5000 }, filters: { providerModels: ['model'] }, limit: 10 }
    await manager.queryUsageEvents({ ...query, pageOnly: true, cursor: 'page-two' })
    await manager.queryUsageEvents({ ...query, summaryOnly: true, groupBy: 'provider_model', groupLimit: 20, groupOffset: 20 })
    assert(sent[0].output_mode === 'page', 'pages use the aggregate-free backend path')
    assert(JSON.stringify(sent[0].group_by) === '[]', 'paging does not request grouped summaries')
    assert(sent[0].cursor === 'page-two' && sent[0].limit === 10, 'the server receives page cursor and limit')
    assert(sent[1].output_mode === 'groups' && JSON.stringify(sent[1].group_by) === '["provider_model"]', 'breakdowns group one dimension at a time')
    assert(sent[1].group_limit === 20 && sent[1].group_offset === 20, 'breakdown expansion is bounded and paginated')
    assert(JSON.stringify(sent[0].filters) === JSON.stringify(sent[1].filters), 'pages and summaries share exact filters')
    assert(sent.every((params) => params.zone_wide === true), 'both requests preserve the permission boundary')
  } finally { sdk.getServiceRpcClient = original }
})


Deno.test('home overview publishes completed slices before a slow model query finishes', async () => {
  const sdk = buckyos as unknown as { getServiceRpcClient: (service: string) => unknown }
  const original = sdk.getServiceRpcClient
  let release!: () => void
  const delayed = new Promise<void>((resolve) => { release = resolve })
  const seen: string[] = []
  const aggregate = { total_requests: 2, total_tokens: 100, finance_totals: [{ currency: 'USD', amount: 0.3 }], finance_complete: true }
  sdk.getServiceRpcClient = () => ({ call: async (_method: string, params: Record<string, unknown>) => {
    const group = (params.group_by as string[] | undefined)?.[0]
    if (group === 'provider_model') {
      assert(params.output_mode === 'groups' && params.group_limit === 20, 'model breakdown is bounded on the server')
      await delayed
    }
    return { total: aggregate, grouped: group ? [{ group: { [group]: 'example' }, aggregate }] : [], buckets: [] }
  } })
  try {
    const manager = createAICCMgr({ useMock: false })
    let complete = false
    const pending = manager.queryUsageOverview({}, ({ section }) => seen.push(section)).then((value) => { complete = true; return value })
    await new Promise((resolve) => setTimeout(resolve, 0))
    assert(seen.includes('today') && seen.includes('providers'), 'fast independent sections publish without waiting for models')
    assert(!complete && !seen.includes('models'), 'slow section remains pending')
    release()
    const overview = await pending
    assert(seen.length === 8 && seen.includes('models'), 'every section publishes its result')
    const expanded = { ...overview, summary: { ...overview.summary, by_model: { ...overview.summary.by_model, extra: 10 } } }
    const merged = mergeUsageOverview(expanded, { section: 'trend', overview })
    assert(merged.summary.by_model.extra === 10, 'late totals do not overwrite expanded model data')
    assert(merged.summary.total_tokens === 100, 'complete totals stay independent of limited groups')
  } finally { release(); sdk.getServiceRpcClient = original }
})
