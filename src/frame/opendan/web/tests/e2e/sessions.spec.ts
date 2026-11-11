import { expect, test, type Page } from '@playwright/test'

const UI = 'ui-7f3a2c91'
const WORK_RUNNING = 'work-b41d09e2'
const WORK_PENDING = 'work-5c8e17aa'
const WORK_FAILED = 'work-e02b66f4'
const CHECK = 'work-20261101-4'
const GOAL_SESSION = 'goal-downloads-tidy-e2'

const open = async (page: Page, hash = '', query = '') => {
  const errors: string[] = []
  page.on('pageerror', (e) => errors.push(e.message))
  await page.goto(`/?data=mock&lang=en${query}${hash}`)
  await expect(page.getByTestId('data-source')).toHaveText('data: mock')
  return errors
}

test('what Stop means follows the kind of session', async ({ page }) => {
  const errors = await open(page, `#/session/${UI}`)
  const ops = page.getByTestId('operations')
  // A conversation is never stopped, only the message it is handling; none is being handled now.
  await expect(ops.getByRole('button', { name: 'Stop current message' })).toBeDisabled()
  await expect(ops.getByRole('button', { name: 'Stop', exact: true })).toHaveCount(0)
  await expect(ops.getByTestId('session-delete')).toHaveCount(0)

  await page.goto(`/?data=mock&lang=en#/session/${WORK_RUNNING}`)
  await expect(ops.getByRole('button', { name: 'Stop', exact: true })).toBeEnabled()
  await expect(ops.getByRole('button', { name: 'Stop current message' })).toHaveCount(0)
  await expect(ops.getByTestId('session-delete')).toHaveCount(0)

  await page.goto(`/?data=mock&lang=en#/session/${GOAL_SESSION}`)
  await expect(ops.getByRole('button', { name: 'Stop', exact: true })).toHaveCount(0)
  await ops.getByRole('button', { name: 'Sleep' }).click()
  const dialog = page.getByRole('dialog')
  await expect(dialog).toContainText('A dormant goal is not activated')
  await dialog.getByRole('button', { name: 'Sleep' }).click()
  await expect(page.getByTestId('operation-result')).toContainText('Dormant')
  await expect(ops.getByRole('button', { name: 'Dormant' })).toBeDisabled()
  await page.goto('/?data=mock&lang=en#/goal/goal-downloads-tidy')
  await expect(page.getByTestId('goal-header')).toContainText('Dormant')
  expect(errors).toEqual([])
})

test('only a finished work whose sub sessions are finished is deleted', async ({ page }) => {
  const errors = await open(page, `#/session/${CHECK}`)
  const ops = page.getByTestId('operations')
  await expect(ops.getByTestId('session-delete')).toBeVisible()
  await page.goto(`/?data=mock&lang=en#/session/${WORK_FAILED}`)
  await expect(ops.getByTestId('session-delete')).toBeVisible()
  await page.goto(`/?data=mock&lang=en#/session/${WORK_PENDING}`)
  await ops.getByTestId('session-delete').click()
  const dialog = page.getByRole('dialog')
  await expect(dialog).toContainText(`Delete session ${WORK_PENDING}?`)
  await expect(dialog).toContainText('Usage statistics shrink accordingly')
  await dialog.getByRole('button', { name: 'Cancel' }).click()
  await ops.getByTestId('session-delete').click()
  await dialog.getByRole('button', { name: 'Delete' }).click()

  await expect(page.getByTestId('agent-card')).toBeVisible()
  const list = page.getByTestId('session-list')
  await list.getByTestId(`home-session-${UI}`).getByRole('button', { name: /Show sub sessions \(1\)/ }).click()
  await expect(list.getByTestId(`home-session-${WORK_RUNNING}`)).toBeVisible()
  await expect(list.getByTestId(`home-session-${WORK_PENDING}`)).toHaveCount(0)
  await expect(page.getByTestId('stat-total')).toContainText('4')
  expect(errors).toEqual([])
})

test('sessions a restart interrupted are shown on the home page and resumed by hand', async ({ page }) => {
  const errors = await open(page, '', '&scene=restart')
  const banner = page.getByTestId('interrupted')
  await expect(banner).toContainText('5 sessions were interrupted by a restart')
  // Only the topmost interrupted sessions are listed: resuming one resumes those interrupted below it.
  await expect(banner.locator('li')).toHaveCount(4)
  await expect(banner.getByTestId('interrupted-work-todo-a1')).toHaveCount(0)
  // A paused session is not at work.
  await expect(page.getByTestId('stat-working')).toContainText('0')
  const list = page.getByTestId('session-list')
  await expect(list.getByTestId('home-session-work-weekly')).toContainText('Interrupted')
  await list.getByTestId(`home-session-${UI}`).getByRole('button', { name: /Show sub sessions/ }).click()
  await expect(list.getByTestId('home-session-work-plan-77')).toContainText('Interrupted')
  await expect(list.getByTestId('home-session-work-user-paused')).toContainText('Paused')
  await expect(list.getByTestId('home-session-work-goal-child')).toContainText('Paused with its goal')

  await banner.getByTestId('interrupted-work-plan-77').getByRole('button', { name: 'Resume' }).click()
  await expect(banner).toContainText('3 sessions were interrupted')
  await list.getByTestId('home-session-work-plan-77').getByRole('button', { name: /Show sub sessions/ }).click()
  await expect(list.getByTestId('home-session-work-todo-a1')).toContainText('Working')

  // An interrupted session may be given up instead.
  await banner.getByTestId('interrupted-work-weekly').getByRole('button', { name: 'Stop' }).click()
  const dialog = page.getByTestId('interrupted-stop')
  await expect(dialog).toContainText('What was already done is not undone.')
  await dialog.getByRole('button', { name: 'Stop', exact: true }).click()
  await expect(dialog).toHaveCount(0)
  await expect(list.getByTestId('home-session-work-weekly')).toContainText('Stopped')

  await banner.getByTestId('resume-all').click()
  await expect(banner).toHaveCount(0)
  await expect(list.getByTestId(`home-session-${WORK_RUNNING}`)).toContainText('Working')
  // What the owner paused, and what sleeps with its goal, is not touched by "resume all".
  await expect(list.getByTestId('home-session-work-user-paused')).toContainText('Paused')
  await expect(list.getByTestId('home-session-work-goal-child')).toContainText('Paused with its goal')
  expect(errors).toEqual([])
})

test('a paused session says what paused it and offers what fits', async ({ page }) => {
  await open(page, '#/session/ui-interrupted', '&scene=restart')
  const ops = page.getByTestId('operations')
  await expect(ops.getByTestId('session-paused')).toContainText('Interrupted · The agent host restarted while it was working')
  await expect(ops.getByRole('button', { name: 'Stop current message' })).toBeEnabled()
  await ops.getByRole('button', { name: 'Resume' }).click()
  await page.getByRole('dialog').getByRole('button', { name: 'Resume' }).click()
  await expect(page.getByTestId('operation-result')).toContainText('resume posted')
  await expect(ops.getByTestId('session-paused')).toHaveCount(0)

  await page.goto('/?data=mock&lang=en&scene=restart#/session/work-goal-child')
  await expect(ops.getByTestId('session-paused')).toContainText('Its goal is dormant')
  await expect(ops.getByTestId('session-paused').getByRole('link')).toHaveAttribute('href', '#/goal/goal-downloads-tidy')
  await expect(ops.getByRole('button', { name: 'Resume' })).toHaveCount(0)

  await page.goto('/?data=mock&lang=en&scene=restart#/session/work-user-paused')
  await expect(ops.getByTestId('session-paused')).toContainText('Paused by the owner')
  await expect(ops.getByRole('button', { name: 'Resume' })).toBeVisible()
})
