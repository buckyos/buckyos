/**
 * Text Editor on a real Zone (issue #675):
 *
 *   BUCKYOS_TEST_ZONE_HOST=test.buckyos.io pnpm exec playwright test --config=playwright.dv.config.ts tests/dv/text-editor.spec.ts
 *
 * Signs in at the Zone root as the owner (devtest), writes its fixtures into `/home/<owner>/te-dv-<run>/` through
 * NFSP and removes them afterwards. Covers the desktop iframe and a standalone tab.
 */
import { createHash } from 'node:crypto'
import { expect, request as playwrightRequest, test, type APIRequestContext, type Browser, type BrowserContext, type FrameLocator, type Page } from '@playwright/test'
import { openFromAllApps } from '../e2e/fixtures/desktop'

const zone = process.env.BUCKYOS_TEST_ZONE_HOST || 'test.buckyos.io'
const origin = `https://${zone}`
const owner = process.env.BUCKYOS_TEST_ADMIN_USER || 'devtest'
const password = process.env.BUCKYOS_TEST_ADMIN_PASSWORD || 'bucky2025'
const editorApp = `text-editor.buckyos.bns.did@${owner}`
const editorOrigin = `https://text-editor.${zone}`
const run = `te-dv-${Date.now().toString(36)}`
const dir = `/home/${owner}/${run}`
const PNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==', 'base64')

function hashPassword(username: string, pw: string, nonce: number): string {
  const original = createHash('sha256').update(`${pw}${username}.buckyos`, 'utf8').digest('base64')
  return createHash('sha256').update(`${original}${nonce}`, 'utf8').digest('base64')
}
async function kapi(request: APIRequestContext, service: string, method: string, params: Record<string, unknown>, token?: string) {
  const seq = Date.now()
  return (await (await request.post(`${origin}/kapi/${service}`, { data: { method, params, sys: token ? [seq, token] : [seq] } })).json()) as { result?: Record<string, unknown>; error?: string }
}
async function apiToken(request: APIRequestContext) {
  const nonce = Date.now()
  const body = await kapi(request, 'control-panel', 'auth.login', { username: owner, password: hashPassword(owner, password, nonce), target: { kind: 'system', service_id: 'control-panel' }, login_nonce: nonce })
  return String(body.result!.session_token)
}
/** Minimal NFSP client for fixtures and for checking what is really on disk. */
async function nfsp(request: APIRequestContext, token: string) {
  let seq = 0
  const call = async (method: string, body: Record<string, unknown>) => {
    const parsed = await (await request.post(`${origin}/nfs/v1/${method}`, { headers: { Authorization: `Bearer ${token}` }, data: body })).json()
    if (!parsed.ok) throw new Error(`${method}: ${JSON.stringify(parsed.error)}`)
    return parsed.result
  }
  const session = (await call('hello', { args: {} })).session
  return {
    mkdir: (path: string) => call('mkdir', { session, seq: ++seq, args: {}, at: { realm: 'dfs', path } }),
    write: async (path: string, bytes: Buffer) => {
      const parent = await call('stat', { session, args: {}, at: { realm: 'dfs', path: path.slice(0, path.lastIndexOf('/')) } })
      const open = await call('open_write', { session, seq: ++seq, args: { parent_ref: parent.ref, name: path.split('/').pop(), size: bytes.length } })
      await request.fetch(`${origin}/nfs/v1/uploads/${open.fb_handle}`, { method: 'PATCH', headers: { Authorization: `Bearer ${token}`, 'Upload-Offset': '0', 'Tus-Resumable': '1.0.0', 'Content-Type': 'application/offset+octet-stream' }, data: bytes })
      await call('commit_file', { session, seq: ++seq, args: { name: path.split('/').pop(), fb_handle: open.fb_handle, lease_id: open.lease.lease_id }, at: { ref: parent.ref } })
    },
    read: async (path: string) => {
      const info = await call('stat', { session, args: {}, want: ['base', 'ident'], at: { realm: 'dfs', path } })
      return (await request.get(`${origin}/nfs/v1/read/${encodeURIComponent(info.node_id ?? info.ref.node_id)}`, { headers: { Authorization: `Bearer ${token}` } })).text()
    },
    remove: (parent: string, name: string) => call('delete', { session, seq: ++seq, args: { name, recursive: true }, at: { realm: 'dfs', path: parent } }).catch(() => {}),
  }
}
async function desktop(browser: Browser): Promise<{ context: BrowserContext; page: Page }> {
  const context = await browser.newContext({ ignoreHTTPSErrors: true, viewport: { width: 1440, height: 1000 } })
  const page = await context.newPage()
  await page.goto(`${origin}/`)
  await page.getByLabel('Username', { exact: true }).fill(owner)
  await page.getByLabel('Password', { exact: true }).fill(password)
  await page.getByRole('button', { name: 'Sign In', exact: true }).click()
  await expect(page.getByRole('button', { name: 'BuckyOS' }).first()).toBeVisible({ timeout: 30_000 })
  return { context, page }
}
const editorWindow = (page: Page, name: string) => page.locator('[data-testid^="window-"]').filter({ hasText: name }).first()
async function openInDesktop(page: Page, name: string) {
  await page.goto(`${origin}/?open=${encodeURIComponent(`cyfs://${dir}/${name}`)}`)
  await expect(editorWindow(page, name)).toBeVisible({ timeout: 60_000 })
  const editor = page.frameLocator('[data-testid="web-app-frame"]')
  await expect(editor.locator('.cm-content')).toBeVisible({ timeout: 30_000 })
  return editor
}

let fs: Awaited<ReturnType<typeof nfsp>>
let api: APIRequestContext
test.beforeAll(async () => {
  api = await playwrightRequest.newContext({ ignoreHTTPSErrors: true })
  fs = await nfsp(api, await apiToken(api))
  await fs.mkdir(dir)
  await fs.write(`${dir}/notes.md`, Buffer.from('# DV notes\n\nfirst line\n'))
  await fs.write(`${dir}/pixel.png`, PNG)
  await fs.write(`${dir}/blob.bin`, Buffer.from([0, 1, 2, 3, 0, 5]))
})
test.afterAll(async () => { await fs.remove(`/home/${owner}`, run); await api.dispose() })
/** The desktop opens the editor in a normal window, narrow enough that its file list starts collapsed. */
async function fileList(editor: FrameLocator | Page) {
  const list = editor.getByRole('listbox', { name: 'Files' })
  if (!await list.isVisible()) await editor.locator('footer').getByRole('button', { name: 'Files', exact: true }).click()
  await expect(list).toBeVisible()
  return list
}

test('HTML entry points revalidate and reject stale epoch validators before loading current assets', async () => {
  const source = `cyfs://${dir}/notes.md`
  const scripts = new Set<string>()
  let currentHtml = ''
  for (const path of ['/', '/index.html', `/?src=${encodeURIComponent(source)}`]) {
    const fresh = await api.get(`${editorOrigin}${path}`)
    expect(fresh.status(), path).toBe(200)
    const html = await fresh.text()
    if (!currentHtml) currentHtml = html
    expect(html, path).toBe(currentHtml)
    expect.soft(fresh.headers()['cache-control'], `${path} cache policy`).toBe('no-cache')
    const oldEtag = `"${Buffer.byteLength(html).toString(16)}-0-0"`
    const staleValidators: Record<string, string>[] = [
      { 'If-None-Match': oldEtag },
      { 'If-Modified-Since': 'Thu, 01 Jan 1970 00:00:00 GMT' },
      { 'If-None-Match': oldEtag, 'If-Modified-Since': 'Thu, 01 Jan 1970 00:00:00 GMT' },
    ]
    for (const headers of staleValidators) {
      const response = await api.get(`${editorOrigin}${path}`, { headers })
      expect(response.status(), `${path} with ${JSON.stringify(headers)}`).toBe(200)
      expect(await response.text(), path).toBe(currentHtml)
      expect.soft(response.headers()['cache-control'], `${path} conditional cache policy`).toBe('no-cache')
    }
    for (const match of html.matchAll(/<script\b[^>]*\bsrc="([^"]+)"/g)) scripts.add(new URL(match[1], editorOrigin).href)
  }
  expect(scripts.size).toBeGreaterThan(0)
  for (const script of scripts) {
    const response = await api.get(script)
    expect(response.status(), script).toBe(200)
    expect(response.headers()['content-type'], script).toMatch(/javascript/)
  }
})

test('the Text Editor icon and Files double-click both load with the browser cache enabled', async ({ browser }) => {
  const { context, page } = await desktop(browser)
  context.setDefaultTimeout(15_000)
  const failedAssets: string[] = []
  const loadedScripts: string[] = []
  context.on('response', response => {
    if (!response.url().startsWith(`${editorOrigin}/assets/`)) return
    if (response.status() >= 400) failedAssets.push(`${response.status()} ${response.url()}`)
    if (response.request().resourceType() === 'script') loadedScripts.push(response.url())
  })
  context.on('requestfailed', request => {
    if (request.url().startsWith(`${editorOrigin}/assets/`)) failedAssets.push(`${request.url()} ${request.failure()?.errorText}`)
  })
  try {
    for (let i = 0; i < 2; i++) {
      const icon = page.getByTestId(`desktop-app-${editorApp}`)
      if (await icon.isVisible()) await icon.click()
      else await openFromAllApps(page, editorApp)
      const editor = page.frameLocator('[data-testid="web-app-frame"]')
      await expect(editor.locator('.welcome h1')).toHaveText(/^(Text Editor|文本编辑器)$/, { timeout: 30_000 })
      const frame = page.frames().find(frame => frame.url().startsWith(`${editorOrigin}/`))!
      expect(new URL(frame.url()).pathname).toBe('/')
      expect(new URL(frame.url()).search).toBe('')
      await editorWindow(page, 'Text Editor').getByRole('button', { name: /^(Close|关闭)$/ }).first().click()
      await expect(page.getByTestId('web-app-frame')).toHaveCount(0)
    }

    const tab = await context.newPage()
    await tab.goto(`${editorOrigin}/`)
    await expect(tab.locator('.welcome h1')).toHaveText(/^(Text Editor|文本编辑器)$/, { timeout: 30_000 })
    await tab.close()

    const filesIcon = page.getByTestId('desktop-app-files')
    if (await filesIcon.isVisible()) await filesIcon.click()
    else await openFromAllApps(page, 'files')
    const files = page.getByTestId('window-files')
    await files.getByTestId('filebrowser').focus()
    await page.keyboard.press('Control+l')
    const path = files.getByRole('textbox', { name: 'path', exact: true })
    await path.fill(dir)
    await path.press('Enter')
    await expect(files.locator('[data-pane="left"]')).toHaveAttribute('data-location', `dfs://${dir}`)
    await expect(files.locator('[data-pane="left"]')).toHaveAttribute('data-list-status', 'ready')
    await files.getByRole('cell', { name: 'notes.md', exact: true }).dblclick()
    await expect(page.frameLocator('[data-testid="web-app-frame"]').locator('.cm-content')).toContainText('DV notes', { timeout: 30_000 })
    const frame = page.frames().find(frame => frame.url().startsWith(`${editorOrigin}/`))!
    expect(new URL(frame.url()).pathname).toBe('/')
    expect(new URL(frame.url()).searchParams.has('src')).toBe(true)
    expect(loadedScripts.length).toBeGreaterThan(0)
    expect(failedAssets).toEqual([])
  } finally {
    await context.close()
  }
})

test('an idle editor window stays quiet, and closing answers in time without a force-close prompt', async ({ browser }) => {
  const { context, page } = await desktop(browser)
  const dialogs: string[] = []; page.on('dialog', d => { dialogs.push(d.message()); void d.dismiss() })
  let nfs = 0; page.on('request', r => { if (r.url().startsWith(`${editorOrigin}/nfs/v1/`)) nfs++ })
  await openInDesktop(page, 'notes.md')
  await page.waitForTimeout(2000)
  const before = nfs
  await page.waitForTimeout(10_000)
  // v0.3: ~650 requests a second from the focus/title loop. Now: periodic disk checks and the watch stream only.
  expect(nfs - before, 'NFSP requests in 10 s of idle').toBeLessThan(30)
  await editorWindow(page, 'notes.md').getByRole('button', { name: 'Close' }).first().click()
  await expect(editorWindow(page, 'notes.md')).toHaveCount(0, { timeout: 3000 })
  expect(dialogs).toEqual([])
  await context.close()
})

test('saving is quick and reliable, and Save as creates the missing folders', async ({ browser }) => {
  const { context, page } = await desktop(browser)
  const failed: string[] = []; page.on('requestfailed', r => failed.push(`${r.url()} ${r.failure()?.errorText}`))
  const editor = await openInDesktop(page, 'notes.md')
  await editor.locator('.cm-content').click()
  for (let i = 0; i < 3; i++) {
    await editor.locator('.cm-content').click()
    await page.keyboard.press('Control+End'); await page.keyboard.insertText(`\nsave ${i}`)
    const started = Date.now()
    await page.keyboard.press('Control+s')
    await expect(editor.getByRole('status')).toHaveText(/^Saved/, { timeout: 15_000 })
    expect(Date.now() - started, `save ${i} duration`).toBeLessThan(5000)
    expect(await fs.read(`${dir}/notes.md`)).toContain(`save ${i}`)
    await editor.getByRole('status').getByRole('button').click()
  }
  await editor.getByRole('button', { name: 'Actions', exact: true }).first().click()
  await editor.getByRole('menuitem', { name: 'Save as…', exact: true }).click()
  await editor.getByLabel('Location', { exact: true }).fill(`cyfs://${dir}/new/deeper`)
  await expect(editor.getByRole('dialog').getByRole('note')).toContainText('will be created')
  await editor.getByLabel('File name', { exact: true }).fill('copy.md')
  await editor.getByRole('button', { name: 'Choose', exact: true }).click()
  await expect(editor.locator('.disk-state')).toHaveText('Saved', { timeout: 15_000 })
  expect(await fs.read(`${dir}/new/deeper/copy.md`)).toContain('save 2')
  expect(failed.filter(f => f.includes('INSUFFICIENT_RESOURCES'))).toEqual([])
  await context.close()
})

test('content that is not text opens inline in the system Preview, and Open with goes to the shell chooser without the editor', async ({ browser }) => {
  const { context, page } = await desktop(browser)
  const logins: string[] = []
  page.on('framenavigated', f => { if (f !== page.mainFrame() && /\/login(\?|$)/.test(new URL(f.url()).pathname + new URL(f.url()).search.slice(0, 1))) logins.push(f.url()) })
  const editor = await openInDesktop(page, 'notes.md')
  const files = await fileList(editor)
  await files.getByRole('option', { name: 'pixel.png' }).click()
  const embed = editor.frameLocator('.preview-pane iframe')
  await expect(embed.getByTestId('content-preview')).toHaveAttribute('data-status', 'ready', { timeout: 30_000 })
  await expect(embed.getByTestId('content-preview')).toHaveAttribute('data-renderer', 'image')
  await files.getByRole('option', { name: 'blob.bin' }).click()
  await expect(editor.locator('.preview-pane')).toHaveAttribute('data-status', /ready|error/, { timeout: 30_000 })
  await expect(page.getByTestId('window-preview')).toHaveCount(0)
  expect(logins).toEqual([])
  await files.getByRole('option', { name: 'pixel.png' }).click()
  await embed.getByTestId('content-preview').hover()
  await embed.getByTestId('content-preview-open-with').click()
  const sheet = page.getByTestId('preview-open-with')
  await expect(sheet).toBeVisible()
  await expect(sheet.getByRole('button', { name: 'Preview', exact: true })).toBeVisible()
  await expect(sheet.getByRole('button', { name: /Text Editor|文本编辑器/ })).toHaveCount(0)
  await sheet.getByRole('button', { name: 'Preview', exact: true }).click()
  await expect(page.getByTestId('window-preview')).toBeVisible()
  await context.close()
})

test('in a standalone tab, Preview stays inline and Open with goes to the Zone root, never to sys.', async ({ browser, request }) => {
  const { context, page } = await desktop(browser)
  const launch = await kapi(request, 'control-panel', 'auth.prepare_app_launch', { app_instance_id: editorApp, redirect_url: `${editorOrigin}/?src=${encodeURIComponent(`cyfs://${dir}/notes.md`)}` }, await apiToken(request))
  expect(launch.result?.status).toBe('ready')
  const tab = await context.newPage()
  await tab.goto(String(launch.result!.callback_url))
  await expect(tab.locator('.cm-content')).toContainText('DV notes', { timeout: 30_000 })
  await (await fileList(tab)).getByRole('option', { name: 'pixel.png' }).click()
  const embed = tab.frameLocator('.preview-pane iframe')
  await expect(embed.getByTestId('content-preview')).toHaveAttribute('data-status', 'ready', { timeout: 30_000 })
  await embed.getByTestId('content-preview').hover()
  const popup = context.waitForEvent('page')
  await embed.getByTestId('content-preview-open-with').click()
  const chooser = await popup
  await chooser.waitForLoadState()
  const url = new URL(chooser.url())
  expect(url.origin).toBe(origin)
  expect(url.searchParams.get('intent')).toBe('chooser')
  expect(url.searchParams.get('from')).toBe('text-editor.buckyos.bns.did')
  const sheet = chooser.getByTestId('preview-open-with')
  await expect(sheet).toBeVisible({ timeout: 30_000 })
  await expect(sheet.getByRole('button', { name: /Text Editor|文本编辑器/ })).toHaveCount(0)
  await context.close()
  void page
})

test('closing a changed document while the server is unreachable keeps the draft on this device', async ({ browser }) => {
  const { context, page } = await desktop(browser)
  const editor = await openInDesktop(page, 'notes.md')
  await page.route(`${editorOrigin}/nfs/v1/**`, route => route.abort('internetdisconnected'))
  await editor.locator('.cm-content').click(); await page.keyboard.press('Control+End'); await page.keyboard.insertText('\noffline words')
  const started = Date.now()
  await editorWindow(page, 'notes.md').getByRole('button', { name: 'Close' }).first().click()
  const dialog = editor.getByRole('alertdialog')
  await expect(dialog).toContainText('stored on this device but not yet on the server', { timeout: 10_000 })
  expect(Date.now() - started).toBeLessThan(6000)
  await dialog.getByRole('button', { name: 'Keep on this device and close', exact: true }).click()
  await expect(editorWindow(page, 'notes.md')).toHaveCount(0)
  await page.unroute(`${editorOrigin}/nfs/v1/**`)
  const again = await openInDesktop(page, 'notes.md')
  await expect(again.locator('.cm-content')).toContainText('offline words', { timeout: 30_000 })
  await context.close()
})
