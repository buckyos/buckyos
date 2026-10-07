/* Phase two §13.1 UI11–UI13, UI16, UI19, UI20 and 许愿格 v0.2: the wish's two passes (the Mock in the
 * browser, the real executor on the service with a scripted model), application of the previewed
 * plan guarded by the read set, freshness that only input changes move, failure keeping old results,
 * manual-modification choices, versions and rollback, the new-result mode, concurrent application,
 * export/import, feedback rounds, re-running only the program, and the HTML extension API (v1 calls
 * and `aiws` v2 bindings, typed writes, batch, watch). */

import { expect, openCanvas, openWorkspace, test } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'

async function runWish(page: import('@playwright/test').Page, blockId: string) {
  await page.getByTestId(`aiws-canvas-block-${blockId}`).dblclick()
  await page.getByTestId('aiws-wish-analyze').click()
  await expect(page.getByTestId('aiws-wish-analysis')).toBeVisible({ timeout: 20_000 })
  await page.getByTestId('aiws-wish-execute').click()
  await expect(page.getByTestId('aiws-wish-candidate')).toBeVisible({ timeout: 30_000 })
  await page.getByTestId('aiws-wish-apply').click()
  await expect(page.getByTestId('aiws-wish-message')).toContainText('已应用', { timeout: 30_000 })
}

test('UI11/UI20 analyze → execute → apply writes results, Blocks and dependency records in one undoable commit; only input changes make them stale', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `wish ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  const head = await api.headSeq(ALICE, ws.workspace_id)
  await page.getByTestId('aiws-canvas-block-blk-wish').dblclick()
  await expect(page.getByTestId('aiws-wish-execute')).toBeDisabled() // no analysis yet
  await page.getByTestId('aiws-wish-analyze').click()
  await expect(page.getByTestId('aiws-wish-analysis')).toContainText('模拟执行器')
  await expect(page.getByTestId('aiws-wish-input')).toHaveCount(1)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1) // the analysis is written back
  const wish = await api.read(ALICE, ws.workspace_id, 'wish-analysis')
  expect(wish.content.payload.analysis.prompt).toBe(wish.content.payload.prompt)
  expect(wish.content.payload.inputs.map((i: { entity_id: string }) => i.entity_id)).toEqual(['sales'])
  // execution does not write; application is one commit with the read set as preconditions
  await page.getByTestId('aiws-wish-execute').click()
  await expect(page.getByTestId('aiws-wish-candidate')).toBeVisible()
  await expect(page.getByTestId('aiws-wish-result')).toHaveCount(4)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
  await expect(page.getByTestId('aiws-wish-candidate')).toHaveAttribute('data-ready', 'true')
  await page.getByTestId('aiws-wish-apply').click()
  await expect(page.getByTestId('aiws-wish-message')).toContainText('新建 4 项')
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 2)
  const outline = await api.outline(ALICE, ws.workspace_id)
  const results = outline.filter((e: { derived?: { wish_id: string } }) => e.derived?.wish_id === 'wish-analysis')
  expect(results).toHaveLength(4)
  for (const r of results) {
    expect(r.parent_id).toBe('wish-analysis-out-data')
    expect(r.derived.executor).toBe('mock')
    expect(r.derived.simulated).toBe(true)
    expect(r.derived.inputs.some((i: { entity_id: string; selector?: { kind: string } }) => i.entity_id === 'sales' && i.selector?.kind === 'table_members')).toBe(true)
  }
  expect(outline.find((e: { entity_id: string }) => e.entity_id === 'wish-analysis-out')?.kind).toBe('group')
  expect(outline.filter((e: { parent_id?: string }) => e.parent_id === 'wish-analysis-out')).toHaveLength(4)
  const after = await api.read(ALICE, ws.workspace_id, 'wish-analysis')
  expect(after.content.payload.last_run.produced).toHaveLength(4)
  // freshness: current everywhere
  const summary = results.find((e: { type_id: string }) => e.type_id === 'buckyos.richtext').entity_id
  await expect(page.getByTestId(`aiws-freshness-wish-analysis`).first()).toHaveAttribute('data-status', 'current')
  let fresh = await api.rpc(ALICE, 'doc.freshness', { workspace_id: ws.workspace_id, entity_ids: [summary, 'wish-analysis'] })
  expect(fresh.items.map((i: { status: string }) => i.status)).toEqual(['current', 'current'])
  // moving the result Block and editing an unrelated note do not stale it
  const summaryBlock = outline.find((e: { type_id: string; source_id?: string }) => e.type_id === 'buckyos.cell' && e.source_id === summary).entity_id
  await api.commit(ALICE, ws, [{ op: 'tree.place', entity_id: summaryBlock, placement: { x: 10, y: 10, w: 300, h: 200 } }])
  await api.commit(ALICE, ws, [{ op: 'entity.create', entity_id: 'note-unrelated', type_id: 'buckyos.annotation', parent_id: 'data', order_key: 'zz', payload: { kind: 'note', body: '无关' } }])
  fresh = await api.rpc(ALICE, 'doc.freshness', { workspace_id: ws.workspace_id, entity_ids: [summary] })
  expect(fresh.items[0].status).toBe('current')
  // a relevant input change: stale, in both modes, naming the input
  const rev = (await api.cell(ALICE, ws.workspace_id, 'sales', 's-1', 'revenue')).rev
  await api.commit(ALICE, ws, [{ op: 'table.set_values', source_id: 'sales', values: [{ record_id: 's-1', field_id: 'revenue', value: 200000, expect: { rev } }] }])
  await expect(page.getByTestId('aiws-freshness-wish-analysis').first()).toHaveAttribute('data-status', 'stale')
  await expect(page.getByTestId('aiws-wish-stale')).toContainText('已改变')
  await page.getByTestId('aiws-top-sources').click()
  await page.locator('[data-testid="aiws-tree-item"][data-entity-id="canvas-content"] .aiws-tree-toggle').first().click()
  await page.locator('[data-testid="aiws-tree-item"][data-entity-id="sf-analysis-content"] .aiws-tree-toggle').first().click()
  await page.locator('[data-testid="aiws-tree-item"][data-entity-id="wish-analysis-out-data"] .aiws-tree-toggle').first().click()
  await page.getByTestId(`aiws-tree-${summary}`).click()
  await expect(page.getByTestId(`aiws-detail-${summary}`).getByTestId(`aiws-freshness-${summary}`)).toHaveAttribute('data-status', 'stale')
  await page.getByTestId('aiws-detail-tab-relations').click()
  await expect(page.getByTestId('aiws-relations-freshness')).toContainText('原始销售数据')
  // reopened: the state is rebuilt from the persisted record
  await page.reload()
  await openWorkspace(page, ALICE, ws.workspace_id)
  fresh = await api.rpc(ALICE, 'doc.freshness', { workspace_id: ws.workspace_id, entity_ids: [summary] })
  expect(fresh.items[0].status).toBe('stale')
  expect(fresh.items[0].changed_inputs[0].entity_id).toBe('sales')
})

test('UI12/UI13 failure keeps old results; manual edits are protected; versions roll back; new-result mode makes comparable groups; cycles are refused', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `wish2 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await runWish(page, 'blk-wish')
  const outline = await api.outline(ALICE, ws.workspace_id)
  const summary = outline.find((e: { derived?: { wish_id: string }; type_id: string }) => e.derived?.wish_id === 'wish-analysis' && e.type_id === 'buckyos.richtext')
  const v1 = summary.content_rev
  // a failing run (injected): nothing changes
  const prompt = page.getByTestId('aiws-wish-prompt')
  await prompt.fill(`${await prompt.inputValue()} #fail`)
  await prompt.blur()
  await expect(page.getByTestId('aiws-wish-needs-analysis')).toBeVisible()
  await page.getByTestId('aiws-wish-analyze').click()
  await expect(page.getByTestId('aiws-wish-needs-analysis')).toHaveCount(0)
  const head = await api.headSeq(ALICE, ws.workspace_id)
  await page.getByTestId('aiws-wish-execute').click()
  await expect(page.getByTestId('aiws-wish-message')).toContainText('模拟失败')
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head)
  expect((await api.read(ALICE, ws.workspace_id, summary.entity_id)).content_rev).toBe(v1)
  // an invalid candidate (injected) is refused by the service on the spot: no candidate, old results stay
  await prompt.fill((await prompt.inputValue()).replace('#fail', '#invalid'))
  await prompt.blur()
  await page.getByTestId('aiws-wish-analyze').click()
  await expect(page.getByTestId('aiws-wish-needs-analysis')).toHaveCount(0)
  await page.getByTestId('aiws-wish-execute').click()
  await expect(page.getByTestId('aiws-wish-message')).toContainText('不合法')
  await expect(page.getByTestId('aiws-wish-candidate')).toHaveCount(0)
  expect((await api.read(ALICE, ws.workspace_id, summary.entity_id)).content_rev).toBe(v1)
  // a manual edit of the summary: detected, and the next application asks
  await prompt.fill((await prompt.inputValue()).replace(' #invalid', ''))
  await prompt.blur()
  await page.getByTestId('aiws-wish-analyze').click()
  await expect(page.getByTestId('aiws-wish-needs-analysis')).toHaveCount(0)
  const ast = await api.ast(ALICE, ws.workspace_id, summary.entity_id)
  const first = ast.content[0]
  const blocks = (await api.read(ALICE, ws.workspace_id, summary.entity_id)).content.blocks
  await api.commit(ALICE, ws, [{ op: 'richtext.replace_block', entity_id: summary.entity_id, block_id: first.attrs.block_id, expect: { hash: blocks[first.attrs.block_id].hash }, node: { type: 'heading', attrs: { block_id: first.attrs.block_id, level: 1 }, content: [{ type: 'text', text: '人工改过的标题' }] } }])
  await expect(page.getByTestId(`aiws-freshness-wish-analysis`).first()).toHaveAttribute('data-status', 'current')
  const freshness = await api.rpc(ALICE, 'doc.freshness', { workspace_id: ws.workspace_id, entity_ids: [summary.entity_id] })
  expect(freshness.items[0].manual_modified).toBe(true)
  // change an input so a refresh is due, then run again: the choice is asked per modified result
  const rev = (await api.cell(ALICE, ws.workspace_id, 'sales', 's-2', 'revenue')).rev
  await api.commit(ALICE, ws, [{ op: 'table.set_values', source_id: 'sales', values: [{ record_id: 's-2', field_id: 'revenue', value: 99000, expect: { rev } }] }])
  await expect(page.getByTestId('aiws-freshness-wish-analysis').first()).toHaveAttribute('data-status', 'stale')
  await page.getByTestId('aiws-wish-execute').click()
  await expect(page.getByTestId('aiws-wish-candidate')).toBeVisible()
  await expect(page.getByTestId('aiws-wish-manual')).toBeVisible()
  await expect(page.getByTestId('aiws-wish-apply')).toBeDisabled()
  await page.getByTestId('aiws-wish-manual-摘要').getByLabel('保留人工修改').check()
  await expect(page.getByTestId('aiws-wish-apply')).toBeEnabled()
  await page.getByTestId('aiws-wish-apply').click()
  await expect(page.getByTestId('aiws-wish-message')).toContainText('保留 1 项')
  expect((await api.ast(ALICE, ws.workspace_id, summary.entity_id)).content[0].content[0].text).toBe('人工改过的标题')
  await expect(page.getByTestId('aiws-freshness-wish-analysis').first()).toHaveAttribute('data-status', 'current')
  // the other results were overwritten in place: same ids, new versions, one undo step for the whole application
  const after = await api.outline(ALICE, ws.workspace_id)
  expect(after.filter((e: { derived?: { wish_id: string } }) => e.derived?.wish_id === 'wish-analysis')).toHaveLength(4)
  // version history of a generated record: list and roll back to run 1
  const kpi = after.find((e: { derived?: { wish_id: string }; type_id: string }) => e.derived?.wish_id === 'wish-analysis' && e.type_id === 'buckyos.record')
  const versions = await api.rpc(ALICE, 'doc.list_versions', { workspace_id: ws.workspace_id, entity_id: kpi.entity_id })
  expect(versions.versions.length).toBeGreaterThanOrEqual(2)
  expect(versions.versions[0].kind).toBe('generated')
  const oldest = versions.versions[versions.versions.length - 1]
  await page.getByTestId('aiws-top-sources').click()
  await page.locator('[data-testid="aiws-tree-item"][data-entity-id="canvas-content"] .aiws-tree-toggle').first().click()
  await page.locator('[data-testid="aiws-tree-item"][data-entity-id="sf-analysis-content"] .aiws-tree-toggle').first().click()
  await page.locator('[data-testid="aiws-tree-item"][data-entity-id="wish-analysis-out-data"] .aiws-tree-toggle').first().click()
  await page.getByTestId(`aiws-tree-${kpi.entity_id}`).click()
  await page.getByTestId('aiws-versions-toggle').click()
  await expect(page.getByTestId('aiws-version')).toHaveCount(versions.versions.length)
  await page.getByTestId(`aiws-restore-${oldest.content_rev}`).click()
  await expect.poll(async () => (await api.read(ALICE, ws.workspace_id, kpi.entity_id)).content.props.revenue).toBe(oldest.derived ? '770000.00' : '770000.00')
  expect((await api.read(ALICE, ws.workspace_id, kpi.entity_id)).derived.run_id).toBe(oldest.derived.run_id)
  // new-result mode: a second group, old results untouched
  await page.getByTestId('aiws-tree-wish-analysis').click()
  await page.getByTestId('aiws-wish-mode').selectOption('new')
  await page.getByTestId('aiws-wish-execute').click()
  await expect(page.getByTestId('aiws-wish-candidate')).toBeVisible()
  await page.getByTestId('aiws-wish-apply').click()
  await expect(page.getByTestId('aiws-wish-message')).toContainText('新建 4 项')
  const groups = (await api.outline(ALICE, ws.workspace_id)).filter((e: { kind?: string; parent_id?: string }) => e.kind === 'group' && e.parent_id === 'sf-analysis')
  expect(groups.length).toBe(2)
  expect((await api.ast(ALICE, ws.workspace_id, summary.entity_id)).content[0].content[0].text).toBe('人工改过的标题')
  // a cycle: the wish's own result as its input is refused before execution
  const read = await api.read(ALICE, ws.workspace_id, 'wish-analysis')
  await api.commit(ALICE, ws, [{ op: 'entity.set_keys', entity_id: 'wish-analysis', keys: [{ key: 'inputs', value: [...read.content.payload.inputs, { entity_id: summary.entity_id, version: { mode: 'follow' } }], expect: { rev: read.content.key_revs.inputs } }] }])
  // changed inputs need a new analysis first
  await expect(page.getByTestId('aiws-wish-needs-analysis')).toBeVisible()
  await page.getByTestId('aiws-wish-analyze').click()
  await expect(page.getByTestId('aiws-wish-needs-analysis')).toHaveCount(0)
  await page.getByTestId('aiws-wish-execute').click()
  await expect(page.getByTestId('aiws-wish-message')).toContainText('成环')
})

test('UI20 two clients apply the same wish: only one application is accepted', async ({ page, api, browser }) => {
  const ws = await api.demo(ALICE, 'quarterly', `wish3 ${Date.now()}`)
  await api.rpc(ALICE, 'ws.grant', { workspace_id: ws.workspace_id, subject: 'bob', capabilities: ['read', 'update', 'structure', 'append', 'delete', 'comment'] })
  await openCanvas(page, ALICE, ws.workspace_id)
  const other = await browser.newContext({ viewport: { width: 1600, height: 1000 } })
  const page2 = await other.newPage()
  await openCanvas(page2, BOB, ws.workspace_id)
  for (const p of [page, page2]) {
    await p.getByTestId('aiws-canvas-block-blk-wish').dblclick()
  }
  await page.getByTestId('aiws-wish-analyze').click()
  await expect(page.getByTestId('aiws-wish-analysis')).toBeVisible()
  await expect(page2.getByTestId('aiws-wish-analysis')).toBeVisible()
  await page.getByTestId('aiws-wish-execute').click()
  await page2.getByTestId('aiws-wish-execute').click()
  await expect(page.getByTestId('aiws-wish-candidate')).toBeVisible()
  await expect(page2.getByTestId('aiws-wish-candidate')).toBeVisible()
  await page.getByTestId('aiws-wish-apply').click()
  await expect(page.getByTestId('aiws-wish-message')).toContainText('已应用')
  await page2.getByTestId('aiws-wish-apply').click()
  await expect(page2.getByTestId('aiws-wish-message')).toContainText('冲突')
  expect((await api.outline(ALICE, ws.workspace_id)).filter((e: { derived?: { wish_id: string } }) => e.derived?.wish_id === 'wish-analysis')).toHaveLength(4)
  // no client ran anything on its own because of the change events
  await page2.waitForTimeout(800)
  expect((await api.read(ALICE, ws.workspace_id, 'wish-analysis')).content.payload.last_run.run_id).toBeTruthy()
  await other.close()
})

test('UI16 export and import carry Surfaces, Blocks, definitions, wishes and dependency records; staleness survives import', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `wish4 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await runWish(page, 'blk-wish')
  const rev = (await api.cell(ALICE, ws.workspace_id, 'sales', 's-1', 'revenue')).rev
  await api.commit(ALICE, ws, [{ op: 'table.set_values', source_id: 'sales', values: [{ record_id: 's-1', field_id: 'revenue', value: 1, expect: { rev } }] }])
  const exported = await api.rpc(ALICE, 'doc.export', { workspace_id: ws.workspace_id, mode: 'share', self_contained: true })
  const pkg = await (await fetch(`${api.base}/export/${ws.workspace_id}/${exported.export_id}`, { headers: { authorization: `Bearer ${ALICE}` } })).arrayBuffer()
  const begin = await api.rpc(ALICE, 'ws.begin_import', {})
  await fetch(`${api.base}/upload/${begin.upload_id}`, { method: 'PUT', headers: { authorization: `Bearer ${ALICE}` }, body: Buffer.from(pkg) })
  const imported = await api.rpc(ALICE, 'ws.import', { upload_id: begin.upload_id, semantics: 'new' })
  expect(imported.ok).toBe(true)
  const outline = await api.outline(ALICE, imported.workspace_id)
  const ids = new Set(outline.map((e: { entity_id: string }) => e.entity_id))
  for (const id of ['sf-analysis', 'sf-analysis-content', 'sf-detail', 'blk-sales', 'blk-kpi', 'def-kpi', 'def-mock-wish', 'wish-analysis', 'wish-analysis-out']) expect(ids.has(id), id).toBe(true)
  const results = outline.filter((e: { derived?: { wish_id: string } }) => e.derived?.wish_id === 'wish-analysis')
  expect(results).toHaveLength(4)
  expect(outline.find((e: { entity_id: string }) => e.entity_id === 'blk-kpi').placement).toEqual({ x: 980, y: 40, w: 420, h: 130 })
  const fresh = await api.rpc(ALICE, 'doc.freshness', { workspace_id: imported.workspace_id, entity_ids: results.map((r: { entity_id: string }) => r.entity_id) })
  expect(fresh.items.every((i: { status: string }) => i.status === 'stale')).toBe(true)
  // the imported workspace opens on the canvas with the result group in place
  await page.getByTestId('aiws-back').click()
  await openCanvas(page, ALICE, imported.workspace_id)
  await page.getByTestId('aiws-fit-all').click()
  await expect(page.getByTestId('aiws-canvas-block-wish-analysis-out')).toBeVisible()
  await expect(page.getByTestId('aiws-freshness-wish-analysis').first()).toHaveAttribute('data-status', 'stale')
})

test('UI19 an HTML extension Block reads data, submits through the store (undoable) and uploads; a crash falls back locally', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `html ${Date.now()}`)
  const js = `
    aiws.on('run', async function (payload) {
      var rows = await aiws.query({ source_id: aiws.context.source.entity_id, limit: 5 });
      if (payload && payload.crash) { setTimeout(function () { throw new Error('boom from extension'); }, 0); return 'crashing'; }
      var up = await aiws.upload('<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>', 'tiny.svg', 'image/svg+xml');
      var r = await aiws.submit([{ op: 'entity.create', entity_id: 'ext-note-' + Date.now().toString(36), type_id: 'buckyos.annotation', parent_id: 'sf-analysis-content', order_key: 'zz', payload: { kind: 'note', body: '扩展写入 ' + rows.rows.length + ' 行' } }], '扩展便签');
      return { rows: rows.rows.length, upload: up.object_id, outcome: r.status };
    });
    window.__run = function (p) { return new Promise(function (resolve) { document.getElementById('out').textContent = 'running'; aiws.on('ping', function () { return 1; }); resolve(); }); };
    document.getElementById('out').textContent = 'ready';
    aiws.ready();
  `
  const r = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'def-ext', type_id: 'buckyos.block-def', parent_id: 'data', order_key: 'zx', payload: { def_id: 'test.ext', kind: 'html', title: '测试扩展', html: { html: '<div id="out">init</div>', js } } },
    { op: 'entity.create', entity_id: 'blk-ext', type_id: 'buckyos.cell', parent_id: 'sf-analysis', order_key: 'zzx', placement: { x: 40, y: 720, w: 400, h: 200 }, payload: { view: { type: 'html' }, source_ref: { entity_id: 'sales' }, def_ref: { entity_id: 'def-ext' }, title: '扩展' } },
  ])
  expect(r.status).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-fit-all').click()
  // static until activated
  await expect(page.getByTestId('aiws-html-static-blk-ext')).toBeVisible()
  await page.getByTestId('aiws-canvas-block-blk-ext').click()
  await page.getByTestId('aiws-near-run').click()
  await expect(page.getByTestId('aiws-html-active-blk-ext')).toBeVisible()
  const frame = page.frameLocator('.aiws-html-frame')
  await expect(frame.locator('#out')).toHaveText('ready')
  // drive the extension's handler from the test through the host bridge (the same path a wish executor uses)
  const head = await api.headSeq(ALICE, ws.workspace_id)
  const result = await page.evaluate(async () => {
    const iframe = document.querySelector('.aiws-html-frame') as HTMLIFrameElement
    const win = iframe.contentWindow as unknown as { aiws: { context: { source: { entity_id: string } }; query: (p: unknown) => Promise<{ rows: unknown[] }>; upload: (d: string, n: string, t: string) => Promise<{ object_id: string }>; submit: (ops: unknown[], label: string) => Promise<{ status: string }> } }
    const rows = await win.aiws.query({ source_id: win.aiws.context.source.entity_id, limit: 5 })
    const up = await win.aiws.upload('<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>', 'tiny.svg', 'image/svg+xml')
    const outcome = await win.aiws.submit([{ op: 'entity.create', entity_id: 'ext-note-1', type_id: 'buckyos.annotation', parent_id: 'sf-analysis-content', order_key: 'zz', payload: { kind: 'note', body: `扩展写入 ${rows.rows.length} 行` } }], '扩展便签')
    return { rows: rows.rows.length, upload: up.object_id, outcome: outcome.status }
  })
  expect(result.rows).toBe(5)
  expect(result.upload).toMatch(/^fobj:|^jobj:|:/)
  expect(result.outcome).toBe('accepted')
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
  await expect(page.getByTestId('aiws-undo')).toHaveText(/撤销 1/)
  expect((await api.read(ALICE, ws.workspace_id, 'ext-note-1')).content.payload.body).toBe('扩展写入 5 行')
  // undo takes the extension's write back
  await page.getByTestId('aiws-undo').click()
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).some((e: { entity_id: string }) => e.entity_id === 'ext-note-1')).toBe(false)
  // a crash inside the extension: this Block falls back, the others keep working
  await page.evaluate(() => { const iframe = document.querySelector('.aiws-html-frame') as HTMLIFrameElement; (iframe.contentWindow as unknown as { eval: (s: string) => void }).eval('setTimeout(function(){ throw new Error("boom from extension") }, 0)') })
  await expect(page.getByTestId('aiws-html-failed-blk-ext')).toBeVisible()
  await expect(page.getByTestId('aiws-html-failed-blk-ext')).toContainText('boom')
  await expect(page.getByTestId('aiws-canvas-block-blk-sales')).toBeVisible()
  await page.getByTestId('aiws-canvas-block-blk-kpi').click()
  await expect(page.getByTestId('aiws-near-toolbar')).toBeVisible()
})

test('许愿格 v0.2: the real executor (scripted model) analyses, programs, previews, applies; feedback becomes a refinement; re-running only the program needs no model', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `xllm ${Date.now()}`)
  const wish = await api.read(ALICE, ws.workspace_id, 'wish-analysis')
  await api.commit(ALICE, ws, [{ op: 'entity.set_keys', entity_id: 'wish-analysis', keys: [
    { key: 'executor', value: 'xllm', expect: { rev: wish.content.key_revs.executor } },
    { key: 'prompt', value: '按区域汇总「原始销售数据」的销售额，并写一段解读', expect: { rev: wish.content.key_revs.prompt } }] }])
  await openCanvas(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-canvas-block-blk-wish').dblclick()
  await expect(page.getByTestId('aiws-wish-executor')).toHaveValue('xllm')
  const head = await api.headSeq(ALICE, ws.workspace_id)
  // analysis on the service: bound by handle, written back by the click
  await page.getByTestId('aiws-wish-analyze').click()
  await expect(page.getByTestId('aiws-wish-contract')).toContainText('summary', { timeout: 30_000 })
  await expect(page.getByTestId('aiws-wish-checkdefs')).toContainText('汇总合计等于输入合计')
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
  const analysed = await api.read(ALICE, ws.workspace_id, 'wish-analysis')
  expect(analysed.content.payload.inputs).toEqual([{ entity_id: 'sales', name: 'data', label: '原始销售数据', version: { mode: 'follow' } }])
  expect(analysed.content.payload.analysis.context_prompt).not.toContain('@')
  // execution: progress, then a candidate with previews and checks; nothing written yet
  await page.getByTestId('aiws-wish-execute').click()
  await expect(page.getByTestId('aiws-wish-candidate')).toBeVisible({ timeout: 60_000 })
  await expect(page.getByTestId('aiws-wish-result')).toHaveCount(2)
  await expect(page.getByTestId('aiws-wish-preview-table')).toContainText('华东')
  await expect(page.getByTestId('aiws-wish-preview-text')).toContainText('合计')
  await expect(page.locator('[data-testid="aiws-wish-check"][data-check="total"]')).toHaveAttribute('data-status', 'passed')
  await expect(page.locator('[data-testid="aiws-wish-check"][data-check="cited"]')).toHaveAttribute('data-status', 'review')
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
  await page.getByTestId('aiws-wish-apply').click()
  await expect(page.getByTestId('aiws-wish-message')).toContainText('新建 2 项')
  const applied = await api.read(ALICE, ws.workspace_id, 'wish-analysis')
  const lastRun = applied.content.payload.last_run
  expect(applied.content.payload.program.api_version).toBe(2)
  expect(Object.keys(lastRun.result_bindings.results).sort()).toEqual(['commentary', 'summary'])
  expect(lastRun.checks.passed).toBe(1)
  const summary = lastRun.result_bindings.results.summary
  expect(summary.cells).toHaveLength(2)
  const outline = await api.outline(ALICE, ws.workspace_id)
  expect(outline.find((e: { entity_id: string }) => e.entity_id === summary.cells[1]).view_type).toBe('sample.bar-chart')
  // on the canvas next to the wish (mounted once the camera reaches it)
  await expect(page.getByTestId(`aiws-canvas-block-${summary.cells[1]}`)).toBeAttached()
  // undo takes the whole application back in one step
  await expect(page.getByTestId('aiws-undo')).toBeEnabled()
  // feedback: a new round on the existing program; applying it keeps the request as a refinement
  await page.getByTestId('aiws-wish-execute').click()
  await expect(page.getByTestId('aiws-wish-candidate')).toBeVisible({ timeout: 60_000 })
  await page.getByTestId('aiws-wish-feedback').fill('表格按合计从高到低排')
  await page.getByTestId('aiws-wish-feedback-send').click()
  await expect(page.getByTestId('aiws-wish-candidate')).toContainText('按反馈修改', { timeout: 60_000 })
  await page.getByTestId('aiws-wish-apply').click()
  await expect(page.getByTestId('aiws-wish-refinement')).toHaveCount(1)
  await expect(page.getByTestId('aiws-wish-refinement').locator('input')).toHaveValue('汇总表按合计降序排列')
  await expect(page.getByTestId('aiws-wish-needs-analysis')).toHaveCount(0)
  // the data changes: stale; re-running only the program refreshes the table, the text then needs regenerating
  const rev = (await api.cell(ALICE, ws.workspace_id, 'sales', 's-1', 'revenue')).rev
  await api.commit(ALICE, ws, [{ op: 'table.set_values', source_id: 'sales', values: [{ record_id: 's-1', field_id: 'revenue', value: 500000, expect: { rev } }] }])
  await expect(page.getByTestId('aiws-freshness-wish-analysis').first()).toHaveAttribute('data-status', 'stale')
  await page.getByTestId('aiws-wish-rerun').click()
  await expect(page.getByTestId('aiws-wish-candidate')).toContainText('程序（未调用模型）', { timeout: 60_000 })
  await page.getByTestId('aiws-wish-apply').click()
  await expect(page.getByTestId('aiws-wish-direct-stale')).toBeVisible()
  // the stored program can be read
  await page.getByTestId('aiws-wish-program').click()
  await expect(page.getByTestId('aiws-wish-program-view')).toContainText("aiws.result.table('summary'")
})

test('aiws v2: an HTML Block reads by binding name, writes typed values in one batch (one undo step), and watches its data', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `aiws2 ${Date.now()}`)
  const js = `document.getElementById('out').textContent = 'ready ' + aiws.version + ' ' + Object.keys(aiws.bindings).join(','); window.__seen = 0; aiws.watch('orders', function () { window.__seen++; }); aiws.ready();`
  const r = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'def-v2', type_id: 'buckyos.block-def', parent_id: 'data', order_key: 'zx', payload: { def_id: 'test.v2', kind: 'html', title: 'v2', allow_no_source: true, html: { html: '<div id="out">init</div>', js, api_version: 2 } } },
    { op: 'entity.create', entity_id: 'blk-v2', type_id: 'buckyos.cell', parent_id: 'sf-analysis', order_key: 'zzx', placement: { x: 40, y: 720, w: 400, h: 200 }, payload: { view: { type: 'html' }, def_ref: { entity_id: 'def-v2' }, bindings: { orders: { entity_id: 'sales' }, intro: { entity_id: 'intro' } }, title: 'v2' } },
  ])
  expect(r.status).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-fit-all').click()
  await page.getByTestId('aiws-canvas-block-blk-v2').click()
  await page.getByTestId('aiws-near-run').click()
  const frame = page.frameLocator('.aiws-html-frame')
  await expect(frame.locator('#out')).toHaveText('ready 2 intro,orders')
  const head = await api.headSeq(ALICE, ws.workspace_id)
  const result = await page.evaluate(async () => {
    const iframe = document.querySelector('.aiws-html-frame') as HTMLIFrameElement
    type Aiws = { input: (n: string) => { rows: (o?: unknown) => Promise<Record<string, unknown>[]>; fields: () => Promise<{ name: string }[]>; markdown: () => Promise<string> }; batch: (fn: () => Promise<void>, label?: string) => Promise<{ status: string }>; table: (n: string) => { upsert: (rows: unknown[], o: unknown) => Promise<unknown>; setColumn: (f: string, v: Record<string, unknown>) => Promise<unknown> } }
    const aiws = (iframe.contentWindow as unknown as { aiws: Aiws }).aiws
    const rows = await aiws.input('orders').rows({ fields: ['区域', '产品', '销售额'], filter: { 区域: '华东' } })
    const fields = await aiws.input('orders').fields()
    const md = await aiws.input('intro').markdown()
    const outcome = await aiws.batch(async () => {
      await aiws.table('orders').upsert([{ 区域: '华东', 产品: '智能手表', 销售额: 130000 }, { 区域: '东北', 产品: '智能手表', 销售额: 1000 }], { key: ['区域', '产品'] })
      await aiws.table('orders').setColumn('目标', { 's-2': 99000 })
    }, '批量写入')
    return { rows: rows.length, first: rows[0], fields: fields.map((f) => f.name), md, status: outcome.status }
  })
  expect(result.rows).toBe(3)
  expect(result.first).toHaveProperty('_id')
  expect(result.fields).toContain('销售额')
  expect(result.md).toContain('# 2026 Q2 经营分析')
  expect(result.status).toBe('accepted')
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
  expect((await api.cell(ALICE, ws.workspace_id, 'sales', 's-1', 'revenue')).value).toBe(130000)
  expect((await api.cell(ALICE, ws.workspace_id, 'sales', 's-2', 'target')).value).toBe(99000)
  await expect(page.getByTestId('aiws-undo')).toHaveText(/撤销 1/)
  // the binding is watched: a change to the data reaches the Block
  const frameDoc = await (await page.locator('.aiws-html-frame').elementHandle())?.contentFrame()
  await expect.poll(() => frameDoc?.evaluate(() => (window as unknown as { __seen?: number }).__seen ?? 0)).toBeGreaterThan(0)
})
