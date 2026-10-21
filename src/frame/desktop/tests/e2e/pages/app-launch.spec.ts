import { test, expect, type Page } from '@playwright/test'
import { MOCK_APP_LAUNCH_KEY, openFromAllApps, routeAppHost } from '../fixtures/desktop'

const COUNT_KEY = 'buckyos.mock.app-launch.count'

async function openSystest(page: Page, scenario?: string) {
  if (scenario) await page.addInitScript(([key, value]) => localStorage.setItem(key, value), [MOCK_APP_LAUNCH_KEY, scenario])
  const callbacks = await routeAppHost(page, 'http://systest.localhost:4173/**', '<main data-testid="systest-fixture">Systest fixture</main>')
  await page.goto('/?scenario=normal')
  await openFromAllApps(page, 'systest')
  await expect(page.getByTestId('window-systest')).toBeVisible()
  return callbacks
}

const launches = (page: Page) => page.evaluate((key) => Number(sessionStorage.getItem(key) ?? 0), COUNT_KEY)
const fixture = (page: Page) => page.frameLocator('[data-testid="systest-frame"]').getByTestId('systest-fixture')

test('a launch is prepared once and survives focus changes (StrictMode dev build)', async ({ page }) => {
  const callbacks = await openSystest(page)
  await expect(fixture(page)).toHaveText('Systest fixture')
  expect(await launches(page)).toBe(1)
  expect(callbacks).toHaveLength(1)
  await openFromAllApps(page, 'settings')
  await page.getByTestId('window-systest').click({ position: { x: 10, y: 10 } })
  await expect(fixture(page)).toHaveText('Systest fixture')
  expect(await launches(page)).toBe(1)
  expect(callbacks).toHaveLength(1)
})

test('access denied is shown without retry or sign-in', async ({ page }) => {
  const callbacks = await openSystest(page, 'access_denied')
  const error = page.getByTestId('window-systest').getByTestId('app-launch-error')
  await expect(error).toHaveAttribute('data-launch-status', 'access_denied')
  await expect(error).toContainText('You do not have access to')
  await expect(error.getByRole('button', { name: 'Retry' })).toHaveCount(0)
  await expect(page.getByTestId('systest-frame')).toHaveCount(0)
  expect(callbacks).toHaveLength(0)
  expect(new URL(page.url()).pathname).toBe('/')
})

test('a transient failure offers retry and the retry opens the App', async ({ page }) => {
  await openSystest(page, 'error')
  const error = page.getByTestId('window-systest').getByTestId('app-launch-error')
  await expect(error).toHaveAttribute('data-launch-status', 'failed')
  await page.evaluate((key) => localStorage.setItem(key, 'ready'), MOCK_APP_LAUNCH_KEY)
  await error.getByRole('button', { name: 'Retry' }).click()
  await expect(fixture(page)).toHaveText('Systest fixture')
  expect(new URL(page.url()).pathname).toBe('/')
})

test('an inactive desktop session is refreshed once and the launch retried', async ({ page }) => {
  const callbacks = await openSystest(page, 'inactive_once')
  await expect(fixture(page)).toHaveText('Systest fixture')
  expect(await launches(page)).toBe(2)
  expect(callbacks).toHaveLength(1)
})

test('a session that stays inactive after a refresh shows retry instead of looping to sign-in', async ({ page }) => {
  await openSystest(page, 'inactive')
  const error = page.getByTestId('window-systest').getByTestId('app-launch-error')
  await expect(error).toHaveAttribute('data-launch-status', 'session_inactive')
  await expect(error.getByRole('button', { name: 'Retry' })).toBeVisible()
  expect(await launches(page)).toBe(2)
  expect(new URL(page.url()).pathname).toBe('/')
})

test('a failed session refresh goes to the desktop sign-in page', async ({ page }) => {
  await openSystest(page, 'signed_out')
  await expect(page).toHaveURL(/\/login\?redirect_url=/)
})

test('an invalid target for opened content falls back to Preview', async ({ page }) => {
  await page.addInitScript(([key, value]) => localStorage.setItem(key, value), [MOCK_APP_LAUNCH_KEY, 'invalid_target'])
  await page.setViewportSize({ width: 1440, height: 900 })
  const callbacks = await routeAppHost(page, /http:\/\/(notes|reader)-fixture\.localhost:4173\//, '<title>Handler fixture</title>')
  await page.goto('/?scenario=normal&contentApps=1')
  await page.getByTestId('desktop-app-files').click()
  await page.locator('aside').getByRole('button', { name: /Documents/ }).first().click()
  await page.getByRole('cell', { name: /^Kyoto Trip Plan\.md/ }).first().dblclick()
  await expect(page.getByTestId('window-preview')).toBeVisible()
  await expect(page.getByTestId('window-notes-fixture').getByTestId('app-launch-error')).toHaveAttribute('data-launch-status', 'invalid_target')
  expect(callbacks).toHaveLength(0)
})
