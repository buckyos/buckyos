import { expect, test, type Page } from '@playwright/test'
import { routeAppHost, routeAppSdk } from '../fixtures/desktop'

// The notes App as the desktop frames it: it shows its address and opens other apps when told to.
const FIXTURE = `<!doctype html><title>Notes</title><output id="href"></output><output id="hash"></output><pre id="init"></pre><pre id="reply"></pre><input id="draft"><script type="module">
  import { AppFrameClient } from '/sdk/app-frame.mjs';
  document.querySelector('#href').textContent = location.pathname + location.search;
  document.querySelector('#hash').textContent = location.hash;
  const client = new AppFrameClient({
    shellOrigin: 'http://127.0.0.1:4173',
    onInit: init => { document.querySelector('#init').textContent = JSON.stringify(init) },
    onBeforeClose: () => document.querySelector('#draft').value ? 'deny' : 'allow',
  });
  window.openApp = request => { void client.openApp(request).then(reply => { document.querySelector('#reply').textContent = JSON.stringify(reply) }) };
</script>`

const frame = (page: Page) => page.frameLocator('[data-testid="web-app-frame"]')
const openFrom = (page: Page, request: object) => page.frames().find((f) => f.url().includes('notes-fixture'))!.evaluate((value) => (window as unknown as { openApp: (request: object) => void }).openApp(value), request)

async function openNotes(page: Page, query = '', window?: object) {
  await page.setViewportSize({ width: 1440, height: 900 })
  await routeAppHost(page, /http:\/\/(notes|reader)-fixture\.localhost:4173\//, FIXTURE)
  await routeAppSdk(page)
  await page.goto(`/?scenario=normal&contentApps=${encodeURIComponent(window ? JSON.stringify({ window }) : '1')}&open-app=notes-fixture${query}`)
  await expect(frame(page).locator('#init')).toContainText('"windowId"')
}

test('the frame tells an App how it is shown and lets it open a page of another app', async ({ page }) => {
  const hash = '#/notes/42?tab=a%2Fb&bfp=app-value'
  await openNotes(page, '&path=' + encodeURIComponent('/notes/1?view=all' + hash))
  await expect(frame(page).locator('#href')).toHaveText('/notes/1?view=all')
  await expect(frame(page).locator('#hash')).toHaveText(hash)
  const init = JSON.parse(await frame(page).locator('#init').textContent() ?? '{}') as Record<string, unknown>
  expect(init).toMatchObject({ presentation: 'window', container: 'browser' })
  expect(init.windowId).toMatch(/^[^|]+\|notes-fixture\|$/)
  expect(init.shellCapabilities).toContain('app.open')

  await openFrom(page, { app: 'settings', path: '/privacy' })
  await expect(frame(page).locator('#reply')).toHaveText('{"ok":true}')
  const settings = page.getByTestId('window-settings')
  await expect(settings.getByRole('heading', { name: 'Public Access' })).toBeVisible()
  // Another page of the same single window.
  await openFrom(page, { app: 'settings', path: '/developer' })
  await expect(settings.getByRole('heading', { name: 'Public Access' })).toHaveCount(0)
  await expect(settings).toHaveCount(1)

  await openFrom(page, { app: 'not-installed' })
  await expect(frame(page).locator('#reply')).toHaveText('{"ok":false,"code":"not_found"}')
  await openFrom(page, { app: 'settings', zone: 'did:web:elsewhere.example' })
  await expect(frame(page).locator('#reply')).toHaveText('{"ok":false,"code":"unsupported"}')
  await openFrom(page, { app: 'settings', path: '//elsewhere.example' })
  await expect(frame(page).locator('#reply')).toHaveText('{"ok":false,"code":"denied"}')
})

test('a single-window App is asked before its window goes to another page', async ({ page }) => {
  await openNotes(page, '&path=%2Fa')
  await frame(page).locator('#draft').fill('unsaved')
  await openFrom(page, { app: 'notes-fixture', path: '/b' })
  await expect(frame(page).locator('#reply')).toHaveText('{"ok":true}')
  await page.waitForTimeout(500)
  await expect(frame(page).locator('#href')).toHaveText('/a')
  await frame(page).locator('#draft').fill('')
  await openFrom(page, { app: 'notes-fixture', path: '/b' })
  await expect(frame(page).locator('#href')).toHaveText('/b')
  await expect(page.getByTestId('window-notes-fixture')).toHaveCount(1)
})

test('reopening a hash route returns to it after the App navigated inside its window', async ({ page }) => {
  const path = '/#/notes/42'
  await openNotes(page, '&path=' + encodeURIComponent(path))
  const app = page.frames().find(f => f.url().includes('notes-fixture'))!
  await app.evaluate(() => { location.hash = '/notes/77' })
  await openFrom(page, { app: 'notes-fixture', path })
  await expect.poll(() => page.frames().find(f => f.url().includes('notes-fixture'))?.url())
    .toBe('http://notes-fixture.localhost:4173' + path)
  await expect(frame(page).locator('#init')).toContainText('"windowId"')
  await expect(page.getByTestId('window-notes-fixture')).toHaveCount(1)
})

test('a multi-window App opens windows by path and reuses them', async ({ page }) => {
  await openNotes(page, '', { instance: 'multi' })
  const windows = page.getByTestId('window-notes-fixture')
  await openFrom(page, { app: 'notes-fixture', window: 'note-1', path: '/1' })
  await expect(windows).toHaveCount(2)
  await openFrom(page, { app: 'notes-fixture', window: 'note-1', path: '/1' })
  await openFrom(page, { app: 'notes-fixture' })
  await page.waitForTimeout(300)
  await expect(windows).toHaveCount(2)
  await openFrom(page, { app: 'notes-fixture', mode: 'new' })
  await expect(windows).toHaveCount(3)
})

test('a window keeps to what its App declares: size, buttons and no resizing', async ({ page }) => {
  await openNotes(page, '', { width: 480, height: 360, resizable: false, allow_minimize: false, allow_maximize: false })
  const win = page.getByTestId('window-notes-fixture')
  const box = (await win.boundingBox())!
  expect([Math.round(box.width), Math.round(box.height)]).toEqual([480, 360])
  await expect(win.getByRole('button', { name: 'Minimize' })).toHaveCount(0)
  await expect(win.getByRole('button', { name: 'Maximize' })).toHaveCount(0)
  await expect(win.getByRole('button', { name: 'Close' })).toBeVisible()
  await expect(page.locator('[data-testid^="window-resize-"]')).toHaveCount(0)
  await page.getByTestId('window-drag-notes-fixture').dblclick({ position: { x: 100, y: 10 } })
  const after = (await win.boundingBox())!
  expect([Math.round(after.width), Math.round(after.height)]).toEqual([480, 360])
})
