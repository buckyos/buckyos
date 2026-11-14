/* UI 框架 PRD §6.1, §7.3, §7.4 (third pass, §18.3): the table and directory editors' pop-overs — the table's ⋯ menu, a
 * confirmation, its filter on a phone, a directory row's menu — are the framework's panel: on a computer a floating
 * panel below the button that opened it, titled, with ✕ (Esc and a press outside close it too); on an emulated Pixel 7
 * the window's standard bottom sheet. The table's filter on a computer stays inline under its title row. Real backend. */

import { devices, type Locator, type Page } from '@playwright/test'
import { expect, openWorkspace, openWorkspacePage, test, type Api } from './fixtures'

const ALICE = 'tok-alice'
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any
type Ws = { workspace_id: string; epoch: string }
const PNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAABAAAAAJCAIAAAC0SDtlAAAAFElEQVR42mPQiDpBEmIY1TAoNAAAk1q5oVLxFXwAAAAASUVORK5CYII=', 'base64')

const member = (scope: Page | Locator, name: string) => scope.locator(`[data-testid="aiws-directory-member"][data-name="${name}"]`)
const rows = async (api: Api, ws: Ws) => (await api.table(ALICE, ws.workspace_id, { source_id: 'tasks' })).rows as { record_id: string }[]

/** A flow page `page` with a directory Block `cell-dir` showing `资料` (a picture and a note). */
async function directoryPage(api: Api, title: string): Promise<Ws> {
  const ws = await api.rpc(ALICE, 'ws.create', { title }) as Ws
  const photo = await api.uploadAsset(ALICE, ws.workspace_id, PNG)
  const note = await api.uploadAsset(ALICE, ws.workspace_id, Buffer.from('a note'))
  const made = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'page-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'a', payload: { kind: 'folder', system: 'surface_content', surface_id: 'page', title: 'Page' } },
    { op: 'entity.create', entity_id: 'page', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'a', payload: { kind: 'surface', layout: { mode: 'flow' }, title: '日记', content_folder_id: 'page-content' } },
    { op: 'entity.create', entity_id: 'dir', type_id: 'buckyos.dir', parent_id: 'page-content', order_key: 'a', name: '资料', payload: { members: {
      'photo.png': { kind: 'file', object_id: photo.object_id }, 'note.txt': { kind: 'file', object_id: note.object_id },
    } } },
    { op: 'entity.create', entity_id: 'cell-dir', type_id: 'buckyos.cell', parent_id: 'page', order_key: 'a', payload: { view: { type: 'directory' }, source_ref: { entity_id: 'dir' }, title: '资料' } },
  ])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  return ws
}

/** The panel is below `button`, inside the window. */
async function below(panel: Locator, button: Locator, page: Page) {
  const b = (await button.boundingBox())!
  const p = (await panel.boundingBox())!
  expect(p.y).toBeGreaterThanOrEqual(b.y + b.height - 1)
  expect(p.x).toBeGreaterThanOrEqual(0)
  expect(p.x + p.width).toBeLessThanOrEqual(page.viewportSize()!.width + 1)
}

test('EP-01 §7.3 (computer) the table\'s ⋯ and its confirmation are floating panels with a title and ✕; its filter stays inline', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `panels-table ${Date.now()}`)
  await openWorkspace(page, ALICE, ws.workspace_id)
  const all = page.getByTestId('aiws-table-cell-all-tasks')
  await expect(all.getByTestId('aiws-row')).toHaveCount(5)

  // the filter: inline under the table's title row, no pop-over
  await all.getByTestId('aiws-filter-open-cell-all-tasks').click()
  await expect(all.getByTestId('aiws-filter-panel')).toBeVisible()
  await expect(page.locator('.aiws-ctl-panel, .aiws-sheet-layer')).toHaveCount(0)
  await all.getByTestId('aiws-filter-open-cell-all-tasks').click()
  await expect(all.getByTestId('aiws-filter-panel')).toHaveCount(0)

  // ⋯: a panel below its button, titled with what it is; ✕ closes it
  const more = all.getByTestId('aiws-table-menu-cell-all-tasks')
  await more.click()
  const menu = page.getByTestId('aiws-table-menu-pop-cell-all-tasks')
  await expect(menu).toHaveClass(/aiws-ctl-panel/)
  await expect(menu.locator('.aiws-ctl-panel-head')).toContainText('表格的更多操作')
  await below(menu, more, page)
  await page.screenshot({ path: 'test-results/aiworkspace-editor-panels/ep01-table-menu.png' })
  // the first entry has the keyboard; the arrows move it
  await expect(menu.locator('.aiws-ctl-menu-item').first()).toBeFocused()
  await page.keyboard.press('ArrowDown')
  await expect(menu.locator('.aiws-ctl-menu-item').nth(1)).toBeFocused()
  await menu.getByTestId('aiws-ctl-panel-close').click()
  await expect(menu).toHaveCount(0)
  // Esc and a press outside close it as well; an entry acts and closes it
  await more.click()
  await page.keyboard.press('Escape')
  await expect(menu).toHaveCount(0)
  await more.click()
  await page.mouse.click(1500, 600)
  await expect(menu).toHaveCount(0)
  await more.click()
  await menu.getByTestId('aiws-ctl-menu-table-layout-cards').click()
  await expect(menu).toHaveCount(0)
  await expect(all).toHaveAttribute('data-layout', 'cards')
  await more.click()
  await menu.getByTestId('aiws-ctl-menu-table-layout-grid').click()
  await expect(all).toHaveAttribute('data-layout', 'grid')

  // a confirmation: the question is the panel's title; Esc keeps the records, the button deletes them
  await all.getByTestId('aiws-row-check-task-43').check()
  const remove = all.getByTestId('aiws-table-selection-delete')
  await remove.click()
  const confirm = page.getByTestId('aiws-table-confirm')
  await expect(confirm).toHaveClass(/aiws-ctl-panel/)
  await expect(confirm.locator('.aiws-ctl-panel-head')).toContainText('删除 1 条记录')
  await expect(confirm.getByTestId('aiws-table-confirm-ok')).toBeFocused()
  await page.keyboard.press('Escape')
  await expect(confirm).toHaveCount(0)
  expect(await rows(api, ws)).toHaveLength(5)
  await remove.click()
  await confirm.getByTestId('aiws-table-confirm-ok').click()
  await expect.poll(async () => (await rows(api, ws)).map((row) => row.record_id)).not.toContain('task-43')
})

test('EP-02 §7.4 (computer) a directory row\'s menu is a floating panel below its ⋯, titled with the file\'s name, keyboard-driven', async ({ page, api }) => {
  const ws = await directoryPage(api, `panels-dir ${Date.now()}`)
  await openWorkspacePage(page, ALICE, ws.workspace_id, 'page')
  const editor = page.getByTestId('aiws-cell-frame-cell-dir').getByTestId('aiws-directory')
  const trigger = member(editor, 'note.txt').getByTestId('aiws-directory-row-menu')
  await trigger.click()
  const actions = page.getByTestId('aiws-directory-row-actions')
  await expect(actions).toHaveClass(/aiws-ctl-panel/)
  await expect(actions.locator('.aiws-ctl-panel-head')).toHaveText('note.txt')
  await below(actions, trigger, page)
  await page.screenshot({ path: 'test-results/aiworkspace-editor-panels/ep02-dir-menu.png' })
  // the first entry has the keyboard, the arrows move it, Esc closes
  const items = actions.locator('.aiws-dir-menu-item:not(:disabled)')
  await expect(items.first()).toBeFocused()
  await page.keyboard.press('ArrowDown')
  await expect(items.nth(1)).toBeFocused()
  await page.keyboard.press('Escape')
  await expect(actions).toHaveCount(0)
  // ✕ closes; an entry acts
  await trigger.click()
  await actions.getByTestId('aiws-ctl-panel-close').click()
  await expect(actions).toHaveCount(0)
  await trigger.click()
  await actions.getByTestId('aiws-directory-rename').click()
  await expect(actions).toHaveCount(0)
  await page.getByTestId('aiws-directory-name').fill('notes.txt')
  await page.getByTestId('aiws-directory-save').click()
  await expect.poll(async () => Object.keys((await api.read(ALICE, ws.workspace_id, 'dir')).content.payload.members as Record<string, Json>).sort()).toEqual(['notes.txt', 'photo.png'])
})

test.describe('on a phone', () => {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars -- the browser comes from the project, the rest is the phone
  const { defaultBrowserType, ...pixel } = devices['Pixel 7']
  test.use(pixel)

  /** A standard bottom sheet: in the sheet layer, the window's width, against its bottom. */
  async function isSheet(page: Page, sheet: Locator) {
    await expect(page.locator('.aiws-sheet-layer').locator(sheet)).toHaveCount(1)
    await expect(sheet).toHaveClass(/aiws-sheet/)
    const box = (await sheet.boundingBox())!
    const viewport = page.viewportSize()!
    expect(box.width).toBeGreaterThanOrEqual(viewport.width - 2)
    expect(box.y + box.height).toBeGreaterThanOrEqual(viewport.height - 2)
  }

  test('EP-03 §7.3 (Pixel 7) the table\'s filter and ⋯ are the standard bottom sheet, titled, with ✕; a tap on the backdrop closes', async ({ page, api }) => {
    const ws = await api.sample(ALICE, `panels-table-phone ${Date.now()}`)
    await openWorkspacePage(page, ALICE, ws.workspace_id, 'surface-main')
    await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-phone', 'true')
    const all = page.getByTestId('aiws-table-cell-all-tasks')
    await expect(all).toHaveAttribute('data-layout', 'cards')
    await all.getByTestId('aiws-filter-open-cell-all-tasks').tap()
    const filter = page.getByTestId('aiws-table-sheet-filter')
    await isSheet(page, filter)
    await expect(filter.locator('.aiws-sheet-head')).toContainText('筛选 / 排序')
    await expect(filter.getByTestId('aiws-filter-panel')).toBeVisible()
    const close = filter.getByTestId('aiws-sheet-close')
    const b = (await close.boundingBox())!
    expect(b.width).toBeGreaterThanOrEqual(44)
    expect(b.height).toBeGreaterThanOrEqual(44)
    await close.tap()
    await expect(filter).toHaveCount(0)

    await all.getByTestId('aiws-table-menu-cell-all-tasks').tap()
    const menu = page.getByTestId('aiws-table-menu-pop-cell-all-tasks')
    await isSheet(page, menu)
    await expect(menu.locator('.aiws-sheet-head')).toContainText('表格的更多操作')
    const item = (await menu.getByTestId('aiws-ctl-menu-table-layout-grid').boundingBox())!
    expect(item.height).toBeGreaterThanOrEqual(44)
    await page.screenshot({ path: 'test-results/aiworkspace-editor-panels/ep03-table-menu.png' })
    // the backdrop above the sheet closes it (tapped over the page's left margin, where the tap's click finds nothing)
    await page.touchscreen.tap(12, 300)
    await expect(menu).toHaveCount(0)
    await all.getByTestId('aiws-table-menu-cell-all-tasks').tap()
    await menu.getByTestId('aiws-ctl-menu-table-layout-grid').tap()
    await expect(menu).toHaveCount(0)
    await expect(all).toHaveAttribute('data-layout', 'grid')
  })

  test('EP-04 §7.4 (Pixel 7) a directory row\'s menu is the standard bottom sheet, titled with the file\'s name; its entries a finger\'s size', async ({ page, api }) => {
    const ws = await directoryPage(api, `panels-dir-phone ${Date.now()}`)
    await openWorkspacePage(page, ALICE, ws.workspace_id, 'page')
    await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-phone', 'true')
    const editor = page.getByTestId('aiws-cell-frame-cell-dir').getByTestId('aiws-directory')
    await member(editor, 'photo.png').getByTestId('aiws-directory-row-menu').tap()
    const actions = page.getByTestId('aiws-directory-row-actions')
    await isSheet(page, actions)
    await expect(actions.locator('.aiws-sheet-head')).toContainText('photo.png')
    for (const id of ['aiws-directory-rename', 'aiws-directory-remove']) {
      const b = (await actions.getByTestId(id).boundingBox())!
      expect(b.height, id).toBeGreaterThanOrEqual(44)
    }
    await page.screenshot({ path: 'test-results/aiworkspace-editor-panels/ep04-dir-menu.png' })
    await actions.getByTestId('aiws-sheet-close').tap()
    await expect(actions).toHaveCount(0)
    // removal from it asks first; the confirmation then removes
    await member(editor, 'photo.png').getByTestId('aiws-directory-row-menu').tap()
    await actions.getByTestId('aiws-directory-remove').tap()
    await expect(actions).toHaveCount(0)
    await page.getByTestId('aiws-directory-confirm').getByTestId('aiws-directory-confirm-ok').tap()
    await expect.poll(async () => Object.keys((await api.read(ALICE, ws.workspace_id, 'dir')).content.payload.members as Record<string, Json>)).toEqual(['note.txt'])
  })
})
