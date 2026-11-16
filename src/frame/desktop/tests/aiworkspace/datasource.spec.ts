/* Data source state and a table view's row limit (AI Native 余下任务 §7.2: B0-2, B0-3, B7-3): a data source defined before
 * its data shows as pending — its definition, never an empty table — and its failed first attempt with the reason; a
 * table view Block may show only its first rows, and "show all rows" takes the limit away. A source written by an Agent's
 * sync task (§8.2): the owner manages the task on the Agent (stubbed here: the standalone backend has no Agent), others see
 * which task it is and AIWS's last attempt; "sync now" is the refresh, which needs the Agent. */
import { devices, type Page } from '@playwright/test'
import { expect, openWorkspace, openWorkspacePage, test, type Api } from './fixtures'

const ALICE = 'tok-alice'

test('B0-3: a data source without data is pending, shows its definition and its last attempt', async ({ page, api }) => {
  const ws = (await api.rpc(ALICE, 'ws.create', { title: `quotes ${Date.now()}` })) as { workspace_id: string; epoch: string }
  await api.commit(ALICE, ws, [{ op: 'entity.create', entity_id: 'quotes', type_id: 'buckyos.table-source', parent_id: 'data', order_key: 'q', name: 'quotes',
    payload: { fields: [{ field_id: 'symbol', name: 'Symbol', type: 'text' }, { field_id: 'close', name: 'Close', type: 'number' }],
      source: { kind: 'zone', path: '/home/alice/sync/quotes.json', def: { meaning: 'Daily closing prices', primary_key: ['symbol'], limit: 100 } } } }])
  await openWorkspace(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-top-sources').click()
  await page.getByTestId('aiws-tree-quotes').click()
  const panel = page.getByTestId('aiws-source')
  await expect(panel).toHaveAttribute('data-state', 'pending', { timeout: 30_000 })
  await expect(panel).toHaveAttribute('data-kind', 'zone')
  await expect(page.getByTestId('aiws-source-pending')).toBeVisible()
  await expect(page.getByTestId('aiws-source-meaning')).toContainText('Daily closing prices')
  await expect(page.locator('[data-testid^="aiws-table-pending-"]').first()).toBeVisible()
  // this deployment reads no Zone files: the first sync fails, says why, and the source stays pending
  await page.getByTestId('aiws-source-refresh').click()
  await expect(page.getByTestId('aiws-source-attempt')).toHaveAttribute('data-status', 'failed', { timeout: 30_000 })
  await expect(page.getByTestId('aiws-source-attempt')).toContainText('not readable')
  await expect(panel).toHaveAttribute('data-state', 'pending')
})

test('B7-3: a table view shows its first rows, and all of them again', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `rows ${Date.now()}`)
  const rev = ((await api.rpc(ALICE, 'doc.read', { workspace_id: ws.workspace_id, entity_id: 'cell-all-tasks' })).content.key_revs.row_limit ?? 0) as number
  await api.commit(ALICE, ws, [{ op: 'entity.set_keys', entity_id: 'cell-all-tasks', keys: [{ key: 'row_limit', value: 2, expect: { rev } }] }])
  await openWorkspace(page, ALICE, ws.workspace_id)
  const all = page.getByTestId('aiws-table-cell-all-tasks')
  await expect(all.getByTestId('aiws-row')).toHaveCount(2)
  await expect(all.getByTestId('aiws-table-count-cell-all-tasks')).toHaveAttribute('data-limited', 'true')
  await all.getByTestId('aiws-table-menu-cell-all-tasks').click()
  await page.getByTestId('aiws-ctl-menu-table-rows').click()
  await expect(all.getByTestId('aiws-row')).toHaveCount(5)
  await expect.poll(async () => (await api.rpc(ALICE, 'doc.read', { workspace_id: ws.workspace_id, entity_id: 'cell-all-tasks' })).content.payload.row_limit ?? null).toBe(null)
})

const BOB = 'tok-bob'
const AGENT = 'bot.example'
const OUT = `/home/alice/agents/${AGENT}/sync/quotes/out.json`

async function syncedQuotes(api: Api, title: string) {
  const ws = (await api.rpc(ALICE, 'ws.create', { title })) as { workspace_id: string; epoch: string }
  await api.commit(ALICE, ws, [{ op: 'entity.create', entity_id: 'quotes', type_id: 'buckyos.table-source', parent_id: 'data', order_key: 'q', name: 'quotes',
    payload: { fields: [{ field_id: 'symbol', name: 'Symbol', type: 'text' }, { field_id: 'close', name: 'Close', type: 'number' }],
      source: { kind: 'zone', path: OUT, refresh: { interval_s: 900 }, sync: { agent_id: AGENT, task_id: 'quotes' }, def: { meaning: 'Daily closing prices', primary_key: ['symbol'] } } } }])
  return ws
}

/** The Agent's `sync.*` kRPC, as the Agent Service answers it (§8.2): every call is kept. */
async function stubAgent(page: Page) {
  const calls: { method: string; params: Record<string, unknown> }[] = []
  let task: Record<string, unknown> = { task_id: 'quotes', title: 'Daily quotes', version: 3, hosts: ['api.exchange.example'], schedule: { every_secs: 900 }, inputs: {},
    secrets: [{ name: 'API_KEY', set: false }], enabled: true, state: 'failing', output: { path: OUT }, failures: 2,
    last_run: { run_id: 'r9', at: '2026-11-16T08:00:00Z', status: 'failed', kind: 'auth', error: 'the exchange refused the key', trigger: 'schedule' } }
  await page.route(`**/kapi/${AGENT}`, async (route) => {
    const body = route.request().postDataJSON() as { method: string; params: Record<string, unknown>; sys?: unknown[] }
    calls.push({ method: body.method, params: body.params })
    if (body.method === 'sync.set_enabled') task = { ...task, enabled: body.params.enabled, state: body.params.enabled ? 'ok' : 'disabled' }
    if (body.method === 'sync.set_secret') task = { ...task, secrets: [{ name: body.params.name, set: body.params.value !== null }] }
    await route.fulfill({ contentType: 'application/json', body: JSON.stringify({ result: task, sys: [body.sys?.[0] ?? 0] }) })
  })
  return calls
}

test('§8.2: the owner sees the sync task on the Agent, turns it off, sets its secret without ever reading it, and syncs now', async ({ page, api }) => {
  const ws = await syncedQuotes(api, `synced ${Date.now()}`)
  const calls = await stubAgent(page)
  await openWorkspace(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-top-sources').click()
  await page.getByTestId('aiws-tree-quotes').click()
  const task = page.getByTestId('aiws-sync-task')
  await expect(task).toHaveAttribute('data-agent', AGENT, { timeout: 30_000 })
  await expect(task).toHaveAttribute('data-enabled', 'true')
  await expect(page.getByTestId('aiws-sync-summary')).toContainText('Daily quotes')
  await expect(page.getByTestId('aiws-sync-summary')).toHaveAttribute('data-state', 'failing')
  await expect(page.getByTestId('aiws-sync-hosts')).toContainText('api.exchange.example')
  await expect(page.getByTestId('aiws-sync-last-run')).toHaveAttribute('data-status', 'failed')
  await expect(page.getByTestId('aiws-sync-last-run')).toContainText('the exchange refused the key')
  await expect(page.getByTestId('aiws-sync-failures')).toBeVisible()
  expect(calls[0]).toEqual({ method: 'sync.get', params: { task_id: 'quotes' } })

  await page.getByTestId('aiws-sync-toggle').click()
  await expect(task).toHaveAttribute('data-enabled', 'false')
  expect(calls).toContainEqual({ method: 'sync.set_enabled', params: { task_id: 'quotes', enabled: false } })

  // a secret is written, never read back: the field empties and only says it is set
  const secret = page.getByTestId('aiws-sync-secret-API_KEY')
  await expect(secret).toHaveAttribute('data-set', 'false')
  await page.getByTestId('aiws-sync-secret-input-API_KEY').fill('sk-very-secret')
  await page.getByTestId('aiws-sync-secret-save-API_KEY').click()
  await expect(secret).toHaveAttribute('data-set', 'true')
  await expect(page.getByTestId('aiws-sync-secret-input-API_KEY')).toHaveValue('')
  expect(calls).toContainEqual({ method: 'sync.set_secret', params: { task_id: 'quotes', name: 'API_KEY', value: 'sk-very-secret' } })
  expect(await page.locator('body').innerText()).not.toContain('sk-very-secret')
  await page.getByTestId('aiws-sync-secret-clear-API_KEY').click()
  await expect(secret).toHaveAttribute('data-set', 'false')
  expect(calls).toContainEqual({ method: 'sync.set_secret', params: { task_id: 'quotes', name: 'API_KEY', value: null } })

  // "sync now" is the source's refresh: the service asks the Agent first — this backend reaches none, so it fails and says so
  const reads = calls.filter((c) => c.method === 'sync.get').length
  await page.getByTestId('aiws-source-refresh').click()
  await expect(page.getByTestId('aiws-source-attempt')).toHaveAttribute('data-status', 'failed', { timeout: 30_000 })
  await expect(page.getByTestId('aiws-source-attempt')).toContainText('no Agent is reachable')
  await expect(page.getByTestId('aiws-source')).toHaveAttribute('data-state', 'pending')
  await expect.poll(() => calls.filter((c) => c.method === 'sync.get').length).toBeGreaterThan(reads)
})

test('§8.2: someone with run sees which task writes the source and AIWS\'s last attempt, and syncs now; the Agent is never asked from their page', async ({ page, api }) => {
  const ws = await syncedQuotes(api, `synced shared ${Date.now()}`)
  await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: 'bob', resource: '/', allow: ['read', 'run'] }] })
  const calls = await stubAgent(page)
  await openWorkspacePage(page, BOB, ws.workspace_id)
  await page.getByTestId('aiws-top-sources').click()
  await page.getByTestId('aiws-tree-quotes').click()
  await expect(page.getByTestId('aiws-sync-task')).toHaveAttribute('data-task', 'quotes', { timeout: 30_000 })
  await expect(page.getByTestId('aiws-sync-task')).toContainText(AGENT)
  await expect(page.getByTestId('aiws-sync-summary')).toHaveCount(0)
  await expect(page.getByTestId('aiws-sync-toggle')).toHaveCount(0)
  await page.getByTestId('aiws-source-refresh').click()
  await expect(page.getByTestId('aiws-source-attempt')).toHaveAttribute('data-status', 'failed', { timeout: 30_000 })
  await expect(page.getByTestId('aiws-source-attempt')).toContainText('no Agent is reachable')
  expect(calls).toEqual([])
})

test.describe('on a phone', () => {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars -- the browser comes from the project, the rest is the phone
  const { defaultBrowserType, ...pixel } = devices['Pixel 7']
  test.use(pixel)

  test('§8.2: the owner sees the sync task within the phone width of the properties panel', async ({ page, api }) => {
    const ws = await syncedQuotes(api, `synced phone ${Date.now()}`)
    await stubAgent(page)
    await openWorkspacePage(page, ALICE, ws.workspace_id)
    await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-phone', 'true')
    await page.getByTestId('aiws-main-menu').tap()
    await page.getByTestId('aiws-menu-data-mode').tap()
    await page.getByTestId('aiws-tree-quotes').tap()
    await page.getByTestId('aiws-dm-phone-more').tap()
    await page.getByTestId('aiws-dm-phone-side-properties').tap()
    const task = page.getByTestId('aiws-sync-task')
    await expect(task).toHaveAttribute('data-enabled', 'true', { timeout: 30_000 })
    await task.scrollIntoViewIfNeeded()
    await page.screenshot({ path: 'test-results/aiworkspace-phone/sync-task.png' })
    const width = page.viewportSize()!.width
    for (const id of ['aiws-sync-task', 'aiws-sync-toggle', 'aiws-sync-secret-input-API_KEY', 'aiws-sync-secret-save-API_KEY', 'aiws-source-refresh']) {
      const box = (await page.getByTestId(id).boundingBox())!
      expect(box.x, id).toBeGreaterThanOrEqual(0)
      expect(box.x + box.width, id).toBeLessThanOrEqual(width)
    }
    expect((await page.getByTestId('aiws-sync-toggle').boundingBox())!.height).toBeGreaterThanOrEqual(44)
    expect(await page.getByTestId('aiws-source').evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true)
  })
})
