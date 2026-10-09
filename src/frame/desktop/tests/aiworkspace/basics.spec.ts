import { readFileSync } from 'node:fs'
import { BACKEND_FIXTURE, BUNDLED_FIXTURE, VECTORS_PATH, expect, expectAllCommitted, openApp, openWorkspace, test, openSide } from './fixtures'

const ALICE = 'tok-alice'

test('(a) the app opens from the desktop shell, creates the sample and shows its 13 entities', async ({ page, api }) => {
  // the bundled fixture is the backend's fixture, byte for byte
  expect(readFileSync(BUNDLED_FIXTURE, 'utf8')).toBe(readFileSync(BACKEND_FIXTURE, 'utf8'))

  await openApp(page, ALICE)
  await expect(page.getByTestId('window-aiworkspace')).toBeVisible()
  const title = `样例 ${Date.now()}`
  // New → template: the sample is created as a new workspace and opened
  await page.getByTestId('aiws-new-template').click()
  await page.getByTestId('aiws-template-sample').click()
  await page.getByLabel('工作区标题', { exact: true }).fill(title)
  await page.getByTestId('aiws-create-template').click()
  await expect(page.getByTestId('aiws-workspace')).toBeVisible({ timeout: 30_000 })
  const workspaceId = await page.getByTestId('aiws-workspace').getAttribute('data-workspace-id') as string

  // the data tree (data-source mode): the data of design §3.8 (no Blocks, no system nodes), the canvas content area collapsed
  await page.getByTestId('aiws-top-sources').click()
  const items = page.getByTestId('aiws-tree-item')
  await expect(items).toHaveCount(8)
  const ids = await items.evaluateAll((nodes) => nodes.map((node) => node.getAttribute('data-entity-id')))
  expect([...ids].sort()).toEqual(['canvas-content', 'diagram', 'note-budget', 'notes', 'project-info', 'shows', 'task-42-details', 'tasks'].sort())
  // the outline has the 18 entities: root, the four system nodes, the Surface with its content folder, and the 11 of §3.8
  expect((await api.rpc(ALICE, 'doc.outline', { workspace_id: workspaceId })).entities).toHaveLength(18)
  await page.getByTestId('aiws-top-canvas').click()

  // the flow Surface shows cells in (order_key, entity_id) order; content entities are not on the page
  const frames = await page.locator('[data-testid^="aiws-cell-frame-"]').evaluateAll((nodes) => nodes.map((node) => node.getAttribute('data-cell-id')))
  expect(frames).toEqual(['cell-notes', 'cell-all-tasks', 'cell-open-tasks', 'cell-info', 'cell-diagram'])

  // every built-in object renders from the real backend
  await expect(page.getByTestId('aiws-table-cell-all-tasks').getByTestId('aiws-row')).toHaveCount(5)
  await expect(page.getByTestId('aiws-cell-frame-cell-open-tasks').getByTestId('aiws-row')).toHaveCount(4)
  await expect(page.getByTestId('aiws-richtext-notes')).toContainText('本周项目进展')
  await expect(page.getByTestId('aiws-embed-cell-open-tasks')).toContainText('定义对象格式')
  await expect(page.getByTestId('aiws-record-project-info')).toContainText('1200.00')
  // asset: fetched with the session token and shown through a blob URL
  const image = page.getByTestId('aiws-asset-image')
  await expect(image).toBeVisible()
  expect(await image.getAttribute('src')).toMatch(/^blob:/)
  await expect.poll(() => image.evaluate((element: HTMLImageElement) => element.naturalWidth)).toBe(16)
  await expect(page.getByTestId('aiws-asset-availability')).toHaveText('可用')
  // annotation with its anchor state (annotations side panel), and the mark on the annotated cell
  await openSide(page, 'annotations')
  await expect(page.getByTestId('aiws-annotation')).toContainText('请核对预算来源')
  await expect(page.getByTestId('aiws-anchor-state')).toHaveText('锚点有效')
  await expect(page.getByTestId('aiws-table-cell-all-tasks').getByTestId('aiws-cell-task-42-budget').getByTestId('aiws-cell-annotation')).toBeVisible()
  await expectAllCommitted(page)
})

test('(b) a table cell edit is persisted and both views of the source reflect it', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `b ${Date.now()}`)
  await openWorkspace(page, ALICE, ws.workspace_id)
  const all = page.getByTestId('aiws-table-cell-all-tasks')
  const open = page.getByTestId('aiws-cell-frame-cell-open-tasks')
  await expect(all.getByTestId('aiws-row')).toHaveCount(5)

  // text
  await all.getByTestId('aiws-cell-task-41-title').getByRole('button').first().click()
  const input = page.getByLabel('任务 task-41', { exact: true })
  await input.fill('定义对象格式（已评审）')
  await input.press('Enter')
  await expect(all.getByTestId('aiws-cell-task-41-title')).toContainText('定义对象格式（已评审）')
  await expect(open.getByTestId('aiws-cell-task-41-title')).toContainText('定义对象格式（已评审）')
  await expectAllCommitted(page)
  expect((await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-41', 'title')).value).toBe('定义对象格式（已评审）')

  // decimal: the backend normalises, the view shows the stored form
  await all.getByTestId('aiws-cell-task-43-budget').getByRole('button').first().click()
  await page.getByLabel('预算 task-43', { exact: true }).fill('0450.5')
  await page.getByLabel('预算 task-43', { exact: true }).press('Enter')
  await expect(all.getByTestId('aiws-cell-task-43-budget')).toContainText('450.50')
  expect((await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-43', 'budget')).value).toBe('450.50')

  // select: marking a task done removes it from the filtered view, the other view keeps it
  await all.getByTestId('aiws-cell-task-44-status').getByRole('button').first().click()
  await page.getByLabel('状态 task-44', { exact: true }).selectOption('option-done')
  await expect(all.getByTestId('aiws-cell-task-44-status')).toContainText('完成')
  await expect(open.getByTestId('aiws-row')).toHaveCount(3)
  await expect(all.getByTestId('aiws-row')).toHaveCount(5)
  expect((await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-44', 'status')).value).toBe('option-done')

  // date + clearing a cell (unset, not empty text)
  await all.getByTestId('aiws-cell-task-43-due').getByRole('button').first().click()
  await page.getByLabel('截止日期 task-43', { exact: true }).fill('2026-12-01')
  await page.getByLabel('截止日期 task-43', { exact: true }).press('Enter')
  await expect.poll(async () => (await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-43', 'due')).value).toBe('2026-12-01')
  await all.getByTestId('aiws-cell-task-41-owner').getByRole('button').first().click()
  await page.getByLabel('负责人 task-41', { exact: true }).fill('')
  await page.getByLabel('负责人 task-41', { exact: true }).press('Enter')
  await expect.poll(async () => (await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-41', 'owner')).is_set).toBe(false)

  // an invalid value is refused by the backend: the input stays, marked as needing attention
  await all.getByTestId('aiws-cell-task-40-budget').getByRole('button').first().click()
  await page.getByLabel('预算 task-40', { exact: true }).fill('12.345')
  await page.getByLabel('预算 task-40', { exact: true }).press('Enter')
  const refused = all.getByTestId('aiws-cell-task-40-budget')
  await expect(refused).toHaveAttribute('data-state', 'needs_attention')
  await expect(refused.getByTestId('aiws-conflict-mine')).toHaveText('12.345')
  expect((await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-40', 'budget')).value).toBe('300.00')
  await refused.getByTestId('aiws-conflict-discard').click()
  await expect(refused).toHaveAttribute('data-state', 'clean')
  await expect(refused).toContainText('300.00')
})

test('(g) V01 vectors hold in the browser through the WASM core', async ({ page }) => {
  // numbers keep their source text (-0.0 must reach the core as written)
  const vectors = JSON.parse(readFileSync(VECTORS_PATH, 'utf8'), function (key, value, context?: { source?: string }) {
    return key === 'input' && typeof value === 'number' && context?.source ? { $raw: context.source } : value
  }) as { valid: unknown[]; invalid: unknown[]; values: unknown[]; chunks: unknown[] }
  expect(vectors.valid.length).toBeGreaterThan(0)
  await page.goto('/?scenario=normal')
  const report = await page.evaluate(async (data) => {
    const path = '/src/app/aiworkspace/wasm/aiworkspace_wasm.js'
    const core = await import(/* @vite-ignore */ path)
    await core.default({ module_or_path: '/src/app/aiworkspace/wasm/aiworkspace_wasm_bg.wasm' })
    const failures: string[] = []
    let checked = 0
    for (const item of data.valid as { input: string; obj_type: string; canonical: string; object_id: string }[]) {
      checked += 1
      try {
        const canonical = core.canonical_json(item.input)
        const id = core.object_id(item.obj_type, item.input)
        if (canonical !== item.canonical) failures.push(`canonical of ${item.input}: ${canonical}`)
        if (id !== item.object_id) failures.push(`object_id of ${item.input}: ${id}`)
        core.verify_object(id, canonical)
      } catch (error) { failures.push(`valid ${item.input} threw ${String(error)}`) }
    }
    for (const item of data.invalid as { input: string; reason: string }[]) {
      checked += 1
      try { core.canonical_json(item.input); failures.push(`invalid input accepted (${item.reason}): ${item.input}`) } catch { /* expected */ }
    }
    for (const item of data.values as { def: unknown; input: unknown; output?: unknown; error?: string }[]) {
      checked += 1
      const raw = item.input && typeof item.input === 'object' && '$raw' in (item.input as object) ? (item.input as { $raw: string }).$raw : JSON.stringify(item.input)
      try {
        const out = core.normalize_value(JSON.stringify(item.def), raw)
        if (item.error) failures.push(`value ${raw} should fail with ${item.error}, got ${out}`)
        else if (out !== JSON.stringify(item.output)) failures.push(`value ${raw}: ${out} != ${JSON.stringify(item.output)}`)
      } catch (error) {
        if (!item.error) failures.push(`value ${raw} threw ${String(error)}`)
        else if (!String(error).includes(item.error)) failures.push(`value ${raw}: wrong error ${String(error)}`)
      }
    }
    for (const item of data.chunks as { hex: string; chunk_id: string; file_object_id: string }[]) {
      checked += 1
      const bytes = new Uint8Array((item.hex.match(/../g) ?? []).map((pair) => parseInt(pair, 16)))
      if (core.chunk_id(bytes) !== item.chunk_id) failures.push(`chunk ${item.hex}`)
      if (core.file_object_id(bytes) !== item.file_object_id) failures.push(`file object ${item.hex}`)
    }
    // the order key function the UI uses for reordering is the backend's
    const between = core.order_key_between('a', 'b')
    if (!(between > 'a' && between < 'b')) failures.push(`order_key_between: ${between}`)
    return { checked, failures }
  }, vectors)
  expect(report.failures).toEqual([])
  expect(report.checked).toBe(vectors.valid.length + vectors.invalid.length + vectors.values.length + vectors.chunks.length)
})

test('(h) a 10 000 row table scrolls with a bounded number of row nodes', async ({ page, api }) => {
  test.setTimeout(240_000)
  const ws = await api.rpc(ALICE, 'ws.create', { title: `h ${Date.now()}` })
  const fields = [{ field_id: 'title', name: '标题', type: 'text', required: true }, { field_id: 'n', name: '序号', type: 'number' }, { field_id: 'done', name: '完成', type: 'boolean' }]
  let result = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'surface-main-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'a', payload: { kind: 'folder', system: 'surface_content', surface_id: 'surface-main', title: '大表' } },
    { op: 'entity.create', entity_id: 'surface-main', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'a', payload: { kind: 'surface', layout: { mode: 'flow' }, title: '大表', content_folder_id: 'surface-main-content' } },
    { op: 'entity.create', entity_id: 'big', type_id: 'buckyos.table-source', parent_id: 'data', order_key: 'a', payload: { title_field_id: 'title', fields } },
    { op: 'entity.create', entity_id: 'cell-big', type_id: 'buckyos.cell', parent_id: 'surface-main', order_key: 'b', payload: { source_ref: { entity_id: 'big' }, view: { type: 'table' }, title: '一万行' } },
  ])
  expect(result.status).toBe('accepted')
  for (let batch = 0; batch < 10; batch++) {
    const records = Array.from({ length: 1000 }, (_, i) => {
      const n = batch * 1000 + i
      return { record_id: `r${String(n).padStart(5, '0')}`, values: { title: `第 ${n} 行`, n, done: n % 2 === 0 } }
    })
    result = await api.commit(ALICE, ws, [{ op: 'table.insert_records', source_id: 'big', records }])
    expect(result.status, JSON.stringify(result).slice(0, 300)).toBe('accepted')
  }
  await openWorkspace(page, ALICE, ws.workspace_id)
  const table = page.getByTestId('aiws-table-cell-big')
  await expect(page.getByTestId('aiws-table-count-cell-big')).toContainText('10000 条记录')
  await expect(table.getByTestId('aiws-cell-r00000-title')).toContainText('第 0 行')
  const rowNodes = () => table.getByTestId('aiws-row').count()
  expect(await rowNodes()).toBeLessThan(60)

  const scroll = page.getByTestId('aiws-table-scroll-cell-big')
  let maxNodes = 0
  for (const fraction of [0.25, 0.5, 0.75, 1]) {
    await scroll.evaluate((element, f) => { element.scrollTop = (element.scrollHeight - element.clientHeight) * f }, fraction)
    await page.waitForTimeout(150)
    maxNodes = Math.max(maxNodes, await rowNodes())
  }
  // the last record becomes visible with its real content (pages are loaded on demand, in order)
  await expect(table.getByTestId('aiws-cell-r09999-title')).toContainText('第 9999 行', { timeout: 60_000 })
  maxNodes = Math.max(maxNodes, await rowNodes())
  expect(maxNodes).toBeLessThan(60)
  expect(await table.locator('[role="gridcell"]').count()).toBeLessThan(60 * 3 + 1)

  // editing deep in the table still works and touches only that cell
  await table.getByTestId('aiws-cell-r09999-title').getByRole('button').first().click()
  await page.getByLabel('标题 r09999', { exact: true }).fill('最后一行（已改）')
  await page.getByLabel('标题 r09999', { exact: true }).press('Enter')
  await expect(table.getByTestId('aiws-cell-r09999-title')).toContainText('最后一行（已改）')
  await expectAllCommitted(page)
  expect((await api.cell(ALICE, ws.workspace_id, 'big', 'r09999', 'title')).value).toBe('最后一行（已改）')
  expect((await api.cell(ALICE, ws.workspace_id, 'big', 'r09998', 'title')).rev).toBeLessThan((await api.cell(ALICE, ws.workspace_id, 'big', 'r09999', 'title')).rev)
})
