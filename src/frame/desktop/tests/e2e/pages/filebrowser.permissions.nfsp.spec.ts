/**
 * File permissions against a real nfs_server with token auth (opt-in).
 *
 * The copy fixture is started only to mint verify-hub-style tokens; nfs_server
 * runs with `--auth-test-config` so `copy-test-user` is the zone owner and
 * `copy-other-user` an ordinary user. From `src/`:
 *
 *   cargo build -p nfs_server && cargo build -p task_manager --example nfs_copy_fixture
 *   VITE_NFS_PROXY=http://127.0.0.1:3262 pnpm --dir frame/desktop exec vite --host 127.0.0.1 --port 4173
 *   FB_ACL_E2E=1 pnpm --dir frame/desktop exec playwright test tests/e2e/pages/filebrowser.permissions.nfsp.spec.ts
 */

import { expect, test, type Browser, type Page } from '@playwright/test'
import { spawn, type ChildProcess } from 'node:child_process'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

const target = resolve('../..', process.env.CARGO_TARGET_DIR ?? 'target', 'debug')
const OWNER = 'copy-test-user'
const OTHER = 'copy-other-user'

let root = ''
let ownerToken = ''
let otherToken = ''
const children: ChildProcess[] = []

async function desktopAs(browser: Browser, token: string): Promise<Page> {
  const context = await browser.newContext({ extraHTTPHeaders: { Authorization: `Bearer ${token}` } })
  const page = await context.newPage()
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/?scenario=normal&fbData=nfsp')
  return page
}

async function filesAs(browser: Browser, token: string): Promise<Page> {
  const page = await desktopAs(browser, token)
  await page.getByTestId('desktop-app-files').click()
  return page
}

test.describe('File permissions on real nfs_server', () => {
  test.skip(!process.env.FB_ACL_E2E, 'set FB_ACL_E2E=1 with built binaries and the proxy on 3262')
  test.describe.configure({ mode: 'serial' })

  test.beforeAll(async () => {
    root = await mkdtemp(join(tmpdir(), 'buckyos-acl-e2e-'))
    for (const dir of ['task', 'data', `home/${OWNER}/shared`, `home/${OTHER}`]) await mkdir(join(root, dir), { recursive: true })
    await writeFile(join(root, `home/${OWNER}/shared/report.txt`), 'shared report\n')
    await writeFile(join(root, `home/${OWNER}/private.txt`), 'owner only\n')
    await writeFile(join(root, `home/${OTHER}/notes.txt`), 'other notes\n')
    children.push(spawn(join(target, 'examples/nfs_copy_fixture'), [join(root, 'task'), '3383'], { stdio: 'ignore' }))
    await expect.poll(() => readFile(join(root, 'task/other-token'), 'utf8').catch(() => ''), { timeout: 30000 }).not.toBe('')
    ownerToken = await readFile(join(root, 'task/user-token'), 'utf8')
    otherToken = await readFile(join(root, 'task/other-token'), 'utf8')
    const { public_key } = JSON.parse(await readFile(join(root, 'task/nfs-copy-test.json'), 'utf8')) as { public_key: string }
    await writeFile(join(root, 'auth.json'), JSON.stringify({ public_key, zone_owner: OWNER }))
    children.push(spawn(join(target, 'nfs_server'), ['--listen', '127.0.0.1:3262', '--data-dir', join(root, 'data'), '--export', `home=${join(root, 'home')}`, '--debug-api', '--auth-test-config', join(root, 'auth.json')], { stdio: 'ignore' }))
    await expect.poll(async () => {
      try { return (await fetch('http://127.0.0.1:3262/nfs/v1/hello', { method: 'POST', body: '{"args":{}}' })).ok } catch { return false }
    }, { timeout: 30000 }).toBe(true)
  })

  test.afterAll(async () => {
    for (const child of children) child.kill('SIGKILL')
    await rm(root, { recursive: true, force: true })
  })

  test('another user sees only their home until the owner shares a folder', async ({ browser }) => {
    let page = await filesAs(browser, otherToken)
    let win = page.getByTestId('window-files')
    await expect(win.getByRole('cell', { name: OTHER, exact: true })).toBeVisible({ timeout: 15000 })
    await expect(win.getByRole('cell', { name: OWNER, exact: true })).toHaveCount(0)
    await page.context().close()

    page = await filesAs(browser, ownerToken)
    win = page.getByTestId('window-files')
    await win.getByRole('cell', { name: OWNER, exact: true }).dblclick()
    await win.getByRole('cell', { name: 'shared', exact: true }).click({ button: 'right' })
    await page.getByRole('menuitem', { name: 'Permissions…', exact: true }).click()
    const dialog = page.getByTestId('permissions-dialog')
    await expect(dialog.getByTestId('permissions-own-access')).toContainText('Full control')
    await expect(dialog.getByTestId(`acl-builtin-user:${OWNER}`)).toContainText('Zone owner')
    await dialog.getByTestId('acl-add-principal').fill(OTHER)
    await dialog.getByTestId('acl-add').click()
    await dialog.getByTestId('acl-save').click()
    await expect(dialog).toHaveCount(0)
    await page.context().close()

    page = await filesAs(browser, otherToken)
    win = page.getByTestId('window-files')
    await win.getByRole('cell', { name: OWNER, exact: true }).dblclick()
    await expect(win.getByRole('cell', { name: 'shared', exact: true })).toBeVisible()
    await expect(win.getByRole('cell', { name: 'private.txt', exact: true })).toHaveCount(0)
    await win.getByRole('cell', { name: 'shared', exact: true }).dblclick()
    await win.getByRole('cell', { name: 'report.txt', exact: true }).click({ button: 'right' })
    await expect(page.getByRole('menuitem', { name: 'Rename…' })).toBeDisabled()
    await page.getByRole('menuitem', { name: 'Permissions…', exact: true }).click()
    await expect(page.getByTestId('permissions-own-access')).toContainText('Read')
    await expect(page.getByTestId('permissions-dialog')).toContainText('Only people with Full control')
    await page.context().close()
  })

  test('the zone owner sees every user\'s permissions in Settings', async ({ browser }) => {
    const page = await desktopAs(browser, ownerToken)
    await page.getByTestId('desktop-app-settings').click()
    const win = page.getByTestId('window-settings')
    await win.getByRole('button', { name: 'File Permissions', exact: true }).click()
    const record = win.getByTestId(`file-permission-/home/${OWNER}/shared`)
    await expect(record).toContainText(`${OTHER} · Read`)
    await expect(win.getByTestId(`file-permissions-group-${OWNER}`)).toBeVisible()
  })
})
