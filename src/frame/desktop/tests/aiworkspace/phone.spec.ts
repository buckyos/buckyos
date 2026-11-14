/* The phone (UI 框架 PRD §6, §15 scenarios 16–23, phone half) on an emulated Pixel 7: the title zone and the status bar
 * are all that stays; the phone's main menu is a sheet entered one level at a time; a tap on a free canvas's Block opens
 * its context window — expand into the data's full-screen editor, properties as a sheet; handwriting from the menu with
 * one finger, two fingers navigating; documents imported and the workspace exported from the menu, the Zone first.
 * Touch input goes through CDP (real multi-touch), not synthetic DOM events. */

import { devices, type CDPSession, type Locator, type Page } from '@playwright/test'
import { readFileSync } from 'node:fs'
import { chooseFromZone, expect, expectAllCommitted, openedPicker, openWorkspacePage, test, zoneGoto, type Api } from './fixtures'

const ALICE = 'tok-alice'
// eslint-disable-next-line @typescript-eslint/no-unused-vars -- the browser comes from the project, the rest is the phone
const { defaultBrowserType, ...pixel } = devices['Pixel 7']
test.use(pixel)

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any
interface Point { x: number; y: number }

const SHOTS = 'test-results/aiworkspace-phone'
const DOCX = 'application/vnd.openxmlformats-officedocument.wordprocessingml.document'
const OFFICE = new URL('../../../aiworkspace/fixtures/office/', import.meta.url).pathname

/** Fingers on the screen, in page coordinates (one touch event per call, like a real digitizer). */
class Fingers {
  private readonly cdp: CDPSession
  constructor(cdp: CDPSession) { this.cdp = cdp }
  static async of(page: Page) { return new Fingers(await page.context().newCDPSession(page)) }
  down(...points: Point[]) { return this.cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: points.map((p, id) => ({ x: p.x, y: p.y, id })) }) }
  move(...points: Point[]) { return this.cdp.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: points.map((p, id) => ({ x: p.x, y: p.y, id })) }) }
  up() { return this.cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] }) }
  async tap(p: Point) { await this.down(p); await this.up() }
  async drag(page: Page, from: Point[], to: Point[], steps = 10) {
    await this.down(...from)
    for (let i = 1; i <= steps; i += 1) {
      await this.move(...from.map((f, n) => ({ x: f.x + ((to[n].x - f.x) * i) / steps, y: f.y + ((to[n].y - f.y) * i) / steps })))
      await page.waitForTimeout(16)
    }
    await page.waitForTimeout(150)
    await this.up()
  }
}

/** ⋯ of the title zone, then the entries by test id, each a tap (a group goes one level in). */
async function phoneMenu(page: Page, ...testIds: string[]) {
  await page.getByTestId('aiws-main-menu').tap()
  const menu = page.getByTestId('aiws-main-menu-list')
  await expect(menu).toBeVisible()
  for (const id of testIds) await menu.getByTestId(id).tap()
}

/** The ids of the entries the phone's menu shows at its current level. */
async function entries(menu: Locator): Promise<string[]> {
  return menu.locator('.aiws-menu-phone > .aiws-menu-entry > button[role="menuitem"]').evaluateAll((items) => items.map((item) => item.getAttribute('data-testid') ?? ''))
}

/** A flow page with the project sample (tables, a record, a picture, rich text), and a free canvas showing its tables and
 * notes: the phone opens on the free canvas. */
async function sampleWithCanvas(api: Api, title: string) {
  const ws = await api.sample(ALICE, title)
  const made = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'board-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'z', payload: { kind: 'folder', system: 'surface_content', surface_id: 'board', title: '看板' } },
    { op: 'entity.create', entity_id: 'board', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'z', payload: { kind: 'surface', layout: { mode: 'free' }, title: '看板', content_folder_id: 'board-content' } },
    { op: 'entity.create', entity_id: 'blk-table', type_id: 'buckyos.cell', parent_id: 'board', order_key: 'a', placement: { x: 40, y: 40, w: 560, h: 320 }, payload: { view: { type: 'table' }, source_ref: { entity_id: 'tasks' }, title: '任务' } },
    { op: 'entity.create', entity_id: 'blk-shape', type_id: 'buckyos.cell', parent_id: 'board', order_key: 'b', placement: { x: 40, y: 420, w: 220, h: 160 }, payload: { view: { type: 'shape' }, config: { shape: 'rect', fill: '#fff2cc' } } },
  ])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  await api.rpc(ALICE, 'ws.set_user_state', { workspace_id: ws.workspace_id, entries: { 'surface:active': 'board' } })
  return ws
}

async function openOnPhone(page: Page, workspaceId: string, surfaceId?: string) {
  await openWorkspacePage(page, ALICE, workspaceId, surfaceId)
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
  await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-phone', 'true')
}

/** Wait until the camera stopped moving, then read where a Block is. */
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

test('UI-PH16 §15 #16 the phone keeps the title zone and the status bar; its main menu reaches play, macros, comments, share, handwriting, import, export and data mode, and nothing a phone cannot use', async ({ page, api }) => {
  const ws = await sampleWithCanvas(api, `ph16 ${Date.now()}`)
  await openOnPhone(page, ws.workspace_id)
  const viewport = page.viewportSize()!
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-mode', 'view')
  // what stays: the title zone and the status bar; no collaboration zone, no tool selection zone, no desktop title zone
  await expect(page.getByTestId('aiws-phone-title')).toBeVisible()
  await expect(page.getByTestId('aiws-status-row')).toBeVisible()
  for (const gone of ['aiws-view-toolbar', 'aiws-main-toolbar', 'aiws-object-toolbar', 'aiws-zoom-menu', 'aiws-share', 'aiws-collab-play']) await expect(page.getByTestId(gone), gone).toHaveCount(0)
  const title = page.getByTestId('aiws-phone-title')
  await expect(title.getByTestId('aiws-surface-switch')).toContainText('看板')
  // the status bar runs along the bottom
  const status = (await page.getByTestId('aiws-status-row').boundingBox())!
  expect(status.x).toBeLessThanOrEqual(1)
  expect(status.width).toBeGreaterThanOrEqual(viewport.width - 2)
  expect(status.y + status.height).toBeGreaterThanOrEqual(viewport.height - 2)

  // the main menu: a sheet from the bottom, its groups one level at a time
  await page.getByTestId('aiws-main-menu').tap()
  const menu = page.getByTestId('aiws-main-menu-list')
  await expect(menu).toBeVisible()
  const sheet = (await menu.boundingBox())!
  expect(sheet.y + sheet.height).toBeGreaterThanOrEqual(viewport.height - 40)
  expect(sheet.width).toBeGreaterThanOrEqual(viewport.width - 2)
  expect(await entries(menu)).toEqual(['aiws-menu-workspace', 'aiws-menu-surface', 'aiws-menu-import-doc', 'aiws-menu-ink', 'aiws-menu-collab', 'aiws-menu-edit', 'aiws-menu-view', 'aiws-menu-data-mode'])
  await page.screenshot({ path: `${SHOTS}/ph16-menu.png` })
  await menu.getByTestId('aiws-menu-workspace').tap()
  expect(await entries(menu)).toEqual(['aiws-menu-new', 'aiws-menu-open-workspace', 'aiws-menu-clone', 'aiws-menu-import', 'aiws-menu-export', 'aiws-back'])
  await menu.getByTestId('aiws-menu-back').tap()
  await menu.getByTestId('aiws-menu-surface').tap()
  expect(await entries(menu)).toEqual(['aiws-menu-new-canvas', 'aiws-menu-page-settings'])
  await expect(menu.getByTestId('aiws-menu-new-canvas')).toContainText('新建流式页')
  await menu.getByTestId('aiws-menu-back').tap()
  await menu.getByTestId('aiws-menu-import-doc').tap()
  expect(await entries(menu)).toEqual(['aiws-menu-import-drawio', 'aiws-menu-import-pdf', 'aiws-menu-import-office', 'aiws-menu-import-table'])
  await menu.getByTestId('aiws-menu-back').tap()
  await menu.getByTestId('aiws-menu-collab').tap()
  expect(await entries(menu)).toEqual(['aiws-menu-share', 'aiws-menu-members', 'aiws-menu-annotate', 'aiws-top-play', 'aiws-menu-guides', 'aiws-menu-macros'])
  // a disabled entry says why, in the menu (no hover on a phone)
  await expect(menu.getByTestId('aiws-top-play')).toBeDisabled()
  await expect(menu.getByTestId('aiws-top-play').locator('.aiws-menu-reason')).toBeVisible()
  await menu.getByTestId('aiws-menu-back').tap()
  await menu.getByTestId('aiws-menu-edit').tap()
  expect(await entries(menu)).toEqual(['aiws-menu-undo', 'aiws-menu-redo'])
  await menu.getByTestId('aiws-menu-back').tap()
  await menu.getByTestId('aiws-menu-view').tap()
  expect(await entries(menu)).toEqual(['aiws-menu-fit-all', 'aiws-menu-zoom'])
  await menu.getByTestId('aiws-menu-close').tap()
  await expect(menu).toHaveCount(0)

  // share from the menu: the dialog fills the screen
  await phoneMenu(page, 'aiws-menu-collab', 'aiws-menu-share')
  const share = page.locator('.aiws-modal-backdrop > .aiws-dialog').first()
  await expect(share).toBeVisible()
  const shareBox = (await share.boundingBox())!
  expect(shareBox.width).toBeGreaterThanOrEqual(viewport.width - 1)
  expect(shareBox.height).toBeGreaterThanOrEqual(viewport.height - 1)
  await page.keyboard.press('Escape')
  await expect(share).toHaveCount(0)

  // on a flow page: no handwriting, undo and redo in the title zone
  await title.getByTestId('aiws-surface-switch').tap()
  await page.getByTestId('aiws-surface-item-surface-main').getByRole('menuitem').tap()
  await expect(page.getByTestId('aiws-flow')).toHaveAttribute('data-mode', 'edit')
  await expect(title.getByTestId('aiws-undo')).toBeVisible()
  await page.getByTestId('aiws-main-menu').tap()
  expect(await entries(menu)).not.toContain('aiws-menu-ink')
  // the data mode is a row of its own: it switches there
  await menu.getByTestId('aiws-menu-data-mode').tap()
  await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-top-mode', 'sources')
})

test('UI-PH17 §15 #17 on a phone\'s flow page, a picture Block shows its fit and preview in its title row; ⋯ has lock, comment and delete', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `ph17 ${Date.now()}`)
  await openOnPhone(page, ws.workspace_id, 'surface-main')
  await expect(page.getByTestId('aiws-flow')).toHaveAttribute('data-mode', 'edit')
  const frame = page.getByTestId('aiws-cell-frame-cell-diagram')
  await frame.scrollIntoViewIfNeeded()
  await frame.locator('.aiws-cell-body').tap()
  const row = page.getByTestId('aiws-flow-row-cell-diagram')
  await expect(row).toBeVisible()
  await expect(row.getByTestId('aiws-near-asset-fit')).toBeVisible()
  await expect(row.getByTestId('aiws-near-asset-preview')).toBeVisible()
  for (const id of ['aiws-near-asset-fit', 'aiws-near-asset-preview', 'aiws-flow-more-cell-diagram']) {
    const b = (await row.getByTestId(id).boundingBox())!
    expect(b.width, id).toBeGreaterThanOrEqual(44)
    expect(b.height, id).toBeGreaterThanOrEqual(44)
  }
  await page.screenshot({ path: `${SHOTS}/ph17-row.png` })
  // the fit is a drop-down: a sheet from the bottom
  await row.getByTestId('aiws-near-asset-fit').tap()
  await expect(page.locator('.aiws-sheet-layer .aiws-sheet-menu')).toBeVisible()
  await page.getByTestId('aiws-menu-close').tap()
  await row.getByTestId('aiws-flow-more-cell-diagram').tap()
  const menu = page.getByTestId('aiws-flow-block-menu')
  await expect(menu).toBeVisible()
  for (const id of ['aiws-menu-lock', 'aiws-menu-annotate', 'aiws-delete-block-cell-diagram']) await expect(menu.getByTestId(id), id).toBeVisible()
  await page.screenshot({ path: `${SHOTS}/ph17-menu.png` })
})

test('UI-PH18 §15 #18 a tap on a table of a phone\'s free canvas opens its context window; "expand" edits the table full screen, the canvas shows it; nothing moves on the canvas', async ({ page, api }) => {
  const ws = await sampleWithCanvas(api, `ph18 ${Date.now()}`)
  await openOnPhone(page, ws.workspace_id)
  const fingers = await Fingers.of(page)
  const viewport = page.viewportSize()!
  const before = (await api.outline(ALICE, ws.workspace_id)).find((e: Json) => e.entity_id === 'blk-table').placement
  const block = await settledBox(page, 'aiws-canvas-block-blk-table')
  await fingers.tap({ x: block.x + block.width / 2, y: block.y + 30 })
  const win = page.getByTestId('aiws-ctxwin')
  await expect(win).toBeVisible()
  await expect(win.getByTestId('aiws-ctxwin-title')).toHaveText('任务')
  for (const id of ['aiws-ctxwin-expand', 'aiws-ctxwin-inspector', 'aiws-ctxwin-annotate', 'aiws-ctxwin-more']) await expect(win.getByTestId(id), id).toBeVisible()
  const box = (await win.boundingBox())!
  expect(box.x).toBeGreaterThanOrEqual(0)
  expect(box.x + box.width).toBeLessThanOrEqual(viewport.width)
  expect(box.y).toBeGreaterThanOrEqual(0)
  expect(box.y + box.height).toBeLessThanOrEqual(viewport.height)
  await page.screenshot({ path: `${SHOTS}/ph18-window.png` })
  // a tap on a blank spot closes it
  await fingers.tap({ x: viewport.width - 20, y: viewport.height - 140 })
  await expect(win).toHaveCount(0)
  // a drag on the Block pans the canvas, the Block stays where it is (view mode)
  const from = { x: block.x + block.width / 2, y: block.y + 60 }
  await fingers.drag(page, [from], [{ x: from.x + 40, y: from.y + 80 }])
  expect((await api.outline(ALICE, ws.workspace_id)).find((e: Json) => e.entity_id === 'blk-table').placement).toEqual(before)

  const moved = await settledBox(page, 'aiws-canvas-block-blk-table')
  await fingers.tap({ x: moved.x + moved.width / 2, y: moved.y + 30 })
  await win.getByTestId('aiws-ctxwin-expand').tap()
  const full = page.getByTestId('aiws-phone-expand')
  await expect(full).toBeVisible()
  await expect(full.getByTestId('aiws-phone-expand-name')).toHaveText('任务')
  const fullBox = (await full.boundingBox())!
  expect(fullBox.width).toBeGreaterThanOrEqual(viewport.width - 1)
  expect(fullBox.height).toBeGreaterThanOrEqual(viewport.height - 1)
  const table = full.getByTestId('aiws-table-source:tasks')
  await expect(table).toHaveAttribute('data-size', 'fullscreen')
  await expect(table).toHaveAttribute('data-layout', 'cards')
  await page.screenshot({ path: `${SHOTS}/ph18-expanded.png` })

  // a new record
  await full.getByTestId('aiws-add-record-source:tasks').tap()
  const form = page.getByTestId('aiws-table-record-panel')
  await expect(form).toHaveAttribute('data-mode', 'new')
  await form.getByLabel('新记录 任务').fill('手机上展开新建')
  const cards = await table.getByTestId('aiws-table-card').count()
  await form.getByTestId('aiws-table-record-add').tap()
  await expect(form).toHaveCount(0)
  await expect(table.getByTestId('aiws-table-card')).toHaveCount(cards + 1)
  // a field of a record
  const card = table.locator('[data-testid="aiws-table-card"][data-record-id="task-42"]')
  await card.scrollIntoViewIfNeeded()
  await page.waitForTimeout(300)
  await card.tap()
  await expect(form).toHaveAttribute('data-record-id', 'task-42')
  const owner = form.getByLabel('负责人')
  await owner.fill('手机')
  await owner.blur()
  const rows = async () => JSON.stringify((await api.table(ALICE, ws.workspace_id, { source_id: 'tasks' })).rows)
  await expect.poll(rows).toContain('手机上展开新建')
  await expect.poll(rows).toContain('"手机"')
  await page.getByTestId('aiws-table-record-close').tap()
  await expectAllCommitted(page)

  // back to the canvas: the table there shows both
  await full.getByTestId('aiws-phone-expand-back').tap()
  await expect(full).toHaveCount(0)
  const onCanvas = page.getByTestId('aiws-canvas-block-blk-table')
  await expect(onCanvas).toContainText('手机上展开新建')
  await expect(onCanvas).toContainText('手机')
  expect((await api.outline(ALICE, ws.workspace_id)).find((e: Json) => e.entity_id === 'blk-table').placement).toEqual(before)
})

test('UI-PH19 §15 #19 "properties" from the context window is a sheet; a colour changed there is one step undone from the menu', async ({ page, api }) => {
  const ws = await sampleWithCanvas(api, `ph19 ${Date.now()}`)
  await openOnPhone(page, ws.workspace_id)
  const fingers = await Fingers.of(page)
  const viewport = page.viewportSize()!
  const shape = await settledBox(page, 'aiws-canvas-block-blk-shape')
  await fingers.tap({ x: shape.x + shape.width / 2, y: shape.y + shape.height / 2 })
  await page.getByTestId('aiws-ctxwin').getByTestId('aiws-ctxwin-inspector').tap()
  const panel = page.getByTestId('aiws-props-panel')
  await expect(panel).toBeVisible()
  const sheet = (await panel.boundingBox())!
  expect(sheet.width).toBeGreaterThanOrEqual(viewport.width - 1)
  expect(sheet.y + sheet.height).toBeGreaterThanOrEqual(viewport.height - 2)
  await page.screenshot({ path: `${SHOTS}/ph19-props.png` })
  const fill = async () => (await api.read(ALICE, ws.workspace_id, 'blk-shape')).content.payload.config?.fill
  const start = await fill()
  await panel.getByTestId('aiws-color-dbe8ff').first().tap()
  await expect.poll(fill).toBe('#dbe8ff')
  await expectAllCommitted(page)
  await panel.getByTestId('aiws-props-close').tap()
  await phoneMenu(page, 'aiws-menu-edit', 'aiws-menu-undo')
  await expect.poll(fill).toBe(start)
})

const PNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAABAAAAAJCAIAAAC0SDtlAAAAFElEQVR42mPQiDpBEmIY1TAoNAAAk1q5oVLxFXwAAAAASUVORK5CYII=', 'base64')
const member = (scope: Page | Locator, name: string) => scope.locator(`[data-testid="aiws-directory-member"][data-name="${name}"]`)
const membersOf = async (api: Api, workspaceId: string, id: string) => (await api.read(ALICE, workspaceId, id)).content.payload.members as Record<string, Json>

test('UI-PH20 §15 #20 through "expand": the table — a new record, a field\'s type, a filter, a deleted record; the directory — added from the Zone, renamed, moved, removed; by taps only', async ({ page, api }) => {
  test.setTimeout(150_000)
  const ws = await sampleWithCanvas(api, `ph20 ${Date.now()}`)
  expect((await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'phone-dir', type_id: 'buckyos.dir', parent_id: 'board-content', order_key: 'b', name: '资料', payload: { members: {} } },
    { op: 'entity.create', entity_id: 'blk-dir', type_id: 'buckyos.cell', parent_id: 'board', order_key: 'c', placement: { x: 320, y: 420, w: 280, h: 200 }, payload: { view: { type: 'directory' }, source_ref: { entity_id: 'phone-dir' }, title: '资料' } },
  ])).status).toBe('accepted')
  await openOnPhone(page, ws.workspace_id)
  const fingers = await Fingers.of(page)
  const expandBlock = async (id: string) => {
    const b = await settledBox(page, `aiws-canvas-block-${id}`)
    await fingers.tap({ x: b.x + b.width / 2, y: b.y + Math.min(30, b.height / 2) })
    await page.getByTestId('aiws-ctxwin').getByTestId('aiws-ctxwin-expand').tap()
    await expect(page.getByTestId('aiws-phone-expand')).toBeVisible()
  }

  // the table
  await expandBlock('blk-table')
  const full = page.getByTestId('aiws-phone-expand')
  const table = full.getByTestId('aiws-table-source:tasks')
  const rows = async () => (await api.table(ALICE, ws.workspace_id, { source_id: 'tasks' })).rows as { record_id: string; values: Record<string, unknown> }[]
  await full.getByTestId('aiws-add-record-source:tasks').tap()
  const form = page.getByTestId('aiws-table-record-panel')
  await form.getByLabel('新记录 任务').fill('展开后新建')
  await form.getByTestId('aiws-table-record-add').tap()
  await expect.poll(async () => (await rows()).some((row) => row.values.title === '展开后新建')).toBe(true)
  await page.getByTestId('aiws-table-menu-source:tasks').tap()
  await page.getByTestId('aiws-ctl-menu-table-fields').tap()
  const fields = page.getByTestId('aiws-table-sheet-fields')
  await fields.getByTestId('aiws-field-open-budget').tap()
  await fields.getByTestId('aiws-migrate-budget').tap()
  await fields.getByLabel('目标类型').selectOption('currency')
  await fields.getByTestId('aiws-migration-precheck').tap()
  await expect(fields.getByTestId('aiws-migration-status')).toContainText('预检通过')
  await fields.getByTestId('aiws-migration-run').tap()
  await expect.poll(async () => ((await api.read(ALICE, ws.workspace_id, 'tasks')).content.fields as { field_id: string; type: string }[]).find((f) => f.field_id === 'budget')?.type).toBe('currency')
  await fields.locator('.aiws-ctl-popover-close').tap()
  await page.getByTestId('aiws-filter-open-source:tasks').tap()
  const filter = page.getByTestId('aiws-table-sheet-filter')
  await filter.getByLabel('筛选字段').selectOption('owner')
  await filter.getByLabel('筛选值').fill('林')
  await filter.getByTestId('aiws-filter-apply').tap()
  await expect(table.getByTestId('aiws-table-card')).toHaveCount(2)
  await table.getByTestId('aiws-table-chip').getByTestId('aiws-table-chip-remove').tap()
  const doomed = table.locator('[data-testid="aiws-table-card"][data-record-id="task-43"]')
  await doomed.scrollIntoViewIfNeeded()
  await page.waitForTimeout(300)
  await doomed.tap()
  await expect(form).toHaveAttribute('data-record-id', 'task-43')
  await form.getByTestId('aiws-table-record-more').tap()
  await page.getByTestId('aiws-ctl-menu-record-delete').tap()
  await page.getByTestId('aiws-table-confirm-ok').tap()
  await expect.poll(async () => (await rows()).some((row) => row.record_id === 'task-43')).toBe(false)
  await expectAllCommitted(page)
  await page.screenshot({ path: `${SHOTS}/ph20-table.png` })
  await page.getByTestId('aiws-table-record-close').tap()
  await full.getByTestId('aiws-phone-expand-back').tap()
  await expect(full).toHaveCount(0)

  // the directory
  await expandBlock('blk-dir')
  const editor = full.getByTestId('aiws-directory')
  await expect(editor).toHaveAttribute('data-size', 'full')
  await expect(full.getByTestId('aiws-phone-expand-name')).toHaveText('资料')
  await editor.getByTestId('aiws-directory-upload').tap()
  const add = page.getByTestId('aiws-directory-add-menu')
  await chooseFromZone(page, () => add.getByTestId('aiws-directory-add-zone').tap(), [
    { name: '发票.png', mimeType: 'image/png', buffer: PNG },
    { name: '清单.txt', mimeType: 'text/plain', buffer: Buffer.from('购物清单') },
  ])
  await expect.poll(async () => Object.keys(await membersOf(api, ws.workspace_id, 'phone-dir')).sort()).toEqual(['发票.png', '清单.txt'].sort())
  await member(editor, '清单.txt').getByTestId('aiws-directory-row-menu').tap()
  const actions = page.getByTestId('aiws-directory-row-actions')
  await actions.getByTestId('aiws-directory-rename').tap()
  await page.getByTestId('aiws-directory-name').fill('周末清单.txt')
  await page.getByTestId('aiws-directory-save').tap()
  await expect.poll(async () => Object.keys(await membersOf(api, ws.workspace_id, 'phone-dir')).sort()).toEqual(['周末清单.txt', '发票.png'].sort())
  await editor.getByTestId('aiws-directory-upload').tap()
  await page.getByTestId('aiws-directory-add-folder').tap()
  await page.getByTestId('aiws-directory-name').fill('票据')
  await page.getByTestId('aiws-directory-save').tap()
  await expect(member(editor, '票据')).toHaveAttribute('data-kind', 'dir')
  const sub = (await membersOf(api, ws.workspace_id, 'phone-dir'))['票据'].entity_id as string
  await member(editor, '发票.png').getByTestId('aiws-directory-row-menu').tap()
  await actions.getByTestId('aiws-directory-move').tap()
  const move = page.getByTestId('aiws-directory-move-dialog')
  await move.locator('[data-testid="aiws-directory-move-folder"][data-name="票据"]').tap()
  await move.getByTestId('aiws-directory-move-here').tap()
  await expect.poll(async () => Object.keys(await membersOf(api, ws.workspace_id, sub))).toEqual(['发票.png'])
  await member(editor, '周末清单.txt').getByTestId('aiws-directory-row-menu').tap()
  await actions.getByTestId('aiws-directory-remove').tap()
  await page.getByTestId('aiws-directory-confirm-ok').tap()
  await expect.poll(async () => Object.keys(await membersOf(api, ws.workspace_id, 'phone-dir'))).toEqual(['票据'])
  await page.screenshot({ path: `${SHOTS}/ph20-directory.png` })
  await full.getByTestId('aiws-phone-expand-back').tap()
  await expect(page.getByTestId('aiws-canvas-block-blk-dir')).toContainText('票据')
})

test('UI-PH22 §15 #22 handwriting from the phone\'s menu: one finger writes, two pan and zoom without a stray stroke, a Block under the finger is not moved; "done" keeps the strokes, the menu undoes one', async ({ page, api }) => {
  const ws = await sampleWithCanvas(api, `ph22 ${Date.now()}`)
  await openOnPhone(page, ws.workspace_id)
  const fingers = await Fingers.of(page)
  const viewport = page.viewportSize()!
  const strokes = async () => (await api.outline(ALICE, ws.workspace_id)).filter((e: Json) => e.view_type === 'ink' && !e.deleted).length
  const shapeAt = (await api.outline(ALICE, ws.workspace_id)).find((e: Json) => e.entity_id === 'blk-shape').placement
  await settledBox(page, 'aiws-canvas-block-blk-shape')
  await phoneMenu(page, 'aiws-menu-ink')
  const bar = page.getByTestId('aiws-ink-bar')
  await expect(bar).toBeVisible()
  // no pen has touched this phone: the pen's own switches are not offered
  await expect(bar.getByTestId('aiws-ink-pressure')).toHaveCount(0)
  await expect(bar.getByTestId('aiws-ink-touch-lock')).toHaveCount(0)
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-mode', 'view')
  // the bar stands above the status bar
  const barBox = (await bar.boundingBox())!
  const statusBox = (await page.getByTestId('aiws-status-row').boundingBox())!
  expect(barBox.y + barBox.height).toBeLessThanOrEqual(statusBox.y + 1)
  expect(barBox.x + barBox.width).toBeLessThanOrEqual(viewport.width)
  await page.screenshot({ path: `${SHOTS}/ph22-bar.png` })

  // one finger writes, also when it starts on a Block: the Block is neither selected nor moved
  const shape = await settledBox(page, 'aiws-canvas-block-blk-shape')
  const on = { x: shape.x + shape.width / 2, y: shape.y + shape.height / 2 }
  await fingers.drag(page, [on], [{ x: on.x + 50, y: on.y + 30 }])
  await expect.poll(strokes).toBe(1)
  expect((await api.outline(ALICE, ws.workspace_id)).find((e: Json) => e.entity_id === 'blk-shape').placement).toEqual(shapeAt)
  await expect(page.getByTestId('aiws-selection-blk-shape')).toHaveCount(0)
  const y = viewport.height * 0.6
  await fingers.drag(page, [{ x: 60, y }], [{ x: 220, y: y + 40 }])
  await expect.poll(strokes).toBe(2)
  await page.screenshot({ path: `${SHOTS}/ph22-written.png` })
  // two fingers pinch: the canvas zooms, nothing is written
  const zoom = Number(await page.getByTestId('aiws-canvas').getAttribute('data-zoom'))
  const mid = { x: viewport.width / 2, y: viewport.height * 0.45 }
  await fingers.drag(page, [{ x: mid.x - 30, y: mid.y }, { x: mid.x + 30, y: mid.y }], [{ x: mid.x - 90, y: mid.y }, { x: mid.x + 90, y: mid.y }])
  await expect.poll(async () => Number(await page.getByTestId('aiws-canvas').getAttribute('data-zoom'))).toBeGreaterThan(zoom * 1.5)
  await page.waitForTimeout(1000)
  expect(await strokes()).toBe(2)

  // done: the bar goes, the strokes stay; the menu's undo takes the last one back
  await bar.getByTestId('aiws-ink-done').tap()
  await expect(bar).toHaveCount(0)
  await expectAllCommitted(page)
  expect(await strokes()).toBe(2)
  await phoneMenu(page, 'aiws-menu-edit', 'aiws-menu-undo')
  await expect.poll(strokes).toBe(1)
})

test('UI-PH23 §15 #23 from the phone\'s menu: a .docx from the Zone becomes an editable flow page (the dialog fills the screen); the workspace package is saved into the Zone by default', async ({ page, api }) => {
  test.setTimeout(150_000)
  const ws = await sampleWithCanvas(api, `ph23 ${Date.now()}`)
  await openOnPhone(page, ws.workspace_id)
  const viewport = page.viewportSize()!
  const known = (await api.outline(ALICE, ws.workspace_id)).filter((e: Json) => e.kind === 'surface' && !e.deleted).map((e: Json) => e.entity_id as string)
  await chooseFromZone(page, () => phoneMenu(page, 'aiws-menu-import-doc', 'aiws-menu-import-office'), { name: 'basic.docx', mimeType: DOCX, buffer: readFileSync(`${OFFICE}docx/basic.docx`) })
  const dialog = page.getByTestId('aiws-office-dialog')
  await expect(dialog.getByTestId('aiws-office-mode-editable')).toBeChecked()
  const box = (await dialog.boundingBox())!
  expect(box.width).toBeGreaterThanOrEqual(viewport.width - 1)
  expect(box.height).toBeGreaterThanOrEqual(viewport.height - 1)
  await page.screenshot({ path: `${SHOTS}/ph23-office.png` })
  await dialog.getByTestId('aiws-office-start').tap()
  await expect(dialog).toHaveCount(0, { timeout: 60_000 })
  let created: Json = null
  await expect.poll(async () => {
    created = (await api.outline(ALICE, ws.workspace_id)).find((e: Json) => e.kind === 'surface' && !e.deleted && !known.includes(e.entity_id))
    return created?.layout?.mode ?? null
  }, { timeout: 60_000 }).toBe('flow')
  await expect(page.getByTestId('aiws-flow')).toHaveAttribute('data-mode', 'edit', { timeout: 20_000 })
  await expect(page.getByTestId('aiws-canvas')).toHaveCount(0)
  // what the import simplified, if anything, is reported (full screen too)
  const report = page.getByTestId('aiws-import-report')
  if (await report.isVisible()) await report.getByTestId('aiws-import-report-close').tap()
  await expect(page.getByTestId('aiws-phone-title').getByTestId('aiws-surface-switch')).toContainText(created.title)

  // export: the dialog fills the screen, "save to the Zone" is its main action
  await phoneMenu(page, 'aiws-menu-workspace', 'aiws-menu-export')
  const exportDialog = page.getByTestId('aiws-export-dialog')
  await expect(exportDialog.getByTestId('aiws-export-zone')).toHaveClass(/is-primary/)
  const exportBox = (await exportDialog.boundingBox())!
  expect(exportBox.width).toBeGreaterThanOrEqual(viewport.width - 1)
  await exportDialog.getByTestId('aiws-export-zone').tap()
  const save = (await openedPicker(page)).dialog
  await zoneGoto(save, ['Documents'])
  await save.getByTestId('select-confirm').tap()
  await expect(exportDialog.getByTestId('aiws-export-message')).toContainText('/home/Documents/', { timeout: 30_000 })
  await page.screenshot({ path: `${SHOTS}/ph23-exported.png` })
})

test('UI-PH33 §15 #33 (phone) an extension control\'s declared action is in the context window of its Block, and runs there', async ({ page, api }) => {
  const ws = await api.freeCanvas(ALICE, `ph33 ${Date.now()}`)
  const built = await api.build(ALICE, ws.workspace_id, {
    'component.json': { manifest_version: 4, ident: { author_did: 'did:bns:alice', name: 'stamp-phone' }, kind: 'declarative', title: 'Stamp', allow_no_source: true, entry: 'declarative.json',
      config_schema: { type: 'object', properties: { note: { type: 'string', default: '' } } },
      actions: [{ id: 'stamp', title: 'Stamp it', icon: 'check', modes: ['edit', 'view'], needs: ['update'], run: { kind: 'config', props: { note: 'stamped' } } }] },
    'declarative.json': { items: [{ kind: 'text', text: 'card' }] },
  })
  expect((await api.commit(ALICE, ws, [{ op: 'entity.create', entity_id: 'card', type_id: 'buckyos.cell', parent_id: 'free', order_key: 'a', placement: { x: 60, y: 60, w: 240, h: 120 },
    payload: { view: { type: 'declarative' }, def_ref: { entity_id: built.def_entity_id } } }])).status).toBe('accepted')
  await openOnPhone(page, ws.workspace_id)
  const fingers = await Fingers.of(page)
  const card = await settledBox(page, 'aiws-canvas-block-card')
  await fingers.tap({ x: card.x + card.width / 2, y: card.y + card.height / 2 })
  const action = page.getByTestId('aiws-ctxwin').getByTestId('aiws-ctxwin-ext-action-stamp')
  await expect(action).toBeVisible()
  await expect(action.locator('svg')).toHaveCount(1)
  await action.tap()
  await expect.poll(async () => (await api.read(ALICE, ws.workspace_id, 'card')).content.payload.config?.note).toBe('stamped')
})

test('UI-PH44 §4.4 the text format bar: a phone\'s text editor hands its format tools to a bar over the keyboard, above the status bar; they act on the text', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `ph44 ${Date.now()}`)
  await openOnPhone(page, ws.workspace_id, 'surface-main')
  await expect(page.getByTestId('aiws-format-bar')).toHaveCount(0)
  await page.getByTestId('aiws-richtext-notes').locator('[data-block-id="n-end"]').tap()
  const bar = page.getByTestId('aiws-format-bar')
  await expect(bar).toBeVisible()
  await expect(bar).toHaveAttribute('data-scope', 'cell-notes')
  // no bubble or MD corner in the text: the tools are in the bar
  await expect(page.getByTestId('aiws-md-toggle')).toHaveCount(0)
  const box = (await bar.boundingBox())!
  const status = (await page.getByTestId('aiws-status-row').boundingBox())!
  expect(box.y + box.height).toBeLessThanOrEqual(status.y + 1)
  expect(box.x).toBeGreaterThanOrEqual(0)
  expect(box.x + box.width).toBeLessThanOrEqual(page.viewportSize()!.width)
  await page.keyboard.press('End')
  await page.keyboard.press('Enter')
  await bar.getByTestId('aiws-near-bold').tap()
  await page.keyboard.type('粗体')
  await expect.poll(async () => JSON.stringify(await api.ast(ALICE, ws.workspace_id, 'notes'))).toContain('"marks":[{"type":"strong"}],"text":"粗体"')
  await page.screenshot({ path: `${SHOTS}/ph44-format.png` })
  // a menu of the bar is a sheet; the bar stays while it is open
  await bar.getByTestId('aiws-near-block-type').tap()
  await expect(page.locator('.aiws-sheet-layer .aiws-sheet-menu')).toBeVisible()
  await expect(bar).toBeVisible()
  await page.getByTestId('aiws-menu-close').tap()
})

test('UI-PH45 §4.5 the phone\'s status bar opens its panel from the bottom; the side drawer of comments is a sheet too', async ({ page, api }) => {
  const ws = await sampleWithCanvas(api, `ph45 ${Date.now()}`)
  await openOnPhone(page, ws.workspace_id)
  const viewport = page.viewportSize()!
  await page.getByTestId('aiws-status').tap()
  const panel = page.getByTestId('aiws-status-detail')
  await expect(panel).toBeVisible()
  const box = (await panel.boundingBox())!
  expect(box.width).toBeGreaterThanOrEqual(viewport.width - 2)
  const row = (await page.getByTestId('aiws-status-row').boundingBox())!
  expect(box.y + box.height).toBeLessThanOrEqual(row.y + 1)
  await page.getByTestId('aiws-status').tap()
  await expect(panel).toHaveCount(0)
  // a comment on a Block from its context window: the comments come up from the bottom
  const fingers = await Fingers.of(page)
  const b = await settledBox(page, 'aiws-canvas-block-blk-table')
  await fingers.tap({ x: b.x + b.width / 2, y: b.y + 30 })
  await page.getByTestId('aiws-ctxwin').getByTestId('aiws-ctxwin-annotate').tap()
  const side = page.getByTestId('aiws-side-panel')
  await expect(side).toHaveAttribute('data-tab', 'annotations')
  const sideBox = (await side.boundingBox())!
  expect(sideBox.width).toBeGreaterThanOrEqual(viewport.width - 2)
  expect(sideBox.y + sideBox.height).toBeGreaterThanOrEqual(viewport.height - 2)
  await page.screenshot({ path: `${SHOTS}/ph45-comments.png` })
})

test('UI-PH23b §6.1 table data imported on a phone\'s free canvas goes into the data only (no Block on the canvas), through a full-screen wizard; the page settings are a sheet', async ({ page, api }) => {
  const ws = await sampleWithCanvas(api, `ph23b ${Date.now()}`)
  await openOnPhone(page, ws.workspace_id)
  const viewport = page.viewportSize()!
  const tables = async () => (await api.outline(ALICE, ws.workspace_id)).filter((e: Json) => e.type_id === 'buckyos.table-source' && !e.deleted)
  const onBoard = async () => (await api.outline(ALICE, ws.workspace_id)).filter((e: Json) => e.parent_id === 'board' && !e.deleted).length
  const known = (await tables()).map((e: Json) => e.entity_id as string)
  const blocks = await onBoard()
  await chooseFromZone(page, () => phoneMenu(page, 'aiws-menu-import-doc', 'aiws-menu-import-table'), { name: '成绩.csv', mimeType: 'text/csv', buffer: Buffer.from('姓名,分数\n张三,90\n李四,85\n') })
  const wizard = page.getByTestId('aiws-table-dialog')
  await expect(wizard).toBeVisible()
  const box = (await wizard.boundingBox())!
  expect(box.width).toBeGreaterThanOrEqual(viewport.width - 1)
  expect(box.height).toBeGreaterThanOrEqual(viewport.height - 1)
  await page.screenshot({ path: `${SHOTS}/ph23b-wizard.png` })
  await wizard.getByTestId('aiws-table-start').tap()
  await expect(wizard).toHaveCount(0, { timeout: 30_000 })
  await expect.poll(async () => (await tables()).filter((e: Json) => !known.includes(e.entity_id)).map((e: Json) => e.parent_id)).toEqual(['data'])
  expect(await onBoard()).toBe(blocks)

  // 画布 → 页面设置…: the icon and the background, as a sheet; the canvas stays in view mode
  await phoneMenu(page, 'aiws-menu-surface', 'aiws-menu-page-settings')
  const settings = page.getByTestId('aiws-icon-picker')
  await expect(settings).toBeVisible()
  const sheet = (await settings.boundingBox())!
  expect(sheet.width).toBeGreaterThanOrEqual(viewport.width - 1)
  expect(sheet.y + sheet.height).toBeGreaterThanOrEqual(viewport.height - 2)
  await settings.locator('[data-testid^="aiws-icon-"][role="option"]').nth(2).tap()
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).find((e: Json) => e.entity_id === 'board').icon ?? null).not.toBeNull()
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-mode', 'view')
})
