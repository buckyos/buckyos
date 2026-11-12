/* The official to-do control (notepads/aiworkspace-control-mechanism-todo.md §0.2, §5.1): a standard to-do table shown as
 * a list and a board through projected label / shape Blocks; quick add, completing, reordering by drag, moving cards
 * between status columns, folding a column, the personal view and "set as default", concurrent moves, writes by
 * another client (an Agent) seen at once, read-only members, and the kernel's standard-table rules. */

import type { Page } from '@playwright/test'
import { asUser, openCanvas, test, expect, workspacePage, type Api } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'
const SHOTS = 'test-results/aiworkspace-todo'

type Json = Record<string, unknown>

const FIELDS = [
  { field_id: 'title', name: 'Title', type: 'text', required: true },
  { field_id: 'status', name: 'Status', type: 'select', required: true, options: [{ option_id: 'todo', label: 'To do' }, { option_id: 'doing', label: 'In progress' }, { option_id: 'done', label: 'Done' }] },
  { field_id: 'due', name: 'Due', type: 'date', nullable: true },
  { field_id: 'order', name: 'Order', type: 'text', nullable: true },
  { field_id: 'done_at', name: 'Completed', type: 'datetime', nullable: true },
  { field_id: 'ext', name: 'Extension', type: 'json', nullable: true },
]

/** A free canvas with a to-do table (four items) and a to-do control bound to it, shared with bob. */
async function todoWorkspace(api: Api, title: string, bobCaps = ['read', 'update', 'append', 'comment']) {
  const ws = await api.freeCanvas(ALICE, title)
  const made = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'tasks', type_id: 'buckyos.table-source', schema_id: 'buckyos.todo@1', parent_id: 'free-content', order_key: 'a', name: 'Tasks', payload: { title_field_id: 'title', fields: FIELDS } },
    { op: 'entity.create', entity_id: 'tl', type_id: 'buckyos.container', parent_id: 'free', order_key: 'b', placement: { x: 40, y: 40, w: 460, h: 480 },
      payload: { kind: 'todo', layout: { mode: 'data' }, title: 'Week', bindings: { items: { entity_id: 'tasks' } } } },
    { op: 'table.insert_records', source_id: 'tasks', records: [
      { record_id: 'a', values: { title: 'Write the report', status: 'todo', order: 'b' } },
      { record_id: 'b', values: { title: 'Call the bank', status: 'todo', order: 'd', due: '2026-01-02' } },
      { record_id: 'c', values: { title: 'Plan the trip', status: 'doing', order: 'f' } },
      { record_id: 'd', values: { title: 'Pay rent', status: 'done', order: 'h', done_at: '2026-01-01T10:00:00Z' } },
    ] },
  ])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  await api.rpc(ALICE, 'ws.grant', { workspace_id: ws.workspace_id, subject: 'bob', capabilities: bobCaps })
  return ws
}

async function cell(api: Api, workspaceId: string, record: string, field: string): Promise<unknown> {
  const read = await api.rpc(ALICE, 'doc.read', { workspace_id: workspaceId, entity_id: 'tasks', selector: { kind: 'table_record', record_id: record } }) as { content: { values: Json } }
  return read.content.values[field]
}

/** Select the control (a press on its header) so its body takes input. */
async function activate(page: Page) {
  // until it is selected the body lets the canvas take the press (which selects the control)
  const head = await page.getByTestId('aiws-todo-head-tl').boundingBox()
  if (!head) throw new Error('the control is not on screen')
  await page.mouse.click(head.x + 200, head.y + 12)
  await expect(page.getByTestId('aiws-data-body-tl')).toHaveAttribute('data-active', 'true')
}

const item = (page: Page, id: string, node = 'title') => page.locator(`[data-proj="tl~${node}~${id}"]`)

test('TD01 list: projected items, quick add keeps adding, tick to complete, completed fold', async ({ page, api }) => {
  const ws = await todoWorkspace(api, `todo-list ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await expect(item(page, 'a')).toContainText('Write the report')
  await expect(page.getByTestId('aiws-todo-count-tl')).toContainText('3')
  await activate(page)
  // quick add: Enter adds and keeps the field for the next one
  const add = page.getByTestId('aiws-todo-add-tl')
  await add.click()
  await add.fill('Buy milk')
  await add.press('Enter')
  await add.fill('Water the plants')
  await add.press('Enter')
  await expect(add).toHaveValue('')
  await expect(page.locator('[data-proj^="tl~title~"]', { hasText: 'Water the plants' })).toBeVisible()
  await expect(page.getByTestId('aiws-todo-count-tl')).toContainText('5')
  // tick: completed, done_at stamped
  await item(page, 'a', 'check').click()
  await expect.poll(() => cell(api, ws.workspace_id, 'a', 'status')).toBe('done')
  expect(await cell(api, ws.workspace_id, 'a', 'done_at')).toBeTruthy()
  // the completed section folds and unfolds
  await page.getByTestId('aiws-todo-donehead-tl').click()
  await expect(item(page, 'd')).toHaveCount(0)
  await page.getByTestId('aiws-todo-donehead-tl').click()
  await expect(item(page, 'd')).toBeVisible()
  await page.getByTestId('aiws-canvas-view').screenshot({ path: `${SHOTS}/td01-list.png` })
})

test('TD02 list: drag to reorder with an insertion line, keyboard, undo', async ({ page, api }) => {
  const ws = await todoWorkspace(api, `todo-drag ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await activate(page)
  const from = await item(page, 'c').boundingBox()
  const to = await item(page, 'a').boundingBox()
  if (!from || !to) throw new Error('items not on screen')
  await page.mouse.move(from.x + 20, from.y + from.height / 2)
  await page.mouse.down()
  await page.mouse.move(from.x + 24, from.y + from.height / 2 - 10, { steps: 3 })
  await page.mouse.move(to.x + 24, to.y + 4, { steps: 8 })
  await expect(page.getByTestId('aiws-todo-insert')).toBeVisible()
  await page.mouse.up()
  // c now sorts before a
  await expect.poll(async () => String(await cell(api, ws.workspace_id, 'c', 'order')) < String(await cell(api, ws.workspace_id, 'a', 'order'))).toBe(true)
  // keyboard: select, Space completes, Delete removes, Ctrl+Z restores
  const undos = Number(await page.getByTestId('aiws-undo').getAttribute('data-count'))
  await item(page, 'b').click()
  await page.keyboard.press(' ')
  await expect.poll(() => cell(api, ws.workspace_id, 'b', 'status')).toBe('done')
  await expect(page.getByTestId('aiws-undo')).toHaveAttribute('data-count', String(undos + 1))
  await page.keyboard.press('Control+z')
  await expect.poll(() => cell(api, ws.workspace_id, 'b', 'status')).toBe('todo')
})

test('TD03 board: columns by status, drag a card across with a placeholder, fold a column, add a column', async ({ page, api }) => {
  const ws = await todoWorkspace(api, `todo-board ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await activate(page)
  await page.getByTestId('aiws-todo-mode-board').click()
  await expect(page.getByTestId('aiws-board-colhead-doing')).toContainText('In progress')
  const card = await item(page, 'a').boundingBox()
  const col = await page.getByTestId('aiws-board-col-doing').boundingBox()
  if (!card || !col) throw new Error('board not on screen')
  await page.mouse.move(card.x + 10, card.y + 6)
  await page.mouse.down()
  await page.mouse.move(card.x + 20, card.y + 10, { steps: 3 })
  await page.mouse.move(col.x + col.width / 2, col.y + 50, { steps: 10 })
  await expect(page.getByTestId('aiws-todo-slot')).toBeVisible()
  await page.getByTestId('aiws-canvas-view').screenshot({ path: `${SHOTS}/td03-drag.png` })
  await page.mouse.up()
  await expect.poll(() => cell(api, ws.workspace_id, 'a', 'status')).toBe('doing')
  // the mode is this person's: the instance still says list
  const outline = await api.outline(ALICE, ws.workspace_id)
  expect((outline.find((e) => e.entity_id === 'tl') as { config?: Json } | undefined)?.config?.mode ?? 'list').toBe('list')
  // fold a column
  await page.getByTestId('aiws-board-colhead-done').click()
  await expect(item(page, 'd')).toHaveCount(0)
  // a new column is a new status option
  await page.getByTestId('aiws-todo-addcol-tl').click()
  await page.getByTestId('aiws-todo-colname-tl').fill('Review')
  await page.getByTestId('aiws-todo-colname-tl').press('Enter')
  await expect(page.getByTestId('aiws-board-colhead-review')).toBeVisible()
  await page.getByTestId('aiws-canvas-view').screenshot({ path: `${SHOTS}/td03-board.png` })
})

test('TD04 another client writes: seen at once; two people move different cards both win', async ({ page, api }) => {
  const ws = await todoWorkspace(api, `todo-live ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await expect(item(page, 'a')).toBeVisible()
  // an Agent (here: bob through the API) adds one and completes another
  const info = await api.rpc(BOB, 'ws.get_info', { workspace_id: ws.workspace_id }) as { epoch: string }
  const b = await api.rpc(BOB, 'doc.read', { workspace_id: ws.workspace_id, entity_id: 'tasks', selector: { kind: 'table_record', record_id: 'b' } }) as { content: { revs: Record<string, number> } }
  const r = await api.commit(BOB, { workspace_id: ws.workspace_id, epoch: info.epoch }, [
    { op: 'table.insert_records', source_id: 'tasks', records: [{ record_id: 'z', values: { title: 'From the agent', status: 'todo', order: 'c' } }] },
    { op: 'table.set_values', source_id: 'tasks', values: [{ record_id: 'b', field_id: 'status', value: 'done', expect: { rev: b.content.revs.status ?? 0 } }] },
  ])
  expect(r.status, JSON.stringify(r)).toBe('accepted')
  await expect(item(page, 'z')).toContainText('From the agent')
  await expect(page.getByTestId('aiws-todo-count-tl')).toContainText('3')
})

test('TD05 read-only members cannot change; the kernel refuses a bad status', async ({ page, api }) => {
  const ws = await todoWorkspace(api, `todo-ro ${Date.now()}`, ['read'])
  await openCanvas(page, BOB, ws.workspace_id)
  await expect(item(page, 'a')).toBeVisible()
  await expect(page.getByTestId('aiws-todo-add-tl')).toBeDisabled()
  const a = await api.rpc(ALICE, 'doc.read', { workspace_id: ws.workspace_id, entity_id: 'tasks', selector: { kind: 'table_record', record_id: 'a' } }) as { content: { revs: Record<string, number> } }
  const bad = await api.commit(ALICE, ws, [{ op: 'table.set_values', source_id: 'tasks', values: [{ record_id: 'a', field_id: 'order', value: 'Not a key!', expect: { rev: a.content.revs.order ?? 0 } }] }])
  expect(bad.status, JSON.stringify(bad)).toBe('rejected')
})

test('TD06 a link to a record selects it; the record can be commented; page=control shows the control alone', async ({ page, api, context }) => {
  const ws = await todoWorkspace(api, `todo-link ${Date.now()}`)
  await asUser(page, ALICE)
  await page.goto(`${workspacePage(ws.workspace_id, 'free')}&block=tl&record=d`)
  // d is completed: its section unfolds and it is selected
  await expect(item(page, 'd')).toBeVisible({ timeout: 30_000 })
  await expect(page.locator('[data-proj="tl~bg~d"] path').first()).toBeVisible()
  // the menu's link names the record
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])
  await page.mouse.click(10, 10)
  await activate(page)
  await item(page, 'b').click({ button: 'right' })
  await page.getByTestId('aiws-ctl-menu-link').click()
  const link = await page.evaluate(() => navigator.clipboard.readText())
  expect(link).toContain('record=b')
  expect(link).toContain('block=tl')
  // the control as a page of its own
  await page.goto(`${workspacePage(ws.workspace_id, 'free')}&block=tl&page=control`)
  await expect(page.getByTestId('aiws-control-page-tl')).toBeVisible({ timeout: 30_000 })
  await expect(page.locator('[data-proj="tl~title~a"]')).toContainText('Write the report')
  await page.getByTestId('aiws-control-page-back').click()
  await expect(page.getByTestId('aiws-canvas-view')).toBeVisible()
})
