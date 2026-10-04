import { expect, test } from '@playwright/test'

test('built UI loads from opendan and every page reads the kRPC service', async ({ page }) => {
  const errors: string[] = []
  const calls: string[] = []
  page.on('pageerror', (e) => errors.push(e.message))
  page.on('request', (r) => {
    if (r.url().endsWith('/kapi/opendan') && r.method() === 'POST') calls.push(r.postDataJSON().method)
  })

  await page.goto('/')
  await expect(page.getByTestId('data-source')).toHaveText('data: krpc')
  await expect(page.getByTestId('agent-card')).toBeVisible()
  await page.getByRole('link', { name: 'Advanced' }).click()
  await expect(page.getByTestId('sessions-panel')).toBeVisible()
  await expect(page.getByTestId('active-panel')).toBeVisible()

  await page.getByRole('link', { name: 'Agent State' }).click()
  for (const id of ['agent-perception', 'agent-artifacts', 'agent-behaviors', 'agent-identity', 'agent-hints']) {
    await expect(page.getByTestId(id)).toBeVisible()
  }
  await page.getByTestId('agent-hints').getByRole('button', { name: 'Recall' }).click()

  await page.getByRole('link', { name: 'Loader' }).click()
  await expect(page.getByTestId('loader-modules').locator('tbody tr').first()).toBeVisible()
  await expect(page.getByTestId('loader-info')).toContainText('did:')

  await page.goto('/#/session/no-such-session')
  await expect(page.getByRole('alert')).toContainText('not_found')

  await expect.poll(() => new Set(calls)).toEqual(
    new Set([
      'agent.profile', 'usage.models', 'ui.bindings', 'loader.status', 'sessions.query', 'activity.active', 'perception.cursor', 'perception.backlog', 'artifacts.list',
      'behaviors.revision', 'behaviors.list', 'behaviors.identity', 'cognition.recall_hints', 'session.read',
    ]),
  )
  await expect(page.getByRole('alert')).toHaveCount(1)
  expect(errors).toEqual([])
})
