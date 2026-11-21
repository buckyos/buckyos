/* The standard table editor (UI 框架 PRD §7.1–§7.3): grid and cards (each person's choice, per device), the record
 * form (a side panel on a computer, the whole screen on a phone) written field by field, new records added in one
 * Commit, filters of several conditions shown as removable chips, selection and its bar, column width and order, a
 * field's default (set, filled into new records by the service, filled in on the form, removed, converted); and
 * §15 scenario 20, the table half, on an emulated Pixel 7: new record, field type, filter, delete — taps only, no hover,
 * right click or keys. The phone's free canvas reaches the editor through "展开" (another package): here the editor is
 * on the flow page, which a phone edits in place. */

import { standardTableOp } from 'buckyos'
import { devices, type Locator, type Page } from '@playwright/test'
import { Fingers } from './touchFingers'
import { expect, expectAllCommitted, fitAll, openWorkspace, openWorkspacePage, test, type Api } from './fixtures'

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
  await all.getByTestId('aiws-table-menu-cell-all-tasks').click()
  await page.getByTestId('aiws-ctl-menu-table-add').click()
  await expect(form).toHaveAttribute('data-mode', 'new')
  await form.getByTestId('aiws-table-record-add').click()
  await expect(form.getByTestId('aiws-record-field-title').getByRole('alert')).toContainText('必填')
  await form.getByLabel('新记录 任务').fill('甲')
  await expect(form.getByLabel('新记录 负责人')).toHaveCount(0)
  await form.getByTestId('aiws-table-record-expand').click()
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

  // a column edge drags its width; a separate handle drags its place (the Block's view, one commit each)
  const widthOf = async (id: string) => ((await cellPayload(api, ws.workspace_id, 'cell-all-tasks')).fields as { field_id: string; width?: number }[] | undefined)?.find((f) => f.field_id === id)?.width ?? 150
  const budgetWidth = await widthOf('budget')
  const edge = (await all.getByTestId('aiws-col-resize-budget').boundingBox())!
  await page.mouse.move(edge.x + edge.width / 2, edge.y + edge.height / 2)
  await page.mouse.down()
  await page.mouse.move(edge.x + edge.width / 2 + 40, edge.y + edge.height / 2, { steps: 4 })
  await page.mouse.move(edge.x + edge.width / 2 + 60, edge.y + edge.height / 2, { steps: 4 })
  await page.mouse.up()
  await expect.poll(() => widthOf('budget')).toBe(budgetWidth + 60)
  await all.getByTestId('aiws-th-due').hover()
  const owner = (await all.getByTestId('aiws-col-handle-due').boundingBox())!
  const title = (await all.getByTestId('aiws-th-title').boundingBox())!
  await page.mouse.move(owner.x + owner.width / 2, owner.y + owner.height / 2)
  await page.mouse.down()
  await page.mouse.move(owner.x - 40, owner.y + owner.height / 2, { steps: 4 })
  await page.mouse.move(title.x + 10, title.y + title.height / 2, { steps: 8 })
  await expect(all.getByTestId('aiws-col-drop')).toBeVisible()
  await page.mouse.up()
  await expect.poll(async () => ((await cellPayload(api, ws.workspace_id, 'cell-all-tasks')).fields as { field_id: string }[] | undefined)?.map((f) => f.field_id)[0]).toBe('due')
  await expect(all.locator('.aiws-th').first()).toContainText('截止日期')
  await expectAllCommitted(page)
})

test('UI-T03 §7.3 a field\'s default: set in its details with the control of its type, filled into new records whoever adds them, filled in on the form, removed, converted by a type change', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `table default ${Date.now()}`)
  await openWorkspace(page, ALICE, ws.workspace_id)
  const all = page.getByTestId('aiws-table-cell-all-tasks')
  const fieldOf = async (id: string) => ((await api.read(ALICE, ws.workspace_id, 'tasks')).content.fields as { field_id: string; type: string; default?: unknown }[]).find((f) => f.field_id === id)
  const valuesOf = async (title: string) => (await rows(api, ws.workspace_id)).find((row) => row.values.title === title)?.values

  // the field's details: a list for a select, a number keyboard's input for a decimal; saved with the button
  await all.getByTestId('aiws-fields-cell-all-tasks').click()
  const manager = all.getByTestId('aiws-field-manager')
  const section = manager.getByTestId('aiws-field-default')
  await manager.getByTestId('aiws-field-open-status').click()
  await section.getByLabel('“状态”的默认值').selectOption('option-done')
  await section.getByTestId('aiws-field-default-save').click()
  await expect.poll(async () => (await fieldOf('status'))?.default).toBe('option-done')
  // the option the default names cannot simply be deleted (it is in use), "delete & clear" can
  await expect(manager.getByTestId('aiws-option-delete-option-done')).toBeDisabled()
  await expect(manager.getByTestId('aiws-option-delete-clear-option-done')).toBeEnabled()
  await manager.getByTestId('aiws-field-back').click()
  await manager.getByTestId('aiws-field-open-budget').click()
  await expect(section.getByLabel('“预算”的默认值')).toHaveAttribute('inputmode', 'decimal')
  await section.getByLabel('“预算”的默认值').fill('100')
  await section.getByTestId('aiws-field-default-save').click()
  await expect.poll(async () => (await fieldOf('budget'))?.default).toBe('100.00')
  await manager.getByTestId('aiws-field-back').click()
  await all.getByTestId('aiws-fields-cell-all-tasks').click()

  const form = page.getByTestId('aiws-table-record-panel')
  const edge = all.getByTestId('aiws-add-record-cell-all-tasks')
  await edge.hover()
  await edge.click()
  await expect(form.locator('.aiws-record-row')).toHaveCount(1)
  await form.getByLabel('新记录 任务').fill('快速')
  await form.getByTestId('aiws-table-record-add').click()
  await expect.poll(() => valuesOf('快速')).toMatchObject({ status: 'option-done', budget: '100.00' })
  expect((await api.commit(ALICE, ws, [{ op: 'table.insert_records', source_id: 'tasks', records: [{ record_id: 'by-agent', values: { title: 'Agent' } }] }])).status).toBe('accepted')
  expect((await api.cell(ALICE, ws.workspace_id, 'tasks', 'by-agent', 'status')).value).toBe('option-done')

  // the new record's form starts with the defaults; a field emptied there gets the default too, as it says
  await all.getByTestId('aiws-table-menu-cell-all-tasks').click()
  await page.getByTestId('aiws-ctl-menu-table-layout-cards').click()
  await all.getByTestId('aiws-table-menu-cell-all-tasks').click()
  await page.getByTestId('aiws-ctl-menu-table-add').click()
  await form.getByTestId('aiws-table-record-expand').click()
  await expect(form).toHaveAttribute('data-mode', 'new')
  await expect(form.getByLabel('新记录 状态')).toHaveValue('option-done')
  await expect(form.getByLabel('新记录 预算')).toHaveValue('100.00')
  await expect(form.getByTestId('aiws-table-field-default')).toHaveCount(0)
  await form.getByLabel('新记录 预算').fill('')
  await expect(form.getByTestId('aiws-record-field-budget').getByTestId('aiws-table-field-default')).toContainText('100')
  await form.getByLabel('新记录 状态').selectOption('option-open')
  await form.getByLabel('新记录 任务').fill('表单')
  await form.getByTestId('aiws-table-record-add').click()
  await expect(form).toHaveCount(0)
  await expect.poll(() => valuesOf('表单')).toMatchObject({ status: 'option-open', budget: '100.00' })

  // removed: new records get nothing there; the records that have it keep it
  await all.getByTestId('aiws-table-menu-cell-all-tasks').click()
  await page.getByTestId('aiws-ctl-menu-table-layout-grid').click()
  await all.getByTestId('aiws-fields-cell-all-tasks').click()
  await manager.getByTestId('aiws-field-open-status').click()
  await section.getByTestId('aiws-field-default-clear').click()
  await expect.poll(async () => (await fieldOf('status'))?.default).toBeUndefined()
  await expect(section.getByTestId('aiws-field-default-clear')).toBeDisabled()
  expect((await api.commit(ALICE, ws, [{ op: 'table.insert_records', source_id: 'tasks', records: [{ record_id: 'after-clear', values: { title: '之后' } }] }])).status).toBe('accepted')
  expect((await valuesOf('之后'))?.status).toBeUndefined()
  expect((await valuesOf('快速'))?.status).toBe('option-done')
  // undoable like any change of the schema
  await page.getByTestId('aiws-undo').click()
  await expect.poll(async () => (await fieldOf('status'))?.default).toBe('option-done')
  await manager.getByTestId('aiws-field-back').click()

  // a type change converts the default as it converts the values; the pre-check says what becomes of it
  await manager.getByTestId('aiws-field-open-budget').click()
  await manager.getByTestId('aiws-migrate-budget').click()
  await manager.getByLabel('目标类型').selectOption('currency')
  await manager.getByTestId('aiws-migration-precheck').click()
  await expect(manager.getByTestId('aiws-migration-status')).toContainText('预检通过')
  await expect(manager.getByTestId('aiws-migration-default')).toContainText('将变为')
  await manager.getByTestId('aiws-migration-run').click()
  await expect.poll(async () => (await fieldOf('budget'))?.default).toEqual({ amount: '100.00', currency: 'USD' })
  await expect(section).toHaveAttribute('data-default', JSON.stringify({ amount: '100.00', currency: 'USD' }))
  await expectAllCommitted(page)
})

test('quick insertion uses hidden field definitions and generates the first unused column name beyond Z', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `table hidden fields ${Date.now()}`)
  const usedNames = Array.from({ length: 26 }, (_, i) => ({ field_id: `field${i}`, name: `列 ${String.fromCharCode(65 + i)}`, type: 'text' }))
  const fields = [
    ...usedNames,
    { field_id: 'flag', name: 'Flag', type: 'boolean', required: true, default: false },
    { field_id: 'quantity', name: 'Quantity', type: 'number', required: true, default: 0 },
    { field_id: 'nullable', name: 'Nullable', type: 'text', required: true, nullable: true },
    { field_id: 'unique', name: 'Unique', type: 'text', unique: true },
    { field_id: 'submitted', name: 'Submitted', type: 'text', required: true, submitter: true },
  ]
  const made = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'hidden', parent_id: 'surface-main-content', type_id: 'buckyos.table-source', order_key: 'v', payload: { fields } },
    { op: 'entity.create', entity_id: 'hidden-view', parent_id: 'surface-main', type_id: 'buckyos.cell', order_key: 'v',
      payload: { view: { type: 'table' }, source_ref: { entity_id: 'hidden' }, fields: [{ field_id: 'field0' }] } },
  ])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  await openWorkspace(page, ALICE, ws.workspace_id)
  const table = page.getByTestId('aiws-table-hidden-view')
  await table.getByTestId('aiws-table-menu-hidden-view').click()
  await page.getByTestId('aiws-ctl-menu-table-add').click()
  await expect.poll(async () => (await api.table(ALICE, ws.workspace_id, { source_id: 'hidden' })).rows[0]?.values)
    .toEqual({ flag: false, quantity: 0, nullable: null, submitted: 'alice' })
  await expect(page.getByTestId('aiws-table-record-panel')).toHaveCount(0)
  const column = table.getByTestId('aiws-edge-column-hidden-view')
  await column.hover()
  await column.click()
  await expect(table.getByRole('columnheader')).toHaveCount(2)
  await expect(table.getByRole('columnheader').last()).toContainText('列 AA')
  const source = (await api.read(ALICE, ws.workspace_id, 'hidden')).content
  const added = source.fields.at(-1)
  expect(added.name).toBe('列 AA')
  expect(added.type).toBe('text')
  expect(added.required).toBeFalsy()
  expect((await cellPayload(api, ws.workspace_id, 'hidden-view')).fields).toEqual([{ field_id: 'field0' }, { field_id: added.field_id }])
})

test('clearing and retrying a required nullable field keeps its key; clearing an optional field removes the key', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `table clear fields ${Date.now()}`)
  const made = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'clears', parent_id: 'surface-main-content', type_id: 'buckyos.table-source', order_key: 'v', payload: {
      fields: [{ field_id: 'r', name: 'Required', type: 'text', required: true, nullable: true }, { field_id: 'o', name: 'Optional', type: 'text', nullable: true }],
    } },
    { op: 'table.insert_records', source_id: 'clears', records: [{ record_id: 'one', values: { r: 'First', o: 'Erase' } }] },
    { op: 'entity.create', entity_id: 'clears-view', parent_id: 'surface-main', type_id: 'buckyos.cell', order_key: 'v',
      payload: { view: { type: 'table' }, source_ref: { entity_id: 'clears' } } },
  ])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  await openWorkspace(page, ALICE, ws.workspace_id)
  const table = page.getByTestId('aiws-table-clears-view')
  const required = table.getByTestId('aiws-cell-one-r')
  await required.locator('.aiws-cell-value').click()
  const input = required.getByRole('textbox')
  await input.fill('')
  const remote = await api.commit(ALICE, ws, [{ op: 'table.set_values', source_id: 'clears', values: [{ record_id: 'one', field_id: 'r', value: 'Concurrent', expect: { rev: made.seq } }] }])
  expect(remote.status, JSON.stringify(remote)).toBe('accepted')
  await input.press('Enter')
  await required.getByTestId('aiws-conflict-keep-mine').click()
  const values = async () => (await api.table(ALICE, ws.workspace_id, { source_id: 'clears' })).rows[0].values
  await expect.poll(values).toEqual({ r: null, o: 'Erase' })
  const optional = table.getByTestId('aiws-cell-one-o')
  await optional.locator('.aiws-cell-value').click()
  await optional.getByRole('textbox').fill('')
  await optional.getByRole('textbox').press('Enter')
  await expect.poll(values).toEqual({ r: null })
})

test('the insert form derives missing fields from the full schema and the current event draft', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `table required fields ${Date.now()}`)
  const operations = (['todo', 'calendar'] as const).flatMap((kind, index) => [
    standardTableOp(kind, { entity_id: kind, parent_id: 'surface-main-content', order_key: index ? 'u' : 't', locale: 'en' }),
    { op: 'entity.create', entity_id: `${kind}-view`, parent_id: 'surface-main', order_key: index ? 'u' : 't', type_id: 'buckyos.cell',
      payload: { view: { type: 'table' }, source_ref: { entity_id: kind }, fields: [{ field_id: 'title' }] } },
  ])
  const made = await api.commit(ALICE, ws, operations)
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  await openWorkspace(page, ALICE, ws.workspace_id)
  const form = page.getByTestId('aiws-table-record-panel')
  const open = async (kind: string) => {
    await page.getByTestId(`aiws-table-menu-${kind}-view`).click()
    await page.getByTestId('aiws-ctl-menu-table-add').click()
  }
  await open('todo')
  await expect(form.locator('.aiws-record-row')).toHaveCount(1)
  await form.getByLabel('新记录 Title').fill('Plan')
  await form.getByTestId('aiws-table-record-add').click()
  await expect.poll(async () => (await api.table(ALICE, ws.workspace_id, { source_id: 'todo' })).rows[0]?.values).toEqual({ title: 'Plan', status: 'todo' })
  await open('calendar')
  await expect(form.locator('.aiws-record-row')).toHaveCount(2)
  await expect(form.getByTestId('aiws-record-field-start')).toBeVisible()
  await form.getByTestId('aiws-table-record-close').click()
  expect((await api.table(ALICE, ws.workspace_id, { source_id: 'calendar' })).total).toBe(0)
  await open('calendar')
  await form.getByLabel('新记录 Title').fill('Holiday')
  await form.getByTestId('aiws-table-record-expand').click()
  await form.getByLabel('新记录 All day').check()
  await expect(form.getByTestId('aiws-record-field-start_date').locator('.aiws-required')).toBeVisible()
  await expect(form.getByTestId('aiws-record-field-start').locator('.aiws-required')).toHaveCount(0)
  await form.getByLabel('新记录 Start date').fill('2026-11-21')
  await form.getByTestId('aiws-table-record-add').click()
  await expect.poll(async () => (await api.table(ALICE, ws.workspace_id, { source_id: 'calendar' })).rows[0]?.values).toEqual({ title: 'Holiday', all_day: true, start_date: '2026-11-21' })
})

test.describe('on a phone', () => {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars -- the browser comes from the project, the rest is the phone
  const { defaultBrowserType, ...pixel } = devices['Pixel 7']
  test.use(pixel)

  /** A finger held still on an element (a long press), through real touch input. */
  async function longPress(page: Page, target: Locator) {
    const status = page.getByTestId('aiws-status-detail')
    if (await status.isVisible()) await status.getByRole('button', { name: '收起工作区状态' }).tap()
    await target.evaluate((el) => el.scrollIntoView({ block: 'center', inline: 'nearest', behavior: 'instant' }))
    await target.tap({ trial: true })
    await expect.poll(() => target.evaluate((el) => {
      const r = el.getBoundingClientRect()
      return el.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2))
    })).toBe(true)
    const r = (await target.boundingBox())!
    const fingers = await Fingers.of(page)
    await fingers.down({ x: r.x + r.width / 2, y: r.y + r.height / 2 })
    await page.waitForTimeout(800)
    await fingers.up()
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
    for (const id of ['aiws-filter-open-cell-all-tasks', 'aiws-table-menu-cell-all-tasks']) await big(page.getByTestId(id), id)
    await page.screenshot({ path: 'test-results/aiworkspace-table/t20-cards.png' })

    // 1 a new record: the empty form fills the screen; added with a tap
    await all.getByTestId('aiws-table-menu-cell-all-tasks').tap()
    await page.getByTestId('aiws-ctl-menu-table-add').tap()
    const form = page.getByTestId('aiws-table-record-panel')
    await expect(form).toHaveAttribute('data-mode', 'new')
    const formBox = (await form.boundingBox())!
    expect(formBox.width).toBeGreaterThanOrEqual(viewport.width - 1)
    await form.getByLabel('新记录 任务').fill('手机上新建')
    await form.getByTestId('aiws-table-record-expand').tap()
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
    await sheet.getByTestId('aiws-sheet-close').tap()
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

  test('UI-T21 §7.3 the grid on a phone: the head and first column stay while the rest scrolls; one tap edits the cell in the bar', async ({ page, api }) => {
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

    // a single tap selects the cell and opens its edit bar
    const cell = all.getByTestId('aiws-cell-task-40-owner')
    await cell.locator('button.aiws-cell-value').tap()
    await expect(cell).toHaveAttribute('aria-selected', 'true')
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
    // The row menu opens its full-screen form.
    await all.getByTestId('aiws-row-handle-task-41').tap()
    await all.getByTestId('aiws-row-menu-task-41').tap()
    await page.getByTestId('aiws-ctl-menu-open-record').tap()
    await expect(page.getByTestId('aiws-table-record-panel')).toHaveAttribute('data-record-id', 'task-41')
    await page.getByTestId('aiws-table-record-close').tap()
    await expect(page.getByTestId('aiws-table-record-panel')).toHaveCount(0)
  })
})

test('grid navigation crosses the loaded page, validates edits and appends one record at the real end', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `grid keys ${Date.now()}`)
  const count = 201
  const made = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'grid', parent_id: 'surface-main-content', order_key: 'u', type_id: 'buckyos.table-source',
      payload: { fields: [{ field_id: 'name', name: 'Name', type: 'text' }, { field_id: 'amount', name: 'Amount', type: 'number' }] } },
    { op: 'table.insert_records', source_id: 'grid', records: Array.from({ length: count }, (_, i) => ({ record_id: `r${String(i).padStart(3, '0')}`, values: {} })) },
    { op: 'entity.create', entity_id: 'grid-view', parent_id: 'surface-main', order_key: 'u', type_id: 'buckyos.cell', payload: { view: { type: 'table' }, source_ref: { entity_id: 'grid' } } },
  ])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  await openWorkspace(page, ALICE, ws.workspace_id)
  const grid = page.getByTestId('aiws-table-grid-view')
  const scroller = grid.getByRole('grid')
  await expect(grid).toHaveAttribute('data-total', String(count))
  await scroller.scrollIntoViewIfNeeded()
  const bounds = (await scroller.boundingBox())!
  await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + bounds.height - 4)
  await expect(grid.getByTestId('aiws-add-record-grid-view')).toHaveCSS('opacity', '1')
  await page.mouse.move(bounds.x + bounds.width - 4, bounds.y + 60)
  await expect(grid.getByTestId('aiws-edge-column-grid-view')).toHaveCSS('opacity', '1')
  await expect(grid.getByTestId('aiws-add-record-grid-view')).toHaveCSS('opacity', '0')
  await scroller.evaluate((el) => { el.scrollTop = el.scrollHeight })
  const value = (id: string, field: string) => grid.getByTestId(`aiws-cell-${id}-${field}`).locator('[data-col]')
  await value('r199', 'amount').focus()
  await page.keyboard.press('Tab')
  await expect(value('r200', 'name')).toBeFocused()
  await page.keyboard.press('Shift+Tab')
  await expect(value('r199', 'amount')).toBeFocused()
  const last = grid.getByTestId('aiws-cell-r200-amount')
  await last.getByRole('button', { name: 'Amount r200', exact: true }).click()
  await last.getByRole('textbox').fill('invalid')
  await last.getByRole('textbox').press('Tab')
  await expect(last.getByRole('alert')).toBeVisible()
  await expect(grid).toHaveAttribute('data-total', String(count))
  await last.getByRole('textbox').fill('323')
  await last.getByRole('textbox').press('Tab')
  await expect(grid).toHaveAttribute('data-total', String(count + 1))
  const appended = grid.locator(`[data-row="${count}"][data-col="0"]`)
  await expect(appended).toBeFocused()
  expect((await api.cell(ALICE, ws.workspace_id, 'grid', 'r200', 'amount')).value).toBe(323)
  const addedId = await appended.evaluate((el) => el.closest<HTMLElement>('[role="gridcell"]')!.dataset.recordId!)
  const end = grid.getByTestId(`aiws-cell-${addedId}-amount`)
  await end.getByRole('button', { name: `Amount ${addedId}`, exact: true }).click()
  await end.getByRole('textbox').fill('42')
  const remote = await api.commit(ALICE, ws, [{ op: 'table.set_values', source_id: 'grid', values: [{ record_id: addedId, field_id: 'amount', value: 7, expect: { rev: 0 } }] }])
  expect(remote.status, JSON.stringify(remote)).toBe('accepted')
  await end.getByRole('textbox').press('Tab')
  await expect(end.getByTestId('aiws-conflict-keep-mine')).toBeVisible()
  await expect(end.locator('[data-col]')).toBeFocused()
  await expect(grid).toHaveAttribute('data-total', String(count + 1))
  await end.getByTestId('aiws-conflict-keep-mine').click()
  await expect.poll(async () => (await api.cell(ALICE, ws.workspace_id, 'grid', addedId, 'amount')).value).toBe(42)
  await end.locator('[data-col]').focus()
  await page.keyboard.down('Tab')
  const final = grid.locator(`[data-row="${count + 1}"][data-col="0"]`)
  await expect(final).toBeFocused()
  for (let i = 0; i < 4; i++) await page.keyboard.down('Tab')
  await page.keyboard.up('Tab')
  await expect(final).toBeFocused()
  await expect(grid).toHaveAttribute('data-total', String(count + 2))
  await final.press('Escape')
  await expect(page.getByTestId('aiws-cell-frame-grid-view')).toBeFocused()
  await page.keyboard.press('Tab')
  await expect(page.getByTestId('aiws-cell-frame-grid-view').locator(':focus')).toHaveCount(0)

  const header = grid.getByTestId('aiws-th-name')
  await header.getByRole('button').click()
  const rename = header.getByRole('textbox')
  await rename.fill('Amount')
  await rename.press('Enter')
  await expect(header.getByRole('alert')).toBeVisible()
  await expect(rename).toHaveValue('Amount')
  await rename.fill('Description')
  await rename.press('Enter')
  await expect(header).toContainText('Description')
  expect((await api.read(ALICE, ws.workspace_id, 'grid')).content.fields.find((field: { field_id: string }) => field.field_id === 'name').name).toBe('Description')
})

async function freeGrid(api: Api, count = 40) {
  const ws = await api.rpc(ALICE, 'ws.create', { title: `canvas grid ${Date.now()}` })
  const data = Array.from({ length: count }, (_, i) => ({ record_id: `t${String(i).padStart(String(count - 1).length, '0')}`, values: {} }))
  const seed = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'board-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'h', payload: { kind: 'folder', system: 'surface_content', surface_id: 'board' } },
    { op: 'entity.create', entity_id: 'board', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'h', payload: { kind: 'surface', layout: { mode: 'free' }, content_folder_id: 'board-content' } },
    { op: 'entity.create', entity_id: 'touch-data', type_id: 'buckyos.table-source', parent_id: 'board-content', order_key: 'h',
      payload: { fields: [{ field_id: 'label', name: 'Label', type: 'text' }, { field_id: 'flag', name: 'Flag', type: 'boolean' }] } },
    { op: 'table.insert_records', source_id: 'touch-data', records: data },
    { op: 'entity.create', entity_id: 'touch-table', type_id: 'buckyos.cell', parent_id: 'board', order_key: 'h', placement: { x: 80, y: 90, w: 660, h: 380 },
      payload: { view: { type: 'table' }, source_ref: { entity_id: 'touch-data' }, title: 'Touch table' } },
  ])
  expect(seed.status, JSON.stringify(seed)).toBe('accepted')
  return ws
}

test('a resting canvas table exposes handles and preserves a late-page anchor across editor activation', async ({ page, api }) => {
  const count = 240
  const ws = await freeGrid(api, count)
  await openWorkspacePage(page, ALICE, ws.workspace_id, 'board')
  await fitAll(page)
  const table = page.getByTestId('aiws-table-touch-table')
  const scrollToEnd = async () => {
    await expect(table).toHaveAttribute('data-compact', 'true')
    await table.getByRole('grid').evaluate((el) => { el.scrollTop = el.scrollHeight })
    await expect(table.getByTestId('aiws-cell-t239-label')).toBeVisible()
  }
  await expect(table).toHaveAttribute('data-total', String(count))
  await scrollToEnd()
  await table.getByTestId('aiws-cell-t238-label').hover()
  const handle = table.getByTestId('aiws-row-handle-t238')
  await expect(handle).toHaveCSS('opacity', '1')
  await expect(table.getByTestId('aiws-col-handle-label')).toHaveCSS('opacity', '1')
  await expect(table).toHaveAttribute('data-compact', 'true')
  await handle.click()
  await table.getByTestId('aiws-row-menu-t238').click()
  await page.getByTestId('aiws-ctl-menu-insert-row-before').click()
  const read = () => api.table(ALICE, ws.workspace_id, { view_id: 'touch-table', limit: 300 })
  await expect.poll(async () => (await read()).total).toBe(count + 1)
  const order = (await read()).rows.map((r: { record_id: string }) => r.record_id)
  expect(order[238]).not.toMatch(/^t\d+$/)
  expect([...order.slice(0, 238), ...order.slice(239)]).toEqual(Array.from({ length: count }, (_, i) => `t${String(i).padStart(3, '0')}`))

  await page.reload()
  await expect(table).toHaveAttribute('data-total', String(count + 1))
  await scrollToEnd()
  const last = table.getByTestId('aiws-cell-t239-label')
  await last.locator('.aiws-cell-value').click()
  await expect(last.getByRole('textbox')).toBeFocused()
  await last.getByRole('textbox').fill('late page')
  await last.getByRole('textbox').press('Enter')
  await expect.poll(async () => (await api.cell(ALICE, ws.workspace_id, 'touch-data', 't239', 'label')).value).toBe('late page')
})

test.describe('iPad grid', () => {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars -- the project supplies Chromium
  const { defaultBrowserType, ...ipad } = devices['iPad (gen 7) landscape']
  test.use(ipad)
  test('a free canvas table accepts one-finger scrolling, two-finger zoom and a single tap to edit', async ({ page, api }) => {
    const ws = await freeGrid(api)
    await openWorkspacePage(page, ALICE, ws.workspace_id, 'board')
    await fitAll(page)
    const frame = page.getByTestId('aiws-canvas-block-touch-table')
    const grid = frame.getByRole('grid')
    await frame.getByTestId('aiws-cell-title-touch-table').tap()
    await expect(frame.getByTestId('aiws-col-handle-label')).toBeVisible()
    const fingers = await Fingers.of(page)
    const point = async (i: number, field: string) => {
      const cell = grid.getByTestId('aiws-row').nth(i).locator(`[role=gridcell][data-field-id="${field}"]`)
      const box = (await cell.boundingBox())!
      return { x: box.x + box.width / 2, y: box.y + box.height / 2 }
    }
    const start = await point(3, 'flag')
    await fingers.drag(page, [start], [{ x: start.x, y: start.y - 70 }])
    await expect.poll(async () => grid.evaluate((el) => el.scrollTop)).toBeGreaterThan(0)
    const world = page.getByTestId('aiws-world')
    const scale = () => world.evaluate((el) => new DOMMatrix(getComputedStyle(el).transform).a)
    const before = await scale()
    const box = (await grid.boundingBox())!
    const mid = { x: box.x + box.width * 0.62, y: box.y + box.height * 0.62 }
    await fingers.drag(page, [{ ...mid, x: mid.x - 25 }, { ...mid, x: mid.x + 25 }], [{ ...mid, x: mid.x - 36 }, { ...mid, x: mid.x + 36 }])
    await expect.poll(scale).toBeGreaterThan(before)
    await expect(grid.getByRole('textbox')).toHaveCount(0)
    const query = () => api.table(ALICE, ws.workspace_id, { source_id: 'touch-data', limit: 50 })
    expect((await query()).rows.every((r: { values: object }) => Object.keys(r.values).length === 0)).toBe(true)
    await grid.evaluate((el) => { el.scrollTop = 0 })
    const cell = frame.getByTestId('aiws-cell-t00-label')
    await cell.locator('.aiws-cell-value').tap()
    const input = cell.getByRole('textbox')
    await expect(input).toBeFocused()
    await input.fill('323')
    await input.press('Enter')
    await expect.poll(async () => (await query()).rows[0].values.label).toBe('323')
  })

  test('touch handles select, resize and insert beside an anchor', async ({ page, api }) => {
    const ws = await api.sample(ALICE, `tablet grid ${Date.now()}`)
    await openWorkspacePage(page, ALICE, ws.workspace_id)
    const grid = page.getByTestId('aiws-table-cell-all-tasks')
    await expect(grid).toHaveAttribute('data-layout', 'grid')
    const handle = grid.getByTestId('aiws-row-handle-task-41')
    await handle.tap()
    await expect(handle).toHaveAttribute('aria-pressed', 'true')
    const resize = grid.getByTestId('aiws-row-resize-task-41')
    const r = (await resize.boundingBox())!
    expect(r.width).toBeGreaterThanOrEqual(44)
    expect(r.height).toBeGreaterThanOrEqual(44)
    const cdp = await page.context().newCDPSession(page)
    const p = { x: r.x + r.width / 2, y: r.y + r.height / 2, id: 1 }
    await cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [p] })
    await cdp.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ ...p, y: p.y + 30 }] })
    await cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] })
    await expect.poll(async () => (await cellPayload(api, ws.workspace_id, 'cell-all-tasks')).config?.row_heights?.['task-41']).toBe(74)
    const changed = (await resize.boundingBox())!
    const start = { x: changed.x + changed.width / 2, y: changed.y + changed.height / 2, id: 2 }
    await cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [start] })
    await cdp.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ ...start, y: start.y + 25 }] })
    await cdp.send('Input.dispatchTouchEvent', { type: 'touchCancel', touchPoints: [] })
    await expect.poll(async () => (await grid.locator('[data-row-index][data-record-id="task-41"]').boundingBox())?.height).toBe(74)
    const source = (await handle.boundingBox())!
    const target = (await grid.getByTestId('aiws-row-handle-task-43').boundingBox())!
    const moving = { x: source.x + source.width / 2, y: source.y + source.height / 2, id: 3 }
    await cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [moving] })
    await cdp.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ ...moving, y: target.y + 3 }] })
    await cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] })
    await expect.poll(async () => (await api.table(ALICE, ws.workspace_id, { view_id: 'cell-all-tasks' })).rows.map((r: { record_id: string }) => r.record_id))
      .toEqual(['task-40', 'task-42', 'task-41', 'task-43', 'task-44'])
    await expect(grid.locator('[data-row-index="2"]')).toHaveAttribute('data-record-id', 'task-41')
    const column = grid.getByTestId('aiws-col-handle-title')
    await column.tap()
    await expect(column).toHaveAttribute('aria-pressed', 'true')
    await grid.getByTestId('aiws-col-menu-title').tap()
    await page.getByTestId('aiws-ctl-menu-insert-column-before').tap()
    await expect(grid.getByRole('columnheader').first()).toContainText('列 A')
  })
})

test('image and canvas fields share readable targets and rendering across the grid, cards and record form', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `table references ${Date.now()}`)
  const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAABAAAAAJCAIAAAC0SDtlAAAAFElEQVR42mPQiDpBEmIY1TAoNAAAk1q5oVLxFXwAAAAASUVORK5CYII=', 'base64')
  const photo = await api.uploadAsset(ALICE, ws.workspace_id, png)
  const document = await api.uploadAsset(ALICE, ws.workspace_id, Buffer.from('plain text'))
  const created = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'gallery', parent_id: 'data', order_key: 'z', type_id: 'buckyos.table-source',
      payload: { fields: [{ field_id: 'title', name: '记录', type: 'text' }], title_field_id: 'title' } },
    { op: 'table.insert_records', source_id: 'gallery', records: [{ record_id: 'r1', values: { title: '展品' } }] },
    { op: 'entity.create', entity_id: 'gallery-view', parent_id: 'surface-main', order_key: '01', type_id: 'buckyos.cell',
      payload: { source_ref: { entity_id: 'gallery' }, view: { type: 'table' }, title: '展示' } },
    ...[['photo', photo.object_id], ['document', document.object_id]].map(([id, object]) => ({
      op: 'entity.create', entity_id: id, parent_id: 'data', order_key: 'z', name: id, type_id: 'buckyos.asset-ref', payload: { object_id: object, file_name: id },
    })),
    ...['sketch', 'twin'].flatMap((id) => [
      { op: 'entity.create', entity_id: `${id}-data`, parent_id: 'canvas-content', order_key: 'z', type_id: 'buckyos.container', payload: { kind: 'folder', system: 'surface_content', surface_id: id, title: '同名' } },
      { op: 'entity.create', entity_id: id, parent_id: 'surfaces', order_key: 'z', type_id: 'buckyos.container', name: id, payload: { kind: 'surface', title: '同名', content_folder_id: `${id}-data`, layout: { mode: 'free' } } },
    ]),
  ])
  expect(created.status, JSON.stringify(created)).toBe('accepted')
  await openWorkspacePage(page, ALICE, ws.workspace_id, 'surface-main')
  const table = page.getByTestId('aiws-table-gallery-view')
  const fieldButton = table.getByTestId('aiws-fields-gallery-view')
  await fieldButton.click()
  const addField = async (name: string, purpose: string) => {
    const add = table.locator('form.aiws-field-add')
    await add.locator('input').fill(name)
    await add.locator('select').selectOption(purpose)
    await add.getByTestId('aiws-add-field').click()
    await expect(add.locator('input')).toHaveValue('')
    const read = await api.read(ALICE, ws.workspace_id, 'gallery')
    const field = (read.content.fields as { field_id: string; type: string; name: string; ref_purpose: string }[]).find((f) => f.name === name)!
    expect(field).toMatchObject({ type: 'object_ref', ref_purpose: purpose })
    return field.field_id
  }
  const imageField = await addField('照片', 'image')
  const canvasField = await addField('关联画布', 'canvas')
  await fieldButton.click()
  const imageCell = table.getByTestId(`aiws-cell-r1-${imageField}`)
  const canvasCell = table.getByTestId(`aiws-cell-r1-${canvasField}`)
  const set = async (field: string) => (await api.cell(ALICE, ws.workspace_id, 'gallery', 'r1', field)).value?.entity_id
  await imageCell.getByRole('button').first().click()
  await expect(imageCell.locator('option[value="document"]')).toHaveCount(0)
  await imageCell.locator('select').selectOption('photo')
  await expect.poll(() => set(imageField)).toBe('photo')
  await expect(imageCell.locator('img')).toHaveAttribute('data-object-id', photo.object_id)
  await expect.poll(() => imageCell.locator('img').evaluate((image: HTMLImageElement) => image.naturalWidth)).toBe(16)
  await imageCell.getByRole('button').first().click()
  await imageCell.locator('input[type=file]').setInputFiles({ name: 'invalid.txt', mimeType: 'text/plain', buffer: Buffer.from('not an image') })
  await expect(imageCell.getByRole('alert')).toBeVisible()
  expect(await set(imageField)).toBe('photo')
  await imageCell.locator('input[type=file]').setInputFiles({ name: 'replacement.png', mimeType: 'image/png', buffer: png })
  await expect.poll(() => set(imageField)).toMatch(/^a[a-z0-9_-]+$/)
  await expect(imageCell.getByRole('img', { name: 'replacement.png' })).toBeVisible()
  const firstUpload = await set(imageField)
  await imageCell.getByRole('button').first().click()
  await imageCell.locator('input[type=file]').setInputFiles({ name: 'replacement.png', mimeType: 'image/png', buffer: png })
  await expect.poll(() => set(imageField)).not.toBe(firstUpload)
  await expect(imageCell.getByRole('img', { name: 'replacement.png' })).toBeVisible()

  await canvasCell.getByRole('button').first().click()
  await expect(canvasCell.locator('option[value="surface-main"]')).toHaveCount(0)
  await expect(canvasCell.locator('option[value="sketch-data"]')).toHaveCount(0)
  await expect(canvasCell.getByRole('option', { name: '同名 · sketch' })).toBeAttached()
  await canvasCell.locator('select').selectOption({ label: '同名 · twin' })
  await expect.poll(() => set(canvasField)).toBe('twin')
  const target = await api.read(ALICE, ws.workspace_id, 'twin')
  const renamed = await api.commit(ALICE, ws, [{ op: 'entity.set_keys', entity_id: 'twin', keys: [{ key: 'title', expect: { rev: target.content.key_revs.title }, value: '工作画布' }] }])
  expect(renamed.status).toBe('accepted')
  await expect(canvasCell).toContainText('工作画布')
  await table.getByTestId('aiws-table-menu-gallery-view').click()
  await page.getByTestId('aiws-ctl-menu-table-layout-cards').click()
  const card = table.getByTestId('aiws-table-card')
  await expect(card.getByRole('img', { name: 'replacement.png' })).toBeVisible()
  await card.click()
  const form = page.getByTestId('aiws-table-record-panel')
  await expect(form.getByRole('img', { name: 'replacement.png' })).toBeVisible()
  await form.getByTestId(`aiws-record-field-${imageField}`).locator('select').selectOption('')
  await expect.poll(() => set(imageField)).toBeUndefined()
  await form.getByTestId('aiws-table-record-close').click()
  await expect(card.getByRole('img')).toHaveCount(0)
  await card.getByRole('button', { name: '进入画布' }).click()
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-surface-id', 'twin')

  const acl = await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [
    { subject: 'bob', resource: '/', allow: ['read'] }, { subject: 'bob', resource: 'twin', deny: ['read'] },
  ] })
  expect(acl.ok, JSON.stringify(acl)).toBe(true)
  await openWorkspacePage(page, 'tok-bob', ws.workspace_id, 'surface-main')
  await expect(canvasCell).toContainText('引用不可用')
  await expect(canvasCell.getByRole('button', { name: '进入画布' })).toHaveCount(0)
  await openWorkspacePage(page, ALICE, ws.workspace_id, 'surface-main')
  await expect(card).toContainText('工作画布')
  const deleted = await api.commit(ALICE, ws, [{ op: 'entity.delete', entity_id: 'twin', expect: { rev: target.life_rev } }])
  expect(deleted.status, JSON.stringify(deleted)).toBe('accepted')
  await expect(card).toContainText('引用不可用')
  await expect(card.getByRole('button', { name: '进入画布' })).toHaveCount(0)
  expect(await set(canvasField)).toBe('twin')
})
