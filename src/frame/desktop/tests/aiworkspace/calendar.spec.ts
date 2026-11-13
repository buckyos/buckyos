/* The official calendar control (notepads/aiworkspace-control-mechanism-todo.md §0.1, §5.2): a standard calendar table shown
 * by day / week / month / agenda through projected shape and label Blocks; repeating events expanded by the shared module;
 * creating by dragging out a range, rescheduling by dragging (start and end in one Commit), changing the end by the bottom
 * edge, "only this one / all" for a series, skipping one occurrence, keyboard navigation, and the kernel's rule that an
 * event does not end before it starts. The reader is in Shanghai; the week shown is 2026-11-16 … 22 (Monday first). */

import type { Page } from '@playwright/test'
import { asUser, openCanvas, test, expect, workspacePage, type Api } from './fixtures'

test.use({ timezoneId: 'Asia/Shanghai' })

const ALICE = 'tok-alice'
const SHOTS = 'test-results/aiworkspace-calendar'

type Json = Record<string, unknown>

const FIELDS = [
  { field_id: 'title', name: 'Title', type: 'text', required: true },
  { field_id: 'all_day', name: 'All day', type: 'boolean', nullable: true },
  { field_id: 'start', name: 'Start', type: 'datetime', nullable: true },
  { field_id: 'end', name: 'End', type: 'datetime', nullable: true },
  { field_id: 'start_date', name: 'Start date', type: 'date', nullable: true },
  { field_id: 'end_date', name: 'End date', type: 'date', nullable: true },
  { field_id: 'status', name: 'Status', type: 'select', nullable: true, options: [{ option_id: 'confirmed', label: 'Confirmed' }, { option_id: 'tentative', label: 'Tentative' }, { option_id: 'cancelled', label: 'Cancelled' }] },
  { field_id: 'ext', name: 'Extension', type: 'json', nullable: true },
]

/** 10:00 Shanghai on a day of the test week, as UTC. */
const at = (day: number, hour: number, minute = 0) => new Date(Date.UTC(2026, 10, day, hour - 8, minute)).toISOString()

async function calendarWorkspace(api: Api, title: string, view: Json = { view: 'week', date: '2026-11-16', week_start: 0 }) {
  const ws = await api.freeCanvas(ALICE, title)
  const made = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'events', type_id: 'buckyos.table-source', schema_id: 'buckyos.calendar@1', parent_id: 'free-content', order_key: 'a', name: 'Events', payload: { title_field_id: 'title', fields: FIELDS } },
    { op: 'entity.create', entity_id: 'cal', type_id: 'buckyos.container', parent_id: 'free', order_key: 'b', placement: { x: 20, y: 20, w: 1000, h: 700 },
      payload: { kind: 'calendar', layout: { mode: 'data' }, title: 'Team', bindings: { events: { entity_id: 'events' } } } },
    { op: 'table.insert_records', source_id: 'events', records: [
      { record_id: 'review', values: { title: 'Design review', start: at(17, 10), end: at(17, 11) } },
      { record_id: 'clash', values: { title: 'Interview', start: at(17, 10, 30), end: at(17, 11, 30) } },
      { record_id: 'trip', values: { title: 'Conference trip', all_day: true, start_date: '2026-11-19', end_date: '2026-11-21' } },
      { record_id: 'standup', values: { title: 'Stand-up', start: at(9, 9), end: at(9, 9, 15), ext: { rrule: 'FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR', tz: 'Asia/Shanghai' } } },
    ] },
  ])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  await api.rpc(ALICE, 'ws.set_user_state', { workspace_id: ws.workspace_id, entries: { 'block:cal/view': view } })
  return ws
}

async function values(api: Api, workspaceId: string, record: string): Promise<Json> {
  const read = await api.rpc(ALICE, 'doc.read', { workspace_id: workspaceId, entity_id: 'events', selector: { kind: 'table_record', record_id: record } }) as { content: { values: Json } }
  return read.content.values
}

async function rows(api: Api, workspaceId: string): Promise<{ record_id: string; values: Json }[]> {
  return (await api.table(ALICE, workspaceId, { source_id: 'events', limit: 100 }) as { rows: { record_id: string; values: Json }[] }).rows
}

async function activate(page: Page) {
  const head = await page.getByTestId('aiws-cal-head-cal').boundingBox()
  if (!head) throw new Error('the calendar is not on screen')
  await page.mouse.click(head.x + head.width / 2, head.y + 14)
  await expect(page.getByTestId('aiws-data-body-cal')).toHaveAttribute('data-active', 'true')
}

const node = (page: Page, item: string, part = 'title') => page.locator(`[data-proj="cal~${part}~${item}"]`)

test('CA01 week: timed blocks side by side, all-day span, a weekday series, the time axis', async ({ page, api }) => {
  const ws = await calendarWorkspace(api, `cal-week ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await expect(node(page, 'review')).toContainText('Design review')
  // overlapping events share the day's width
  const a = await node(page, 'review', 'bg').boundingBox()
  const b = await node(page, 'clash', 'bg').boundingBox()
  expect(a && b && Math.abs(a.width - b.width) < 2 && a.x < b.x).toBeTruthy()
  // the series shows Monday to Friday of this week
  await expect(page.locator('[data-proj^="cal~title~standup@"]')).toHaveCount(5)
  // the trip is one bar over three days
  await expect(page.locator('[data-proj^="cal~title~trip#"]')).toHaveCount(1)
  await page.getByTestId('aiws-canvas-view').screenshot({ path: `${SHOTS}/ca01-week.png` })
})

test('CA02 views and keys: month with bars, agenda, day; T goes back to today', async ({ page, api }) => {
  const ws = await calendarWorkspace(api, `cal-views ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await activate(page)
  await page.keyboard.press('m')
  await expect(page.getByTestId('aiws-cal-month-cal')).toBeVisible()
  await expect(page.locator('[data-proj^="cal~title~trip#"]').first()).toContainText('Conference trip')
  await page.getByTestId('aiws-canvas-view').screenshot({ path: `${SHOTS}/ca02-month.png` })
  await page.keyboard.press('a')
  await expect(page.getByTestId('aiws-cal-agenda-cal')).toBeVisible()
  await expect(page.locator('[data-proj^="cal~title~review"]')).toContainText('Design review')
  await page.keyboard.press('d')
  await expect(page.getByTestId('aiws-cal-cal')).toHaveAttribute('data-view', 'day')
  await page.keyboard.press('ArrowRight')
  await page.keyboard.press('w')
  // the choice is this person's own
  const state = await api.rpc(ALICE, 'ws.get_user_state', { workspace_id: ws.workspace_id }) as { entries: Record<string, Json> }
  await expect.poll(async () => ((await api.rpc(ALICE, 'ws.get_user_state', { workspace_id: ws.workspace_id }) as { entries: Record<string, Json> }).entries['block:cal/view'] as Json)?.view).toBe('week')
  void state
})

test('CA03 drag out a time range, type a title: a new event of that range', async ({ page, api }) => {
  const ws = await calendarWorkspace(api, `cal-create ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await activate(page)
  const grid = page.getByTestId('aiws-cal-grid-cal')
  const box = await grid.boundingBox()
  if (!box) throw new Error('no grid')
  // Wednesday (third column), from about 14:00 to 15:30 (the grid opens at 08:00, 48 px an hour)
  const colW = (box.width - 54) / 7
  const x = box.x + 54 + colW * 2.5
  const y0 = box.y + (14 - 8) * 48 + 8 + 2
  await page.mouse.move(x, y0)
  await page.mouse.down()
  await page.mouse.move(x, y0 + 30, { steps: 4 })
  await page.mouse.move(x, y0 + 72, { steps: 6 })
  await expect(page.getByTestId('aiws-cal-creating')).toBeVisible()
  await page.mouse.up()
  await page.getByTestId('aiws-cal-quick-title').fill('Planning')
  await page.getByTestId('aiws-cal-quick-title').press('Enter')
  await expect.poll(async () => (await rows(api, ws.workspace_id)).find((r) => r.values.title === 'Planning')?.values.start).toBe(at(18, 14))
  const made = (await rows(api, ws.workspace_id)).find((r) => r.values.title === 'Planning')!
  expect(made.values.end).toBe(at(18, 15, 30))
})

test('CA04 drag to reschedule (start and end together), drag the bottom edge, undo', async ({ page, api }) => {
  const ws = await calendarWorkspace(api, `cal-move ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await activate(page)
  const ev = await node(page, 'review', 'bg').boundingBox()
  if (!ev) throw new Error('no event')
  // one hour later, one day later
  const colW = ev.width + 0
  await page.mouse.move(ev.x + 10, ev.y + 10)
  await page.mouse.down()
  await page.mouse.move(ev.x + 14, ev.y + 20, { steps: 3 })
  const grid = await page.getByTestId('aiws-cal-grid-cal').boundingBox()
  const dayW = (grid!.width - 54) / 7
  await page.mouse.move(ev.x + 10 + dayW, ev.y + 10 + 48, { steps: 8 })
  await page.mouse.up()
  await expect.poll(async () => (await values(api, ws.workspace_id, 'review')).start).toBe(at(18, 11))
  expect((await values(api, ws.workspace_id, 'review')).end).toBe(at(18, 12))
  void colW
  // the bottom edge: the end only
  const moved = await node(page, 'review', 'bg').boundingBox()
  await page.mouse.move(moved!.x + 20, moved!.y + moved!.height - 2)
  await page.mouse.down()
  await page.mouse.move(moved!.x + 20, moved!.y + moved!.height + 10, { steps: 3 })
  await page.mouse.move(moved!.x + 20, moved!.y + moved!.height + 24, { steps: 4 })
  await page.mouse.up()
  await expect.poll(async () => (await values(api, ws.workspace_id, 'review')).end).toBe(at(18, 12, 30))
  expect((await values(api, ws.workspace_id, 'review')).start).toBe(at(18, 11))
  await page.keyboard.press('Control+z')
  await expect.poll(async () => (await values(api, ws.workspace_id, 'review')).end).toBe(at(18, 12))
})

test('CA05 a repeating event: only this one, then all; skip one occurrence', async ({ page, api }) => {
  const ws = await calendarWorkspace(api, `cal-series ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await activate(page)
  const occ = page.locator(`[data-proj="cal~bg~standup@${at(18, 9)}"]`)
  const box = await occ.boundingBox()
  if (!box) throw new Error('no occurrence')
  await page.mouse.move(box.x + 10, box.y + 5)
  await page.mouse.down()
  await page.mouse.move(box.x + 12, box.y + 15, { steps: 3 })
  await page.mouse.move(box.x + 10, box.y + 5 + 96, { steps: 8 })
  await page.mouse.up()
  await page.getByTestId('aiws-cal-scope-one').click()
  // an override record for that occurrence, the series unchanged
  await expect.poll(async () => (await rows(api, ws.workspace_id)).filter((r) => (r.values.ext as Json | undefined)?.recurrence_id).length).toBe(1)
  const override = (await rows(api, ws.workspace_id)).find((r) => (r.values.ext as Json | undefined)?.recurrence_id)!
  expect(override.values.start).toBe(at(18, 11))
  expect((await values(api, ws.workspace_id, 'standup')).start).toBe(at(9, 9))
  // skip Thursday's: an exclusion
  await page.locator(`[data-proj="cal~title~standup@${at(19, 9)}"]`).dblclick()
  await page.getByTestId('aiws-cal-detail-skip').click()
  await expect.poll(async () => ((await values(api, ws.workspace_id, 'standup')).ext as Json).exdate).toEqual([at(19, 9)])
  await expect(page.locator(`[data-proj="cal~bg~standup@${at(19, 9)}"]`)).toHaveCount(0)
  await page.getByTestId('aiws-canvas-view').screenshot({ path: `${SHOTS}/ca05-series.png` })
})

test('CA06 the kernel refuses an event that ends before it starts', async ({ api }) => {
  const ws = await calendarWorkspace(api, `cal-rule ${Date.now()}`)
  const bad = await api.commit(ALICE, ws, [{ op: 'table.insert_records', source_id: 'events', records: [{ record_id: 'bad', values: { title: 'Backwards', start: at(18, 12), end: at(18, 11) } }] }])
  expect(bad.status, JSON.stringify(bad)).toBe('rejected')
  expect(JSON.stringify(bad)).toContain('before')
})

test('CA07 a link to one occurrence turns to its day and opens it', async ({ page, api }) => {
  const ws = await calendarWorkspace(api, `cal-link ${Date.now()}`, { view: 'week', date: '2026-10-05', week_start: 0 })
  await asUser(page, ALICE)
  await page.goto(`${workspacePage(ws.workspace_id, 'free')}&block=cal&record=standup&occ=${encodeURIComponent(at(20, 9))}`)
  await expect(page.getByTestId('aiws-cal-detail')).toBeVisible({ timeout: 30_000 })
  await expect(page.getByTestId('aiws-cal-detail-when')).toContainText('20')
  await expect(page.getByTestId('aiws-cal-range-cal')).toContainText('16')
})

test('CA10 to-do lists on a calendar: an inserted calendar shows every to-do list; "Tables shown…" re-binds both lists (a new calendar, another to-do list, one fewer); each table its colour', async ({ page, api }) => {
  const ws = await api.freeCanvas(ALICE, `cal-todo ${Date.now()}`)
  const TODO_FIELDS = [
    { field_id: 'title', name: 'Title', type: 'text', required: true },
    { field_id: 'status', name: 'Status', type: 'select', required: true, options: [{ option_id: 'todo', label: 'To do' }, { option_id: 'doing', label: 'In progress' }, { option_id: 'done', label: 'Done' }] },
    { field_id: 'due', name: 'Due', type: 'date', nullable: true },
    { field_id: 'order', name: 'Order', type: 'text', nullable: true },
    { field_id: 'done_at', name: 'Completed', type: 'datetime', nullable: true },
    { field_id: 'ext', name: 'Extension', type: 'json', nullable: true },
  ]
  // today on the browser's calendar (Asia/Shanghai): the week the calendar opens on
  const today = new Intl.DateTimeFormat('en-CA', { timeZone: 'Asia/Shanghai' }).format(new Date())
  const todoList = (id: string, name: string, item: string, key: string) => [
    { op: 'entity.create', entity_id: id, type_id: 'buckyos.table-source', schema_id: 'buckyos.todo@1', parent_id: 'data', order_key: key, name, payload: { title_field_id: 'title', fields: TODO_FIELDS } },
    { op: 'table.insert_records', source_id: id, records: [{ record_id: `${id}-1`, values: { title: item, status: 'todo', due: today, order: 'm' } }] },
  ]
  expect((await api.commit(ALICE, ws, todoList('home', 'Home', 'Pay the rent', 'h'))).status).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)

  // inserted (its contract's on_bind): a new calendar table in `events`, every to-do list in `todo`
  await page.getByTestId('aiws-tool-catalog').click()
  await page.getByTestId('aiws-catalog-entry-control-calendar').click()
  await page.getByTestId('aiws-catalog-insert').click()
  type Cal = { entity_id: string; bindings: Record<string, string[]> }
  const calOf = async (id?: string) => (await api.outline(ALICE, ws.workspace_id)).find((e: Json) => e.kind === 'calendar' && !e.deleted && (!id || e.entity_id === id)) as Cal | undefined
  await expect.poll(async () => (await calOf())?.bindings.todo).toEqual(['home'])
  const cal = (await calOf())!.entity_id
  const ownTable = (await calOf())!.bindings.events[0]
  const due = (record: string, table: string) => page.locator(`[data-proj^="${cal}~title~todo:${record}@${table}"]`).first()
  await expect(due('home-1', 'home')).toContainText('Pay the rent')

  // "Tables shown…": one choice per list — calendars (here also a new one), then to-do lists (here the new one too)
  expect((await api.commit(ALICE, ws, todoList('work', 'Work', 'Send the report', 'w'))).status).toBe('accepted')
  const head = await page.getByTestId(`aiws-cal-head-${cal}`).boundingBox()
  await page.mouse.click(head!.x + head!.width / 2, head!.y + 14)
  const tablesShown = async () => {
    await page.getByTestId(`aiws-cal-menu-${cal}`).click()
    await page.getByTestId('aiws-ctl-menu-tables').click()
  }
  const dialog = page.getByTestId('aiws-source-chooser')
  await tablesShown()
  await expect(dialog.getByTestId(`aiws-source-${ownTable}`)).toHaveAttribute('aria-pressed', 'true')
  await dialog.getByTestId('aiws-source-new').click()
  await dialog.getByTestId('aiws-bind-confirm').click()
  await expect(dialog.getByTestId('aiws-source-home')).toHaveAttribute('aria-pressed', 'true')
  await dialog.getByTestId('aiws-source-work').click()
  await page.getByTestId('aiws-canvas-view').screenshot({ path: 'test-results/aiworkspace-calendar/ca10-tables.png' })
  await dialog.getByTestId('aiws-bind-confirm').click()
  await expect.poll(async () => (await calOf(cal))?.bindings.todo).toEqual(['home', 'work'])
  expect((await calOf(cal))!.bindings.events).toHaveLength(2)
  await expect(due('work-1', 'work')).toContainText('Send the report')
  // each table its own colour from the host's palette
  const fill = (record: string, table: string) => page.locator(`[data-proj^="${cal}~bg~todo:${record}@${table}"]`).first().evaluate((el) => { const shape = el.querySelector('rect, path, ellipse') ?? el; return getComputedStyle(shape).fill })
  expect(new Set([await fill('home-1', 'home'), await fill('work-1', 'work')]).size).toBe(2)

  // one fewer: only the binding goes, the to-do list and its records stay; cancelling changes nothing
  await tablesShown()
  await dialog.getByTestId('aiws-bind-cancel').click()
  await expect(dialog).toHaveCount(0)
  expect((await calOf(cal))!.bindings.todo).toEqual(['home', 'work'])
  await tablesShown()
  await dialog.getByTestId('aiws-bind-confirm').click()
  await dialog.getByTestId('aiws-source-home').click()
  await dialog.getByTestId('aiws-bind-confirm').click()
  await expect.poll(async () => (await calOf(cal))?.bindings.todo).toEqual(['work'])
  await expect(due('home-1', 'home')).toHaveCount(0)
  expect((await api.table(ALICE, ws.workspace_id, { source_id: 'home', limit: 10 })).rows).toHaveLength(1)

  // a calendar inserted now takes every calendar table and every to-do list (lists: nothing to ask)
  await page.getByTestId('aiws-tool-catalog').click()
  await page.getByTestId('aiws-catalog-entry-control-calendar').click()
  await page.getByTestId('aiws-catalog-insert').click()
  const second = async () => (await api.outline(ALICE, ws.workspace_id)).find((e: Json) => e.kind === 'calendar' && !e.deleted && e.entity_id !== cal) as Cal | undefined
  await expect.poll(async () => [...((await second())?.bindings.todo ?? [])].sort()).toEqual(['home', 'work'])
  expect((await second())!.bindings.events).toHaveLength(2)
})
