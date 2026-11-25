import { expect, test, type Page } from '@playwright/test'

const status = { agent_id: 'jarvis.test', agent_did: 'did:web:jarvis.test', who: 'app:jarvis.test@devtest', agent_root: '/agent', started_at_ms: 1, modules: [], hosted: [], errors: [] }

const serve = async (page: Page) => {
  const service = { calls: 0, down: false }
  await page.route('**/api/opendan', (route) => {
    if (route.request().method() === 'GET') return route.fulfill({ json: { app_id: null } })
    if (service.down) return route.abort('connectionrefused')
    service.calls++
    return route.fulfill({ json: { sys: [route.request().postDataJSON().sys[0]], result: status } })
  })
  return service
}

test('polling goes on while the system reports no network', async ({ page }) => {
  await page.addInitScript(() => Object.defineProperty(navigator, 'onLine', { get: () => false }))
  const service = await serve(page)
  await page.goto('/?data=krpc&lang=en#/loader')
  await expect(page.getByTestId('loader-info')).toContainText('jarvis.test')
  await page.evaluate(() => window.dispatchEvent(new Event('offline')))
  const before = service.calls
  await expect.poll(() => service.calls, { timeout: 15_000 }).toBeGreaterThanOrEqual(before + 2)
})

test('polling resumes by itself once the service answers again', async ({ page }) => {
  const service = await serve(page)
  await page.goto('/?data=krpc&lang=en#/loader')
  await expect(page.getByTestId('loader-info')).toContainText('jarvis.test')
  service.down = true
  await expect(page.getByRole('alert').first()).toContainText('transport', { timeout: 10_000 })
  service.down = false
  await expect(page.getByRole('alert')).toHaveCount(0, { timeout: 10_000 })
  const before = service.calls
  await expect.poll(() => service.calls, { timeout: 15_000 }).toBeGreaterThanOrEqual(before + 2)
})
