/* The private schedule (notepads/aiworkspace-control-mechanism-todo.md G9, G11, §5.1 1 / 7, §5.2 6): the Workspace named
 * `schedule` with its fixed tables `todo` / `calendar` and a page holding both controls, created on first use by the same
 * module the Agent commands use (buckyos-websdk ScheduleClient); concurrent first uses give one Workspace; what an
 * Agent writes shows in the open controls. */

import { ScheduleClient, type ScheduleTransport } from 'buckyos'
import { openCanvas, test, expect, type Api } from './fixtures'

test.use({ timezoneId: 'Asia/Shanghai' })

const transport = (api: Api, token: string): ScheduleTransport => ({
  call: async <T,>(method: string, params: Record<string, unknown>) => {
    const result = await api.rpc(token, method, params) as Record<string, unknown>
    if (result.ok === false) {
      const e = result.error as { code: string; detail: string; sub_code?: string }
      throw Object.assign(new Error(`${e.code}: ${e.detail}`), { code: e.code, subCode: e.sub_code })
    }
    return result as T
  },
})

test('SC01 first use creates the schedule once; to-dos and events through the Agent path show in the controls', async ({ page, api }) => {
  // a fresh principal for this test: the schedule is per owner
  const token = 'tok-alice'
  const clients = [new ScheduleClient(transport(api, token), { locale: 'en' }), new ScheduleClient(transport(api, token), { locale: 'en' })]
  const [a, b] = await Promise.all(clients.map((c) => c.ensure()))
  expect(a.workspace_id).toBe(b.workspace_id)
  expect([a.todo, a.calendar]).toEqual(['todo', 'calendar'])
  const listed = await api.rpc(token, 'ws.list', {}) as { workspaces: { workspace_id: string; name?: string }[] }
  expect(listed.workspaces.filter((w) => w.name === 'schedule')).toHaveLength(1)

  const s = clients[0]
  const milk = await s.addTodo({ title: `Buy milk ${Date.now()}` })
  const bank = await s.addTodo({ title: 'Call the bank', due: '2026-11-18' }, { before: milk.record_id })
  const all = await s.todos()
  expect(all.findIndex((i) => i.record_id === bank.record_id)).toBeLessThan(all.findIndex((i) => i.record_id === milk.record_id))
  await s.completeTodo(milk.record_id)
  expect((await s.todos()).some((i) => i.record_id === milk.record_id)).toBe(false)
  await s.moveTodo(bank.record_id, { status: 'doing' })
  expect((await s.todos()).find((i) => i.record_id === bank.record_id)?.status).toBe('doing')

  const standup = await s.addEvent({ title: 'Stand-up', start: '2026-11-16T01:00:00.000Z', end: '2026-11-16T01:15:00.000Z', ext: { rrule: 'FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR', tz: 'Asia/Shanghai' } })
  const week = { fromMs: Date.parse('2026-11-15T16:00:00Z'), toMs: Date.parse('2026-11-22T16:00:00Z') }
  expect((await s.events(week, { viewerTz: 'Asia/Shanghai' })).filter((o) => o.row.record_id === standup)).toHaveLength(5)
  // "today not needed": skip Wednesday's; "push Thursday's by 30 minutes"
  await s.deleteEvent(`${standup}@2026-11-18T01:00:00.000Z`, 'one')
  await s.editEvent(`${standup}@2026-11-19T01:00:00.000Z`, { times: { allDay: false, start: '2026-11-19T01:30:00.000Z', end: '2026-11-19T01:45:00.000Z' } }, 'one')
  const occ = (await s.events(week, { viewerTz: 'Asia/Shanghai' })).filter((o) => o.id.startsWith(`${standup}@`))
  expect(occ.map((o) => new Date(o.startMs!).toISOString())).toEqual(['2026-11-16T01:00:00.000Z', '2026-11-17T01:00:00.000Z', '2026-11-19T01:30:00.000Z', '2026-11-20T01:00:00.000Z'])

  // the page of the schedule shows both controls; an Agent's write appears while it is open
  await api.rpc(token, 'ws.set_user_state', { workspace_id: a.workspace_id, entries: { 'block:schedule-board-calendar/view': { view: 'week', date: '2026-11-16', week_start: 0 } } })
  await openCanvas(page, token, a.workspace_id)
  await expect(page.getByTestId('aiws-todo-schedule-board-todo')).toBeVisible()
  await expect(page.getByTestId('aiws-cal-schedule-board-calendar')).toBeVisible()
  await expect(page.locator(`[data-proj="schedule-board-todo~title~${bank.record_id}"]`)).toContainText('Call the bank')
  const late = await s.addTodo({ title: 'Added by the agent' })
  await expect(page.locator(`[data-proj="schedule-board-todo~title~${late.record_id}"]`)).toContainText('Added by the agent')
  // the to-do's due date is on the calendar
  await expect(page.locator(`[data-proj^="schedule-board-calendar~title~todo:${bank.record_id}"]`)).toContainText('Call the bank')
  await page.getByTestId('aiws-canvas-view').screenshot({ path: 'test-results/aiworkspace-calendar/sc01-schedule.png' })
})
