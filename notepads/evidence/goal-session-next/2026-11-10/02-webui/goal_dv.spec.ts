import { expect, test, type Page } from '@playwright/test'

const GOAL = process.env.GOAL_ID ?? ''
const BUILTIN = process.env.BUILTIN_GOAL_ID ?? ''
const OUT = process.env.EVIDENCE_DIR ?? 'test-results-zone'
const WIDTH = process.env.DV_WIDTH ? Number(process.env.DV_WIDTH) : 0

const signIn = async (page: Page) => {
  await page.locator('input[type="text"]').fill('devtest')
  await page.locator('input[type="password"]').fill('bucky2025')
  await page.getByRole('button', { name: 'Sign In', exact: true }).click()
}

const operate = async (page: Page, name: string, reason: string, shot: string) => {
  const ops = page.getByTestId('goal-operations')
  await ops.getByRole('button', { name, exact: true }).click()
  const dialog = page.getByRole('dialog')
  await dialog.locator('input.input').fill(reason)
  await dialog.getByRole('button', { name, exact: true }).click()
  await expect(dialog).toHaveCount(0)
  await expect(page.getByTestId('goal-operation-result')).toContainText('Done')
  const result = await page.getByTestId('goal-operation-result').innerText()
  await page.screenshot({ path: `${OUT}/${shot}.png`, fullPage: true })
  return result
}

test('goal status operations through the zone gateway', async ({ page }) => {
  test.setTimeout(180_000)
  if (WIDTH) await page.setViewportSize({ width: WIDTH, height: 844 })
  const errors: string[] = []
  const denied: string[] = []
  page.on('pageerror', (e) => errors.push(e.message))
  page.on('response', async (r) => {
    if (!r.url().endsWith('/api/opendan')) return
    const body = await r.text().catch(() => '')
    if (body.includes('No permission') || r.status() !== 200) denied.push(`${r.status()} ${r.request().postDataJSON()?.method}: ${body.slice(0, 160)}`)
  })
  await page.goto('/?lang=en')
  await expect(page).toHaveURL(/\/login\?/)
  await signIn(page)
  await expect(page.getByTestId('data-source')).toHaveText('data: krpc')
  await expect(page.getByTestId('goals')).toBeVisible()
  await page.screenshot({ path: `${OUT}/00-home-goals.png`, fullPage: true })

  await page.goto(`/?lang=en#/goal/${GOAL}`)
  const header = page.getByTestId('goal-header')
  const ops = page.getByTestId('goal-operations')
  await expect(header).toContainText('Active')
  const log: string[] = []
  log.push(`sleep: ${await operate(page, 'Sleep', 'dv sleep', '01-sleep')}`)
  await expect(ops.getByRole('button', { name: 'Wake', exact: true })).toBeVisible()
  log.push(`wake: ${await operate(page, 'Wake', 'dv wake', '02-wake')}`)
  await expect(ops.getByRole('button', { name: 'Sleep', exact: true })).toBeVisible()
  log.push(`restart: ${await operate(page, 'Restart', 'dv restart', '03-restart')}`)
  await expect(page.getByTestId('goal-definition').getByTestId('goal-epoch-2')).toContainText('restarted')
  log.push(`archive: ${await operate(page, 'Archive', 'dv archive', '04-archive')}`)
  await expect(ops.getByRole('button', { name: 'Reopen', exact: true })).toBeVisible()
  await expect(ops.getByRole('button', { name: 'Sleep', exact: true })).toHaveCount(0)
  log.push(`reopen: ${await operate(page, 'Reopen', 'dv reopen', '05-reopen')}`)
  await expect(header).toContainText('Active')
  await expect(page.getByTestId('goal-definition').getByTestId('goal-epoch-3')).toBeVisible()
  await expect(page.getByTestId('goal-changes').getByTestId('change-row').first()).toBeVisible()

  await page.goto(`/?lang=en#/goal/${BUILTIN}`)
  await expect(page.getByTestId('goal-builtin-info')).toBeVisible()
  const archive = page.getByTestId('goal-operations').getByRole('button', { name: 'Archive', exact: true })
  await expect(archive).toBeDisabled()
  await expect(page.getByTestId('goal-operations')).toContainText('The built-in goal is not archived')
  await page.screenshot({ path: `${OUT}/06-builtin-no-archive.png`, fullPage: true })
  log.push('builtin: Archive is disabled')
  console.log(log.join('\n'))
  expect(denied).toEqual([])
  expect(errors).toEqual([])
})
