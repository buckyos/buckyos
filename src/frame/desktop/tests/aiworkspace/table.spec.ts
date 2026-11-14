/* The standard table editor (UI 框架 PRD §7.1–§7.3): grid and cards (each person's choice, per device), the record
 * form (a side panel on a computer, the whole screen on a phone) written field by field, new records added in one
 * Commit, filters of several conditions shown as removable chips, selection and its bar, column width and order; and
 * §15 scenario 20, the table half, on an emulated Pixel 7: new record, field type, filter, delete — taps only, no hover,
 * right click or keys. The phone's free canvas reaches the editor through "展开" (another package): here the editor is
 * on the flow page, which a phone edits in place. */

import { devices, type Page } from '@playwright/test'
import { expect, expectAllCommitted, openWorkspace, openWorkspacePage, test, type Api } from './fixtures'

const ALICE = 'tok-alice'

const rows = async (api: Api, workspaceId: string) => (await api.table(ALICE, workspaceId, { source_id: 'tasks' })).rows as { record_id: string; values: Record<string, unknown> }[]
const cellPayload = async (api: Api, workspaceId: string, id: string) => (await api.rpc(ALICE, 'doc.read', { workspace_id: workspaceId, entity_id: id })).content.payload
const userState = async (api: Api, workspaceId: string, key: string) => (await api.rpc(ALICE, 'ws.get_user_state', { workspace_id: workspaceId })).entries[key] ?? null

test('UI-T01 §7.3 on a computer: grid and cards per person, the record form beside the table, written field by field, prev / next, delete asked first', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `table form ${Date.now()}`)
  await openWorkspace(page, ALICE, ws.workspace_id)
  const all = page.getByTestId('aiws-table-cell-all-tasks')
  const other = page.getByTestId('aiws-cell-frame-cell-open-tasks').getByTestId('aiws-table-cell-open-tasks')
  // a computer starts with the grid
  await expect(all).toHaveAttribute('data-layout', 'grid')
  await expect(all.getByTestId('aiws-row')).toHaveCount(5)

  // cards: the title and at most three more fields; the choice is this person's, for this Block only, and no commit
  const head = await api.headSeq(ALICE, ws.workspace_id)
  await all.getByTestId('aiws-table-menu-cell-all-tasks').click()
  await page.getByTestId('aiws-ctl-menu-table-layout-cards').click()
  await expect(all).toHaveAttribute('data-layout', 'cards')
  await expect(all.getByTestId('aiws-table-card')).toHaveCount(5)
  const card = all.locator('[data-testid="aiws-table-card"][data-record-id="task-41"]')
  await expect(card).toContainText('定义对象格式')
  await expect(card.locator('.aiws-table-card-line')).toHaveCount(3)
  await expect(other).toHaveAttribute('data-layout', 'grid')
  await expect.poll(() => userState(api, ws.workspace_id, 'block:cell-all-tasks/table')).toEqual({ layout: { desktop: 'cards' } })
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head)

  // a card opens its record's form: a side panel, every field one above the other, the control of each type
  await card.click()
  const form = page.getByTestId('aiws-table-record-panel')
  await expect(form).toHaveAttribute('data-record-id', 'task-41')
  await expect(form.getByTestId('aiws-table-record-title')).toHaveText('定义对象格式')
  const box = (await form.boundingBox())!
  expect(box.x).toBeGreaterThan(page.viewportSize()!.width / 2)
  await expect(form.locator('.aiws-record-row')).toHaveCount(6)
  await expect(form.getByLabel('截止日期', { exact: true })).toHaveAttribute('type', 'date')
  await expect(form.getByLabel('预算', { exact: true })).toHaveAttribute('inputmode', 'decimal')
  await expect(form.getByTestId('aiws-record-field-title').locator('.aiws-required')).toBeVisible()
  // a field is written when it is left: one undoable step, like a cell of the grid
  await form.getByLabel('负责人', { exact: true }).fill('钱')
  await form.getByLabel('负责人', { exact: true }).press('Tab')
  await expect.poll(async () => (await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-41', 'owner')).value).toBe('钱')
  await expectAllCommitted(page)
  await page.getByTestId('aiws-undo').click()
  await expect.poll(async () => (await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-41', 'owner')).value).toBe('王')
  await expect(form.getByLabel('负责人', { exact: true })).toHaveValue('王')
  // a choice is written when it is chosen
  await form.getByLabel('状态', { exact: true }).selectOption('option-done')
  await expect.poll(async () => (await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-41', 'status')).value).toBe('option-done')
  // a required field left empty is refused under the field, nothing is written
  const seq = await api.headSeq(ALICE, ws.workspace_id)
  await form.getByLabel('任务', { exact: true }).fill('')
  await form.getByLabel('任务', { exact: true }).press('Enter')
  await expect(form.getByTestId('aiws-record-field-title').getByRole('alert')).toContainText('必填')
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(seq)
  // Esc takes the typing back; Esc again closes the panel
  await form.getByLabel('任务', { exact: true }).press('Escape')
  await expect(form.getByLabel('任务', { exact: true })).toHaveValue('定义对象格式')
  await form.getByLabel('任务', { exact: true }).press('Escape')
  await expect(form).toHaveCount(0)

  // previous / next follow the view's order
  await card.click()
  await form.getByTestId('aiws-table-record-next').click()
  await expect(form).toHaveAttribute('data-record-id', 'task-42')
  await form.getByTestId('aiws-table-record-prev').click()
  await expect(form).toHaveAttribute('data-record-id', 'task-41')
  // the record's ⋯: comment, copy link, delete (asked first, then undoable)
  await form.getByTestId('aiws-table-record-more').click()
  const menu = page.getByTestId('aiws-table-record-menu')
  await expect(menu.getByTestId('aiws-ctl-menu-record-comment')).toBeEnabled()
  await expect(menu.getByTestId('aiws-ctl-menu-record-link')).toBeEnabled()
  await menu.getByTestId('aiws-ctl-menu-record-delete').click()
  await expect(page.getByTestId('aiws-table-confirm')).toContainText('定义对象格式')
  await page.getByTestId('aiws-table-confirm-ok').click()
  await expect.poll(async () => (await rows(api, ws.workspace_id)).length).toBe(4)
  await expect(form).toHaveAttribute('data-record-id', 'task-42')
  await page.getByTestId('aiws-undo').click()
  await expect.poll(async () => (await rows(api, ws.workspace_id)).length).toBe(5)
  await form.getByTestId('aiws-table-record-close').click()

  // a new record: the empty form, added in one Commit; "add and continue" keeps it open for the next
  await all.getByTestId('aiws-add-record-cell-all-tasks').click()
  await expect(form).toHaveAttribute('data-mode', 'new')
  await form.getByTestId('aiws-table-record-add').click()
  await expect(form.getByTestId('aiws-record-field-title').getByRole('alert')).toContainText('必填')
  await form.getByLabel('新记录 任务').fill('甲')
  await form.getByLabel('新记录 负责人').fill('林')
  await form.getByTestId('aiws-table-record-add-next').click()
  await expect(form.getByTestId('aiws-table-record-status')).toBeVisible()
  await expect(form.getByLabel('新记录 任务')).toHaveValue('')
  await form.getByLabel('新记录 任务').fill('乙')
  await form.getByTestId('aiws-table-record-add').click()
  await expect(form).toHaveCount(0)
  await expect.poll(async () => (await rows(api, ws.workspace_id)).filter((row) => row.values.title === '甲' || row.values.title === '乙').length).toBe(2)
  expect((await rows(api, ws.workspace_id)).find((row) => row.values.title === '甲')?.values.owner).toBe('林')

  // back to the grid: the head of each row opens the same form
  await all.getByTestId('aiws-table-menu-cell-all-tasks').click()
  await page.getByTestId('aiws-ctl-menu-table-layout-grid').click()
  await expect(all).toHaveAttribute('data-layout', 'grid')
  await all.getByTestId('aiws-row-open-task-40').click()
  await expect(form).toHaveAttribute('data-record-id', 'task-40')
  await form.getByTestId('aiws-table-record-close').click()
})

test('UI-T02 §7.3 on a computer: several filter conditions as chips under the title row, each removable; selection and its bar; column width and order', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `table view ${Date.now()}`)
  await openWorkspace(page, ALICE, ws.workspace_id)
  const all = page.getByTestId('aiws-table-cell-all-tasks')
  const head = await api.headSeq(ALICE, ws.workspace_id)

  // two conditions: 林's tasks with a budget above 500
  await all.getByTestId('aiws-filter-open-cell-all-tasks').click()
  const panel = all.getByTestId('aiws-filter-panel')
  await panel.getByLabel('筛选字段').selectOption('owner')
  await panel.getByLabel('筛选值').fill('林')
  await panel.getByTestId('aiws-filter-add-condition').click()
  const second = panel.getByTestId('aiws-filter-row').nth(1)
  await second.getByLabel('筛选字段').selectOption('budget')
  await second.getByLabel('筛选运算符').selectOption('gt')
  await second.getByLabel('筛选值').fill('500')
  await panel.getByTestId('aiws-filter-apply').click()
  await expect(all.getByTestId('aiws-row')).toHaveCount(1)
  await expect(all.getByTestId('aiws-row')).toContainText('完成第一期架构验证')
  const chips = all.getByTestId('aiws-table-chip')
  await expect(chips).toHaveCount(2)
  await expect(chips.first()).toHaveAttribute('data-kind', 'session')
  // one chip removed: the other condition stays
  await chips.filter({ hasText: '预算' }).getByTestId('aiws-table-chip-remove').click()
  await expect(chips).toHaveCount(1)
  await expect(all.getByTestId('aiws-row')).toHaveCount(2)
  await chips.first().getByTestId('aiws-table-chip-remove').click()
  await expect(all.getByTestId('aiws-row')).toHaveCount(5)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head)

  // ticking row heads selects; the bar copies or deletes them (asked first)
  await all.getByTestId('aiws-row-check-task-43').check()
  await all.getByTestId('aiws-row-check-task-44').check()
  await expect(all.getByTestId('aiws-table-selection-count')).toHaveText('已选 2 条')
  await all.getByTestId('aiws-table-selection-delete').click()
  await expect(page.getByTestId('aiws-table-confirm')).toContainText('删除 2 条记录')
  await page.getByTestId('aiws-table-confirm-ok').click()
  await expect.poll(async () => (await rows(api, ws.workspace_id)).map((row) => row.record_id).sort()).toEqual(['task-40', 'task-41', 'task-42'])
  await expect(all.getByTestId('aiws-table-selection-cancel')).toHaveCount(0)
  await page.getByTestId('aiws-undo').click()
  await expect.poll(async () => (await rows(api, ws.workspace_id)).length).toBe(5)

  // a column edge drags its width; a header drags its place (the Block's view, one commit each)
  const widthOf = async (id: string) => ((await cellPayload(api, ws.workspace_id, 'cell-all-tasks')).fields as { field_id: string; width?: number }[] | undefined)?.find((f) => f.field_id === id)?.width ?? 150
  const budgetWidth = await widthOf('budget')
  const edge = (await all.getByTestId('aiws-col-resize-budget').boundingBox())!
  await page.mouse.move(edge.x + edge.width / 2, edge.y + edge.height / 2)
  await page.mouse.down()
  await page.mouse.move(edge.x + edge.width / 2 + 40, edge.y + edge.height / 2, { steps: 4 })
  await page.mouse.move(edge.x + edge.width / 2 + 60, edge.y + edge.height / 2, { steps: 4 })
  await page.mouse.up()
  await expect.poll(() => widthOf('budget')).toBe(budgetWidth + 60)
  const owner = (await all.getByTestId('aiws-th-due').boundingBox())!
  const title = (await all.getByTestId('aiws-th-title').boundingBox())!
  await page.mouse.move(owner.x + 30, owner.y + owner.height / 2)
  await page.mouse.down()
  await page.mouse.move(owner.x - 40, owner.y + owner.height / 2, { steps: 4 })
  await page.mouse.move(title.x + 10, title.y + title.height / 2, { steps: 8 })
  await expect(all.getByTestId('aiws-col-drop')).toBeVisible()
  await page.mouse.up()
  await expect.poll(async () => ((await cellPayload(api, ws.workspace_id, 'cell-all-tasks')).fields as { field_id: string }[] | undefined)?.map((f) => f.field_id)[0]).toBe('due')
  await expect(all.locator('.aiws-th').first()).toContainText('截止日期')
  await expectAllCommitted(page)
})

test.describe('on a phone', () => {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars -- the browser comes from the project, the rest is the phone
  const { defaultBrowserType, ...pixel } = devices['Pixel 7']
  test.use(pixel)

  /** A finger held still on an element (a long press), through real touch input. */
  async function longPress(page: Page, target: { boundingBox: () => Promise<{ x: number; y: number; width: number; height: number } | null> }) {
    const b = (await target.boundingBox())!
    const cdp = await page.context().newCDPSession(page)
    const point = { x: b.x + b.width / 2, y: b.y + b.height / 2, id: 0 }
    await cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [point] })
    await page.waitForTimeout(800)
    await cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] })
  }
  const big = async (locator: { boundingBox: () => Promise<{ width: number; height: number } | null> }, what: string) => {
    const b = (await locator.boundingBox())!
    expect(b.width, what).toBeGreaterThanOrEqual(44)
    expect(b.height, what).toBeGreaterThanOrEqual(44)
  }

  test('UI-T20 §15 #20 (the table) on a phone: a new record, a field\'s type, a filter, a deleted record — by taps only', async ({ page, api }) => {
    const ws = await api.sample(ALICE, `table phone ${Date.now()}`)
    await openWorkspacePage(page, ALICE, ws.workspace_id)
    await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
    await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-phone', 'true')
    const all = page.getByTestId('aiws-table-cell-all-tasks')
    const viewport = page.viewportSize()!
    // a phone starts with cards; nothing is wider than the screen
    await expect(all).toHaveAttribute('data-layout', 'cards')
    await expect(all.getByTestId('aiws-table-card')).toHaveCount(5)
    const tableBox = (await all.boundingBox())!
    expect(tableBox.x + tableBox.width).toBeLessThanOrEqual(viewport.width + 1)
    for (const id of ['aiws-filter-open-cell-all-tasks', 'aiws-table-menu-cell-all-tasks', 'aiws-add-record-cell-all-tasks']) await big(page.getByTestId(id), id)
    await page.screenshot({ path: 'test-results/aiworkspace-table/t20-cards.png' })

    // 1 a new record: the empty form fills the screen; added with a tap
    await page.getByTestId('aiws-add-record-cell-all-tasks').tap()
    const form = page.getByTestId('aiws-table-record-panel')
    await expect(form).toHaveAttribute('data-mode', 'new')
    const formBox = (await form.boundingBox())!
    expect(formBox.width).toBeGreaterThanOrEqual(viewport.width - 1)
    await form.getByLabel('新记录 任务').fill('手机上新建')
    await form.getByLabel('新记录 状态').selectOption('option-open')
    await expect(form.getByLabel('新记录 预算')).toHaveAttribute('inputmode', 'decimal')
    await big(form.getByTestId('aiws-table-record-add'), 'add')
    await big(form.getByTestId('aiws-table-record-add-next'), 'add and continue')
    await page.screenshot({ path: 'test-results/aiworkspace-table/t20-new.png' })
    await form.getByTestId('aiws-table-record-add').tap()
    await expect(form).toHaveCount(0)
    await expect.poll(async () => (await rows(api, ws.workspace_id)).find((row) => row.values.title === '手机上新建')?.values.status).toBe('option-open')
    await expect(all.getByTestId('aiws-table-card')).toHaveCount(6)

    // 2 a field's type: ⋯ → fields (a sheet) → the field's details → change type, pre-checked, then run
    await page.getByTestId('aiws-table-menu-cell-all-tasks').tap()
    await page.getByTestId('aiws-ctl-menu-table-fields').tap()
    const sheet = page.getByTestId('aiws-table-sheet-fields')
    await expect(sheet).toBeVisible()
    await big(sheet.getByTestId('aiws-field-open-budget'), 'field row')
    await big(sheet.getByTestId('aiws-field-drag-budget'), 'drag handle')
    await sheet.getByTestId('aiws-field-open-budget').tap()
    await expect(sheet.getByTestId('aiws-field-detail')).toHaveAttribute('data-field-id', 'budget')
    await sheet.getByTestId('aiws-migrate-budget').tap()
    await sheet.getByLabel('目标类型').selectOption('currency')
    await sheet.getByTestId('aiws-migration-precheck').tap()
    await expect(sheet.getByTestId('aiws-migration-status')).toContainText('预检通过')
    await page.screenshot({ path: 'test-results/aiworkspace-table/t20-field.png' })
    await sheet.getByTestId('aiws-migration-run').tap()
    await expect.poll(async () => ((await api.read(ALICE, ws.workspace_id, 'tasks')).content.fields as { field_id: string; type: string }[]).find((f) => f.field_id === 'budget')?.type).toBe('currency')
    await sheet.locator('.aiws-ctl-popover-close').tap()
    await expect(sheet).toHaveCount(0)

    // 3 a filter: a sheet from the bottom; the condition shows as a chip under the title row and goes with a tap
    await page.getByTestId('aiws-filter-open-cell-all-tasks').tap()
    const filter = page.getByTestId('aiws-table-sheet-filter')
    await expect(filter).toBeVisible()
    const filterBox = (await filter.boundingBox())!
    expect(filterBox.y + filterBox.height).toBeGreaterThanOrEqual(viewport.height - 2)
    await filter.getByLabel('筛选字段').selectOption('owner')
    await filter.getByLabel('筛选值').fill('林')
    await filter.getByTestId('aiws-filter-apply').tap()
    await expect(filter).toHaveCount(0)
    await expect(all.getByTestId('aiws-table-card')).toHaveCount(2)
    const chip = all.getByTestId('aiws-table-chip')
    await expect(chip).toContainText('负责人')
    await chip.getByTestId('aiws-table-chip-remove').tap()
    await expect(all.getByTestId('aiws-table-card')).toHaveCount(6)

    // 4 a deleted record: a long press selects, the bar deletes, asked first
    const doomed = all.locator('[data-testid="aiws-table-card"][data-record-id="task-43"]')
    await doomed.scrollIntoViewIfNeeded()
    await longPress(page, doomed)
    await expect(all).toHaveAttribute('data-selecting', 'true')
    await expect(doomed).toHaveAttribute('aria-pressed', 'true')
    await expect(page.getByTestId('aiws-table-record-panel')).toHaveCount(0)
    await big(all.getByTestId('aiws-table-selection-delete'), 'delete selected')
    await all.getByTestId('aiws-table-selection-delete').tap()
    await page.getByTestId('aiws-table-confirm-ok').tap()
    await expect.poll(async () => (await rows(api, ws.workspace_id)).some((row) => row.record_id === 'task-43')).toBe(false)
    await expect(all.getByTestId('aiws-table-card')).toHaveCount(5)
    // and from a record's own ⋯
    await all.locator('[data-testid="aiws-table-card"][data-record-id="task-44"]').tap()
    await expect(form).toHaveAttribute('data-record-id', 'task-44')
    await form.getByTestId('aiws-table-record-more').tap()
    await page.getByTestId('aiws-ctl-menu-record-delete').tap()
    await page.getByTestId('aiws-table-confirm-ok').tap()
    await expect.poll(async () => (await rows(api, ws.workspace_id)).some((row) => row.record_id === 'task-44')).toBe(false)
    await expectAllCommitted(page)
  })

  test('UI-T21 §7.3 the grid on a phone: the head and first column stay while the rest scrolls; a tap selects a cell, a second tap edits it in the bar', async ({ page, api }) => {
    const ws = await api.sample(ALICE, `table phone grid ${Date.now()}`)
    await openWorkspacePage(page, ALICE, ws.workspace_id)
    await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
    const all = page.getByTestId('aiws-table-cell-all-tasks')
    await page.getByTestId('aiws-table-menu-cell-all-tasks').tap()
    await page.getByTestId('aiws-ctl-menu-table-layout-grid').tap()
    await expect(all).toHaveAttribute('data-layout', 'grid')
    await expect(all.getByTestId('aiws-row')).toHaveCount(5)
    await expect.poll(() => userState(api, ws.workspace_id, 'block:cell-all-tasks/table')).toEqual({ layout: { phone: 'grid' } })
    // the head and the first column stay put when the grid scrolls sideways; the first column leaves room for the rest
    const first = all.getByTestId('aiws-cell-task-40-title')
    const before = (await first.boundingBox())!
    expect(before.width).toBeLessThanOrEqual(151)
    const ownerBefore = (await all.getByTestId('aiws-cell-task-40-owner').boundingBox())!
    await all.getByTestId('aiws-table-scroll-cell-all-tasks').evaluate((el) => { el.scrollLeft = 200 })
    await expect.poll(async () => Math.round(ownerBefore.x - ((await all.getByTestId('aiws-cell-task-40-owner').boundingBox())?.x ?? 0))).toBe(200)
    const after = (await first.boundingBox())!
    expect(Math.abs(after.x - before.x)).toBeLessThan(2)
    await all.getByTestId('aiws-table-scroll-cell-all-tasks').evaluate((el) => { el.scrollLeft = 0 })

    // a tap selects, a second tap opens the edit bar at the bottom; done writes it
    const cell = all.getByTestId('aiws-cell-task-40-owner')
    await cell.locator('button.aiws-cell-value').tap()
    await expect(cell).toHaveAttribute('aria-selected', 'true')
    await expect(page.getByTestId('aiws-table-edit-bar')).toHaveCount(0)
    await cell.locator('button.aiws-cell-value').tap()
    const bar = page.getByTestId('aiws-table-edit-bar')
    await expect(bar).toBeVisible()
    const barBox = (await bar.boundingBox())!
    expect(barBox.y + barBox.height).toBeGreaterThan(page.viewportSize()!.height - 4)
    await big(bar.getByTestId('aiws-table-edit-bar-done'), 'done')
    await bar.getByLabel('负责人 task-40', { exact: true }).fill('钱')
    await bar.getByTestId('aiws-table-edit-bar-done').tap()
    await expect(bar).toHaveCount(0)
    await expect.poll(async () => (await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-40', 'owner')).value).toBe('钱')
    // cancel leaves the value alone
    await cell.locator('button.aiws-cell-value').tap()
    await bar.getByLabel('负责人 task-40', { exact: true }).fill('不要')
    await bar.getByTestId('aiws-table-edit-bar-cancel').tap()
    await expect(bar).toHaveCount(0)
    await expectAllCommitted(page)
    expect((await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-40', 'owner')).value).toBe('钱')
    // the row's head opens the full-screen form
    await all.getByTestId('aiws-row-open-task-41').tap()
    await expect(page.getByTestId('aiws-table-record-panel')).toHaveAttribute('data-record-id', 'task-41')
    await page.getByTestId('aiws-table-record-close').tap()
    await expect(page.getByTestId('aiws-table-record-panel')).toHaveCount(0)
  })
})
