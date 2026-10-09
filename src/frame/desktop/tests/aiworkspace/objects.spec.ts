/* 标准对象的交互改进 (doc/workspace/标准对象的交互改进.md §9): object states, hover affordances, the Miro-style
 * selection box with rotation, the icon near toolbar, in-place editing, locking and the version check. */

import { devices, type Page } from '@playwright/test'
import { blockCenter, fitAll, openCanvas, setCanvasMode, test, expect, type Api } from './fixtures'

const ALICE = 'tok-alice'
const SHOTS = 'test-results/aiworkspace-objects'

/** The quarterly demo plus a note, a shape and a second rich text on the analysis canvas. */
async function objectsWorkspace(api: Api, title: string) {
  const ws = await api.demo(ALICE, 'quarterly', title)
  await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'note-1', type_id: 'buckyos.annotation', parent_id: 'sf-analysis-content', order_key: 'x1', payload: { kind: 'note', body: '记得核对华南的成本', style: { color: '#fff2cc' } } },
    { op: 'entity.create', entity_id: 'blk-note', type_id: 'buckyos.cell', parent_id: 'sf-analysis', order_key: 'x1', placement: { x: 40, y: 640, w: 220, h: 160 }, payload: { view: { type: 'note' }, source_ref: { entity_id: 'note-1' } } },
    { op: 'entity.create', entity_id: 'blk-shape', type_id: 'buckyos.cell', parent_id: 'sf-analysis', order_key: 'x2', placement: { x: 300, y: 640, w: 160, h: 120 }, payload: { view: { type: 'shape' }, title: '流程', config: { shape: 'ellipse', fill: '#e8f0fe', stroke: '#4f8df7' } } },
  ])
  return ws
}

async function shot(page: Page, name: string) {
  await page.getByTestId('aiws-canvas-view').screenshot({ path: `${SHOTS}/${name}.png` })
}

async function hover(page: Page, blockId: string) {
  const c = await blockCenter(page, blockId)
  await page.mouse.move(c.x - 5, c.y - 5)
  await page.mouse.move(c.x, c.y)
}

test('OX01/OX02/OX06 resting objects draw no host frame; hover shows the outline and affordances; selection shows round handles', async ({ page, api }) => {
  const ws = await objectsWorkspace(api, `ox01 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await expect(page.getByTestId('aiws-canvas-block-blk-note')).toBeVisible()
  // OX01: the host frame is transparent and borderless; the types draw their own look
  for (const id of ['blk-intro', 'blk-sales', 'blk-kpi']) {
    const frame = page.getByTestId(`aiws-canvas-block-${id}`)
    await expect(frame).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)')
    await expect(frame).toHaveCSS('border-top-width', '0px')
  }
  await page.mouse.move(5, 500)
  await shot(page, 'ox01-rest-light')

  // OX02: a solid outline at once, the name label after a short delay; nothing while dragging
  await hover(page, 'blk-sales')
  await expect(page.getByTestId('aiws-hover-outline')).toBeVisible()
  await expect(page.getByTestId('aiws-afford-name')).toContainText('原始销售数据')
  await expect(page.getByTestId('aiws-afford-add-row')).toBeVisible()
  await shot(page, 'ox02-hover-table')
  await page.mouse.move(5, 500)
  await expect(page.getByTestId('aiws-afford-name')).toHaveCount(0)
  await hover(page, 'blk-wish')
  await expect(page.getByTestId('aiws-afford-wish-run')).toBeVisible()
  await expect(page.getByTestId('aiws-afford-open')).toBeVisible()
  await shot(page, 'ox02-hover-wish')
  await hover(page, 'blk-note')
  await expect(page.getByTestId('aiws-afford-reply')).toBeVisible()

  // OX06: corner handles are circles, edges resize, the rotation handle sits outside the bottom-left corner
  await page.getByTestId('aiws-canvas-block-blk-shape').click()
  await expect(page.getByTestId('aiws-selection-blk-shape')).toBeVisible()
  for (const h of ['nw', 'ne', 'se', 'sw']) await expect(page.getByTestId(`aiws-handle-${h}`)).toBeVisible()
  await expect(page.getByTestId('aiws-handle-n')).toHaveCount(0)
  await expect(page.getByTestId('aiws-edge-e')).toBeAttached()
  await expect(page.getByTestId('aiws-rotate-handle')).toBeVisible()
  await expect(page.getByTestId('aiws-near-shape-kind')).toBeVisible()
  await shot(page, 'ox06-selected-shape')

  // dragging hides the affordances and the toolbar
  const c = await blockCenter(page, 'blk-shape')
  await page.mouse.move(c.x, c.y)
  await page.mouse.down()
  await page.mouse.move(c.x + 40, c.y + 10, { steps: 4 })
  await expect(page.getByTestId('aiws-near-toolbar')).toHaveCount(0)
  await expect(page.locator('.aiws-afford')).toHaveCount(0)
  await page.mouse.up()
})

test('OX01 dark theme: resting, hovered and selected objects read on the dark canvas', async ({ page, api }) => {
  const ws = await objectsWorkspace(api, `ox01d ${Date.now()}`)
  await page.addInitScript(() => { window.localStorage.setItem('buckyos.prototype.theme.v1', 'dark') })
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await expect(page.getByTestId('aiws-canvas-block-blk-note')).toBeVisible()
  await page.mouse.move(5, 500)
  await shot(page, 'ox01-rest-dark')
  await hover(page, 'blk-wish')
  await expect(page.getByTestId('aiws-afford-name')).toBeVisible()
  await shot(page, 'ox02-hover-wish-dark')
  await page.getByTestId('aiws-canvas-block-blk-note').click()
  await expect(page.getByTestId('aiws-near-note-color')).toBeVisible()
  await shot(page, 'ox06-selected-note-dark')
})

test('OX03/OX13 a wish opens in the right panel; "run" there or on hover starts its next step; the card stays a card', async ({ page, api }) => {
  const ws = await objectsWorkspace(api, `ox03 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  // hover → run: the panel opens and the analysis (first pass) runs at once
  await hover(page, 'blk-wish')
  await page.getByTestId('aiws-afford-wish-run').click()
  await expect(page.locator('[data-testid="aiws-side-panel"][data-tab="wish"]')).toBeVisible()
  await expect(page.getByTestId('aiws-wish-analysis')).toContainText('模拟执行器', { timeout: 20_000 })
  await expect(page.getByTestId('aiws-wish-card-state')).toContainText('正在右侧面板中编辑')
  await expect(page.getByTestId('aiws-canvas-block-blk-wish').locator('[data-role="editor"]')).toHaveCount(0)
  await shot(page, 'ox13-wish-panel')
  // the canvas stays usable: picking an object offers it as an input
  await page.getByTestId('aiws-canvas-block-blk-intro').click()
  await expect(page.getByTestId('aiws-wish-add-selection')).toContainText('1 项')
  await page.getByTestId('aiws-wish-add-selection').click()
  await expect(page.getByTestId('aiws-wish-input')).toHaveCount(2)
  // the toolbar's run executes once the analysis is current again
  await page.getByTestId('aiws-canvas-block-blk-wish').click()
  await page.getByTestId('aiws-near-wish-run').click()
  await expect(page.getByTestId('aiws-wish-needs-analysis')).toHaveCount(0, { timeout: 20_000 })
  await page.getByTestId('aiws-near-wish-run').click()
  await expect(page.getByTestId('aiws-wish-candidate')).toBeVisible({ timeout: 30_000 })
  // closing the panel: the card is a plain card again
  await page.getByTestId('aiws-side-close').click()
  await expect(page.getByTestId('aiws-wish-card-state')).toHaveCount(0)
})

test('OX04/OX12 the table: "+ row" on hover adds a record; filter, fields and new record are toolbar tools, not a bar in the Block', async ({ page, api }) => {
  const ws = await objectsWorkspace(api, `ox04 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  const before = (await api.rpc(ALICE, 'doc.query', { workspace_id: ws.workspace_id, source_id: 'sales', limit: 1 })).total
  await hover(page, 'blk-sales')
  await page.getByTestId('aiws-afford-add-row').click()
  const frame = page.getByTestId('aiws-canvas-block-blk-sales')
  await expect(frame.getByLabel('新记录 区域')).toBeFocused()
  await page.keyboard.type('东北')
  await frame.getByLabel('新记录 产品').fill('雪地靴')
  await frame.getByTestId('aiws-add-record-submit').click()
  await expect.poll(async () => (await api.rpc(ALICE, 'doc.query', { workspace_id: ws.workspace_id, source_id: 'sales', limit: 1 })).total).toBe(before + 1)
  // editing: no bar inside the Block; the tools are in the near toolbar and show which panel is open
  await expect(frame.locator('.aiws-table-bar')).toHaveCount(0)
  await page.getByTestId('aiws-near-table-filter').click()
  await expect(frame.locator('.aiws-filter-panel')).toBeVisible()
  await expect(page.getByTestId('aiws-near-table-filter')).toHaveAttribute('aria-pressed', 'true')
  await page.getByTestId('aiws-near-table-fields').click()
  await expect(frame.locator('.aiws-fields')).toBeVisible()
  // a plain selection offers the same tools and opens the editor on the right panel
  await page.keyboard.press('Escape')
  await page.mouse.click(600, 860)
  await page.getByTestId('aiws-canvas-block-blk-sales').click({ position: { x: 20, y: 10 } })
  await page.getByTestId('aiws-near-table-filter').click()
  await expect(frame.locator('.aiws-filter-panel')).toBeVisible()
})

test('OX05/OX11 notes: typing starts editing, the colour and size come from the toolbar, a reply is an annotation on the note', async ({ page, api }) => {
  const ws = await objectsWorkspace(api, `ox05 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  const note = page.getByTestId('aiws-canvas-block-blk-note')
  await note.click()
  // a selected note takes typing at once; its look does not change while editing (no frame, no status row)
  await page.keyboard.type('!')
  await expect(note.getByLabel('便签内容')).toBeFocused()
  await expect(note.locator('.aiws-note-meta')).toHaveCount(0)
  await page.keyboard.press('Escape')
  await page.mouse.click(600, 860)
  await expect.poll(async () => (await api.read(ALICE, ws.workspace_id, 'note-1')).content.payload.body).toBe('记得核对华南的成本!')
  // colour and size
  await note.click()
  await page.getByTestId('aiws-near-note-color').click()
  await page.getByTestId('aiws-color-dbe8ff').click()
  await expect.poll(async () => (await api.read(ALICE, ws.workspace_id, 'note-1')).content.payload.style.color).toBe('#dbe8ff')
  await expect(note.locator('.aiws-note')).toHaveCSS('background-color', 'rgb(219, 232, 255)')
  await page.getByTestId('aiws-near-note-size').click()
  await page.getByTestId('aiws-near-note-size-l').click()
  await expect(note.locator('.aiws-note')).toHaveAttribute('data-size', 'l')
  // reply: an annotation whose target is the note; the note counts it and opens the filtered panel
  await page.getByTestId('aiws-near-note-reply').click()
  await expect(page.getByTestId('aiws-annotation-draft-target')).toContainText('回复便签')
  await page.getByLabel('批注内容').fill('已核对，没问题')
  await page.getByTestId('aiws-annotation-save').click()
  await expect(page.getByTestId('aiws-note-replies-note-1')).toHaveText('1 条回复')
  const reply = (await api.outline(ALICE, ws.workspace_id)).find((e: { type_id: string; target_id?: string }) => e.type_id === 'buckyos.annotation' && e.target_id === 'note-1')
  expect(reply).toBeTruthy()
  await page.getByTestId('aiws-note-replies-note-1').click()
  await expect(page.getByTestId('aiws-annotations-filter')).toBeVisible()
  await expect(page.getByTestId('aiws-annotation')).toHaveCount(1)
  await page.getByTestId('aiws-annotations-filter-clear').click()
  await expect(page.getByTestId('aiws-annotations-filter')).toHaveCount(0)
})

test('OX06/OX07 resize by corner and edge (notes keep their proportions), rotate with the handle (Shift snaps 15°), and the turn survives reload, grouping and hit tests', async ({ page, api }) => {
  const ws = await objectsWorkspace(api, `ox07 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  const placement = async (id: string) => (await api.outline(ALICE, ws.workspace_id)).find((e: { entity_id: string }) => e.entity_id === id).placement
  // edge: one direction only
  await page.getByTestId('aiws-canvas-block-blk-shape').click()
  const edge = await page.getByTestId('aiws-handle-ne').boundingBox()
  const east = await page.getByTestId('aiws-handle-se').boundingBox()
  if (!edge || !east) throw new Error('no handles')
  const ex = edge.x + edge.width / 2, ey = (edge.y + east.y) / 2 + edge.height / 2
  await page.mouse.move(ex, ey)
  await page.mouse.down()
  await page.mouse.move(ex + 30, ey + 30, { steps: 5 })
  await expect(page.getByTestId('aiws-gesture-hint')).toContainText('×')
  await page.mouse.up()
  await expect.poll(async () => (await placement('blk-shape')).w).toBeGreaterThan(160)
  expect((await placement('blk-shape')).h).toBe(120)
  // a note's corner keeps the proportions
  await page.getByTestId('aiws-canvas-block-blk-note').click()
  const se = await page.getByTestId('aiws-handle-se').boundingBox()
  if (!se) throw new Error('no se handle')
  await page.mouse.move(se.x + se.width / 2, se.y + se.height / 2)
  await page.mouse.down()
  await page.mouse.move(se.x + 60, se.y + 5, { steps: 5 })
  await page.mouse.up()
  await expect.poll(async () => (await placement('blk-note')).w).toBeGreaterThan(220)
  const resized = await placement('blk-note')
  expect(Math.abs(resized.w / resized.h - 220 / 160)).toBeLessThan(0.02)
  // rotate with Shift: snapped to 15°
  await page.getByTestId('aiws-canvas-block-blk-shape').click()
  const handle = await page.getByTestId('aiws-rotate-handle').boundingBox()
  const box = await page.getByTestId('aiws-canvas-block-blk-shape').boundingBox()
  if (!handle || !box) throw new Error('no rotation handle')
  const centre = { x: box.x + box.width / 2, y: box.y + box.height / 2 }
  await page.mouse.move(handle.x + handle.width / 2, handle.y + handle.height / 2)
  await page.keyboard.down('Shift')
  await page.mouse.down()
  // a quarter turn clockwise about the centre
  const r = Math.hypot(handle.x + handle.width / 2 - centre.x, handle.y + handle.height / 2 - centre.y)
  const a0 = Math.atan2(handle.y + handle.height / 2 - centre.y, handle.x + handle.width / 2 - centre.x)
  for (let i = 1; i <= 6; i++) { const a = a0 + (Math.PI / 2) * (i / 6) + 0.05; await page.mouse.move(centre.x + r * Math.cos(a), centre.y + r * Math.sin(a)) }
  await expect(page.getByTestId('aiws-gesture-hint')).toContainText('°')
  await page.mouse.up()
  await page.keyboard.up('Shift')
  await expect.poll(async () => (await placement('blk-shape')).rotation).toBe(90)
  await expect(page.getByTestId('aiws-canvas-block-blk-shape')).toHaveAttribute('data-rotation', '90')
  await expect(page.getByTestId('aiws-selection-blk-shape')).toHaveAttribute('transform', /rotate\(90 /)
  // a rotated Block is hit inside its turned shape
  await page.mouse.click(600, 860)
  await page.mouse.click(centre.x, centre.y)
  await expect(page.getByTestId('aiws-selection-blk-shape')).toBeVisible()
  // moving keeps the turn; grouping and ungrouping keep it too
  await page.keyboard.press('ArrowRight')
  await expect.poll(async () => (await placement('blk-shape')).x).toBe(301)
  expect((await placement('blk-shape')).rotation).toBe(90)
  await page.keyboard.down('Shift')
  await page.getByTestId('aiws-canvas-block-blk-note').click()
  await page.keyboard.up('Shift')
  await page.getByTestId('aiws-near-group').click()
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).find((e: { entity_id: string }) => e.entity_id === 'blk-shape').parent_id).not.toBe('sf-analysis')
  expect((await placement('blk-shape')).rotation).toBe(90)
  await page.keyboard.press('Control+Shift+G')
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).find((e: { entity_id: string }) => e.entity_id === 'blk-shape').parent_id).toBe('sf-analysis')
  expect((await placement('blk-shape')).rotation).toBe(90)
  // after a reload the turn is drawn the same; double-clicking the handle turns it back
  await page.reload()
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await expect(page.getByTestId('aiws-canvas-block-blk-shape')).toHaveAttribute('data-rotation', '90')
  await page.getByTestId('aiws-canvas-block-blk-shape').click()
  await page.getByTestId('aiws-rotate-handle').dblclick()
  await expect.poll(async () => (await placement('blk-shape')).rotation ?? 0).toBe(0)
})

test('OX08/OX09 several objects: member outlines and one group box; locked objects keep still, refuse deletion and offer only unlock', async ({ page, api }) => {
  const ws = await objectsWorkspace(api, `ox09 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await page.getByTestId('aiws-canvas-block-blk-note').click()
  await page.keyboard.down('Shift')
  await page.getByTestId('aiws-canvas-block-blk-shape').click()
  await page.keyboard.up('Shift')
  await expect(page.getByTestId('aiws-multi-selection')).toBeVisible()
  await expect(page.locator('.aiws-selection-box.is-member')).toHaveCount(2)
  await expect(page.getByTestId('aiws-near-group')).toBeVisible()
  await expect(page.getByTestId('aiws-handle-se')).toHaveCount(0)
  // lock the shape
  await page.mouse.click(600, 860)
  await page.getByTestId('aiws-canvas-block-blk-shape').click()
  await page.getByTestId('aiws-near-lock').click()
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).find((e: { entity_id: string }) => e.entity_id === 'blk-shape').locked).toBe(true)
  await expect(page.getByTestId('aiws-lock-badge')).toBeVisible()
  await expect(page.getByTestId('aiws-handle-se')).toHaveCount(0)
  await expect(page.getByTestId('aiws-rotate-handle')).toHaveCount(0)
  await expect(page.getByTestId('aiws-near-unlock')).toBeVisible()
  await expect(page.getByTestId('aiws-near-shape-kind')).toHaveCount(0)
  await shot(page, 'ox09-locked')
  // dragging does not move it; Delete skips it with a note
  const before = (await api.outline(ALICE, ws.workspace_id)).find((e: { entity_id: string }) => e.entity_id === 'blk-shape').placement
  const c = await blockCenter(page, 'blk-shape')
  await page.mouse.move(c.x, c.y)
  await page.mouse.down()
  await page.mouse.move(c.x + 60, c.y + 40, { steps: 5 })
  await page.mouse.up()
  await page.keyboard.press('Delete')
  await expect(page.getByTestId('aiws-notice').filter({ hasText: '已锁定' })).toBeVisible()
  const after = (await api.outline(ALICE, ws.workspace_id)).find((e: { entity_id: string }) => e.entity_id === 'blk-shape')
  expect(after.placement).toEqual(before)
  expect(after.deleted ?? false).toBe(false)
  // in a selection with others only the unlocked ones move
  await page.keyboard.down('Shift')
  await page.getByTestId('aiws-canvas-block-blk-note').click()
  await page.keyboard.up('Shift')
  await page.keyboard.press('ArrowDown')
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).find((e: { entity_id: string }) => e.entity_id === 'blk-note').placement.y).toBe(641)
  expect((await api.outline(ALICE, ws.workspace_id)).find((e: { entity_id: string }) => e.entity_id === 'blk-shape').placement).toEqual(before)
  // unlock
  await page.mouse.click(600, 860)
  await page.getByTestId('aiws-canvas-block-blk-shape').click()
  await page.getByTestId('aiws-near-unlock').click()
  await expect(page.getByTestId('aiws-handle-se')).toBeVisible()
  expect((await api.outline(ALICE, ws.workspace_id)).find((e: { entity_id: string }) => e.entity_id === 'blk-shape').locked ?? null).toBeNull()
  // view mode: a selection box, no handles, no type tools
  await setCanvasMode(page, 'view')
  await page.getByTestId('aiws-canvas-block-blk-shape').click()
  await expect(page.getByTestId('aiws-selection-blk-shape')).toBeVisible()
  await expect(page.getByTestId('aiws-handle-se')).toHaveCount(0)
  await expect(page.getByTestId('aiws-near-shape-kind')).toHaveCount(0)
})

test('OX10 rich text: double-click, Enter or typing opens it in place; the format tools are in the near toolbar and follow the caret; links come from a searchable list', async ({ page, api }) => {
  const ws = await objectsWorkspace(api, `ox10 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  const intro = page.getByTestId('aiws-canvas-block-blk-intro')
  // typing on a selected text appends at the end
  await intro.click()
  await expect(page.getByTestId('aiws-near-text-size')).toBeVisible()
  await page.keyboard.type('X')
  const prose = page.getByTestId('aiws-richtext-intro')
  await expect(prose).toBeFocused()
  await expect(prose).toContainText('模拟"。X')
  // no toolbar, lock bar or form inside the Block; the format tools are in the near toolbar
  await expect(intro.locator('.aiws-inline-tools, .aiws-toolbar, .aiws-lock-bar, select')).toHaveCount(0)
  await expect(page.getByTestId('aiws-near-bold')).toBeVisible()
  await expect(page.getByTestId('aiws-near-bold')).toHaveAttribute('aria-pressed', 'false')
  await page.keyboard.press('Shift+ArrowLeft')
  await page.getByTestId('aiws-near-bold').click()
  await expect(page.getByTestId('aiws-near-bold')).toHaveAttribute('aria-pressed', 'true')
  await expect(prose.locator('strong')).toHaveText('X')
  await shot(page, 'ox10-richtext-editing')
  // block type and the searchable link picker
  await page.getByTestId('aiws-near-block-type').click()
  await page.getByTestId('aiws-near-block-type-h2').click()
  await expect(prose.locator('h2')).toHaveCount(1)
  await page.keyboard.press('End')
  await page.getByTestId('aiws-near-link').click()
  await page.getByTestId('aiws-link-search').fill('原始销售')
  await page.getByTestId('aiws-link-pick-sales').click()
  await expect(prose.locator('.aiws-object-link')).toContainText('原始销售数据')
  // Esc leaves the editor with the text committed
  await page.keyboard.press('Escape')
  await page.mouse.click(600, 860)
  await expect.poll(async () => JSON.stringify((await api.read(ALICE, ws.workspace_id, 'intro')).content.content)).toContain('"type":"strong"')
  // a size for the whole Block
  await intro.click()
  await page.getByTestId('aiws-near-text-size').click()
  await page.getByTestId('aiws-near-text-size-l').click()
  await expect(intro.locator('[data-text-size="l"]')).toBeVisible()
})

test('OX14 the near toolbar avoids the floating toolbars and moves what does not fit into "more"; the context menu lists the same actions', async ({ page, api }) => {
  const ws = await objectsWorkspace(api, `ox14 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await page.getByTestId('aiws-canvas-block-blk-intro').dblclick()
  await expect(page.getByTestId('aiws-near-bold')).toBeVisible()
  const slots = () => page.locator('[data-testid="aiws-near-toolbar"] [data-near-index] [data-testid^="aiws-near-"]').evaluateAll((els) => els.map((el) => el.getAttribute('data-testid') ?? ''))
  const full = await slots()
  // the application window narrowed (as UI-P02 does): the toolbar stays inside it and clear of the floating toolbars
  const root = page.getByTestId('aiws-root')
  for (const width of [1024, 768, 560]) {
    await root.evaluate((el, w) => { el.style.width = `${w}px`; el.style.flex = 'none' }, width)
    const area = await page.getByTestId('aiws-canvas').boundingBox()
    if (!area) throw new Error('no canvas')
    await expect.poll(async () => { const b = await page.getByTestId('aiws-near-toolbar').boundingBox(); return b ? b.x + b.width : Infinity }).toBeLessThanOrEqual(area.x + area.width)
    const bar = await page.getByTestId('aiws-near-toolbar').boundingBox()
    const tools = await page.getByTestId('aiws-main-toolbar').boundingBox()
    if (bar && tools) expect(bar.y >= tools.y + tools.height || bar.x >= tools.x + tools.width).toBe(true)
  }
  const shown = await slots()
  expect(shown.length).toBeLessThan(full.length)
  // what does not fit is in "more" (the AI button is the last of the row)
  await page.getByTestId('aiws-near-more').click()
  await expect(page.getByRole('menu').getByTestId('aiws-near-ai')).toBeVisible()
  await page.keyboard.press('Escape')
  await root.evaluate((el) => { el.style.width = ''; el.style.flex = '' })
  // the context menu of a Block: the same actions as "more", delete included
  await page.mouse.click(600, 860)
  await page.getByTestId('aiws-canvas-block-blk-shape').click({ button: 'right' })
  await expect(page.getByTestId('aiws-menu-delete')).toBeVisible()
  await expect(page.getByTestId('aiws-menu-lock')).toBeVisible()
})

test('OX19 a service on another protocol version stops this page with "please refresh" instead of a broken session', async ({ page, api }) => {
  const ws = await objectsWorkspace(api, `ox19 ${Date.now()}`)
  // the service answers as a newer version would
  await page.route('**/kapi/aiworkspace**', async (route) => {
    const body = route.request().postDataJSON() as { method?: string } | null
    if (body?.method !== 'ws.get_info') { await route.continue(); return }
    const response = await route.fetch()
    const json = await response.json() as { result?: { protocol_version?: string } }
    if (json.result) json.result.protocol_version = '9.9'
    await route.fulfill({ response, json })
  })
  await openCanvas(page, ALICE, ws.workspace_id).catch(() => undefined)
  await expect(page.getByTestId('aiws-stopped')).toContainText('请刷新页面', { timeout: 30_000 })
  await expect(page.getByTestId('aiws-canvas-block-blk-note')).toBeVisible()
})

test.describe('OX15 a tablet (touch, not a phone)', () => {
  const { defaultBrowserType: _browser, ...tablet } = devices['iPad Pro 11 landscape']
  void _browser
  test.use({ ...tablet })

  test('handles are large enough for a finger and the affordances show on selection', async ({ page, api }) => {
    const ws = await objectsWorkspace(api, `ox15 ${Date.now()}`)
    await openCanvas(page, ALICE, ws.workspace_id)
    await fitAll(page)
    await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-pointer', 'touch')
    const c = await blockCenter(page, 'blk-wish')
    await page.touchscreen.tap(c.x, c.y)
    await expect(page.getByTestId('aiws-selection-blk-wish')).toBeVisible()
    await expect(page.getByTestId('aiws-handle-se')).toHaveAttribute('r', '7')
    await expect(page.locator('.aiws-handle-hit').first()).toHaveAttribute('r', '22')
    // no hover on touch: the affordances come with the selection
    await expect(page.getByTestId('aiws-afford-wish-run')).toBeVisible()
    await expect(page.getByTestId('aiws-rotate-handle')).toHaveCSS('width', '36px')
    await shot(page, 'ox15-tablet-selected')
  })
})
