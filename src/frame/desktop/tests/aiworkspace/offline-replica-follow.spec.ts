/* The window holding the offline replica, online (UI 框架 PRD §7.4, §8; 18.3): a directory upload reports its
 * progress and a cancel stops the transfer itself, as in an online window; and the permissions follow changes made
 * elsewhere at once — the replica's long poll wakes on `acl_rev` (权限需求 R4), no polling. */

import type { Locator, Page } from '@playwright/test'
import { expect, prepareOffline, test, type Api } from './offline-fixtures'
import { openPermissions } from './fixtures'

const ALICE = 'tok-alice'

const member = (scope: Page | Locator, name: string) => scope.locator(`[data-testid="aiws-directory-member"][data-name="${name}"]`)

/** A flow page with a directory Block `资料` holding one file. */
async function workspace(api: Api, title: string) {
  const ws = await api.rpc(ALICE, 'ws.create', { title }) as { workspace_id: string; epoch: string }
  const a = await api.uploadAsset(ALICE, ws.workspace_id, Buffer.from('file a'))
  const made = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'page-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'a', payload: { kind: 'folder', system: 'surface_content', surface_id: 'page', title: 'Page' } },
    { op: 'entity.create', entity_id: 'page', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'a', payload: { kind: 'surface', layout: { mode: 'flow' }, title: 'Page', content_folder_id: 'page-content' } },
    { op: 'entity.create', entity_id: 'files', type_id: 'buckyos.dir', parent_id: 'page-content', order_key: 'a', name: '资料', payload: { members: { 'a.txt': { kind: 'file', object_id: a.object_id } } } },
    { op: 'entity.create', entity_id: 'cell-files', type_id: 'buckyos.cell', parent_id: 'page', order_key: 'a', payload: { view: { type: 'directory' }, source_ref: { entity_id: 'files' }, title: '资料' } },
  ])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  return ws
}

async function closeStatus(page: Page) {
  if (await page.getByTestId('aiws-status-detail').isVisible()) await page.getByTestId('aiws-status').click()
  await expect(page.getByTestId('aiws-status-detail')).toHaveCount(0)
}

async function fromDevice(page: Page, editor: Locator, files: { name: string; mimeType: string; buffer: Buffer }[]) {
  const chooser = page.waitForEvent('filechooser')
  await editor.getByTestId('aiws-directory-upload').click()
  await page.getByTestId('aiws-directory-add-device').click()
  await (await chooser).setFiles(files)
}

test('OFF-R1 (§7.4) a replica window\'s directory upload shows its progress; cancelling stops the transfer; the next one is added', async ({ page, context, api, net }) => {
  test.setTimeout(150_000)
  const ws = await workspace(api, `off-r1 ${Date.now()}`)
  await prepareOffline(page, net, ALICE, ws.workspace_id)
  await closeStatus(page)
  const editor = page.getByTestId('aiws-cell-frame-cell-files').getByTestId('aiws-directory')
  await expect(member(editor, 'a.txt')).toBeVisible()
  await expect(page.getByTestId('aiws-mode')).toHaveAttribute('data-mode', 'replica')

  // a slow link: 4 MiB at 256 KiB/s take about 16 s
  const cdp = await context.newCDPSession(page)
  await cdp.send('Network.enable')
  await cdp.send('Network.emulateNetworkConditions', { offline: false, latency: 0, downloadThroughput: -1, uploadThroughput: 256 * 1024 })
  await fromDevice(page, editor, [{ name: 'big.txt', mimeType: 'text/plain', buffer: Buffer.alloc(4 * 1024 * 1024, 'x') }])
  const big = editor.locator('[data-testid="aiws-directory-upload-job"][data-name="big.txt"]')
  // the bytes sent reach the row (a percentage, not just "preparing")
  await expect(big.getByTestId('aiws-directory-upload-status')).toHaveText(/\d+%/, { timeout: 30_000 })
  // cancelled: the transfer itself stops, long before it would have finished
  await big.getByTestId('aiws-directory-upload-cancel').click()
  await expect(big).toHaveAttribute('data-state', 'cancelled', { timeout: 4_000 })
  await cdp.send('Network.emulateNetworkConditions', { offline: false, latency: 0, downloadThroughput: -1, uploadThroughput: -1 })
  expect(Object.keys((await api.read(ALICE, ws.workspace_id, 'files')).content.payload.members)).toEqual(['a.txt'])

  await fromDevice(page, editor, [{ name: 'small.txt', mimeType: 'text/plain', buffer: Buffer.from('small') }])
  await expect(member(editor, 'small.txt')).toBeVisible()
  await expect.poll(async () => Object.keys((await api.read(ALICE, ws.workspace_id, 'files')).content.payload.members).sort()).toEqual(['a.txt', 'small.txt'])
})

test('OFF-R2 (§8, 权限需求 R4) a replica window follows a permission change made elsewhere at once, without polling', async ({ page, api, net }) => {
  const ws = await workspace(api, `off-r2 ${Date.now()}`)
  await prepareOffline(page, net, ALICE, ws.workspace_id)
  await closeStatus(page)
  await openPermissions(page)
  const editor = page.getByTestId('aiws-permissions').first()
  await editor.getByTestId('aiws-perm-tab-rules').click()
  const bob = editor.locator('[data-testid="aiws-perm-rule"][data-subject="bob"]')
  await expect(bob).toHaveCount(0)
  await expect(page.getByTestId('aiws-mode')).toHaveAttribute('data-mode', 'replica')
  // somebody else (here: the API) grants bob reading: the open editor shows it well within the old 15 s poll
  const granted = await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: 'bob', resource: '/', allow: ['read'] }] })
  expect(granted.ok, JSON.stringify(granted)).toBe(true)
  await expect(bob).toHaveCount(1, { timeout: 8_000 })
  await expect(bob.getByTestId('aiws-perm-rule-allow').locator('[data-cap="read"]')).toBeVisible()
  // and taken back
  await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: 'bob', resource: '/', allow: [], deny: [] }] })
  await expect(bob).toHaveCount(0, { timeout: 8_000 })
})
