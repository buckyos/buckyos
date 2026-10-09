import { expect, editCell, grantPersistence, openCard, openDesktop, prepareOffline, saveSummary, serviceWorkerReady, test } from './offline-fixtures'
import { closeWorkspace, openMock, openStatus, backToList } from './fixtures'

const ALICE = 'tok-alice'

test('Block snapshots are cached and displayed after an offline cold start', async ({ page, context, api, net }) => {
  const ws = await api.sample(ALICE, `snapshot-offline ${Date.now()}`)
  const bytes = Buffer.from('<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40"><rect width="40" height="40" fill="red"/></svg>')
  const begin = await api.rpc(ALICE, 'asset.begin_upload', { workspace_id: ws.workspace_id, size: bytes.length })
  await fetch(`${api.base}/upload/${begin.upload_id}`, { method: 'PUT', headers: { authorization: `Bearer ${ALICE}` }, body: bytes })
  const asset = await api.rpc(ALICE, 'asset.finish_upload', { workspace_id: ws.workspace_id, upload_id: begin.upload_id })
  expect((await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'snapshot-def', type_id: 'buckyos.block-def', parent_id: 'data', order_key: 'zz', payload: { def_id: 'test.snapshot', kind: 'html', allow_no_source: true, html: { html: '<div/>', js: 'aiws.ready()' } } },
    { op: 'entity.create', entity_id: 'snapshot-cell', type_id: 'buckyos.cell', parent_id: 'surface-main', order_key: 'zz', payload: { view: { type: 'html' }, def_ref: { entity_id: 'snapshot-def' }, config: { snapshot: { object_id: asset.object_id } } } },
  ])).status).toBe('accepted')
  await api.rpc(ALICE, 'ws.set_user_state', { workspace_id: ws.workspace_id, entries: { 'canvas:mode': 'view' } })
  await prepareOffline(page, net, ALICE, ws.workspace_id)
  await serviceWorkerReady(page)
  await expect(page.getByTestId('aiws-prepare-report')).toContainText('资产已缓存 2 个')
  await net.down()
  await context.setOffline(true)
  await page.close()
  const cold = await context.newPage()
  await cold.goto(`${net.origin}/?scenario=normal`)
  await cold.getByTestId('desktop-app-aiworkspace').click()
  await backToList(cold)
  await expect(cold.getByTestId('aiws-prepared-list')).toBeVisible({ timeout: 30_000 })
  await openCard(cold, ws.workspace_id)
  const snapshot = cold.getByTestId('aiws-html-static-snapshot-cell').locator('img')
  await expect(snapshot).toBeVisible()
  await expect.poll(() => snapshot.evaluate((image: HTMLImageElement) => image.naturalWidth)).toBe(40)
  await expect(cold.locator('.aiws-html-frame')).toHaveCount(0)
})

test('an offline cold start of a workspace address opens the prepared replica in a tab of its own', async ({ page, context, api, net }) => {
  const ws = await api.sample(ALICE, `tab-offline ${Date.now()}`)
  await prepareOffline(page, net, ALICE, ws.workspace_id)
  await serviceWorkerReady(page)
  await page.close()
  await net.down()
  await context.setOffline(true)
  const cold = await context.newPage()
  await cold.goto(`${net.origin}/workspace/${ws.workspace_id}`)
  expect(await cold.evaluate(() => navigator.serviceWorker.controller !== null)).toBe(true)
  await expect(cold.getByTestId('aiws-tab').getByTestId('aiws-workspace')).toHaveAttribute('data-workspace-id', ws.workspace_id, { timeout: 30_000 })
  await expect(cold.getByTestId('aiws-mode')).toHaveAttribute('data-mode', 'replica')
})

test('V15 prepare offline, cut the network, edit, close, cold start offline, reconnect: accepted exactly once', async ({ page, context, api, net }) => {
  const ws = await api.sample(ALICE, `v15 ${Date.now()}`)
  await prepareOffline(page, net, ALICE, ws.workspace_id)
  await serviceWorkerReady(page)
  // the one asset of the sample was pulled into the offline asset area; the application itself is cached too
  await expect(page.getByTestId('aiws-prepare-report')).toContainText('资产已缓存 1 个')
  await expect(page.getByTestId('aiws-app-cache-missing')).toHaveCount(0)
  // served without COOP/COEP: the page is not cross-origin isolated, and opfs-sahpool does not need it
  expect(await page.evaluate(() => window.crossOriginIsolated)).toBe(false)
  const headBefore = await api.headSeq(ALICE, ws.workspace_id)

  // ---- the network goes away for real: connection refused for the page, the Worker and the service worker; the browser reports offline too
  await net.down()
  await context.setOffline(true)
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'offline', { timeout: 40_000 })
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-browser-offline', 'true')

  // ---- edit a table cell and type into the rich text
  const cell = await editCell(page, 'task-40', '负责人', 'owner', '离线改的负责人')
  await expect(cell).toHaveAttribute('data-state', 'saved_locally')
  await expect(cell.getByTestId('aiws-edit-state')).toHaveText('已保存到本设备')
  await expect(cell).toContainText('离线改的负责人')
  const end = page.getByTestId('aiws-richtext-notes').locator('[data-block-id="n-end"]')
  await end.click()
  await page.keyboard.press('End')
  await page.keyboard.type(' 离线写的文字')
  await openStatus(page)
  await expect(page.locator('[data-testid="aiws-edit-entry"][data-edit-id="rt:notes"]')).toHaveAttribute('data-state', 'saved_locally')
  expect((await saveSummary(page)).unsaved).toBe(0)
  const queued = (await saveSummary(page)).pending
  expect(queued).toBeGreaterThanOrEqual(2)
  // nothing reached the backend
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(headBefore)

  // ---- close the page; open a new one while still offline: a cold start through the service worker
  await page.close()
  const cold = await context.newPage()
  await cold.goto(`${net.origin}/?scenario=normal`)
  await cold.getByTestId('desktop-app-aiworkspace').click()
  await backToList(cold)
  await expect(cold.getByTestId('aiws-prepared-list')).toBeVisible({ timeout: 30_000 })
  await expect(cold.getByTestId('aiws-list-error')).toBeVisible()
  // the page did not come from the network: there is none
  expect(await cold.evaluate(() => fetch('/kapi/aiworkspace/healthz').then(() => 'reachable', () => 'unreachable'))).toBe('unreachable')
  expect(await cold.evaluate(() => navigator.serviceWorker.controller !== null)).toBe(true)
  await openCard(cold, ws.workspace_id)
  await expect(cold.getByTestId('aiws-mode')).toHaveAttribute('data-mode', 'replica')
  await expect(cold.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'offline', { timeout: 40_000 })
  // content and the pending queue are there
  const coldCell = cold.getByTestId('aiws-table-cell-all-tasks').getByTestId('aiws-cell-task-40-owner')
  await expect(coldCell).toContainText('离线改的负责人')
  await expect(coldCell).toHaveAttribute('data-state', 'saved_locally')
  await expect(cold.getByTestId('aiws-richtext-notes').locator('[data-block-id="n-end"]')).toContainText('离线写的文字')
  expect((await saveSummary(cold)).pending).toBe(queued)
  // the image comes from the asset area in OPFS
  await expect(cold.getByTestId('aiws-asset-image')).toBeVisible()
  expect(await cold.getByTestId('aiws-asset-image').evaluate((image: HTMLImageElement) => image.naturalWidth)).toBeGreaterThan(0)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(headBefore)

  // ---- reconnect
  await context.setOffline(false)
  await net.up()
  await expect(cold.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live', { timeout: 40_000 })
  await expect.poll(async () => (await saveSummary(cold)).pending, { timeout: 30_000 }).toBe(0)
  const after = await saveSummary(cold)
  expect([after.unsaved, after.local, after.attention]).toEqual([0, 0, 0])
  // exactly once: one commit per queued submission, and the values are what was typed
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(headBefore + queued)
  expect((await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-40', 'owner')).value).toBe('离线改的负责人')
  const text = JSON.stringify(await api.ast(ALICE, ws.workspace_id, 'notes'))
  expect(text.split('离线写的文字').length - 1).toBe(1)
  const changes = await api.rpc(ALICE, 'doc.get_changes', { workspace_id: ws.workspace_id, epoch: ws.epoch, after_seq: headBefore })
  const keys = (changes.changes as { idempotency_key: string }[]).map((change) => change.idempotency_key)
  expect(new Set(keys).size).toBe(queued)
  await expect(coldCell).toContainText('离线改的负责人')
})

test('opening the app offline without anything prepared says so; it is not a blank page', async ({ page, context, net }) => {
  await openDesktop(page, net, ALICE)
  await expect(page.getByTestId('aiws-list')).toBeVisible()
  await serviceWorkerReady(page)
  await net.down()
  await page.close()
  const cold = await context.newPage()
  await cold.goto(`${net.origin}/?scenario=normal`)
  await cold.getByTestId('desktop-app-aiworkspace').click()
  await backToList(cold)
  await expect(cold.getByTestId('aiws-list-error')).toBeVisible({ timeout: 30_000 })
  await expect(cold.getByTestId('aiws-prepared-list')).toContainText('浏览器网络在线，但连接不到 BuckyOS 后台')
  await expect(cold.getByTestId('aiws-prepared-list')).toContainText('本设备上没有准备过离线的工作区')
})

test('V15 application resources missing from the cache: a clear state instead of a blank page; local data untouched', async ({ page, context, api, net }) => {
  const ws = await api.sample(ALICE, `v15-missing ${Date.now()}`)
  await prepareOffline(page, net, ALICE, ws.workspace_id)
  await serviceWorkerReady(page)
  await net.down()
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'offline', { timeout: 40_000 })
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-browser-offline', 'false') // the browser has a network, BuckyOS is unreachable
  const cell = await editCell(page, 'task-40', '负责人', 'owner', '资源丢失前的输入')
  await expect(cell).toHaveAttribute('data-state', 'saved_locally')
  // the browser evicts one cached application file (here: the Replica Worker's script)
  const removed = await page.evaluate(async () => {
    for (const name of await caches.keys()) {
      const cache = await caches.open(name)
      const victim = (await cache.keys()).find((request) => /replica\.worker/.test(request.url))
      if (victim && await cache.delete(victim)) return victim.url
    }
    return null
  })
  expect(removed).toMatch(/replica\.worker/)
  await page.close()

  const cold = await context.newPage()
  const response = await cold.goto(`${net.origin}/?scenario=normal`)
  expect(response?.status()).toBe(503)
  await expect(cold.getByTestId('desktop-offline-unavailable')).toBeVisible()
  await expect(cold.getByTestId('desktop-offline-unavailable')).toContainText('无法离线启动')
  await expect(cold.getByTestId('desktop-offline-unavailable')).toContainText('缺少 1 个文件')
  await expect(cold.getByTestId('desktop-offline-unavailable')).toContainText('离线数据没有被改动')

  // with the network back the application loads again and the input saved before is still queued, then sent
  await net.up()
  await cold.getByRole('button', { name: /重新载入/ }).click()
  await cold.getByTestId('desktop-app-aiworkspace').click()
  await backToList(cold)
  await openCard(cold, ws.workspace_id)
  await expect(cold.getByTestId('aiws-mode')).toHaveAttribute('data-mode', 'replica')
  await expect.poll(async () => (await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-40', 'owner')).value, { timeout: 30_000 }).toBe('资源丢失前的输入')
  await expect.poll(async () => (await saveSummary(cold)).pending, { timeout: 30_000 }).toBe(0)
})

test('persistent storage refused by the browser: offline is unavailable with the reason, the window stays online direct', async ({ page, api, net }) => {
  const ws = await api.sample(ALICE, `no-persist ${Date.now()}`)
  await openDesktop(page, net, ALICE)
  await openCard(page, ws.workspace_id)
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
  await openStatus(page)
  await page.getByTestId('aiws-prepare-offline').click()
  await expect(page.getByTestId('aiws-offline-unavailable')).toContainText('离线不可用')
  await expect(page.getByTestId('aiws-offline-unavailable')).toContainText('持久化存储')
  await expect(page.getByTestId('aiws-mode')).toHaveAttribute('data-mode', 'direct')
  // nothing was stored and editing works as before, directly against the backend
  expect(await page.evaluate(() => window.localStorage.getItem('aiworkspace.offline.index'))).toBeNull()
  const cell = await editCell(page, 'task-40', '负责人', 'owner', '直连修改')
  await expect(cell).toContainText('直连修改')
  await expect.poll(async () => (await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-40', 'owner')).value).toBe('直连修改')
})

test('a reader without workspace-level read cannot prepare: the backend\'s refusal is shown', async ({ page, api, net }) => {
  const ws = await api.sample(ALICE, `scoped ${Date.now()}`)
  await api.rpc(ALICE, 'ws.grant', { workspace_id: ws.workspace_id, subject: 'bob', scope_entity_id: 'surface-main', capabilities: ['read'] })
  await openDesktop(page, net, 'tok-bob')
  await grantPersistence(page.context(), page)
  await openCard(page, ws.workspace_id)
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
  await openStatus(page)
  await page.getByTestId('aiws-prepare-offline').click()
  await expect(page.getByTestId('aiws-offline-unavailable')).toContainText('PERMISSION_DENIED')
  await expect(page.getByTestId('aiws-mode')).toHaveAttribute('data-mode', 'direct')
})

test('offline: undo removes an unsent submission without the network; Mock run and write locks say they need the backend', async ({ page, api, net }) => {
  const ws = await api.sample(ALICE, `offline-misc ${Date.now()}`)
  const meta = (await api.rpc(ALICE, 'doc.read', { workspace_id: ws.workspace_id, entity_id: 'project-info' })).meta_rev
  expect((await api.commit(ALICE, ws, [{ op: 'entity.set_write_policy', entity_id: 'project-info', policy: 'lock_required', expect: { rev: meta } }])).status).toBe('accepted')
  // a URL query table: its rows live at the source, the replica holds only the definition
  expect((await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'events', type_id: 'buckyos.table-source', parent_id: 'data', order_key: 'v', payload: {
      data_mode: 'url_query', source_ref: { kind: 'url_query', source_url: 'fixture://events?rows=1000&snapshot=1', query: {} },
      fields: [{ field_id: 'event_id', name: '事件', type: 'text' }] } },
    { op: 'entity.create', entity_id: 'cell-events', type_id: 'buckyos.cell', parent_id: 'surface-main', order_key: 'w', payload: { source_ref: { entity_id: 'events' }, view: { type: 'table' }, title: '事件（URL 查询表）' } },
  ])).status).toBe('accepted')
  await prepareOffline(page, net, ALICE, ws.workspace_id)
  const head = await api.headSeq(ALICE, ws.workspace_id)
  // online, the holder window forwards the query of this table to the backend (no error is shown)
  await expect(page.getByTestId('aiws-table-cell-events')).toBeVisible()
  await expect(page.getByTestId('aiws-table-error-cell-events')).toHaveCount(0)
  await net.down()
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'offline', { timeout: 40_000 })

  // a pending submission is an undo entry of its own: undoing it is a local removal
  const cell = await editCell(page, 'task-40', '负责人', 'owner', '将被撤销')
  await expect(cell).toHaveAttribute('data-state', 'saved_locally')
  expect((await saveSummary(page)).pending).toBe(1)
  await expect(page.getByTestId('aiws-undo')).toHaveText('撤销 1')
  await page.getByTestId('aiws-undo').click()
  await expect.poll(async () => (await saveSummary(page)).pending).toBe(0)
  await expect(cell).toContainText('林')
  await expect(cell).not.toContainText('将被撤销')
  await expect(page.getByTestId('aiws-undo-problem')).toHaveCount(0)
  // redo queues the same operations again
  await page.getByTestId('aiws-redo').click()
  await expect.poll(async () => (await saveSummary(page)).pending).toBe(1)
  await expect(cell).toContainText('将被撤销')

  // a lock_required object is read-only while the backend is unreachable
  const lock = page.getByTestId('aiws-lock-project-info')
  await lock.getByTestId('aiws-lock-acquire').click()
  await expect(page.getByTestId('aiws-notice').filter({ hasText: '需要写锁的对象在后台不可达时只读' })).toBeVisible()
  await expect(lock).toHaveAttribute('data-held', 'false')
  await expect(page.getByTestId('aiws-record-project-info').getByRole('button').first()).toBeDisabled()

  // the Mock run happens on the backend: offline it says so and produces nothing
  await openMock(page)
  await page.getByTestId('aiws-mock-start').click()
  await expect(page.getByTestId('aiws-mock-message')).toContainText('没有运行')
  await expect(page.getByTestId('aiws-mock-run')).toHaveCount(0)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head)

  // a URL query table opened offline: no rows are invented, the view says its data needs the backend
  await closeWorkspace(page)
  await openCard(page, ws.workspace_id)
  await expect(page.getByTestId('aiws-mode')).toHaveAttribute('data-mode', 'replica')
  await expect(page.getByTestId('aiws-table-error-cell-events')).toContainText('URL 查询表')
  await expect(page.getByTestId('aiws-table-error-cell-events')).toContainText('当前离线')
  await expect(page.getByTestId('aiws-table-cell-events').getByTestId('aiws-row')).toHaveCount(0)
  await expect(page.getByTestId('aiws-table-cell-all-tasks').getByTestId('aiws-cell-task-40-owner')).toContainText('将被撤销')

  await net.up()
  await expect.poll(async () => (await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-40', 'owner')).value, { timeout: 40_000 }).toBe('将被撤销')
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
})
