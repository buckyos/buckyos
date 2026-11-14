/* The directory's standard editor and the directory Block (UI framework PRD §7.4; acceptance §15 #20 directory half,
 * #21): one editor in the data mode, in a Block on a canvas and on a flow page; on an emulated Pixel 7 everything is
 * done by tapping (no hover, right click or keyboard shortcut), files come from the Zone first. Also §7.5: an asset's
 * replacement is chosen in the Zone picker. */

import { devices, type Locator, type Page } from '@playwright/test'
import { chooseFromZone, expect, openCanvas, openWorkspace, openWorkspacePage, test, type Api } from './fixtures'

const ALICE = 'tok-alice'
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any
const PNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAABAAAAAJCAIAAAC0SDtlAAAAFElEQVR42mPQiDpBEmIY1TAoNAAAk1q5oVLxFXwAAAAASUVORK5CYII=', 'base64')

const member = (scope: Page | Locator, name: string) => scope.locator(`[data-testid="aiws-directory-member"][data-name="${name}"]`)
const membersOf = async (api: Api, workspaceId: string, id: string) => (await api.read(ALICE, workspaceId, id)).content.payload.members as Record<string, Json>

/** A directory `photos`: a picture, a child directory `nested` (one file) and a folder of names `docs/`. */
async function directories(api: Api, ws: { workspace_id: string; epoch: string }, parent = 'data') {
  const photo = await api.uploadAsset(ALICE, ws.workspace_id, PNG)
  const note = await api.uploadAsset(ALICE, ws.workspace_id, Buffer.from('inner note'))
  const made = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'nested', type_id: 'buckyos.dir', parent_id: parent, order_key: 'a', name: 'Nested', payload: { members: { 'inner.txt': { kind: 'file', object_id: note.object_id } } } },
    { op: 'entity.create', entity_id: 'photos', type_id: 'buckyos.dir', parent_id: parent, order_key: 'b', name: '照片', payload: { members: {
      'photo.png': { kind: 'file', object_id: photo.object_id },
      'nested': { kind: 'dir', entity_id: 'nested' },
      'docs/a.txt': { kind: 'file', object_id: note.object_id },
    } } },
  ])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  return { photo, note }
}

test('DIR-21 (§15 #21) the data mode opens a directory from the object tree, goes into a subdirectory and a folder, and back', async ({ page, api }) => {
  const ws = await api.freeCanvas(ALICE, `dir-21 ${Date.now()}`)
  await directories(api, ws)
  await openWorkspace(page, ALICE, ws.workspace_id)
  // existing directory data can be shown on a canvas: the catalog's "existing data" makes a directory Block of it
  await page.getByTestId('aiws-tool-catalog').click()
  await page.getByTestId('aiws-catalog-tab-existing').click()
  await page.getByTestId('aiws-picker-data').selectOption('photos')
  await page.getByTestId('aiws-picker-confirm').click()
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).filter((e: Json) => e.view_type === 'directory' && e.source_id === 'photos' && !e.deleted).length).toBe(1)

  await page.getByTestId('aiws-top-sources').click()
  await page.getByTestId('aiws-tree-photos').click()
  const editor = page.getByTestId('aiws-detail-photos').getByTestId('aiws-directory')
  await expect(editor).toHaveAttribute('data-size', 'tab')
  // folders and subdirectories first, then files
  await expect(editor.getByTestId('aiws-directory-member')).toHaveCount(3)
  expect(await editor.getByTestId('aiws-directory-member').evaluateAll((rows) => rows.map((row) => row.getAttribute('data-name')))).toEqual(['docs', 'nested', 'photo.png'])

  // the subdirectory opens in the same editor; the breadcrumb goes back
  await member(editor, 'nested').getByTestId('aiws-directory-open').click()
  await expect(page.getByTestId('aiws-detail-photos')).toBeVisible()
  await expect(editor.getByTestId('aiws-directory-crumb')).toHaveText(['照片', 'nested'])
  await expect(member(editor, 'inner.txt')).toBeVisible()
  await editor.locator('[data-testid="aiws-directory-crumb"][data-depth="0"]').click()
  await expect(member(editor, 'photo.png')).toBeVisible()

  // a folder of names (`docs/…`) too; "up one level" returns
  await member(editor, 'docs').getByTestId('aiws-directory-open').click()
  await expect(editor.getByTestId('aiws-directory-crumb')).toHaveText(['照片', 'docs'])
  await expect(member(editor, 'a.txt')).toBeVisible()
  await editor.getByTestId('aiws-directory-up').click()
  await expect(editor.getByTestId('aiws-directory-member')).toHaveCount(3)

  // a file previews beside the list; the list can show thumbnails
  await member(editor, 'photo.png').getByTestId('aiws-directory-open').click()
  await expect(editor.getByTestId('aiws-directory-preview')).toHaveAttribute('data-name', 'photo.png')
  await editor.getByTestId('aiws-directory-preview').getByTestId('content-preview-exit').click()
  await expect(editor.getByTestId('aiws-directory-preview')).toHaveCount(0)
  await editor.getByTestId('aiws-directory-view').click()
  await expect(editor).toHaveAttribute('data-view', 'grid')
  await expect(member(editor, 'photo.png').locator('img')).toBeVisible()
})

test('DIR-02 a directory Block from the insert catalog on a free canvas: add from this device, same names, size limit, cancel an upload, subdirectory', async ({ page, api }) => {
  test.setTimeout(120_000)
  const ws = await api.freeCanvas(ALICE, `dir-02 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  // not a standing toolbar button: the catalog lists it under data
  await expect(page.getByTestId('aiws-object-toolbar').getByTestId('aiws-tool-insert-directory')).toHaveCount(0)
  await page.getByTestId('aiws-tool-catalog').click()
  await page.getByTestId('aiws-catalog-search').fill('目录')
  await page.getByTestId('aiws-catalog-entry-block-directory').click()
  await page.getByTestId('aiws-catalog-title').fill('资料')
  await page.getByTestId('aiws-catalog-insert').click()
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).filter((e: Json) => e.view_type === 'directory' && !e.deleted).length).toBe(1)
  const cell = (await api.outline(ALICE, ws.workspace_id)).find((e: Json) => e.view_type === 'directory')
  const dirId = cell.source_id as string
  expect((await api.outline(ALICE, ws.workspace_id)).find((e: Json) => e.entity_id === dirId)?.type_id).toBe('buckyos.dir')

  // selected after insertion: the near toolbar's "add" opens the editor with its add menu
  await page.getByTestId('aiws-near-directory-add').click()
  await expect(page.getByTestId('aiws-directory-add-menu')).toBeVisible()
  await expect(page.locator('[data-testid="aiws-directory"][data-variant="edit"]')).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('aiws-directory-empty')).toBeVisible()

  // this device (the secondary entry): two files
  const fromDevice = async (open: () => Promise<unknown>, files: { name: string; mimeType: string; buffer: Buffer }[]) => {
    const chooser = page.waitForEvent('filechooser')
    await open()
    await (await chooser).setFiles(files)
  }
  await fromDevice(() => page.getByTestId('aiws-directory-empty-device').click(), [
    { name: 'a.txt', mimeType: 'text/plain', buffer: Buffer.from('first a') },
    { name: 'b.txt', mimeType: 'text/plain', buffer: Buffer.from('first b') },
  ])
  await expect.poll(async () => Object.keys(await membersOf(api, ws.workspace_id, dirId)).sort()).toEqual(['a.txt', 'b.txt'])
  const editor = page.locator('[data-testid="aiws-directory"][data-variant="edit"]')
  await expect(member(editor, 'a.txt')).toBeVisible()

  // the same names again: replace one, keep both for the other
  const firstA = (await membersOf(api, ws.workspace_id, dirId))['a.txt'].object_id
  await fromDevice(async () => { await editor.getByTestId('aiws-directory-upload').click(); await page.getByTestId('aiws-directory-add-device').click() }, [
    { name: 'a.txt', mimeType: 'text/plain', buffer: Buffer.from('second a') },
    { name: 'b.txt', mimeType: 'text/plain', buffer: Buffer.from('second b') },
  ])
  const conflict = page.getByTestId('aiws-directory-conflict')
  await expect(conflict).toContainText('a.txt')
  await conflict.getByTestId('aiws-directory-conflict-replace').click()
  await expect(conflict).toContainText('b.txt')
  await conflict.getByTestId('aiws-directory-conflict-keep').click()
  await expect.poll(async () => Object.keys(await membersOf(api, ws.workspace_id, dirId)).sort()).toEqual(['a.txt', 'b (2).txt', 'b.txt'])
  expect((await membersOf(api, ws.workspace_id, dirId))['a.txt'].object_id).not.toBe(firstA)

  // over the limit: the upload says which limit, nothing is added
  await fromDevice(async () => { await editor.getByTestId('aiws-directory-upload').click(); await page.getByTestId('aiws-directory-add-device').click() },
    [{ name: 'big.bin', mimeType: 'application/octet-stream', buffer: Buffer.alloc(33 * 1024 * 1024) }])
  const big = editor.locator('[data-testid="aiws-directory-upload-job"][data-name="big.bin"]')
  await expect(big).toHaveAttribute('data-state', 'failed')
  await expect(big.getByTestId('aiws-directory-upload-status')).toContainText('32 MiB')
  await big.getByTestId('aiws-directory-upload-dismiss').click()
  await expect(big).toHaveCount(0)

  // an upload held on the wire shows its row; cancelling it adds nothing
  let release = () => {}
  const held = new Promise<void>((resolve) => { release = resolve })
  await page.route(/\/ndm\/v1\/uploads/, async (route) => { await held; await route.continue().catch(() => undefined) })
  await fromDevice(async () => { await editor.getByTestId('aiws-directory-upload').click(); await page.getByTestId('aiws-directory-add-device').click() },
    [{ name: 'slow.txt', mimeType: 'text/plain', buffer: Buffer.from('slow upload') }])
  const slow = editor.locator('[data-testid="aiws-directory-upload-job"][data-name="slow.txt"]')
  await expect(slow).toHaveAttribute('data-state', 'sending')
  await slow.getByTestId('aiws-directory-upload-cancel').click()
  release()
  await expect(slow).toHaveAttribute('data-state', 'cancelled')
  await page.unroute(/\/ndm\/v1\/uploads/)
  await page.waitForTimeout(500)
  expect(Object.keys(await membersOf(api, ws.workspace_id, dirId))).not.toContain('slow.txt')

  // a new subdirectory is a directory of its own, linked here; a file moves into it by dragging its row onto it
  await editor.getByTestId('aiws-directory-upload').click()
  await page.getByTestId('aiws-directory-add-folder').click()
  await page.getByTestId('aiws-directory-name').fill('归档')
  await page.getByTestId('aiws-directory-save').click()
  await expect(member(editor, '归档')).toHaveAttribute('data-kind', 'dir')
  const archive = (await membersOf(api, ws.workspace_id, dirId))['归档']
  expect(archive.kind).toBe('dir')
  await member(editor, 'b (2).txt').dragTo(member(editor, '归档'))
  await expect.poll(async () => Object.keys(await membersOf(api, ws.workspace_id, archive.entity_id))).toEqual(['b (2).txt'])
  expect(Object.keys(await membersOf(api, ws.workspace_id, dirId))).not.toContain('b (2).txt')
})

test('DIR-03 a member\'s link opens its folder and previews it in the directory Block', async ({ page, context, api }) => {
  const ws = await api.rpc(ALICE, 'ws.create', { title: `dir-03 ${Date.now()}` }) as { workspace_id: string; epoch: string }
  const note = await api.uploadAsset(ALICE, ws.workspace_id, Buffer.from('linked note'))
  const made = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'page-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'a', payload: { kind: 'folder', system: 'surface_content', surface_id: 'page', title: 'Page' } },
    { op: 'entity.create', entity_id: 'page', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'a', payload: { kind: 'surface', layout: { mode: 'flow' }, title: 'Page', content_folder_id: 'page-content' } },
    { op: 'entity.create', entity_id: 'linked', type_id: 'buckyos.dir', parent_id: 'page-content', order_key: 'a', name: '资料', payload: { members: { 'notes/week.txt': { kind: 'file', object_id: note.object_id }, 'top.txt': { kind: 'file', object_id: note.object_id } } } },
    { op: 'entity.create', entity_id: 'cell-linked', type_id: 'buckyos.cell', parent_id: 'page', order_key: 'a', payload: { view: { type: 'directory' }, source_ref: { entity_id: 'linked' }, title: '资料' } },
  ])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  await openWorkspacePage(page, ALICE, ws.workspace_id, 'page')
  await context.grantPermissions(['clipboard-read', 'clipboard-write'], { origin: new URL(page.url()).origin })
  const editor = page.getByTestId('aiws-cell-frame-cell-linked').getByTestId('aiws-directory')
  await member(editor, 'notes').getByTestId('aiws-directory-open').click()
  await member(editor, 'week.txt').getByTestId('aiws-directory-row-menu').click()
  await page.getByTestId('aiws-directory-row-actions').getByTestId('aiws-directory-copy-link').click()
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toContain('record=')
  const link = await page.evaluate(() => navigator.clipboard.readText())
  expect(new URL(link).searchParams.get('record')).toBe('notes/week.txt')
  expect(new URL(link).searchParams.get('block')).toBe('cell-linked')
  await page.goto('about:blank')
  await page.goto(link)
  await expect(editor.getByTestId('aiws-directory-crumb')).toHaveText(['资料', 'notes'])
  await expect(editor.getByTestId('aiws-directory-preview')).toHaveAttribute('data-name', 'week.txt')
})

test.describe('on a phone', () => {
// eslint-disable-next-line @typescript-eslint/no-unused-vars -- the browser comes from the project, the rest is the phone
const { defaultBrowserType, ...pixel } = devices['Pixel 7']
test.use(pixel)

test('DIR-20 (§15 #20, directory) a phone adds from the Zone, renames, moves and removes on a flow page by tapping only', async ({ page, context, api }) => {
  test.setTimeout(150_000)
  const ws = await api.rpc(ALICE, 'ws.create', { title: `dir-20 ${Date.now()}` }) as { workspace_id: string; epoch: string }
  const made = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'page-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'a', payload: { kind: 'folder', system: 'surface_content', surface_id: 'page', title: 'Page' } },
    { op: 'entity.create', entity_id: 'page', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'a', payload: { kind: 'surface', layout: { mode: 'flow' }, title: '日记', content_folder_id: 'page-content' } },
    { op: 'entity.create', entity_id: 'phone-dir', type_id: 'buckyos.dir', parent_id: 'page-content', order_key: 'a', name: '资料', payload: { members: {} } },
    { op: 'entity.create', entity_id: 'cell-dir', type_id: 'buckyos.cell', parent_id: 'page', order_key: 'a', payload: { view: { type: 'directory' }, source_ref: { entity_id: 'phone-dir' }, title: '资料' } },
  ])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  await openWorkspacePage(page, ALICE, ws.workspace_id, 'page')
  await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-phone', 'true')
  await expect(page.getByTestId('aiws-flow')).toHaveAttribute('data-mode', 'edit')
  const editor = page.getByTestId('aiws-cell-frame-cell-dir').getByTestId('aiws-directory')
  await expect(editor).toHaveAttribute('data-variant', 'edit')
  const viewport = page.viewportSize()!

  // fingers' targets are at least 44 × 44, and the editor fits the screen
  const big = async (locator: Locator) => { const b = (await locator.boundingBox())!; expect(b.width, await locator.getAttribute('data-testid') ?? '').toBeGreaterThanOrEqual(44); expect(b.height).toBeGreaterThanOrEqual(44) }
  await big(editor.getByTestId('aiws-directory-upload'))
  await big(editor.getByTestId('aiws-directory-more'))
  expect(await editor.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true)

  // ＋ Add → from the Zone (the first entry): two files
  await editor.getByTestId('aiws-directory-upload').tap()
  const add = page.getByTestId('aiws-directory-add-menu')
  await expect(add.getByTestId('aiws-directory-add-zone')).toBeVisible()
  await big(add.getByTestId('aiws-directory-add-zone'))
  const sheet = (await add.boundingBox())!
  expect(sheet.x).toBeGreaterThanOrEqual(0)
  expect(sheet.x + sheet.width).toBeLessThanOrEqual(viewport.width + 1)
  await chooseFromZone(page, () => add.getByTestId('aiws-directory-add-zone').tap(), [
    { name: '发票.png', mimeType: 'image/png', buffer: PNG },
    { name: '清单.txt', mimeType: 'text/plain', buffer: Buffer.from('购物清单') },
  ])
  await expect.poll(async () => Object.keys(await membersOf(api, ws.workspace_id, 'phone-dir')).sort()).toEqual(['发票.png', '清单.txt'].sort())
  await expect(member(editor, '发票.png')).toBeVisible()

  // rename from the row's ⋯
  await big(member(editor, '清单.txt').getByTestId('aiws-directory-row-menu'))
  await member(editor, '清单.txt').getByTestId('aiws-directory-row-menu').tap()
  const actions = page.getByTestId('aiws-directory-row-actions')
  await big(actions.getByTestId('aiws-directory-rename'))
  await actions.getByTestId('aiws-directory-rename').tap()
  await page.getByTestId('aiws-directory-name').fill('周末清单.txt')
  await page.getByTestId('aiws-directory-save').tap()
  await expect(member(editor, '周末清单.txt')).toBeVisible()
  await expect.poll(async () => Object.keys(await membersOf(api, ws.workspace_id, 'phone-dir')).sort()).toEqual(['周末清单.txt', '发票.png'].sort())

  // a subdirectory, then "move to…" into it
  await editor.getByTestId('aiws-directory-upload').tap()
  await page.getByTestId('aiws-directory-add-folder').tap()
  await page.getByTestId('aiws-directory-name').fill('票据')
  await page.getByTestId('aiws-directory-save').tap()
  await expect(member(editor, '票据')).toHaveAttribute('data-kind', 'dir')
  const sub = (await membersOf(api, ws.workspace_id, 'phone-dir'))['票据'].entity_id as string
  await member(editor, '发票.png').getByTestId('aiws-directory-row-menu').tap()
  await actions.getByTestId('aiws-directory-move').tap()
  const move = page.getByTestId('aiws-directory-move-dialog')
  await expect(move.getByTestId('aiws-directory-move-here')).toBeDisabled()
  await move.locator('[data-testid="aiws-directory-move-folder"][data-name="票据"]').tap()
  await move.getByTestId('aiws-directory-move-here').tap()
  await expect(member(editor, '发票.png')).toHaveCount(0)
  await expect.poll(async () => Object.keys(await membersOf(api, ws.workspace_id, sub))).toEqual(['发票.png'])

  // into the subdirectory and back by the breadcrumb
  await member(editor, '票据').getByTestId('aiws-directory-open').tap()
  await expect(member(editor, '发票.png')).toBeVisible()
  // a file previews over the whole screen on a phone
  await member(editor, '发票.png').getByTestId('aiws-directory-open').tap()
  const preview = page.getByTestId('aiws-directory-preview')
  await expect(preview).toBeVisible()
  const shown = (await preview.boundingBox())!
  expect(shown.width).toBeGreaterThanOrEqual(viewport.width - 1)
  await preview.getByTestId('content-preview-exit').tap()
  await expect(preview).toHaveCount(0)
  await editor.getByTestId('aiws-directory-up').tap()
  await expect(member(editor, '周末清单.txt')).toBeVisible()

  // remove asks first
  await member(editor, '周末清单.txt').getByTestId('aiws-directory-row-menu').tap()
  await actions.getByTestId('aiws-directory-remove').tap()
  await page.getByTestId('aiws-directory-confirm-ok').tap()
  await expect(member(editor, '周末清单.txt')).toHaveCount(0)
  await expect.poll(async () => Object.keys(await membersOf(api, ws.workspace_id, 'phone-dir'))).toEqual(['票据'])

  // a long press selects (several rows, one bar of actions)
  const cdp = await context.newCDPSession(page)
  const row = (await member(editor, '票据').getByTestId('aiws-directory-open').boundingBox())!
  const at = { x: row.x + row.width / 2, y: row.y + row.height / 2 }
  await cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: at.x, y: at.y, id: 0 }] })
  await page.waitForTimeout(800)
  await cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] })
  await expect(editor.getByTestId('aiws-directory-selection')).toBeVisible()
  await expect(member(editor, '票据')).toHaveAttribute('data-selected', 'true')
  await big(editor.getByTestId('aiws-directory-bulk-remove'))
  await editor.getByTestId('aiws-directory-bulk-cancel').tap()
  await expect(editor.getByTestId('aiws-directory-selection')).toHaveCount(0)
})
})

test('DIR-05 (§7.5) an asset in the data mode is replaced with a file chosen in the Zone', async ({ page, api }) => {
  const ws = await api.freeCanvas(ALICE, `dir-05 ${Date.now()}`)
  const photo = await api.uploadAsset(ALICE, ws.workspace_id, PNG)
  const made = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'pic', type_id: 'buckyos.asset-ref', parent_id: 'data', order_key: 'a', payload: { object_id: photo.object_id, file_name: '封面.png' } },
  ])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  await openWorkspace(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-top-sources').click()
  await page.getByTestId('aiws-tree-pic').click()
  const replacement = Buffer.concat([PNG, Buffer.from([1, 2, 3])])
  await chooseFromZone(page, () => page.getByTestId('aiws-asset-replace-file').click(), { name: '新封面.png', mimeType: 'image/png', buffer: replacement })
  await expect.poll(async () => (await api.read(ALICE, ws.workspace_id, 'pic')).content.payload.object_id).not.toBe(photo.object_id)
  const id = (await api.read(ALICE, ws.workspace_id, 'pic')).content.payload.object_id
  const bytes = await fetch(`${api.base}/asset/${ws.workspace_id}/${id}`, { headers: { authorization: `Bearer ${ALICE}` } })
  expect(Buffer.from(await bytes.arrayBuffer())).toEqual(replacement)
})
