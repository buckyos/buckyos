/* A directory's subdirectories are part of that directory (UI 框架 PRD Q25, §7.4, §15 #20 #21): a new one is a
 * `folder` member — no directory entity of its own, nothing new in the object tree —, it can be empty and hold more
 * folders, and renaming, moving into and out of it and removing it (with everything in it, said before it is
 * confirmed) change only that directory's members, each in one Commit that one undo takes back. Also Q26: the table's
 * ⋯ has no "import data" any more; table data is imported from the object tree (and the main menu, the insert
 * catalogue). */

import { devices, type Locator, type Page } from '@playwright/test'
import { expect, openWorkspace, openWorkspacePage, test, treeMenu, type Api } from './fixtures'

const ALICE = 'tok-alice'
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

const member = (scope: Page | Locator, name: string) => scope.locator(`[data-testid="aiws-directory-member"][data-name="${name}"]`)
const membersOf = async (api: Api, workspaceId: string, id = 'files') => (await api.read(ALICE, workspaceId, id)).content.payload.members as Record<string, Json>
const namesOf = async (api: Api, workspaceId: string, id = 'files') => Object.keys(await membersOf(api, workspaceId, id)).sort()
const directoriesOf = async (api: Api, workspaceId: string) => (await api.outline(ALICE, workspaceId)).filter((e: Json) => e.type_id === 'buckyos.dir' && !e.deleted).map((e: Json) => e.entity_id)

/** A flow page with a directory Block `资料`: two files and a folder of names `notes/` (no `folder` member). */
async function workspace(api: Api, title: string) {
  const ws = await api.rpc(ALICE, 'ws.create', { title }) as { workspace_id: string; epoch: string }
  const a = await api.uploadAsset(ALICE, ws.workspace_id, Buffer.from('file a'))
  const b = await api.uploadAsset(ALICE, ws.workspace_id, Buffer.from('file b'))
  const made = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'page-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'a', payload: { kind: 'folder', system: 'surface_content', surface_id: 'page', title: 'Page' } },
    { op: 'entity.create', entity_id: 'page', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'a', payload: { kind: 'surface', layout: { mode: 'flow' }, title: 'Page', content_folder_id: 'page-content' } },
    { op: 'entity.create', entity_id: 'files', type_id: 'buckyos.dir', parent_id: 'page-content', order_key: 'a', name: '资料', payload: { members: {
      'a.txt': { kind: 'file', object_id: a.object_id }, 'b.txt': { kind: 'file', object_id: b.object_id }, 'notes/x.txt': { kind: 'file', object_id: a.object_id },
    } } },
    { op: 'entity.create', entity_id: 'cell-files', type_id: 'buckyos.cell', parent_id: 'page', order_key: 'a', payload: { view: { type: 'directory' }, source_ref: { entity_id: 'files' }, title: '资料' } },
  ])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  return ws
}

async function dataMode(page: Page) {
  if (await page.getByTestId('aiws-top-sources').isVisible()) await page.getByTestId('aiws-top-sources').click()
  await expect(page.getByTestId('aiws-sources')).toBeVisible()
}

async function rowAction(page: Page, editor: Locator, name: string, action: string) {
  await member(editor, name).getByTestId('aiws-directory-row-menu').click()
  await page.getByTestId('aiws-directory-row-actions').getByTestId(action).click()
}

async function newFolder(page: Page, open: () => Promise<unknown>, name: string) {
  await open()
  await page.getByTestId('aiws-directory-name').fill(name)
  await page.getByTestId('aiws-directory-save').click()
}

test('DIR-25 (Q25, §15 #21) a subdirectory is part of its directory: empty, several levels, renamed, moved in and out, removed whole, undone', async ({ page, api }) => {
  test.setTimeout(150_000)
  const ws = await workspace(api, `dir-25 ${Date.now()}`)
  await openWorkspacePage(page, ALICE, ws.workspace_id, 'page')
  const editor = page.getByTestId('aiws-cell-frame-cell-files').getByTestId('aiws-directory')
  await expect(member(editor, 'a.txt')).toBeVisible()
  const undo = async (names: string[]) => {
    await page.getByTestId('aiws-undo').click()
    await expect.poll(() => namesOf(api, ws.workspace_id)).toEqual(names)
  }

  // a new subdirectory: an empty folder of this directory, no entity of its own
  await newFolder(page, async () => { await editor.getByTestId('aiws-directory-upload').click(); await page.getByTestId('aiws-directory-add-folder').click() }, '归档')
  await expect(member(editor, '归档')).toHaveAttribute('data-folder', 'true')
  await expect(member(editor, '归档')).toContainText('0 项')
  expect((await membersOf(api, ws.workspace_id))['归档']).toEqual({ kind: 'folder' })
  expect(await directoriesOf(api, ws.workspace_id)).toEqual(['files'])
  // folders first (归档 and the folder of names notes), then the files
  const rows = await editor.getByTestId('aiws-directory-member').evaluateAll((items) => items.map((row) => row.getAttribute('data-name') ?? ''))
  expect([rows.slice(0, 2).sort(), rows.slice(2)]).toEqual([['notes', '归档'], ['a.txt', 'b.txt']])

  // into it (empty: the add entries), one level more, and into that
  await member(editor, '归档').getByTestId('aiws-directory-open').click()
  await expect(editor.getByTestId('aiws-directory-crumb')).toHaveText(['资料', '归档'])
  await expect(editor.getByTestId('aiws-directory-empty')).toBeVisible()
  await newFolder(page, () => editor.getByTestId('aiws-directory-empty-folder').click(), '2026')
  await expect.poll(() => namesOf(api, ws.workspace_id)).toEqual(['a.txt', 'b.txt', 'notes/x.txt', '归档', '归档/2026'])
  await member(editor, '2026').getByTestId('aiws-directory-open').click()
  await expect(editor.getByTestId('aiws-directory-crumb')).toHaveText(['资料', '归档', '2026'])
  await expect(editor.getByTestId('aiws-directory-empty')).toBeVisible()
  await editor.locator('[data-testid="aiws-directory-crumb"][data-depth="0"]').click()
  await expect(member(editor, '归档')).toContainText('1 项')

  // a file moved into the second level; one undo takes it back
  await rowAction(page, editor, 'a.txt', 'aiws-directory-move')
  const move = page.getByTestId('aiws-directory-move-dialog')
  await move.locator('[data-testid="aiws-directory-move-folder"][data-name="归档"]').click()
  await move.locator('[data-testid="aiws-directory-move-folder"][data-name="2026"]').click()
  await move.getByTestId('aiws-directory-move-here').click()
  await expect(member(editor, 'a.txt')).toHaveCount(0)
  await expect.poll(() => namesOf(api, ws.workspace_id)).toEqual(['b.txt', 'notes/x.txt', '归档', '归档/2026', '归档/2026/a.txt'])
  await undo(['a.txt', 'b.txt', 'notes/x.txt', '归档', '归档/2026'])
  await expect(member(editor, 'a.txt')).toBeVisible()
  await member(editor, 'b.txt').dragTo(member(editor, '归档'))
  await expect.poll(() => namesOf(api, ws.workspace_id)).toEqual(['a.txt', 'notes/x.txt', '归档', '归档/2026', '归档/b.txt'])

  // renamed: the folder, what is in it and the folder inside it move together, in one Commit
  await rowAction(page, editor, '归档', 'aiws-directory-rename')
  await page.getByTestId('aiws-directory-name').fill('存档')
  await page.getByTestId('aiws-directory-save').click()
  await expect(member(editor, '存档')).toBeVisible()
  await expect.poll(() => namesOf(api, ws.workspace_id)).toEqual(['a.txt', 'notes/x.txt', '存档', '存档/2026', '存档/b.txt'])
  await undo(['a.txt', 'notes/x.txt', '归档', '归档/2026', '归档/b.txt'])
  await expect(member(editor, '归档')).toBeVisible()

  // moved out again: the folder it leaves stays, empty
  await member(editor, '归档').getByTestId('aiws-directory-open').click()
  await rowAction(page, editor, 'b.txt', 'aiws-directory-move')
  await move.locator('[data-testid="aiws-directory-move-top"]').click()
  await move.locator('[data-testid="aiws-directory-move-root"][data-entity-id="files"]').click()
  await move.getByTestId('aiws-directory-move-here').click()
  await expect.poll(() => namesOf(api, ws.workspace_id)).toEqual(['a.txt', 'b.txt', 'notes/x.txt', '归档', '归档/2026'])
  await expect(editor.getByTestId('aiws-directory-crumb')).toHaveText(['资料', '归档'])
  await expect(member(editor, '2026')).toBeVisible()
  // a folder of names keeps existing when its last file goes (a `folder` member takes its place)
  await editor.locator('[data-testid="aiws-directory-crumb"][data-depth="0"]').click()
  await member(editor, 'notes').getByTestId('aiws-directory-open').click()
  await rowAction(page, editor, 'x.txt', 'aiws-directory-remove')
  await page.getByTestId('aiws-directory-confirm-ok').click()
  await expect.poll(() => namesOf(api, ws.workspace_id)).toEqual(['a.txt', 'b.txt', 'notes', '归档', '归档/2026'])
  await expect(editor.getByTestId('aiws-directory-crumb')).toHaveText(['资料', 'notes'])
  await expect(editor.getByTestId('aiws-directory-empty')).toBeVisible()
  await editor.locator('[data-testid="aiws-directory-crumb"][data-depth="0"]').click()

  // removed whole: the confirmation says how much goes; one undo brings all of it back
  await member(editor, 'a.txt').dragTo(member(editor, '归档'))
  await expect.poll(() => namesOf(api, ws.workspace_id)).toEqual(['b.txt', 'notes', '归档', '归档/2026', '归档/a.txt'])
  await rowAction(page, editor, '归档', 'aiws-directory-remove')
  const confirm = page.getByTestId('aiws-directory-confirm')
  await expect(confirm).toContainText('1 个文件、2 个文件夹')
  await confirm.getByTestId('aiws-directory-confirm-ok').click()
  await expect(member(editor, '归档')).toHaveCount(0)
  await expect.poll(() => namesOf(api, ws.workspace_id)).toEqual(['b.txt', 'notes'])
  await undo(['b.txt', 'notes', '归档', '归档/2026', '归档/a.txt'])
  await expect(member(editor, '归档')).toContainText('2 项')
  // nothing of it ever became a directory of its own
  expect(await directoriesOf(api, ws.workspace_id)).toEqual(['files'])
})

test('DIR-26 (Q25, §15 #21) the data mode: a new subdirectory is not a new object of the tree; the editor opens it and comes back', async ({ page, api }) => {
  const ws = await api.freeCanvas(ALICE, `dir-26 ${Date.now()}`)
  const a = await api.uploadAsset(ALICE, ws.workspace_id, Buffer.from('file a'))
  const made = await api.commit(ALICE, ws, [{ op: 'entity.create', entity_id: 'files', type_id: 'buckyos.dir', parent_id: 'data', order_key: 'a', name: '资料', payload: { members: { 'b.txt': { kind: 'file', object_id: a.object_id } } } }])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  await openWorkspace(page, ALICE, ws.workspace_id)
  await dataMode(page)
  await page.getByTestId('aiws-tree-files').click()
  const editor = page.getByTestId('aiws-detail-files').getByTestId('aiws-directory')
  await expect(member(editor, 'b.txt')).toBeVisible()
  const treeItems = await page.getByTestId('aiws-tree-item').count()
  await newFolder(page, async () => { await editor.getByTestId('aiws-directory-upload').click(); await page.getByTestId('aiws-directory-add-folder').click() }, '照片')
  await expect(member(editor, '照片')).toHaveAttribute('data-folder', 'true')
  await expect(page.getByTestId('aiws-tree-item')).toHaveCount(treeItems)
  expect(await directoriesOf(api, ws.workspace_id)).toEqual(['files'])
  await member(editor, '照片').getByTestId('aiws-directory-open').click()
  await expect(editor.getByTestId('aiws-directory-crumb')).toHaveText(['资料', '照片'])
  await editor.getByTestId('aiws-directory-up').click()
  await expect(member(editor, 'b.txt')).toBeVisible()
})

test('TBL-26 (Q26) the table\'s ⋯ has no "import data"; table data is still imported from the object tree', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `tbl-26 ${Date.now()}`)
  await openWorkspace(page, ALICE, ws.workspace_id)
  const all = page.getByTestId('aiws-table-cell-all-tasks')
  await all.getByTestId('aiws-table-menu-cell-all-tasks').click()
  const menu = page.getByTestId('aiws-table-menu-pop-cell-all-tasks')
  await expect(menu.getByTestId('aiws-ctl-menu-table-copy')).toBeVisible()
  await expect(menu.getByTestId('aiws-ctl-menu-table-import')).toHaveCount(0)
  await expect(menu).not.toContainText('导入')
  await page.keyboard.press('Escape')
  await dataMode(page)
  await treeMenu(page, null, 'aiws-dm-menu-import')
  await expect(page.getByTestId('aiws-datatree-import-table')).toBeVisible()
})

test.describe('on a phone', () => {
// eslint-disable-next-line @typescript-eslint/no-unused-vars -- the browser comes from the project, the rest is the phone
const { defaultBrowserType, ...pixel } = devices['Pixel 7']
test.use(pixel)

test('DIR-27 (Q25, §15 #20) a phone removes a whole subdirectory by tapping: the sheet says what goes', async ({ page, api }) => {
  const ws = await workspace(api, `dir-27 ${Date.now()}`)
  await openWorkspacePage(page, ALICE, ws.workspace_id, 'page')
  await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-phone', 'true')
  const editor = page.getByTestId('aiws-cell-frame-cell-files').getByTestId('aiws-directory')
  await member(editor, 'notes').getByTestId('aiws-directory-row-menu').tap()
  await page.getByTestId('aiws-directory-row-actions').getByTestId('aiws-directory-remove').tap()
  await expect(page.getByTestId('aiws-directory-confirm')).toContainText('1 个文件、1 个文件夹')
  await page.getByTestId('aiws-directory-confirm-ok').tap()
  await expect(member(editor, 'notes')).toHaveCount(0)
  await expect.poll(() => namesOf(api, ws.workspace_id)).toEqual(['a.txt', 'b.txt'])
})
})
