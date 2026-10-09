import { expect, test, type Page } from '@playwright/test'

const UI = 'ui-7f3a2c91'
const WORK_RUNNING = 'work-b41d09e2'
const WORK_PENDING = 'work-5c8e17aa'

const open = async (page: Page, hash: string) => {
  const errors: string[] = []
  page.on('pageerror', (e) => errors.push(e.message))
  await page.goto(`/?data=mock#/${hash}`)
  await expect(page.getByTestId('data-source')).toHaveText('data: mock')
  return errors
}

test('sessions page lists the tree, the active view and filters', async ({ page }) => {
  const errors = await open(page, 'sessions')
  const sessions = page.getByTestId('sessions-panel')
  await expect(sessions.locator('tbody tr')).toHaveCount(5)
  const ids = await sessions.locator('tbody tr td:first-child a').allTextContents()
  expect(ids.indexOf(WORK_RUNNING)).toBeGreaterThan(ids.indexOf(UI))
  await expect(page.getByTestId(`session-row-${WORK_RUNNING}`)).toContainText('turn open')
  await expect(page.getByTestId(`session-row-${WORK_PENDING}`)).toContainText('pending')
  await expect(page.getByTestId(`session-row-${UI}`)).toContainText('loaded')
  await expect(page.getByTestId('active-panel').locator('tbody tr')).toHaveCount(2)
  await expect(page.getByTestId('active-panel')).toContainText('artifact:art-weekly-report')

  await page.getByLabel('run_state').selectOption('finished')
  await expect(sessions.locator('tbody tr')).toHaveCount(3)
  await page.getByLabel('kind').selectOption('self_check')
  await expect(sessions.locator('tbody tr')).toHaveCount(1)
  expect(errors).toEqual([])
})

test('session detail shows state, worklog and outbox', async ({ page }) => {
  const errors = await open(page, 'sessions')
  await page.getByTestId(`session-row-${WORK_RUNNING}`).getByRole('link', { name: WORK_RUNNING }).click()
  await expect(page.getByTestId('session-turn')).toContainText('task-55')
  await expect(page.getByTestId('session-process-stack')).toContainText('create_sub_context → work_do')
  await expect(page.getByTestId('session-pending-events')).toContainText('scanned 9/14 files')
  await expect(page.getByTestId('session-runtime')).toContainText('rt-tmux-07')
  await expect(page.getByTestId('session-behaviors')).toContainText('sha256:9a41c0de')
  await expect(page.getByTestId('session-report')).toContainText('Weekly downloads')
  await page.getByTestId('worklog-row').last().click()
  await expect(page.getByTestId('session-worklog').locator('pre')).toContainText('"t": "created"')

  await page.getByTestId('session-summary').getByRole('link', { name: UI }).click()
  const outbox = page.getByTestId('session-outbox')
  await expect(outbox.locator('tbody tr')).toHaveCount(3)
  await expect(outbox).toContainText('post_send timeout')
  await expect(page.getByTestId('session-summary').getByRole('link', { name: WORK_PENDING })).toBeVisible()
  expect(errors).toEqual([])
})

test('stop asks for confirmation and finishes the session', async ({ page }) => {
  await open(page, `session/${WORK_RUNNING}`)
  const ops = page.getByTestId('operations')
  await ops.getByRole('button', { name: 'Stop' }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByRole('button', { name: 'Cancel' }).click()
  await expect(dialog).toBeHidden()
  await expect(page.getByTestId('operation-result')).toHaveCount(0)

  await ops.getByRole('button', { name: 'Stop' }).click()
  await dialog.getByLabel('Reason (optional)').fill('wrong folder')
  await dialog.getByRole('button', { name: 'Stop session' }).click()
  await expect(page.getByTestId('operation-result')).toContainText('stop posted')
  await expect(page.getByTestId('session-summary')).toContainText('Stopped: wrong folder')
  await expect(ops.getByRole('button', { name: 'Stop' })).toHaveCount(0)
})

test('decide is offered only for a pending acceptance and needs confirmation', async ({ page }) => {
  await open(page, `session/${WORK_RUNNING}`)
  await expect(page.getByTestId('operations').getByRole('button', { name: 'Accept' })).toHaveCount(0)

  await page.goto(`/?data=mock#/session/${WORK_PENDING}`)
  const ops = page.getByTestId('operations')
  await expect(ops.getByRole('button', { name: 'Stop' })).toHaveCount(0)
  await expect(ops.getByLabel('Message')).toHaveCount(0)
  await ops.getByRole('button', { name: 'Accept' }).click()
  const dialog = page.getByRole('dialog')
  await expect(dialog).toContainText(WORK_PENDING)
  await dialog.getByRole('button', { name: 'Accept' }).click()
  await expect(page.getByTestId('operation-result')).toContainText('decide(accept) posted')
  await expect(page.getByText('acceptance: accepted')).toBeVisible()
  await expect(ops.getByRole('button', { name: 'Accept' })).toHaveCount(0)
})

test('post sends a message after confirmation', async ({ page }) => {
  await open(page, `session/${UI}`)
  const ops = page.getByTestId('operations')
  await expect(ops.getByRole('button', { name: 'Post' })).toBeDisabled()
  await ops.getByLabel('Message').fill('How far is the summary?')
  await ops.getByRole('button', { name: 'Post' }).click()
  const dialog = page.getByRole('dialog')
  await expect(dialog).toContainText('How far is the summary?')
  await dialog.getByRole('button', { name: 'Post message' }).click()
  await expect(page.getByTestId('operation-result')).toContainText('message posted at index')
  await expect(page.getByTestId('session-worklog')).toContainText('How far is the summary?')
  await expect(ops.getByLabel('Message')).toHaveValue('')
})

test('agent state page shows perception, artifacts, behaviors, identity and hints', async ({ page }) => {
  const errors = await open(page, 'agent')
  const perception = page.getByTestId('agent-perception')
  await expect(perception).toContainText('512 → 1388')
  await perception.getByRole('button', { name: 'Records' }).first().click()
  await expect(perception).toContainText('Renamed 212 photos')

  const artifacts = page.getByTestId('agent-artifacts')
  await artifacts.getByRole('button', { name: 'Versions' }).first().click()
  await expect(artifacts).toContainText('mv 212 files')

  const behaviors = page.getByTestId('agent-behaviors')
  await expect(behaviors).toContainText('sha256:9a41c0de')
  await behaviors.getByRole('button', { name: 'work_do' }).click()
  await expect(page.getByTestId('behavior-config')).toContainText('"loop": "function_call"')
  await expect(page.getByTestId('agent-identity')).toContainText('personal agent of devtest')

  const hints = page.getByTestId('agent-hints')
  await hints.getByLabel('Tags').fill('report')
  await hints.getByRole('button', { name: 'Recall' }).click()
  await expect(hints.locator('tbody tr')).toHaveCount(1)
  await expect(hints).toContainText('markdown')
  expect(errors).toEqual([])
})

test('loader page shows modules, hosted sessions, inbox bridge and errors', async ({ page }) => {
  const errors = await open(page, 'loader')
  await expect(page.getByTestId('loader-info')).toContainText('did:bns:jarvis.devtest')
  await expect(page.getByTestId('loader-modules').locator('tbody tr')).toHaveCount(5)
  await expect(page.getByTestId('loader-hosted').locator('tbody tr')).toHaveCount(3)
  await expect(page.getByTestId('loader-hosted')).toContainText('turn_open')
  await expect(page.getByTestId('loader-ui')).toContainText('session input queue is full')
  await expect(page.getByTestId('loader-errors').locator('tbody tr')).toHaveCount(2)
  await page.getByRole('button', { name: 'Refresh' }).click()
  await expect(page.getByTestId('loader-modules').locator('tbody tr')).toHaveCount(5)
  expect(errors).toEqual([])
})
