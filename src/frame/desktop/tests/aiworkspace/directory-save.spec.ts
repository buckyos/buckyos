/* "保存到 Zone" of a directory's files (UI 框架 PRD §7.4, needs `export`; the Zone is the user's main file system): a
 * row's ⋯ saves one file through the system file picker's save mode, the selection bar saves several into the folder
 * chosen in its folder mode — a taken name is never written over —, and downloading to this device stays the other
 * entry. Without `export` both are disabled and say why. */

import type { Locator, Page } from '@playwright/test'
import { expect, openedPicker, openWorkspace, test, zoneGoto, type Api } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'

const member = (scope: Page | Locator, name: string) => scope.locator(`[data-testid="aiws-directory-member"][data-name="${name}"]`)

/** A flow page with a directory Block of two text files. */
async function workspace(api: Api, title: string) {
  const ws = await api.rpc(ALICE, 'ws.create', { title }) as { workspace_id: string; epoch: string }
  const a = await api.uploadAsset(ALICE, ws.workspace_id, Buffer.from('file a'))
  const b = await api.uploadAsset(ALICE, ws.workspace_id, Buffer.from('file b'))
  const made = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'page-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'a', payload: { kind: 'folder', system: 'surface_content', surface_id: 'page', title: 'Page' } },
    { op: 'entity.create', entity_id: 'page', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'a', payload: { kind: 'surface', layout: { mode: 'flow' }, title: 'Page', content_folder_id: 'page-content' } },
    { op: 'entity.create', entity_id: 'files', type_id: 'buckyos.dir', parent_id: 'page-content', order_key: 'a', name: '资料', payload: { members: { 'a.txt': { kind: 'file', object_id: a.object_id }, 'b.txt': { kind: 'file', object_id: b.object_id } } } },
    { op: 'entity.create', entity_id: 'cell-files', type_id: 'buckyos.cell', parent_id: 'page', order_key: 'a', payload: { view: { type: 'directory' }, source_ref: { entity_id: 'files' }, title: '资料' } },
  ])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  return ws
}

test('§7.4 目录“保存到 Zone”：行菜单保存一个文件（保存模式），多选保存到所选文件夹、同名不覆盖；下载是次要入口；没有 export 时禁用并说明', async ({ page, api }) => {
  test.setTimeout(150_000)
  const ws = await workspace(api, `dir-save ${Date.now()}`)
  await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: 'bob', resource: '/', allow: ['read'] }] })
  await openWorkspace(page, ALICE, ws.workspace_id)
  const editor = page.getByTestId('aiws-cell-frame-cell-files').getByTestId('aiws-directory')
  await expect(member(editor, 'a.txt')).toBeVisible()

  // one file: its ⋯ has 保存到 Zone first, 下载 after it
  await member(editor, 'a.txt').getByTestId('aiws-directory-row-menu').click()
  const actions = page.getByTestId('aiws-directory-row-actions')
  const ids = await actions.locator('[data-testid^="aiws-directory-"]').evaluateAll((els) => els.map((el) => el.getAttribute('data-testid')))
  expect(ids.indexOf('aiws-directory-save-zone')).toBeGreaterThanOrEqual(0)
  expect(ids.indexOf('aiws-directory-save-zone')).toBeLessThan(ids.indexOf('aiws-directory-download'))
  await actions.getByTestId('aiws-directory-save-zone').click()
  const save = (await openedPicker(page)).dialog
  await zoneGoto(save, ['Documents'])
  await expect(save.getByTestId('select-name')).toHaveValue('a.txt')
  await save.getByTestId('select-confirm').click()
  await expect(page.getByTestId('aiws-notice').filter({ hasText: '已保存到 /home/Documents/a.txt' })).toBeVisible({ timeout: 30_000 })

  // several: the selection bar, into the folder chosen; a.txt is taken there, so it becomes "a (2).txt"
  await member(editor, 'a.txt').getByTestId('aiws-directory-row-menu').click()
  await actions.getByTestId('aiws-directory-select').click()
  await member(editor, 'b.txt').getByTestId('aiws-directory-open').click()
  const bar = editor.getByTestId('aiws-directory-selection')
  await expect(bar).toContainText('2')
  await bar.getByTestId('aiws-directory-bulk-save-zone').click()
  const folder = (await openedPicker(page)).dialog
  await zoneGoto(folder, ['Documents'])
  await folder.getByTestId('select-confirm').click()
  await expect(page.getByTestId('aiws-notice').filter({ hasText: '已把 2 个文件保存到 /home/Documents' })).toBeVisible({ timeout: 30_000 })
  await expect(bar).toHaveCount(0)
  // the folder now has the first a.txt, its copy and b.txt
  await member(editor, 'b.txt').getByTestId('aiws-directory-row-menu').click()
  await actions.getByTestId('aiws-directory-save-zone').click()
  const check = (await openedPicker(page)).dialog
  await zoneGoto(check, ['Documents'])
  for (const name of ['a.txt', 'a (2).txt', 'b.txt']) await expect(check.locator('[data-testid="select-list"]').getByText(name, { exact: true })).toBeVisible()
  await page.keyboard.press('Escape')
})

test('§7.4 没有 export：目录的“保存到 Zone”和“下载”都禁用并说明', async ({ page, api }) => {
  const ws = await workspace(api, `dir-save-ro ${Date.now()}`)
  await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: 'bob', resource: '/', allow: ['read'] }] })
  await openWorkspace(page, BOB, ws.workspace_id)
  const editor = page.getByTestId('aiws-cell-frame-cell-files').getByTestId('aiws-directory')
  await member(editor, 'a.txt').getByTestId('aiws-directory-row-menu').click()
  const actions = page.getByTestId('aiws-directory-row-actions')
  for (const id of ['aiws-directory-save-zone', 'aiws-directory-download']) {
    await expect(actions.getByTestId(id)).toBeDisabled()
  }
  await expect(actions).toContainText('导出')
})
