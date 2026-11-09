import { expect, test, type Page } from '@playwright/test'
import { MOCK_DESKTOP_FOLDER_KEY, seedDesktopFolder } from '../fixtures/desktop'

const accountKey = 'buckyos.mock.control-panel.v1'
const defaultsKey = 'buckyos.mock.desktop-state.system-info-widget'
const widget = { kind: 'file' as const, size: 24, mtime: 1, text: JSON.stringify({ widget: 'system-info' }) }

declare global {
  interface Window {
    systemInfoFixture: { delay: number; data: Record<string, unknown> | null; error: string | null }
  }
}

async function seedCard(page: Page, account = { user_id: 'bob', user_name: 'Bob', user_type: 'user' }) {
  await seedDesktopFolder(page, {}, { 'System Info.widget': widget })
  await page.addInitScript(([key, value]) => localStorage.setItem(key, JSON.stringify(value)), [accountKey, { zone: 'owner-zone', account, zone_presentation: { display_name: 'Home Lab' } }] as const)
}

async function softwareFixture(page: Page, delay = 0) {
  await page.addInitScript((wait) => {
    window.systemInfoFixture = { delay: wait, data: { version: '0.7.2', build_version: '0.7.2+build-widget-test', release_channel: 'beta', target: 'test-platform', session_token: 'must-not-be-copied' }, error: null }
  }, delay)
  await page.route(/\/src\/api\/settings\.ts$/, (route) => route.fulfill({
    contentType: 'text/javascript',
    body: `export * from '/src/api/settings.ts?system-info-original';
      export async function fetchBuckyOSInfo() {
        const fixture = structuredClone(window.systemInfoFixture);
        if (fixture.delay) await new Promise(resolve => setTimeout(resolve, fixture.delay));
        return {data: fixture.data, error: fixture.error};
      }`,
  }))
}

async function copySummary(page: Page) {
  await page.getByTestId('system-info-copy').click()
  await expect(page.getByTestId('system-info-copy')).toContainText('Copied')
  return JSON.parse(await page.evaluate(() => navigator.clipboard.readText()))
}

test.beforeEach(async ({ context, page }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])
  await page.setViewportSize({ width: 1280, height: 860 })
})

test('adds one descriptor through Appearance, preserves a colliding file and persists across reload', async ({ page }) => {
  const existing = JSON.stringify({ widget: 'notepad', text: 'Keep this note' })
  await seedDesktopFolder(page, { Settings: 'settings' }, { 'System Info.widget': { ...widget, text: existing } })
  await page.goto('/?scenario=normal')
  await page.getByTestId('desktop-app-settings').click()
  await page.getByTestId('window-settings').getByRole('button', { name: 'Appearance', exact: true }).click()
  const add = page.getByTestId('settings-add-system-info')
  await add.click()
  await expect(add).toBeDisabled()
  await expect(page.getByTestId('system-info-widget')).toHaveCount(1)
  const entries = await page.evaluate((key) => JSON.parse(localStorage.getItem(key)!).entries, MOCK_DESKTOP_FOLDER_KEY)
  expect(entries['System Info.widget'].text).toBe(existing)
  expect(JSON.parse(entries['System Info (2).widget'].text)).toEqual({ widget: 'system-info' })
  await page.reload()
  await expect(page.getByTestId('system-info-widget')).toHaveCount(1)
  await expect(page.getByTestId('system-info-fields')).toContainText('0.7.0')
})

for (const height of [860, 553]) test(`keeps Getting started and adds system info on the left: height ${height}`, async ({ page }) => {
  await page.setViewportSize({ width: 1080, height })
  await seedDesktopFolder(page, { Settings: 'settings' }, {
    'Clock.widget': { ...widget, text: JSON.stringify({ widget: 'clock' }) },
    'Getting Started.widget': { ...widget, text: JSON.stringify({ widget: 'notepad', text: 'Getting started\nKeep my notes' }) },
  })
  await page.goto('/?scenario=normal')
  const note = page.getByTestId('desktop-item-Getting Started.widget')
  await expect(note).toContainText('Getting started')
  const before = await note.boundingBox()
  const original = await page.evaluate((key) => JSON.parse(localStorage.getItem(key)!).entries['Getting Started.widget'].text, MOCK_DESKTOP_FOLDER_KEY)
  const settings = page.getByTestId('desktop-app-settings')
  const settingsBefore = await settings.boundingBox()
  await settings.click()
  await page.getByTestId('window-settings').getByRole('button', { name: 'Appearance', exact: true }).click()
  await page.getByTestId('settings-add-system-info').click()
  await expect(page.getByTestId('settings-add-system-info')).toBeDisabled()
  await page.getByTestId('window-drag-settings').getByRole('button', { name: 'Close', exact: true }).click()
  await expect(page.getByTestId('system-info-fields')).toContainText('0.7.0')
  const card = page.getByTestId('system-info-widget')
  const position = () => card.evaluate((el) => {
    const slide = el.closest('.swiper-slide')!
    const rect = el.getBoundingClientRect()
    const parent = slide.getBoundingClientRect()
    return { page: [...document.querySelectorAll('.swiper-slide')].indexOf(slide), x: rect.x - parent.x, y: rect.y - parent.y }
  })
  const info = await position()
  const left = await note.evaluate((el) => el.getBoundingClientRect().x - el.closest('.swiper-slide')!.getBoundingClientRect().x)
  expect(Math.abs(info.x - left)).toBeLessThan(2)
  if (height === 860) {
    expect(info.page).toBe(0)
    const box = await card.boundingBox()
    expect(Math.abs(box!.x - before!.x)).toBeLessThan(2)
    expect(box!.y).toBeGreaterThanOrEqual(before!.y + before!.height)
    await expect(page.getByTestId('system-info-copy')).toBeInViewport()
  } else {
    expect(info.page).toBeGreaterThan(0)
  }
  expect(await note.boundingBox()).toEqual(before)
  expect(await settings.boundingBox()).toEqual(settingsBefore)
  const saved = await page.evaluate((key) => JSON.parse(localStorage.getItem(key)!).entries['Getting Started.widget'].text, MOCK_DESKTOP_FOLDER_KEY)
  expect(saved).toBe(original)
  await page.reload()
  await expect(note).toContainText('Getting started')
  await expect.poll(position).toEqual(info)
})

test('shows the signed-in user independently from the Zone owner and copies only selected fields', async ({ page }) => {
  await seedCard(page)
  await softwareFixture(page)
  const errors: string[] = []
  page.on('pageerror', (error) => errors.push(error.message))
  await page.goto('/?scenario=normal')
  const card = page.getByTestId('system-info-widget')
  await expect(card).toContainText('bob')
  await expect(card).toContainText('Home Lab')
  await expect(card.getByText('owner-zone.web3.buckyos.ai', { exact: true })).toBeInViewport()
  await expect(card).toContainText('0.7.2+build-widget-test')
  const summary = await copySummary(page)
  expect(summary.account).toEqual({ user_id: 'bob', user_type: 'user' })
  expect(summary.zone).toMatchObject({ name: 'Home Lab', hostname: 'owner-zone.web3.buckyos.ai', did: 'did:bns:owner-zone' })
  expect(JSON.stringify(summary)).not.toContain('must-not-be-copied')
  expect(summary.buckyos.version).toBe('0.7.2')
  await page.getByTestId('system-info-details').click()
  await expect(page.getByRole('dialog')).toContainText('did:bns:owner-zone')
  await page.getByRole('button', { name: 'Open settings', exact: true }).click()
  await expect(page.getByTestId('settings-general-page')).toBeVisible()
  expect(errors).toEqual([])
})

test('refreshes account and Zone after returning to the page and clears the signed-out account', async ({ page }) => {
  await seedCard(page)
  await page.goto('/?scenario=normal')
  await expect(page.getByTestId('system-info-fields')).toContainText('bob')
  await page.evaluate((key) => {
    const state = JSON.parse(localStorage.getItem(key)!)
    state.account = { user_id: 'carol', user_name: 'Carol', user_type: 'limited' }
    state.zone = 'second-zone'
    state.zone_presentation = { display_name: 'Second Zone' }
    localStorage.setItem(key, JSON.stringify(state))
    document.dispatchEvent(new Event('visibilitychange'))
  }, accountKey)
  await expect(page.getByTestId('system-info-fields')).toContainText('carol')
  await expect(page.getByTestId('system-info-fields')).toContainText('Limited')
  await expect(page.getByTestId('system-info-fields')).toContainText('second-zone.web3.buckyos.ai')
  await expect(page.getByTestId('system-info-fields')).not.toContainText('bob')
  const copied = await copySummary(page)
  expect(copied.account.user_id).toBe('carol')
  await page.evaluate((key) => {
    const state = JSON.parse(localStorage.getItem(key)!)
    state.session = { kind: 'anonymous' }
    localStorage.setItem(key, JSON.stringify(state))
    document.dispatchEvent(new Event('visibilitychange'))
  }, accountKey)
  await expect(page.getByTestId('system-info-fields')).toContainText('Not signed in')
  await expect(page.getByTestId('system-info-fields')).not.toContainText('carol')
})

test('loading, partial failure and recovery do not invent versions or copy stale values', async ({ page }) => {
  await seedCard(page)
  await softwareFixture(page, 1500)
  await page.goto('/?scenario=normal')
  const card = page.getByTestId('system-info-widget')
  await expect(card).toContainText('Loading…')
  await expect(card).toContainText('0.7.2+build-widget-test')
  await page.evaluate(() => { window.systemInfoFixture = { delay: 0, data: null, error: 'permission denied' } })
  await page.getByTestId('system-info-refresh').click()
  await expect(card).toContainText('Some information could not be loaded')
  await expect(card).toContainText('bob')
  await expect(card).not.toContainText('0.7.2')
  const copied = await copySummary(page)
  expect(copied.buckyos).toBeNull()
  expect(copied.account.user_id).toBe('bob')
  await page.evaluate(() => { window.systemInfoFixture = { delay: 0, data: { version: '0.7.3', build_version: null }, error: null } })
  await page.getByTestId('system-info-refresh').click()
  await expect(card).toContainText('0.7.3')
  await expect(card).not.toContainText('Some information could not be loaded')
})

for (const theme of ['light', 'dark']) test(`long identifiers and Chinese text fit narrow screens: ${theme}`, async ({ page }) => {
  await seedCard(page, { user_id: 'long-account-identifier-for-a-friend', user_name: 'Friend', user_type: 'admin' })
  await page.addInitScript((mode) => {
    localStorage.setItem('buckyos.prototype.locale.v1', 'zh-CN')
    localStorage.setItem('buckyos.prototype.theme.v1', mode)
  }, theme)
  for (const width of [1280, 390]) {
    await page.setViewportSize({ width, height: 860 })
    await page.goto('/?scenario=normal')
    const card = page.getByTestId('system-info-widget')
    await expect(page.locator('html')).toHaveAttribute('data-theme', theme)
    await expect(card).toContainText('管理员')
    await expect(card).toContainText('登录用户 ID')
    await expect(page.getByTestId('system-info-copy')).toBeInViewport()
    await expect(page.getByTestId('system-info-details')).toBeInViewport()
    expect(await card.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true)
    await page.screenshot({ path: `test-results/system-info-${width}-${theme}.png` })
  }
})

test('a failed folder write is reported and reconciled, without leaving a fake widget', async ({ page }) => {
  await seedDesktopFolder(page, { Settings: 'settings' })
  await page.route(/\/src\/api\/desktop_items\.ts$/, (route) => route.fulfill({
    contentType: 'text/javascript',
    body: `export * from '/src/api/desktop_items.ts?system-info-original'; export async function writeDesktopItem() { throw new Error('permission denied'); }`,
  }))
  await page.goto('/?scenario=normal')
  await page.getByTestId('desktop-app-settings').click()
  await page.getByTestId('window-settings').getByRole('button', { name: 'Appearance', exact: true }).click()
  await page.getByTestId('settings-add-system-info').click()
  await expect(page.getByTestId('system-info-widget')).toHaveCount(0)
  await expect(page.getByTestId('settings-add-system-info')).toBeEnabled()
  await expect(page.getByText('The Desktop folder could not be changed.')).toBeVisible()
})

test('removing a card deletes its descriptor and does not add it back on reload', async ({ page }) => {
  await page.goto('/?scenario=normal')
  await expect(page.getByTestId('system-info-fields')).toContainText('0.7.0')
  await page.getByTestId('system-info-widget').click({ button: 'right' })
  await page.getByRole('menuitem', { name: 'Remove from desktop' }).click()
  await expect(page.getByTestId('system-info-widget')).toHaveCount(0)
  await page.evaluate(() => localStorage.removeItem('buckyos.layout.desktop.v3'))
  await page.reload()
  await expect(page.getByTestId('system-info-widget')).toHaveCount(0)
  await page.getByTestId('desktop-app-settings').click()
  await page.getByTestId('window-settings').getByRole('button', { name: 'Appearance', exact: true }).click()
  await page.getByTestId('settings-add-system-info').click()
  await expect(page.getByTestId('system-info-widget')).toHaveCount(1)
  await page.reload()
  await expect(page.getByTestId('system-info-widget')).toHaveCount(1)
})

test('an existing desktop gets system info once without replacing a colliding note', async ({ page }) => {
  const note = JSON.stringify({ widget: 'notepad', text: 'My existing notes' })
  await seedDesktopFolder(page, { Settings: 'settings' }, { 'System Info.widget': { ...widget, text: note } }, { systemInfoInitialized: false })
  await page.goto('/?scenario=normal')
  await expect(page.getByTestId('system-info-fields')).toContainText('0.7.0')
  const entries = await page.evaluate((key) => JSON.parse(localStorage.getItem(key)!).entries, MOCK_DESKTOP_FOLDER_KEY)
  expect(entries['System Info.widget'].text).toBe(note)
  expect(JSON.parse(entries['System Info (2).widget'].text)).toEqual({ widget: 'system-info' })
  expect(await page.evaluate((key) => JSON.parse(localStorage.getItem(key)!), defaultsKey)).toEqual({ initialized: true })
  await page.reload()
  await expect(page.getByTestId('system-info-widget')).toHaveCount(1)
  expect(await page.evaluate((key) => JSON.parse(localStorage.getItem(key)!).entries, MOCK_DESKTOP_FOLDER_KEY)).toEqual(entries)
})

test('a renamed card from the preview is kept when default display is introduced', async ({ page }) => {
  await seedDesktopFolder(page, {}, { 'My Zone.widget': widget }, { systemInfoInitialized: false })
  await page.goto('/?scenario=normal')
  await expect(page.getByTestId('system-info-fields')).toContainText('0.7.0')
  const entries = await page.evaluate((key) => JSON.parse(localStorage.getItem(key)!).entries, MOCK_DESKTOP_FOLDER_KEY)
  expect(Object.keys(entries)).toEqual(['My Zone.widget'])
  expect(await page.evaluate((key) => JSON.parse(localStorage.getItem(key)!), defaultsKey)).toEqual({ initialized: true })
})

test('automatic creation failure leaves the desktop usable and retries after reload', async ({ page }) => {
  await seedDesktopFolder(page, { Settings: 'settings' }, {}, { systemInfoInitialized: false })
  await page.route(/\/src\/api\/desktop_items\.ts$/, (route) => route.fulfill({
    contentType: 'text/javascript',
    body: `export * from '/src/api/desktop_items.ts?system-info-original'; export async function writeDesktopItem() { throw new Error('permission denied'); }`,
  }))
  await page.goto('/?scenario=normal')
  await expect(page.getByTestId('desktop-app-settings')).toBeVisible()
  await expect(page.getByTestId('system-info-widget')).toHaveCount(0)
  expect(await page.evaluate((key) => localStorage.getItem(key), defaultsKey)).toBeNull()
  await page.unroute(/\/src\/api\/desktop_items\.ts$/)
  await page.reload()
  await expect(page.getByTestId('system-info-fields')).toContainText('0.7.0')
  expect(await page.evaluate((key) => JSON.parse(localStorage.getItem(key)!), defaultsKey)).toEqual({ initialized: true })
})

test('an unavailable initialization record does not reset an existing desktop', async ({ page }) => {
  await seedDesktopFolder(page, { Settings: 'settings' }, {}, { systemInfoInitialized: false })
  await page.route(/\/src\/api\/desktop_state\.ts$/, (route) => route.fulfill({
    contentType: 'text/javascript',
    body: `export * from '/src/api/desktop_state.ts?system-info-original';
      import { readDesktopState as original } from '/src/api/desktop_state.ts?system-info-original';
      export async function readDesktopState(key) {
        if (key === 'system-info-widget') throw new Error('state unavailable');
        return original(key);
      }`,
  }))
  await page.goto('/?scenario=normal')
  await expect(page.getByTestId('desktop-app-settings')).toBeVisible()
  await expect(page.getByTestId('system-info-widget')).toHaveCount(0)
  expect(await page.evaluate((key) => Object.keys(JSON.parse(localStorage.getItem(key)!).entries), MOCK_DESKTOP_FOLDER_KEY)).toEqual(['Settings.lnk'])
})

test('a fresh desktop with limited height keeps the default card on a later page at the left', async ({ page }) => {
  await page.setViewportSize({ width: 1080, height: 553 })
  await page.goto('/?scenario=normal')
  const card = page.getByTestId('system-info-widget')
  await expect(card).toContainText('0.7.0')
  await expect.poll(() => card.evaluate((el) => [...document.querySelectorAll('.swiper-slide')].indexOf(el.closest('.swiper-slide')!))).toBeGreaterThan(0)
  const cardPage = await card.evaluate((el) => [...document.querySelectorAll('.swiper-slide')].indexOf(el.closest('.swiper-slide')!))
  expect(cardPage).toBeGreaterThan(0)
  const left = (await page.locator('.cp-clock').boundingBox())!.x
  await page.locator('.swiper-pagination-bullet').nth(cardPage).click()
  await expect.poll(async () => Math.abs((await card.boundingBox())!.x - left)).toBeLessThan(2)
  await expect(page.getByTestId('system-info-copy')).toBeInViewport()
})
