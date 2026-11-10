import { mkdirSync, mkdtempSync, readFileSync, realpathSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test, expect, type Frame, type Page } from '@playwright/test'
import { routeAppHost } from '../fixtures/desktop'
import { selectGoto, selectRow } from '../fixtures/selectDialog'

// The fixture App is fulfilled by Playwright (no address) and reaches the dev server on 127.0.0.1; see preview-embed.spec.ts.
test.use({ launchOptions: { args: ['--disable-features=LocalNetworkAccessChecks,PrivateNetworkAccessForNavigations,PrivateNetworkAccessRespectPreflightResults'] } })

/**
 * The system file picker (`sysdlg/select`) as a third-party App reaches it: over the App Frame inside a Desktop
 * window (the shell draws the dialog), and framed over the App's own page in a standalone tab. The fixture App
 * uses the SDK's `AppFrameClient.selectContent`, as any App would.
 */
const SHELL = 'http://127.0.0.1:4173'
// The fixture App ships the SDK build itself, as an App does.
const SDK_DIST = join(realpathSync('node_modules/buckyos'), 'dist')
const fixture = `<!doctype html><title>Picker fixture</title><textarea id="draft" style="width:300px;height:80px"></textarea><div style="height:3000px"></div>
<script type="module">
  import { AppFrameClient } from '/sdk/app-frame.mjs'
  const client = window.client = new AppFrameClient({ shellOrigin: '${SHELL}', systemOrigin: '${SHELL}' })
  const describe = async item => item.origin !== 'local' ? item
    : item.kind === 'file' ? { ...item, file: { name: item.file.name, size: item.file.size, text: await item.file.text() } }
    : { ...item, files: await Promise.all(item.files.map(async f => ({ relativePath: f.relativePath, text: await f.file.text() }))) }
  window.results = []; window.replies = []
  window.pick = async params => {
    try { const r = await client.selectContent(params); if (r.action === 'selected') r.items = await Promise.all(r.items.map(describe)); window.results.push(r) }
    catch (e) { window.results.push({ error: e.code ?? String(e) }) }
  }
  window.raw = (path, params) => parent.postMessage({ protocol: 'buckyos.app-frame', version: 1, type: 'sysdlg.open', id: crypto.randomUUID(), payload: { path, params } }, '${SHELL}')
  addEventListener('message', e => { if (e.data?.type === 'sysdlg.openResult') window.replies.push(e.data.payload) })
  window.ready = true
</script>`
const ref = (path: string) => ({ kind: 'cyfs-path', path: `cyfs://${path}` })
const pick = (app: Frame, params: unknown) => app.evaluate((p) => { void (window as any).pick(p) }, params)
const results = (app: Frame) => app.evaluate(() => (window as any).results)

async function routeFixture(page: Page) {
  await routeAppHost(page, /http:\/\/(notes|reader)-fixture\.localhost:4173\//, fixture)
  await page.route(/-fixture\.localhost:4173\/sdk\//, route => route.fulfill({ contentType: 'text/javascript', body: readFileSync(join(SDK_DIST, new URL(route.request().url()).pathname.split('/').pop()!)) }))
}

async function openInWindow(page: Page, query = '') {
  await page.setViewportSize({ width: 1440, height: 900 })
  await routeFixture(page)
  await page.goto(`/?scenario=normal&contentApps=1${query}`)
  await page.getByTestId('desktop-app-files').click()
  await page.locator('aside').getByRole('button', { name: /Documents/ }).first().click()
  await page.getByRole('cell', { name: /^Kyoto Trip Plan\.md/ }).first().dblclick()
  const win = page.getByTestId('window-notes-fixture')
  await expect(win).toBeVisible()
  await expect.poll(() => page.frames().some(f => f.url().includes('notes-fixture'))).toBe(true)
  const app = page.frames().find(f => f.url().includes('notes-fixture'))!
  await app.waitForFunction(() => (window as any).ready === true)
  // the handshake has completed once the shell's capabilities are known to the App
  await app.waitForFunction(() => !!(window as any).client.init)
  return { win, app, dialog: win.getByTestId('select-dialog') }
}

test('open: the shell draws the picker over the App window; accept, limit and selection across folders', async ({ page }) => {
  const { win, app, dialog } = await openInWindow(page)
  await pick(app, { mode: 'open', multiple: true, limit: 2, accept: [{ mime: ['text/markdown', 'application/pdf'] }], start: ref('/home/Documents'), title: 'Attach' })
  await expect(dialog).toBeVisible()
  // only this window is covered, and the requester's name is the system's, next to the title the App chose
  await expect(win.getByTestId('window-dialog')).toContainText('Attach')
  await expect(win.getByTestId('window-dialog')).toContainText('Requested by Notebook Fixture')
  await expect(page.getByTestId('window-files').getByTestId('window-dialog')).toHaveCount(0)
  await expect(dialog.getByTestId('select-local')).toHaveCount(0)
  await expect(selectRow(dialog, '2026 Q1 Review.docx')).toHaveAttribute('aria-disabled', 'true')
  await selectRow(dialog, 'Kyoto Trip Plan.md').click()
  // a second request while this one is open is refused, and so is a dialog that is not on the shell's list
  await pick(app, { mode: 'open' })
  await expect.poll(async () => (await results(app)).at(-1)).toEqual({ error: 'busy' })
  await selectGoto(dialog, '/home/Pictures')
  await selectGoto(dialog, '/home/Documents')
  await expect(selectRow(dialog, 'Kyoto Trip Plan.md')).toHaveAttribute('aria-selected', 'true')
  await selectRow(dialog, 'Friend Contract Draft.pdf').click()
  await expect(dialog.getByTestId('select-count')).toContainText('up to 2')
  await dialog.getByTestId('select-confirm').click()
  await expect(dialog).toHaveCount(0)
  await expect.poll(async () => (await results(app)).length).toBe(2)
  const result = (await results(app)).at(-1)
  expect(result.action).toBe('selected')
  expect(result.items.map((i: any) => [i.origin, i.kind, i.source.path, i.hints.name])).toEqual([
    ['zone', 'file', 'cyfs:///home/Documents/Kyoto Trip Plan.md', 'Kyoto Trip Plan.md'],
    ['zone', 'file', 'cyfs:///home/Documents/Friend Contract Draft.pdf', 'Friend Contract Draft.pdf'],
  ])
  expect(result.items[1].hints.mime).toBe('application/pdf')
  expect(JSON.stringify(result)).not.toMatch(/token|https?:/)
})

test('refusals: unknown dialog, invalid parameters, and cancelling is a result', async ({ page }) => {
  const { app, dialog } = await openInWindow(page)
  await app.evaluate(() => (window as any).raw('sysdlg/app_installer', {}))
  await app.evaluate(() => (window as any).raw('sysdlg/select', { mode: 'open', limit: 9999 }))
  await app.evaluate(() => (window as any).raw('sysdlg/select', { mode: 'open', surprise: true }))
  await expect.poll(() => app.evaluate(() => (window as any).replies)).toEqual([{ ok: false, code: 'unsupported' }, { ok: false, code: 'invalid_params' }, { ok: false, code: 'invalid_params' }])
  await pick(app, {})
  await expect(dialog).toBeVisible()
  await dialog.press('Escape')
  await expect.poll(async () => (await results(app)).at(-1)).toEqual({ action: 'cancelled' })
  await pick(app, { mode: 'folder' })
  await expect(dialog).toHaveAttribute('data-mode', 'folder')
  await dialog.getByTestId('select-cancel').click()
  await expect.poll(async () => (await results(app)).length).toBe(2)
  expect((await results(app)).at(-1)).toEqual({ action: 'cancelled' })
})

test('save: an existing name is asked about and reported as overwrite; folder: only a writable folder', async ({ page }) => {
  const { app, dialog } = await openInWindow(page)
  await pick(app, { mode: 'save', start: ref('/home/Documents'), suggestedName: 'Kyoto Trip Plan.md', multiple: true })
  await expect(dialog.getByTestId('select-name')).toHaveValue('Kyoto Trip Plan.md')
  await dialog.getByTestId('select-confirm').click()
  await expect(dialog.getByTestId('select-conflict')).toContainText('Kyoto Trip Plan.md')
  await dialog.getByTestId('select-replace').click()
  await expect.poll(async () => (await results(app)).at(-1)).toEqual({ action: 'save', parent: ref('/home/Documents'), name: 'Kyoto Trip Plan.md', overwrite: true })
  await pick(app, { mode: 'save', start: ref('/home/Documents/Kyoto Trip Plan.md') })
  // a file as the start opens its folder and offers its name
  await expect(dialog.getByTestId('select-name')).toHaveValue('Kyoto Trip Plan.md')
  await dialog.getByTestId('select-name').fill('bad/name')
  await expect(dialog.getByTestId('select-confirm')).toBeDisabled()
  await dialog.getByTestId('select-name').fill('brand new.md')
  await expect(dialog.getByTestId('select-confirm')).toBeEnabled()
  await dialog.getByTestId('select-name').press('Enter')
  await expect.poll(async () => (await results(app)).at(-1)).toEqual({ action: 'save', parent: ref('/home/Documents'), name: 'brand new.md', overwrite: false })

  await pick(app, { mode: 'folder', writable: true, exclude: [ref('/home/Pictures')] })
  await selectGoto(dialog, '/home')
  await expect(selectRow(dialog, 'Pictures')).toHaveAttribute('aria-disabled', 'true')
  // folders only
  await selectRow(dialog, 'Documents').click()
  await expect(dialog.locator('[data-testid="select-row"][data-kind="file"]')).toHaveCount(0)
  await dialog.getByTestId('select-confirm').click()
  await expect.poll(async () => (await results(app)).length).toBe(3)
  const picked = (await results(app)).at(-1)
  expect(picked).toEqual({ action: 'selected', items: [{ origin: 'zone', kind: 'folder', source: ref('/home/Documents'), hints: { name: 'Documents' } }] })
})

test('folder and save need a writable folder only when it will be written to', async ({ page }) => {
  const { app, dialog } = await openInWindow(page, '&fbCapabilities=readonly')
  await pick(app, { mode: 'folder', writable: true, start: ref('/home/Documents') })
  await expect(dialog.getByTestId('select-readonly')).toBeVisible()
  await expect(dialog.getByTestId('select-confirm')).toBeDisabled()
  await expect(dialog.getByTestId('select-new-folder')).toHaveCount(0)
  await dialog.getByTestId('select-cancel').click()
  await expect.poll(async () => (await results(app)).length).toBe(1)
  await pick(app, { mode: 'save', start: ref('/home/Documents'), suggestedName: 'a.txt' })
  await expect(dialog.getByTestId('select-readonly')).toBeVisible()
  await expect(dialog.getByTestId('select-confirm')).toBeDisabled()
  await dialog.getByTestId('select-cancel').click()
  await expect.poll(async () => (await results(app)).length).toBe(2)
  await pick(app, { mode: 'folder', start: ref('/home/Documents') })
  await expect(dialog.getByTestId('select-confirm')).toBeEnabled()
  await dialog.getByTestId('select-confirm').click()
  await expect.poll(async () => (await results(app)).at(-1)?.action).toBe('selected')
})

test('a folder larger than one page keeps loading, and a start that does not exist falls back to home', async ({ page }) => {
  const { app, dialog } = await openInWindow(page)
  await pick(app, { mode: 'open', start: ref('/home/stress-10k') })
  const rows = dialog.getByTestId('select-row')
  await expect(rows).toHaveCount(200)
  await dialog.getByTestId('select-list').evaluate((el) => el.scrollTo(0, el.scrollHeight))
  await expect.poll(() => rows.count()).toBeGreaterThan(200)
  await dialog.getByTestId('select-cancel').click()
  await expect.poll(async () => (await results(app)).length).toBe(1)
  await pick(app, { mode: 'open', start: ref('/home/no/such/place') })
  await expect(dialog.getByTestId('select-crumbs')).toHaveText(/home$/)
})

test('this device: files and a folder reach the App as File objects; refused picks stay in the dialog', async ({ page }) => {
  const { app, dialog } = await openInWindow(page)
  const dir = mkdtempSync(join(tmpdir(), 'sysdlg-'))
  const folder = join(dir, 'album')
  mkdirSync(join(folder, 'inner'), { recursive: true })
  for (const [name, text] of [['a.txt', 'alpha'], ['b.txt', 'beta'], ['c.md', 'gamma']]) writeFileSync(join(dir, name), text)
  writeFileSync(join(folder, 'one.txt'), 'one'); writeFileSync(join(folder, 'inner', 'two.txt'), 'two')

  await pick(app, { mode: 'open', multiple: true, limit: 2, accept: [{ ext: ['.txt'] }], allowLocal: true, start: ref('/home/Documents') })
  await expect(dialog.getByTestId('select-local')).toBeVisible()
  await selectRow(dialog, 'Kyoto Trip Plan.md').click({ force: true })
  const input = page.getByTestId('select-local-input')
  await expect(input).toHaveAttribute('accept', '.txt')
  await input.setInputFiles([join(dir, 'a.txt'), join(dir, 'c.md')])
  await expect(dialog.getByTestId('select-notice')).toContainText('c.md')
  await input.setInputFiles([join(dir, 'a.txt'), join(dir, 'b.txt'), join(dir, 'c.md')])
  await expect(dialog.getByTestId('select-notice')).toContainText('up to 2')
  expect(await results(app)).toEqual([])
  await input.setInputFiles([join(dir, 'a.txt'), join(dir, 'b.txt')])
  await expect(dialog).toHaveCount(0)
  await expect.poll(async () => (await results(app)).length).toBe(1)
  const files = (await results(app)).at(-1)
  expect(files.items.map((i: any) => [i.origin, i.kind, i.file.name, i.file.size, i.file.text, i.hints.name])).toEqual([
    ['local', 'file', 'a.txt', 5, 'alpha', 'a.txt'], ['local', 'file', 'b.txt', 4, 'beta', 'b.txt'],
  ])

  await pick(app, { mode: 'folder', allowLocal: true })
  await page.getByTestId('select-local-input').setInputFiles(folder)
  await expect(dialog).toHaveCount(0)
  await expect.poll(async () => (await results(app)).length).toBe(2)
  const picked = (await results(app)).at(-1)
  expect(picked.items).toHaveLength(1)
  expect(picked.items[0]).toMatchObject({ origin: 'local', kind: 'folder', name: 'album' })
  expect(picked.items[0].files.map((f: any) => [f.relativePath, f.text]).sort()).toEqual([['album/inner/two.txt', 'two'], ['album/one.txt', 'one']])
  // a folder that must be written to cannot come from this device
  await pick(app, { mode: 'folder', writable: true, allowLocal: true })
  await expect(dialog).toBeVisible()
  await expect(dialog.getByTestId('select-local')).toHaveCount(0)
})

test('standalone tab: the picker page is framed over the App and the page is restored afterwards', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 })
  await routeFixture(page)
  await page.goto('http://notes-fixture.localhost:4173/')
  await page.waitForFunction(() => (window as any).ready === true)
  const app = page.mainFrame()
  const draft = page.locator('#draft')
  await draft.fill('unsent draft')
  await page.evaluate(() => window.scrollTo(0, 400))
  const before = await page.evaluate(() => ({ style: document.body.style.cssText, html: document.documentElement.style.cssText, inert: [...document.body.children].some(el => (el as HTMLElement).inert) }))
  await draft.focus()

  await pick(app, { mode: 'open', start: ref('/home/Documents') })
  const layer = page.locator('[data-buckyos-sysdlg]')
  const picker = page.frameLocator('[data-buckyos-sysdlg] iframe')
  await expect(picker.getByTestId('select-dialog')).toBeVisible({ timeout: 20_000 })
  await expect(picker.getByTestId('select-embed')).toContainText('Requested by notes-fixture.localhost:4173')
  await pick(app, { mode: 'open' })
  await expect.poll(async () => (await results(app)).at(-1)).toEqual({ error: 'busy' })
  await picker.locator('[data-testid="select-row"][data-name="Kyoto Trip Plan.md"]').dblclick()
  await expect(layer).toHaveCount(0)
  await expect.poll(async () => (await results(app)).length).toBe(2)
  expect((await results(app)).at(-1)).toMatchObject({ action: 'selected', items: [{ origin: 'zone', kind: 'file', source: ref('/home/Documents/Kyoto Trip Plan.md') }] })
  await expect(draft).toHaveValue('unsent draft')
  await expect(draft).toBeFocused()
  expect(await page.evaluate(() => window.scrollY)).toBe(400)
  expect(await page.evaluate(() => ({ style: document.body.style.cssText, html: document.documentElement.style.cssText, inert: [...document.body.children].some(el => (el as HTMLElement).inert) }))).toEqual(before)
  expect(await page.evaluate(() => document.querySelectorAll('iframe').length)).toBe(0)

  // cancelling, and a local file through the system page's own input
  await pick(app, { mode: 'save', suggestedName: 'x.txt' })
  await picker.getByTestId('select-cancel').click()
  await expect.poll(async () => (await results(app)).at(-1)).toEqual({ action: 'cancelled' })
  const dir = mkdtempSync(join(tmpdir(), 'sysdlg-'))
  writeFileSync(join(dir, 'local.txt'), 'from this device')
  await pick(app, { mode: 'open', allowLocal: true })
  await picker.getByTestId('select-local-input').setInputFiles(join(dir, 'local.txt'))
  await expect(layer).toHaveCount(0)
  await expect.poll(async () => (await results(app)).length).toBe(4)
  expect((await results(app)).at(-1).items[0]).toMatchObject({ origin: 'local', kind: 'file', file: { name: 'local.txt', text: 'from this device' } })
  // parameters outside the contract are refused by the system page
  await pick(app, { mode: 'open', limit: 100000 })
  await expect.poll(async () => (await results(app)).at(-1)).toEqual({ error: 'invalid_params' })
  await expect(layer).toHaveCount(0)
})

test('standalone tab: a picker page that never answers offers Retry and Cancel', async ({ page }) => {
  await routeFixture(page)
  await page.route(`${SHELL}/embed/sysdlg/select`, route => route.fulfill({ contentType: 'text/html', body: '<!doctype html><title>silent</title>' }))
  await page.goto('http://notes-fixture.localhost:4173/')
  await page.waitForFunction(() => (window as any).ready === true)
  await pick(page.mainFrame(), { mode: 'open' })
  const layer = page.locator('[data-buckyos-sysdlg]')
  await expect(layer.getByRole('alert')).toContainText('Unable to load the file picker', { timeout: 15_000 })
  await expect(layer.getByRole('button', { name: 'Retry' })).toBeVisible()
  await layer.getByRole('button', { name: 'Cancel' }).click()
  await expect(layer).toHaveCount(0)
  await expect.poll(async () => (await results(page.mainFrame())).at(-1)).toEqual({ error: 'load_failed' })
})

test('the picker page refuses to work for a page outside the Zone', async ({ page }) => {
  await page.route('http://outside.example/', route => route.fulfill({ contentType: 'text/html', body: `<iframe id="embed" src="${SHELL}/embed/sysdlg/select#bse=x&po=${encodeURIComponent('http://outside.example')}"></iframe>` }))
  await page.goto('http://outside.example/')
  await expect(page.frameLocator('#embed').getByRole('alert')).toBeVisible({ timeout: 15_000 })
  await expect(page.frameLocator('#embed').getByTestId('select-dialog')).toHaveCount(0)
})
