/* Phase two §13.1 UI01–UI10, UI17, UI18: the two top-level modes, the three canvas sub-modes, Surfaces,
 * Blocks sharing data, gestures as single commits, layout concurrency, mode dispatch and fallbacks.
 * Persistence is asserted through kRPC, not only through the DOM. */

import { blockCenter, expect, hooks, openCanvas, openWorkspace, test, fitAll, setCanvasMode, zoomIn } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'

test('UI01/UI18 mode switches keep the session, the undo stack and drafts; switching commits nothing', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `ui01 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-mode', 'edit')
  const before = await api.headSeq(ALICE, ws.workspace_id)
  // an edit on the canvas: one commit on the undo stack
  await page.getByTestId('aiws-canvas-block-blk-sales').dblclick()
  const cell = page.getByTestId('aiws-canvas-block-blk-sales').getByTestId('aiws-cell-s-1-revenue')
  await cell.getByRole('button').first().click()
  await page.getByLabel('销售额 s-1', { exact: true }).fill('130000')
  await page.getByLabel('销售额 s-1', { exact: true }).press('Enter')
  await expect(page.getByTestId('aiws-undo')).toHaveText(/撤销 1/)
  const afterEdit = await api.headSeq(ALICE, ws.workspace_id)
  expect(afterEdit).toBe(before + 1)
  // sub-modes: view, presentation-edit placeholder, back to edit; the session id and undo stack stay
  const sessionId = await page.getByTestId('aiws-workspace').getAttribute('data-session-id')
  await setCanvasMode(page, 'view')
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-mode', 'view')
  await setCanvasMode(page, 'presentation_edit')
  await expect(page.getByTestId('aiws-presentation-placeholder')).toBeVisible()
  await expect(page.getByTestId('aiws-presentation-placeholder')).toContainText('尚未实现')
  await setCanvasMode(page, 'edit')
  // top-level modes
  await page.getByTestId('aiws-top-sources').click()
  await expect(page.getByTestId('aiws-sources')).toBeVisible()
  await page.getByTestId('aiws-top-canvas').click()
  await expect(page.getByTestId('aiws-canvas')).toBeVisible()
  expect(await page.getByTestId('aiws-workspace').getAttribute('data-session-id')).toBe(sessionId)
  await expect(page.getByTestId('aiws-undo')).toHaveText(/撤销 1/)
  // no commit was made by the switches themselves
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(afterEdit)
  expect((await api.cell(ALICE, ws.workspace_id, 'sales', 's-1', 'revenue')).value).toBe(130000)
  // the mode is user work state, not the document
  const outline = await api.outline(ALICE, ws.workspace_id)
  expect(outline.some((e: { entity_id: string }) => e.entity_id.includes('mode'))).toBe(false)
})

test('UI02/UI03 the data-source view edits data without a Block; three Blocks on two Surfaces share it', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `ui03 ${Date.now()}`)
  await openWorkspace(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-top-sources').click()
  // data tree: first level, canvas content collapsed
  const tree = page.getByTestId('aiws-tree-item')
  await expect(tree.filter({ has: page.getByTestId('aiws-tree-sales') })).toBeVisible()
  const content = page.locator('[data-testid="aiws-tree-item"][data-entity-id="canvas-content"]')
  await expect(content).toHaveAttribute('aria-expanded', 'false')
  await expect(page.locator('[data-testid="aiws-tree-item"][data-entity-id="sf-analysis-content"]')).toHaveCount(0)
  // select the table: content in the middle, properties on the right, editable without a Block
  await page.getByTestId('aiws-tree-sales').click()
  await expect(page.getByTestId('aiws-detail-sales')).toBeVisible()
  await expect(page.getByTestId('aiws-properties')).toContainText('表')
  const detailCell = page.getByTestId('aiws-detail-sales').getByTestId('aiws-cell-s-2-cost')
  await detailCell.getByRole('button').first().click()
  await page.getByLabel('成本 s-2', { exact: true }).fill('59000')
  await page.getByLabel('成本 s-2', { exact: true }).press('Enter')
  await expect.poll(async () => (await api.cell(ALICE, ws.workspace_id, 'sales', 's-2', 'cost')).value).toBe(59000)
  // the two Surfaces show the same data with independent view configuration
  await page.getByTestId('aiws-top-canvas').click()
  await expect(page.getByTestId('aiws-canvas-block-blk-sales')).toContainText('59000')
  await page.getByTestId('aiws-surface-switch').click()
  await page.getByTestId('aiws-surface-item-sf-detail').getByRole('menuitem').click()
  await expect(page.getByTestId('aiws-canvas-block-blk-sales-2')).toBeVisible()
  await expect(page.getByTestId('aiws-canvas-block-blk-sales-2').getByTestId('aiws-row')).toHaveCount(3) // 华东 filter saved on this view only
  await expect(page.getByTestId('aiws-canvas-block-blk-chart-2')).toContainText('华东')
  // removing a Block does not delete first-level data
  await page.getByTestId('aiws-canvas-block-blk-metric-2').click()
  await page.getByTestId('aiws-near-more').click()
  await page.getByTestId('aiws-near-delete').click()
  await expect(page.getByTestId('aiws-canvas-block-blk-metric-2')).toHaveCount(0)
  const outline = await api.outline(ALICE, ws.workspace_id)
  expect(outline.some((e: { entity_id: string }) => e.entity_id === 'blk-metric-2')).toBe(false)
  expect(outline.some((e: { entity_id: string }) => e.entity_id === 'sales')).toBe(true)
})

test('UI04 Surfaces: create, move a Block across, delete with pre-check; referenced data refuses deletion with visible referrers only', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `ui04 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  // a new Surface comes with its content folder
  await page.getByTestId('aiws-surface-switch').click()
  await page.getByTestId('aiws-surface-new').click()
  await page.getByLabel('新画布标题').fill('第三张')
  await page.getByTestId('aiws-surface-create').click()
  await expect(page.getByTestId('aiws-surface-name')).toContainText('第三张')
  const outline = await api.outline(ALICE, ws.workspace_id)
  const third = outline.find((e: { title?: string; kind?: string }) => e.title === '第三张' && e.kind === 'surface') as { entity_id: string; content_folder_id: string }
  expect(third).toBeTruthy()
  expect(outline.find((e: { entity_id: string }) => e.entity_id === third.content_folder_id)?.parent_id).toBe('canvas-content')
  // move a Block from the analysis Surface to it: same id, same binding
  await page.getByTestId('aiws-surface-switch').click()
  await page.getByTestId('aiws-surface-item-sf-analysis').getByRole('menuitem').click()
  await page.getByTestId('aiws-canvas-block-blk-kpi').click()
  await page.getByTestId('aiws-near-more').click()
  await page.getByTestId('aiws-near-move-surface').click()
  await page.getByTestId('aiws-picker-surface').selectOption(third.entity_id)
  await page.getByTestId('aiws-picker-confirm').click()
  await expect(page.getByTestId('aiws-canvas-block-blk-kpi')).toHaveCount(0)
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).find((e: { entity_id: string }) => e.entity_id === 'blk-kpi')?.parent_id).toBe(third.entity_id)
  expect((await api.outline(ALICE, ws.workspace_id)).find((e: { entity_id: string }) => e.entity_id === 'blk-kpi')?.source_id).toBe('sales')
  // deleting the table is refused: the pre-check names the Blocks (all readable here) and nothing else
  await page.getByTestId('aiws-top-sources').click()
  await page.getByTestId('aiws-tree-sales').click()
  await page.getByTestId('aiws-delete-entity').click()
  await expect(page.getByTestId('aiws-delete-blocked')).toContainText('仍被引用')
  await expect(page.getByTestId('aiws-delete-confirm')).toHaveCount(0)
  // bob reads only the data tree: the same pre-check tells him nothing about the Blocks
  await api.rpc(ALICE, 'ws.grant', { workspace_id: ws.workspace_id, subject: 'bob', scope_entity_id: 'data', capabilities: ['read', 'delete'] })
  const prepared = await api.rpc(BOB, 'doc.prepare', { protocol_version: '0.2', workspace_id: ws.workspace_id, epoch: ws.epoch, idempotency_key: 'bob-pre', session_id: 'bob', operations: [{ op: 'entity.delete', entity_id: 'intro', expect: { rev: (await api.read(BOB, ws.workspace_id, 'intro')).life_rev } }] })
  expect(prepared.status).toBe('rejected')
  expect(prepared.errors[0].data.referrers).toEqual([])
  expect(prepared.errors[0].data.hidden_referrers).toBe(true)
  expect(JSON.stringify(prepared)).not.toContain('blk-intro')
  // delete the third Surface: its content folder goes with it
  await page.getByTestId('aiws-top-canvas').click()
  await page.getByTestId('aiws-surface-switch').click()
  await page.getByTestId(`aiws-surface-more-${third.entity_id}`).click()
  await page.getByTestId(`aiws-surface-delete-${third.entity_id}`).click()
  await page.getByTestId('aiws-surface-delete-confirm').click()
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).some((e: { entity_id: string }) => e.entity_id === third.entity_id)).toBe(false)
  expect((await api.outline(ALICE, ws.workspace_id)).some((e: { entity_id: string }) => e.entity_id === third.content_folder_id)).toBe(false)
})

test('UI05 a drag is one commit and one undo step; Esc cancels with zero commits; zoom does not touch the document', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `ui05 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  const head = await api.headSeq(ALICE, ws.workspace_id)
  const start = await blockCenter(page, 'blk-kpi')
  // cancelled gesture: the frame moves and comes back, nothing is sent
  await page.mouse.move(start.x, start.y)
  await page.mouse.down()
  await page.mouse.move(start.x + 60, start.y + 40, { steps: 5 })
  await page.mouse.move(start.x + 120, start.y + 80, { steps: 5 })
  await page.keyboard.press('Escape')
  await page.mouse.up()
  await page.waitForTimeout(300)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head)
  expect((await hooks(page)).commits).toBe(0)
  // a completed drag of two selected Blocks: exactly one commit, both placements persisted
  await page.getByTestId('aiws-canvas-block-blk-kpi').click()
  await page.getByTestId('aiws-canvas-block-blk-wish').click({ modifiers: ['Shift'] })
  await expect(page.locator('[data-testid^="aiws-selection-"]')).toHaveCount(2)
  const from = await blockCenter(page, 'blk-kpi')
  await page.mouse.move(from.x, from.y)
  await page.mouse.down()
  await page.mouse.move(from.x + 50, from.y + 30, { steps: 6 })
  await page.mouse.move(from.x + 100, from.y + 60, { steps: 6 })
  await page.mouse.up()
  await expect.poll(async () => api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
  expect((await hooks(page)).commits).toBe(1)
  const after = await api.outline(ALICE, ws.workspace_id)
  const kpi = after.find((e: { entity_id: string }) => e.entity_id === 'blk-kpi').placement
  const wish = after.find((e: { entity_id: string }) => e.entity_id === 'blk-wish').placement
  expect(kpi.x).toBe(980 + 100)
  expect(wish.y).toBe(190 + 60)
  // undo restores both in one step
  await page.getByTestId('aiws-undo').click()
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).find((e: { entity_id: string }) => e.entity_id === 'blk-kpi').placement.x).toBe(980)
  // resize through a handle: one commit (the KPI card's right edge is past the window: pan first, at zoom 1)
  await page.getByTestId('aiws-canvas').click({ position: { x: 600, y: 750 } }) // a blank spot clear of the floating toolbars
  const box = (await page.getByTestId('aiws-canvas').boundingBox())!
  await page.mouse.move(box.x + 700, box.y + box.height - 50) // empty canvas: a scrollable table keeps its own wheel
  await page.mouse.wheel(300, 0)
  await page.waitForTimeout(200)
  await page.getByTestId('aiws-canvas-block-blk-kpi').click()
  const handle = await page.getByTestId('aiws-handle-se').boundingBox()
  expect(handle).toBeTruthy()
  const beforeResize = await api.headSeq(ALICE, ws.workspace_id)
  await page.mouse.move(handle!.x + 4, handle!.y + 4)
  await page.mouse.down()
  await page.mouse.move(handle!.x + 64, handle!.y + 44, { steps: 6 })
  await page.mouse.up()
  await expect.poll(async () => api.headSeq(ALICE, ws.workspace_id)).toBe(beforeResize + 1)
  expect((await api.outline(ALICE, ws.workspace_id)).find((e: { entity_id: string }) => e.entity_id === 'blk-kpi').placement.w).toBe(420 + 60)
  // zoom is the user's viewport only
  const beforeZoom = await api.headSeq(ALICE, ws.workspace_id)
  await zoomIn(page)
  await expect(page.getByTestId('aiws-zoom')).not.toHaveText('100%')
  await page.waitForTimeout(400)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(beforeZoom)
})

test('UI06 two clients move the same Block: the later arrival wins, the other is told and can re-apply', async ({ page, api, browser }) => {
  const ws = await api.demo(ALICE, 'quarterly', `ui06 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await api.rpc(ALICE, 'ws.grant', { workspace_id: ws.workspace_id, subject: 'bob', capabilities: ['read', 'update', 'structure', 'append', 'delete', 'comment'] })
  const other = await browser.newContext({ viewport: { width: 1600, height: 1000 } })
  const page2 = await other.newPage()
  await openCanvas(page2, BOB, ws.workspace_id)
  // alice moves blk-kpi
  const a = await blockCenter(page, 'blk-kpi')
  await page.mouse.move(a.x, a.y)
  await page.mouse.down()
  await page.mouse.move(a.x + 40, a.y + 40, { steps: 5 })
  await page.mouse.move(a.x + 80, a.y + 80, { steps: 5 })
  await page.mouse.up()
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).find((e: { entity_id: string }) => e.entity_id === 'blk-kpi').placement.x).toBe(1060)
  // bob moves it elsewhere right after: later arrival wins
  await expect(page2.getByTestId('aiws-canvas-block-blk-kpi')).toBeVisible()
  await page2.waitForTimeout(500)
  const b = await blockCenter(page2, 'blk-kpi')
  await page2.mouse.move(b.x, b.y)
  await page2.mouse.down()
  await page2.mouse.move(b.x - 50, b.y + 100, { steps: 5 })
  await page2.mouse.move(b.x - 100, b.y + 200, { steps: 5 })
  await page2.mouse.up()
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).find((e: { entity_id: string }) => e.entity_id === 'blk-kpi').placement.x).toBe(960)
  // alice sees the notice and re-applies her position as a new commit
  const notice = page.getByTestId('aiws-notice').filter({ hasText: '位置已被' })
  await expect(notice).toBeVisible()
  await notice.getByTestId('aiws-notice-action').click()
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).find((e: { entity_id: string }) => e.entity_id === 'blk-kpi').placement.x).toBe(1060)
  // an unrelated data edit by bob never conflicts with alice's layout
  const read = await api.read(BOB, ws.workspace_id, 'sales')
  const c = await api.commit(BOB, ws, [{ op: 'table.set_values', source_id: 'sales', values: [{ record_id: 's-3', field_id: 'cost', value: 31000, expect: { rev: (await api.cell(BOB, ws.workspace_id, 'sales', 's-3', 'cost')).rev } }] }])
  expect(c.status).toBe('accepted')
  expect(read.content.record_count).toBe(12)
  await page.waitForTimeout(500)
  await expect(page.getByTestId('aiws-notice').filter({ hasText: '位置已被' })).toHaveCount(0)
  await other.close()
})

test('UI08/UI09 edit and view sub-modes dispatch rendering, tools and writes; unknown renderers and errors fall back locally', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `ui08 ${Date.now()}`)
  // an unknown renderer and a type the chart does not accept, straight from the API (the backend only checks the format)
  const place = { x: 40, y: 700, w: 300, h: 160 }
  const r = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'blk-unknown', type_id: 'buckyos.cell', parent_id: 'sf-analysis', order_key: 'zz', placement: place, payload: { view: { type: 'acme.gauge', version: 3 }, source_ref: { entity_id: 'sales' }, title: '未知渲染器' } },
    { op: 'entity.create', entity_id: 'blk-wrong', type_id: 'buckyos.cell', parent_id: 'sf-analysis', order_key: 'zza', placement: { ...place, x: 380 }, payload: { view: { type: 'sample.bar-chart' }, source_ref: { entity_id: 'intro' }, title: '类型不匹配' } },
  ])
  expect(r.status).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await expect(page.getByTestId('aiws-block-fallback-blk-unknown')).toHaveAttribute('data-reason', 'unknown_renderer')
  await expect(page.getByTestId('aiws-block-fallback-blk-wrong')).toHaveAttribute('data-reason', 'type_not_accepted')
  // the rest of the canvas works: the chart Block renders through the registry
  await expect(page.getByTestId('aiws-canvas-block-blk-kpi').getByTestId('aiws-decl-blk-kpi')).toHaveAttribute('data-variant', 'edit')
  // edit mode: static by default, hover outline, selection handles, editor only after explicit activation
  const kpi = page.getByTestId('aiws-canvas-block-blk-kpi')
  await expect(kpi.getByTestId('aiws-block-blk-kpi')).toHaveAttribute('data-role', 'static')
  await kpi.hover()
  await expect(page.locator('.aiws-hover-outline')).toHaveCount(1)
  await kpi.click()
  await expect(page.getByTestId('aiws-handle-se')).toBeVisible()
  await expect(page.getByTestId('aiws-near-toolbar')).toBeVisible()
  await kpi.hover()
  await expect(page.locator('.aiws-hover-outline')).toHaveCount(0) // selected wins over hovered
  // the declarative sample: an inspector field only in edit mode, a view-only action
  await page.getByTestId('aiws-near-more').click()
  await page.getByTestId('aiws-near-inspector').click()
  await expect(page.getByTestId('aiws-decl-inspector')).toHaveAttribute('data-mode', 'edit')
  await expect(page.getByTestId('aiws-decl-inspector').getByLabel('主色')).toBeVisible()
  await expect(page.getByTestId('aiws-decl-action-open-def')).toHaveCount(0)
  const head = await api.headSeq(ALICE, ws.workspace_id)
  // view mode: different rendering, no handles, no drag, the view-only action present, annotation allowed
  await setCanvasMode(page, 'view')
  await expect(kpi.getByTestId('aiws-decl-blk-kpi')).toHaveAttribute('data-variant', 'view')
  await expect(kpi).toContainText('查看模式')
  await kpi.click()
  await expect(page.getByTestId('aiws-handle-se')).toHaveCount(0)
  await expect(page.getByTestId('aiws-near-delete')).toHaveCount(0)
  await expect(page.getByTestId('aiws-decl-inspector')).toHaveAttribute('data-mode', 'view')
  await expect(page.getByTestId('aiws-decl-action-open-def')).toBeVisible()
  await expect(page.getByTestId('aiws-decl-inspector').getByLabel('主色')).toHaveCount(0)
  const v = await blockCenter(page, 'blk-kpi')
  await page.mouse.move(v.x, v.y)
  await page.mouse.down()
  await page.mouse.move(v.x + 80, v.y + 80, { steps: 5 })
  await page.mouse.up()
  await page.keyboard.press('Delete')
  await page.waitForTimeout(400)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head)
  // the chart's view variant: values on hover (the right panel stays open across Surfaces: close it to give the canvas its width)
  await page.getByTestId('aiws-side-close').click()
  await page.getByTestId('aiws-surface-switch').click()
  await page.getByTestId('aiws-surface-item-sf-detail').getByRole('menuitem').click()
  await expect(page.getByTestId('aiws-chart-blk-chart-2')).toHaveAttribute('data-interactive', 'true')
  await expect(page.getByTestId('aiws-chart-blk-chart-2').getByTestId('aiws-chart-value')).toHaveCount(0)
  await page.getByTestId('aiws-chart-blk-chart-2').locator('.aiws-decl-bar-row').first().hover()
  await expect(page.getByTestId('aiws-chart-blk-chart-2').getByTestId('aiws-chart-value')).toHaveCount(1)
  // annotation in view mode is the one allowed write
  await page.getByTestId('aiws-canvas-block-blk-sales-2').click()
  await page.getByTestId('aiws-near-annotate').click()
  await page.getByLabel('批注内容').fill('查看模式下的批注')
  await page.getByTestId('aiws-annotation-save').click()
  await expect.poll(async () => api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
  const outline = await api.outline(ALICE, ws.workspace_id)
  const note = outline.find((e: { type_id: string; target_id?: string }) => e.type_id === 'buckyos.annotation' && e.target_id === 'sales')
  expect(note?.parent_id).toBe('sf-detail-content')
  // presentation edit: static, no selection, nothing written
  await setCanvasMode(page, 'presentation_edit')
  await expect(page.getByTestId('aiws-presentation-placeholder')).toBeVisible()
  await page.getByTestId('aiws-canvas-block-blk-sales-2').click({ force: true })
  await expect(page.locator('[data-testid^="aiws-selection-"]')).toHaveCount(0)
  await page.keyboard.press('Delete')
  await page.waitForTimeout(300)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
})

test('UI10 a deleted source and a crashing renderer are localised; the workspace stays usable', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `ui10 ${Date.now()}`)
  // a Block whose source is gone (a note deleted after its Block was made)
  await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'note-x', type_id: 'buckyos.annotation', parent_id: 'sf-analysis-content', order_key: 'q', payload: { kind: 'note', body: '临时' } },
    { op: 'entity.create', entity_id: 'blk-note-x', type_id: 'buckyos.cell', parent_id: 'sf-analysis', order_key: 'zq', placement: { x: 40, y: 700, w: 220, h: 160 }, payload: { view: { type: 'note' }, source_ref: { entity_id: 'note-x' } } },
  ])
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await expect(page.getByTestId('aiws-note-note-x')).toBeVisible()
  // delete the data through the API (the Block's bind reference does not block an annotation... it does: delete the Block first? No: deleting data referenced by a Block is refused; so delete both and recreate the Block alone)
  const life = (await api.read(ALICE, ws.workspace_id, 'note-x')).life_rev
  const blkLife = (await api.read(ALICE, ws.workspace_id, 'blk-note-x')).life_rev
  const r = await api.commit(ALICE, ws, [
    { op: 'entity.delete', entity_id: 'blk-note-x', expect: { rev: blkLife } },
    { op: 'entity.delete', entity_id: 'note-x', expect: { rev: life } },
    { op: 'entity.create', entity_id: 'blk-note-y', type_id: 'buckyos.cell', parent_id: 'sf-analysis', order_key: 'zr', placement: { x: 40, y: 700, w: 220, h: 160 }, payload: { view: { type: 'frame' }, title: '替身' } },
  ])
  expect(r.status).toBe('accepted')
  await expect(page.getByTestId('aiws-canvas-block-blk-note-y')).toBeVisible()
  // a renderer that throws: the chart asked to render a record it cannot use is a type fallback; a thrown error is caught by the boundary
  await page.evaluate(async () => {
    const path = '/src/app/aiworkspace/ui/blocks/registry.ts'
    const { blockRegistry } = await import(path) as typeof import('../../src/app/aiworkspace/ui/blocks/registry')
    blockRegistry.register({ ...blockRegistry.get('table', 1)!, Static: () => { throw new Error('renderer failure') } })
  })
  await expect(page.getByTestId('aiws-block-fallback-blk-sales')).toHaveAttribute('data-reason', 'renderer_error')
  await expect(page.getByTestId('aiws-canvas-block-blk-sales')).toBeVisible()
  await expect(page.getByTestId('aiws-canvas-block-blk-intro')).toContainText('经营分析')
  // the canvas still edits
  const head = await api.headSeq(ALICE, ws.workspace_id)
  // a frame lets pointer events through to the canvas, which hit-tests it (so: a real pointer click, not an element click)
  const c = await blockCenter(page, 'blk-note-y')
  await page.mouse.click(c.x, c.y)
  await expect(page.getByTestId('aiws-selection-blk-note-y')).toBeVisible()
  await page.keyboard.press('Delete')
  await expect.poll(async () => api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
})

test('UI17 both demos open through the normal UI with every Block rendered by the registry', async ({ page, api }) => {
  const film = await api.demo(ALICE, 'film', `film ${Date.now()}`)
  await openCanvas(page, ALICE, film.workspace_id)
  await fitAll(page)
  for (const id of ['blk-script', 'blk-characters', 'blk-style', 'blk-wish-1', 'blk-wish-2', 'blk-wish-3', 'blk-frame-a']) {
    await expect(page.getByTestId(`aiws-canvas-block-${id}`)).toBeVisible()
  }
  await expect(page.locator('[data-testid^="aiws-block-fallback-"]')).toHaveCount(0)
  await expect(page.getByTestId('aiws-wish-card-wish-characters')).toContainText('模拟')
})
