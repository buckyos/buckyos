/* UI 框架 PRD §10, second pass (§18.3): the Block types that drew their own toolbar — connectors, mind map nodes, the mind
 * map itself, wishes — declare actions and properties, and the Block toolbar, the context menu, the properties panel and
 * the phone's context window come from that one declaration; the wish's hover "run" is one of its actions; the
 * properties panel of several Blocks shows what they share and writes it in one step; the to-do and calendar controls
 * use the framework's panels and the Block's menu instead of their own ⋯ (§7.5). Real backend. */

import { devices, type CDPSession, type Page } from '@playwright/test'
import { blockCenter, core, fitAll, openCanvas, openStatus, openWorkspacePage, setCanvasMode, test, expect, type Api } from './fixtures'

const ALICE = 'tok-alice'

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

const shape = (id: string, x: number, y: number, fill: string, extra: Json = {}) => ({ op: 'entity.create', entity_id: id, type_id: 'buckyos.cell', parent_id: 'free', order_key: `k${id}`,
  placement: { x, y, w: 160, h: 100 }, payload: { view: { type: 'shape' }, title: id.toUpperCase(), config: { shape: 'rect', fill, stroke: '#4f8df7' }, ...extra } })
const AUTO = { x: 'auto', y: 'auto', w: 'auto', h: 'auto' }
const node = (id: string, parent: string, order: string, title: string) => ({ op: 'entity.create', entity_id: id, type_id: 'buckyos.cell', parent_id: parent, order_key: order, placement: AUTO, payload: { view: { type: 'mindnode' }, title } })

/** A free canvas with two shapes and a line between them, a mind map (root, two branches, one of them with a child)
 * and a label. */
async function contractWorkspace(api: Api, title: string) {
  const ws = await api.freeCanvas(ALICE, title)
  const made = await api.commit(ALICE, ws, [
    shape('a', 60, 60, '#dbe8ff'), shape('b', 560, 60, '#ffd9d9'),
    { op: 'entity.create', entity_id: 'ab', type_id: 'buckyos.cell', parent_id: 'free', order_key: 'kz', placement: { x: 220, y: 100, w: 340, h: 20 },
      payload: { view: { type: 'connector', version: 1 }, start: { entity_id: 'a', anchor: { kind: 'named', id: 'e' } }, end: { entity_id: 'b', anchor: { kind: 'named', id: 'w' } } } },
    { op: 'entity.create', entity_id: 'mm', type_id: 'buckyos.container', parent_id: 'free', order_key: 'm', placement: { x: 60, y: 300, w: 'auto', h: 'auto' },
      payload: { kind: 'mindmap', layout: { mode: 'tree' }, title: '计划', config: { direction: 'right', edge: 'curve' } } },
    node('mroot', 'mm', 'a', '计划'), node('n1', 'mroot', 'a', '市场'), node('n1a', 'n1', 'a', '访谈'), node('n2', 'mroot', 'b', '产品'),
    { op: 'entity.create', entity_id: 'lbl', type_id: 'buckyos.cell', parent_id: 'free', order_key: 'l', placement: { x: 760, y: 300, w: 200, h: 40 }, payload: { view: { type: 'label' }, title: '说明' } },
  ])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  return ws
}

/** The screen point at ratio `t` along a drawn line (mapped through its SVG's viewBox). */
async function linePoint(page: Page, id: string, t: number): Promise<{ x: number; y: number }> {
  return page.getByTestId(`aiws-canvas-block-${id}`).evaluate((frame, ratio) => {
    const svg = frame.querySelector('svg')!
    const path = frame.querySelector('path.aiws-connector-path') as SVGPathElement
    const p = path.getPointAtLength(path.getTotalLength() * ratio)
    const [vx, vy, vw, vh] = (svg.getAttribute('viewBox') ?? '0 0 1 1').split(/\s+/).map(Number)
    const r = svg.getBoundingClientRect()
    return { x: r.x + ((p.x - vx) * r.width) / vw, y: r.y + ((p.y - vy) * r.height) / vh }
  }, t)
}

/** Add a Block to the selection (Shift and a press at its centre). */
async function shiftClick(page: Page, id: string) {
  const c = await blockCenter(page, id)
  await page.keyboard.down('Shift')
  await page.mouse.click(c.x, c.y)
  await page.keyboard.up('Shift')
}

const payloadOf = async (api: Api, ws: { workspace_id: string }, id: string): Promise<Json> => (await api.read(ALICE, ws.workspace_id, id)).content.payload
const outlineOf = async (api: Api, ws: { workspace_id: string }, id: string): Promise<Json> => (await api.outline(ALICE, ws.workspace_id)).find((e: Json) => e.entity_id === id)
const panel = (page: Page) => page.getByTestId('aiws-props-panel')
const toolbar = (page: Page) => page.getByTestId('aiws-near-toolbar')

/** The Block menu of the selection (Shift+F10), its entries' test ids. */
async function blockMenu(page: Page): Promise<string[]> {
  await page.keyboard.press('Shift+F10')
  const menu = page.getByTestId('aiws-context-menu')
  await expect(menu).toBeVisible()
  return menu.locator('[data-testid^="aiws-menu-"]').evaluateAll((items) => items.map((item) => item.getAttribute('data-testid') ?? ''))
}

test('§15-33/34 a connector: its tools are its actions and quick properties — the toolbar, the menu and the properties panel read one declaration', async ({ page, api }) => {
  const ws = await contractWorkspace(api, `bc-line ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  const mid = await linePoint(page, 'ab', 0.5)
  await page.mouse.click(mid.x, mid.y)
  // primary actions (edit the label, reset the route — no bends yet) and the quick look properties
  for (const id of ['line-label', 'line-reset', 'line-route', 'line-start-cap', 'line-end-cap', 'line-dash']) await expect(toolbar(page).getByTestId(`aiws-near-${id}`), id).toBeAttached()
  await expect(toolbar(page).getByTestId('aiws-near-line-reset')).toBeDisabled()
  // the same actions in the Block menu: reset in the type's group (first), editing the label in "open" (it is the line's
  // "edit content", bound to Enter)
  const items = await blockMenu(page)
  expect(items.slice(0, 2)).toEqual(['aiws-menu-line-reset', 'aiws-menu-line-label'])
  await expect(page.getByTestId('aiws-menu-line-reset')).toBeDisabled()
  await page.keyboard.press('Escape')
  // the panel lists every property, the quick ones too: the route changed there is the toolbar's route
  await page.keyboard.press('F2')
  const route = panel(page).getByTestId('aiws-connector-field-route')
  await expect(route).toHaveValue('straight')
  await route.selectOption('elbow')
  await expect.poll(async () => (await payloadOf(api, ws, 'ab')).route).toBe('elbow')
  await toolbar(page).getByTestId('aiws-near-line-route').click()
  await expect(page.getByTestId('aiws-near-line-route-elbow')).toHaveAttribute('aria-checked', 'true')
  await page.keyboard.press('Escape')
  // and the toolbar's dash is the panel's
  await toolbar(page).getByTestId('aiws-near-line-dash').click()
  await page.getByTestId('aiws-near-line-dash-dotted').click()
  await expect.poll(async () => (await payloadOf(api, ws, 'ab')).config?.dash).toBe('dotted')
  await expect(panel(page).getByTestId('aiws-connector-field-dash')).toHaveValue('dotted')
  // a default stored as absent: rounded corners are on by default, turned off they are stored
  const rounded = panel(page).getByTestId('aiws-connector-field-rounded')
  await rounded.scrollIntoViewIfNeeded()
  await expect(rounded).toBeChecked()
  await rounded.click()
  await expect.poll(async () => (await payloadOf(api, ws, 'ab')).config?.rounded).toBe(false)
  // each change one undoable step
  await panel(page).getByTestId('aiws-props-head').click()
  await page.keyboard.press('Control+z')
  await expect.poll(async () => (await payloadOf(api, ws, 'ab')).config?.rounded ?? true).toBe(true)
  await expect.poll(async () => (await payloadOf(api, ws, 'ab')).config?.dash).toBe('dotted')
})

test('§15-33/34 a mind map: the node\'s and the map\'s tools are actions and properties — toolbar, menu and panel alike', async ({ page, api }) => {
  const ws = await contractWorkspace(api, `bc-mind ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await page.getByTestId('aiws-canvas-block-n1').click()
  for (const id of ['mindmap-add-child', 'mindmap-add-sibling', 'mindmap-insert', 'mindmap-fold', 'mindmap-branch']) await expect(toolbar(page).getByTestId(`aiws-near-${id}`), id).toBeAttached()
  const items = await blockMenu(page)
  for (const id of ['aiws-menu-mindmap-add-child', 'aiws-menu-mindmap-add-sibling', 'aiws-menu-mindmap-insert', 'aiws-menu-mindmap-fold']) expect(items, id).toContain(id)
  // folding from the menu: the toolbar's fold shows it pressed, named "expand"
  await page.getByTestId('aiws-menu-mindmap-fold').click()
  await expect(page.getByTestId('aiws-canvas-block-n1a')).toHaveCount(0)
  await expect(toolbar(page).getByTestId('aiws-near-mindmap-fold')).toHaveAttribute('aria-pressed', 'true')
  await expect(toolbar(page).getByTestId('aiws-near-mindmap-fold')).toHaveAttribute('aria-label', '展开')
  await toolbar(page).getByTestId('aiws-near-mindmap-fold').click()
  await expect(page.getByTestId('aiws-canvas-block-n1a')).toBeVisible()
  // the branch colour is a property: in the panel, written as the node's own config
  await page.keyboard.press('F2')
  await panel(page).getByTestId('aiws-mindnode-field-branch_color').getByTestId('aiws-color-d1242f').click()
  await expect.poll(async () => (await outlineOf(api, ws, 'n1')).branch_color).toBe('#d1242f')
  await page.getByTestId('aiws-props-close').click()
  // the root has no branch colour and no sibling
  await page.getByTestId('aiws-canvas-block-mroot').click()
  await expect(toolbar(page).getByTestId('aiws-near-mindmap-add-child')).toBeAttached()
  await expect(toolbar(page).getByTestId('aiws-near-mindmap-add-sibling')).toHaveCount(0)
  await expect(toolbar(page).getByTestId('aiws-near-mindmap-branch')).toHaveCount(0)
  // the map itself: direction, branches, spacing as quick properties; outline and exports as actions
  // a press on the map where no node is (its frame's corner)
  const map = (await page.getByTestId('aiws-canvas-block-mm').boundingBox())!
  await page.mouse.click(map.x + 4, map.y + 4)
  await expect(page.getByTestId('aiws-selection-mm')).toBeVisible()
  for (const id of ['mindmap-outline', 'mindmap-direction', 'mindmap-edge', 'mindmap-spacing']) await expect(toolbar(page).getByTestId(`aiws-near-${id}`), id).toBeAttached()
  const mapItems = await blockMenu(page)
  for (const id of ['aiws-menu-mindmap-outline', 'aiws-menu-mindmap-copy-markdown', 'aiws-menu-mindmap-export-markdown', 'aiws-menu-mindmap-export-opml']) expect(mapItems, id).toContain(id)
  await page.keyboard.press('Escape')
  await page.keyboard.press('F2')
  const direction = panel(page).getByTestId('aiws-mindmap-field-direction')
  await expect(direction).toHaveValue('right')
  await direction.selectOption('left')
  await expect.poll(async () => (await outlineOf(api, ws, 'mm')).config?.direction).toBe('left')
  await toolbar(page).getByTestId('aiws-near-mindmap-direction').click()
  await expect(page.getByTestId('aiws-near-mindmap-direction-left')).toHaveAttribute('aria-checked', 'true')
})

test('§10.1 the wish\'s hover "run" is its action: on the toolbar and in the menu; outside edit mode it runs only a macro that can run, and says why not', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `bc-wish ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await page.getByTestId('aiws-canvas-block-blk-wish').click()
  await expect(toolbar(page).getByTestId('aiws-near-wish-run')).toBeEnabled()
  await expect(toolbar(page).getByTestId('aiws-near-wish-open')).toBeAttached()
  expect(await blockMenu(page)).toContain('aiws-menu-wish-run')
  await page.keyboard.press('Escape')
  // view mode: offered, disabled with the reason (no macro yet); hovering shows no run button
  await setCanvasMode(page, 'view')
  await page.getByTestId('aiws-canvas-block-blk-wish').click()
  const run = toolbar(page).getByTestId('aiws-near-wish-run')
  await expect(run).toBeDisabled()
  await expect(run).toHaveAttribute('title', /还不能在这里运行/)
  await expect(toolbar(page).getByTestId('aiws-near-wish-open')).toHaveCount(0)
  await page.mouse.move(5, 500)
  const card = (await page.getByTestId('aiws-canvas-block-blk-wish').boundingBox())!
  await page.mouse.move(card.x + card.width / 2, card.y + card.height / 2)
  await expect(page.getByTestId('aiws-afford-name')).toBeVisible()
  await expect(page.getByTestId('aiws-afford-wish-run')).toHaveCount(0)
})

test('§10.1 the wish\'s "run" in view mode runs its macro as the panel\'s run does — its start and end in the work log', async ({ page, api }) => {
  test.slow()
  // the scripted model (mock-llm.mjs) analyses and codes the real executor's wish in this browser
  const llm = `http://127.0.0.1:${process.env.AIWS_E2E_LLM_PORT}/v1`
  await page.addInitScript((baseUrl) => window.localStorage.setItem('aiworkspace.dev', JSON.stringify({ llm: { baseUrl } })), llm)
  const ws = await api.demo(ALICE, 'quarterly', `bc-macro ${Date.now()}`)
  const wish = await api.read(ALICE, ws.workspace_id, 'wish-analysis')
  expect((await api.commit(ALICE, ws, [{ op: 'entity.set_keys', entity_id: 'wish-analysis', keys: [
    { key: 'executor', value: 'xllm', expect: { rev: wish.content.key_revs.executor } },
    { key: 'prompt', value: '按区域汇总「原始销售数据」的销售额，并写一段解读', expect: { rev: wish.content.key_revs.prompt } }] }])).status).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  // analysed and coded in edit mode (its macro and trial run), applied
  await page.getByTestId('aiws-canvas-block-blk-wish').dblclick()
  await page.getByTestId('aiws-wish-analyze').click()
  await expect(page.getByTestId('aiws-wish-contract')).toContainText('summary', { timeout: 30_000 })
  await expect(page.getByTestId('aiws-wish-main')).toHaveAttribute('data-main', 'needs_coding')
  await page.getByTestId('aiws-wish-next').click()
  await expect(page.getByTestId('aiws-wish-candidate')).toHaveAttribute('data-stage', 'code', { timeout: 60_000 })
  await page.getByTestId('aiws-wish-apply').click()
  await expect(page.getByTestId('aiws-wish-main')).toHaveAttribute('data-main', 'current', { timeout: 30_000 })
  await page.getByTestId('aiws-side-close').click()
  // view mode: "run" is enabled and runs the macro (no panel opens)
  await setCanvasMode(page, 'view')
  await fitAll(page)
  await page.getByTestId('aiws-canvas-block-blk-wish').click()
  const run = toolbar(page).getByTestId('aiws-near-wish-run')
  await expect(run).toBeEnabled({ timeout: 15_000 })
  await run.click()
  await expect.poll(async () => ((await api.runs(ALICE, ws.workspace_id, 'wish-analysis')) as { stage: string }[])[0]?.stage, { timeout: 30_000 }).toBe('run')
  await expect(page.locator('[data-testid="aiws-side-panel"][data-tab="wish"]')).toHaveCount(0)
  await openStatus(page)
  const entry = page.getByTestId('aiws-status-log').locator('[data-testid="aiws-status-log-entry"][data-kind="run"][data-phase="end"][data-target="wish-analysis"]')
  await expect(entry.first()).toBeVisible({ timeout: 30_000 })
})

test('§10.2 several Blocks: the properties they share, "several values" where they differ, one change written to all in one undoable step', async ({ page, api }) => {
  const ws = await contractWorkspace(api, `bc-multi ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await page.getByTestId('aiws-canvas-block-a').click()
  await shiftClick(page, 'b')
  await page.keyboard.press('F2')
  await expect(panel(page).getByTestId('aiws-props-multi')).toHaveAttribute('data-ready', 'true')
  // two shapes: the shape's own sections; their fills differ
  const fill = panel(page).getByTestId('aiws-shape-field-fill')
  await expect(panel(page).getByTestId('aiws-shape-field-fill-mixed')).toBeVisible()
  await expect(fill.locator('[aria-pressed="true"]')).toHaveCount(0)
  await expect(panel(page).getByTestId('aiws-props-section-type-look')).toBeVisible()
  // the text itself is no shared property (each shape keeps its own)
  await expect(panel(page).getByTestId('aiws-shape-field-text')).toHaveCount(0)
  const before = await api.headSeq(ALICE, ws.workspace_id)
  await fill.getByTestId('aiws-color-dcf3d6').click()
  await expect.poll(async () => [(await payloadOf(api, ws, 'a')).config.fill, (await payloadOf(api, ws, 'b')).config.fill]).toEqual(['#dcf3d6', '#dcf3d6'])
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(before + 1)
  await expect(panel(page).getByTestId('aiws-shape-field-fill-mixed')).toHaveCount(0)
  await expect(fill.getByTestId('aiws-color-dcf3d6')).toHaveAttribute('aria-pressed', 'true')
  // one step back restores both
  await panel(page).getByTestId('aiws-props-head').click()
  await page.keyboard.press('Control+z')
  await expect.poll(async () => [(await payloadOf(api, ws, 'a')).config.fill, (await payloadOf(api, ws, 'b')).config.fill]).toEqual(['#dbe8ff', '#ffd9d9'])
  // a shape and a label: of different types, the property of the same name and kind (the font size), nothing else
  await page.getByTestId('aiws-props-close').click()
  await page.getByTestId('aiws-canvas-block-a').click()
  await shiftClick(page, 'lbl')
  await page.keyboard.press('F2')
  await expect(panel(page).getByTestId('aiws-props-multi')).toHaveAttribute('data-ready', 'true')
  const size = panel(page).getByTestId('aiws-multi-field-font_size')
  await expect(size).toHaveValue('14')
  await expect(panel(page).getByTestId('aiws-multi-field-fill')).toHaveCount(0)
  await size.fill('20')
  await size.press('Enter')
  await expect.poll(async () => [(await payloadOf(api, ws, 'a')).config.font_size, (await payloadOf(api, ws, 'lbl')).config?.font_size]).toEqual([20, 20])
  // a lock for all of them, as before
  await expect(panel(page).getByTestId('aiws-props-locked')).toBeVisible()
})

/** A free canvas with a to-do table (two items) and a to-do control. */
async function todoWorkspace(api: Api, title: string) {
  const ws = await api.freeCanvas(ALICE, title)
  const fields = [
    { field_id: 'title', name: 'Title', type: 'text', required: true },
    { field_id: 'status', name: 'Status', type: 'select', required: true, options: [{ option_id: 'todo', label: 'To do' }, { option_id: 'done', label: 'Done' }] },
    { field_id: 'due', name: 'Due', type: 'date', nullable: true },
    { field_id: 'order', name: 'Order', type: 'text', nullable: true },
    { field_id: 'done_at', name: 'Completed', type: 'datetime', nullable: true },
    { field_id: 'ext', name: 'Extension', type: 'json', nullable: true },
  ]
  const made = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'tasks', type_id: 'buckyos.table-source', schema_id: 'buckyos.todo@1', parent_id: 'free-content', order_key: 'a', name: 'Tasks', payload: { title_field_id: 'title', fields } },
    { op: 'entity.create', entity_id: 'tl', type_id: 'buckyos.container', parent_id: 'free', order_key: 'b', placement: { x: 20, y: 20, w: 340, h: 420 },
      payload: { kind: 'todo', layout: { mode: 'data' }, title: 'Week', bindings: { items: { entity_id: 'tasks' } } } },
    { op: 'table.insert_records', source_id: 'tasks', records: [
      { record_id: 'a', values: { title: 'Write the report', status: 'todo', order: 'b' } },
      { record_id: 'd', values: { title: 'Pay rent', status: 'done', order: 'h', done_at: '2026-01-01T10:00:00Z' } },
    ] },
  ])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  return ws
}

test('§7.5 a to-do control: the ⋯ of its title row is the Block\'s menu, with its view menu\'s items; its item menu and details are the framework\'s floating panels', async ({ page, api }) => {
  const ws = await todoWorkspace(api, `bc-todo ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  const head = (await page.getByTestId('aiws-todo-head-tl').boundingBox())!
  await page.mouse.click(head.x + 150, head.y + 12)
  await expect(page.getByTestId('aiws-data-body-tl')).toHaveAttribute('data-active', 'true')
  // ⋯ opens the Block's whole menu (the same list as its toolbar's "more"): the view menu's items with the common ones
  await page.getByTestId('aiws-todo-menu-tl').click()
  const blockMenu = page.getByTestId('aiws-context-menu')
  await expect(blockMenu.getByTestId('aiws-menu-inspector')).toBeVisible()
  await blockMenu.getByTestId('aiws-menu-control-hide-done').click()
  await expect(page.locator('[data-proj="tl~title~d"]')).toHaveCount(0)
  await toolbar(page).getByTestId('aiws-near-more').click()
  await expect(page.getByTestId('aiws-near-control-hide-done')).toHaveAttribute('aria-checked', 'true')
  await page.keyboard.press('Escape')
  // an item's menu, then its details: a panel with a title and ✕ beside the item
  await page.locator('[data-proj="tl~title~a"]').click({ button: 'right' })
  const menu = page.getByTestId('aiws-todo-itemmenu-tl')
  await expect(menu).toHaveClass(/aiws-ctl-panel/)
  await menu.getByTestId('aiws-ctl-menu-detail').click()
  const detail = page.getByTestId('aiws-todo-detail-tl')
  await expect(detail).toHaveClass(/aiws-panel/)
  await expect(detail.locator('.aiws-ctl-panel-head')).toContainText('详情')
  await expect(detail.getByTestId('aiws-todo-form-a')).toBeVisible()
  await detail.getByTestId('aiws-ctl-panel-close').click()
  await expect(detail).toHaveCount(0)
})

// ---- the phone

class Fingers {
  private readonly cdp: CDPSession
  constructor(cdp: CDPSession) { this.cdp = cdp }
  static async of(page: Page) { return new Fingers(await page.context().newCDPSession(page)) }
  async tap(p: { x: number; y: number }) {
    await this.cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: p.x, y: p.y, id: 0 }] })
    await this.cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] })
  }
}

async function settledBox(page: Page, testId: string) {
  let previous = ''
  await expect.poll(async () => {
    const now = await page.getByTestId('aiws-world').evaluate((el) => getComputedStyle(el).transform)
    const still = now === previous
    previous = now
    return still
  }, { intervals: [150] }).toBe(true)
  return (await page.getByTestId(testId).boundingBox())!
}

async function openOnPhone(page: Page, workspaceId: string) {
  await openWorkspacePage(page, ALICE, workspaceId)
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
  await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-phone', 'true')
}

test.describe('phone', () => {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars -- the browser comes from the project, the rest is the phone
  const { defaultBrowserType, ...pixel } = devices['Pixel 7']
  test.use(pixel)

  test('§15-33 (phone) the context window shows the view-mode actions of the same declarations: a node\'s fold, a wish\'s run (disabled, with why); a line\'s properties sheet writes', async ({ page, api }) => {
    const ws = await contractWorkspace(api, `bc-phone ${Date.now()}`)
    const order = (await api.outline(ALICE, ws.workspace_id)).filter((e: Json) => e.parent_id === 'free-content').map((e: Json) => e.order_key as string).sort().at(-1)
    expect((await api.commit(ALICE, ws, [
      { op: 'entity.create', entity_id: 'w1', type_id: 'buckyos.wish', parent_id: 'free-content', order_key: core().order_key_between(order, undefined), name: '许愿',
        payload: { title: '许愿', prompt: '汇总', executor: 'xllm', output_mode: 'overwrite', output: { container_id: 'free-content', surface_id: 'free', name: '结果' } } },
      { op: 'entity.create', entity_id: 'blk-w1', type_id: 'buckyos.cell', parent_id: 'free', order_key: 'w', placement: { x: 60, y: 560, w: 360, h: 220 }, payload: { view: { type: 'wish' }, source_ref: { entity_id: 'w1' } } },
    ])).status).toBe('accepted')
    await openOnPhone(page, ws.workspace_id)
    const fingers = await Fingers.of(page)
    const win = page.getByTestId('aiws-ctxwin')
    // a node with a child: its fold (a reader's own view) is a view-mode action; adding children is not
    const n1 = await settledBox(page, 'aiws-canvas-block-n1')
    await fingers.tap({ x: n1.x + n1.width / 2, y: n1.y + n1.height / 2 })
    await expect(win.getByTestId('aiws-ctxwin-mindmap-fold')).toBeVisible()
    await expect(win.getByTestId('aiws-ctxwin-mindmap-add-child')).toHaveCount(0)
    await win.getByTestId('aiws-ctxwin-mindmap-fold').tap()
    await expect(page.getByTestId('aiws-canvas-block-n1a')).toHaveCount(0)
    // a wish without a macro: "run" is there, disabled, and says why
    const wish = await settledBox(page, 'aiws-canvas-block-blk-w1')
    await fingers.tap({ x: wish.x + wish.width / 2, y: wish.y + wish.height / 2 })
    const run = win.getByTestId('aiws-ctxwin-wish-run')
    await expect(run).toBeVisible()
    await expect(run).toBeDisabled()
    await expect(run).toHaveAttribute('title', /还不能在这里运行/)
    // a line: its properties as a sheet, written from there (the phone's explicit way to write, Q12)
    const mid = await linePoint(page, 'ab', 0.5)
    await fingers.tap(mid)
    await win.getByTestId('aiws-ctxwin-inspector').tap()
    const route = panel(page).getByTestId('aiws-connector-field-route')
    await expect(route).toBeVisible()
    await route.selectOption('curve')
    await expect.poll(async () => (await payloadOf(api, ws, 'ab')).route).toBe('curve')
  })

  test('§7.5 (phone) a to-do item\'s details are the window\'s bottom sheet; the control\'s view menu is in the Block menu', async ({ page, api }) => {
    const ws = await todoWorkspace(api, `bc-todo-phone ${Date.now()}`)
    await openOnPhone(page, ws.workspace_id)
    const fingers = await Fingers.of(page)
    await settledBox(page, 'aiws-canvas-block-tl')
    const item = (await page.locator('[data-proj="tl~title~a"]').boundingBox())!
    await fingers.tap({ x: item.x + item.width / 2, y: item.y + item.height / 2 })
    const sheet = page.locator('.aiws-sheet-layer').getByTestId('aiws-todo-detail-tl')
    await expect(sheet).toBeVisible()
    await expect(sheet.getByTestId('aiws-todo-form-a')).toBeVisible()
    const box = (await sheet.boundingBox())!
    const viewport = page.viewportSize()!
    expect(box.width).toBeGreaterThanOrEqual(viewport.width - 2)
    expect(box.y + box.height).toBeGreaterThanOrEqual(viewport.height - 2)
    await sheet.getByTestId('aiws-sheet-close').tap()
    await expect(sheet).toHaveCount(0)
    // the ⋯ of its title row: the Block's whole menu, a sheet, with the view menu's items
    await page.getByTestId('aiws-todo-menu-tl').tap()
    await expect(page.getByTestId('aiws-context-menu')).toHaveClass(/is-sheet/)
    await page.getByTestId('aiws-menu-control-hide-done').tap()
    await expect(page.locator('[data-proj="tl~title~d"]')).toHaveCount(0)
  })
})
