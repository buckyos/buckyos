/**
 * "My files" and the computer (NFSP `host` realm) against a real nfs_server with
 * token auth (opt-in). The copy fixture mints verify-hub-style tokens, including
 * BuckyOS sudo tokens for the zone owner (`sudo-token`: 30 minutes,
 * `sudo-token-short`: 20 s from the fixture's start); nfs_server runs with
 * `--host-root` as a stand-in for the Desktop Lite computer. Verify Hub itself
 * is not running, so its `sudo_by_password` answer is served by the test with
 * the fixture's token. From `src/`:
 *
 *   cargo build -p nfs_server && cargo build -p task_manager --example nfs_copy_fixture
 *   VITE_NFS_PROXY=http://127.0.0.1:3262 pnpm --dir frame/desktop exec vite --host 127.0.0.1 --port 4173
 *   FB_HOST_E2E=1 pnpm --dir frame/desktop exec playwright test tests/e2e/pages/filebrowser.host.nfsp.spec.ts
 */

import { expect, test, type Browser, type Page } from '@playwright/test'
import { spawn, type ChildProcess } from 'node:child_process'
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

const target = resolve('../..', process.env.CARGO_TARGET_DIR ?? 'target', 'debug')
const OWNER = 'copy-test-user'
const OTHER = 'copy-other-user'

let root = ''
let ownerToken = ''
let otherToken = ''
let sudoToken = ''
const children: ChildProcess[] = []

async function filesAs(browser: Browser, token: string, width = 1440): Promise<{ page: Page; sudoRequests: Record<string, unknown>[] }> {
  const context = await browser.newContext({ viewport: { width, height: 900 } })
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])
  // The user's session for requests without their own token (the sudo session sends its own Bearer).
  await context.route(/\/(nfs\/v1|ndm|bucket\/v1)\//, (route) => {
    const headers = route.request().headers()
    return route.continue({ headers: headers.authorization ? headers : { ...headers, authorization: `Bearer ${token}` } })
  })
  const page = await context.newPage()
  const sudoRequests: Record<string, unknown>[] = []
  await page.route('**/kapi/verify-hub', async (route) => {
    const body = route.request().postDataJSON() as { method?: string; params?: Record<string, unknown> }
    if (body.method !== 'sudo_by_password') return route.continue()
    sudoRequests.push(body.params ?? {})
    await route.fulfill({ json: { result: { session_token: sudoToken } } })
  })
  await page.goto('/?scenario=normal&fbData=nfsp')
  await page.getByTestId('desktop-app-files').click()
  return { page, sudoRequests }
}

test.describe('Files on real nfs_server: My files and the computer', () => {
  test.skip(!process.env.FB_HOST_E2E, 'set FB_HOST_E2E=1 with built binaries and the proxy on 3262')
  test.describe.configure({ mode: 'serial' })

  test.beforeAll(async () => {
    root = await mkdtemp(join(tmpdir(), 'buckyos-host-e2e-'))
    for (const dir of ['task', 'data', 'named-store', `home/${OWNER}/Documents`, `home/${OTHER}`, 'computer/Users/alice/Documents', 'computer/System']) await mkdir(join(root, dir), { recursive: true })
    await writeFile(join(root, `home/${OWNER}/Documents/zone.txt`), 'zone bytes\n')
    await writeFile(join(root, 'computer/Users/alice/Documents/My Report.txt'), 'computer bytes\n')
    await chmod(join(root, 'computer/System'), 0o000)
    children.push(spawn(join(target, 'examples/nfs_copy_fixture'), [join(root, 'task'), '3384'], { stdio: 'ignore' }))
    await expect.poll(() => readFile(join(root, 'task/other-token'), 'utf8').catch(() => ''), { timeout: 30000 }).not.toBe('')
    ownerToken = await readFile(join(root, 'task/user-token'), 'utf8')
    otherToken = await readFile(join(root, 'task/other-token'), 'utf8')
    sudoToken = await readFile(join(root, 'task/sudo-token'), 'utf8')
    const { public_key } = JSON.parse(await readFile(join(root, 'task/nfs-copy-test.json'), 'utf8')) as { public_key: string }
    await writeFile(join(root, 'auth.json'), JSON.stringify({ public_key, zone_owner: OWNER }))
    children.push(spawn(join(target, 'nfs_server'), [
      '--listen', '127.0.0.1:3262', '--data-dir', join(root, 'data'), '--export', `home=${join(root, 'home')}`,
      '--debug-api', '--auth-test-config', join(root, 'auth.json'), '--copy-test-config', join(root, 'task/nfs-copy-test.json'),
      '--named-store', join(root, 'named-store'), '--host-root', join(root, 'computer'), '--host-label', 'Test PC',
    ], { stdio: 'ignore' }))
    await expect.poll(async () => {
      try { return (await fetch('http://127.0.0.1:3262/nfs/v1/hello', { method: 'POST', body: '{"args":{}}' })).ok } catch { return false }
    }, { timeout: 30000 }).toBe(true)
  })

  test.afterAll(async () => {
    for (const child of children) child.kill('SIGKILL')
    await chmod(join(root, 'computer/System'), 0o755).catch(() => undefined)
    await rm(root, { recursive: true, force: true })
  })

  test('My files is the signed-in user\'s own home; an ordinary user sees no computer', async ({ browser }) => {
    let { page } = await filesAs(browser, ownerToken)
    let win = page.getByTestId('window-files')
    await expect(win.locator('main[data-pane="left"]')).toHaveAttribute('data-location', `dfs:///home/${OWNER}`)
    await expect(win.getByRole('tab', { name: 'My files', exact: true })).toBeVisible()
    await expect(win.getByTestId('sidebar-unlock-host')).toContainText('Test PC')
    await page.context().close()
    ;({ page } = await filesAs(browser, otherToken))
    win = page.getByTestId('window-files')
    await expect(win.locator('main[data-pane="left"]')).toHaveAttribute('data-location', `dfs:///home/${OTHER}`)
    await expect(win.getByRole('cell', { name: OWNER, exact: true })).toHaveCount(0)
    await expect(win.getByTestId('sidebar-unlock-host')).toHaveCount(0)
    await page.context().close()
  })

  test('the owner unlocks the computer with a 30-minute sudo token, browses it and copies a file into My files', async ({ browser }) => {
    const { page, sudoRequests } = await filesAs(browser, ownerToken)
    const win = page.getByTestId('window-files')
    // A normal session cannot list the computer.
    const denied = await page.evaluate(async (token) => {
      const call = (method: string, body: unknown) => fetch(`/nfs/v1/${method}`, { method: 'POST', headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' }, body: JSON.stringify(body) }).then((r) => r.json())
      const hello = await call('hello', { args: {} }) as { result: { session: string } }
      return call('list', { session: hello.result.session, at: { realm: 'host', path: '/' }, args: {} })
    }, ownerToken) as { ok: boolean; error?: { code: string } }
    expect(denied.ok).toBe(false)
    expect(denied.error?.code).toBe('SUDO_REQUIRED')

    await win.getByTestId('sidebar-unlock-host').click()
    await page.getByRole('textbox', { name: 'Password' }).fill('secret')
    await page.keyboard.press('Enter')
    await expect(win.getByRole('tab', { name: 'Test PC', exact: true })).toBeVisible()
    expect(sudoRequests[0]).toMatchObject({ aud: 'nfs-server', expires_in_secs: 1800 })
    const aside = win.locator('aside')
    await expect(aside.getByRole('button', { name: 'System', exact: true })).toBeDisabled()
    await aside.getByRole('button', { name: 'Expand Users' }).click()
    await aside.getByRole('button', { name: 'Expand alice' }).click()
    await aside.getByRole('button', { name: 'Documents', exact: true }).last().click()
    await expect(win.getByText('file:///Users/alice/Documents', { exact: true })).toBeVisible()
    const report = win.getByRole('cell', { name: 'My Report.txt', exact: true })
    await report.click({ button: 'right' })
    await page.getByRole('menuitem', { name: 'Copy path' }).click()
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe('file:///Users/alice/Documents/My%20Report.txt')
    await report.click({ button: 'right' })
    await expect(page.getByRole('menuitem', { name: 'Share…' })).toHaveCount(0)
    await page.getByRole('menuitem', { name: 'Copy files' }).click()
    await aside.getByRole('button', { name: 'My files', exact: true }).click()
    await win.getByRole('cell', { name: 'Documents', exact: true }).dblclick()
    await win.getByTestId('filebrowser').focus()
    await page.keyboard.press('Control+v')
    await expect(win.getByTestId('batch-results')).toContainText('1 succeeded', { timeout: 30000 })
    expect(await readFile(join(root, `home/${OWNER}/Documents/My Report.txt`), 'utf8')).toBe('computer bytes\n')
    await page.context().close()
  })
})
