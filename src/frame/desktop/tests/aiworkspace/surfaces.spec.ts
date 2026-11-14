/* UI 框架 PRD §4.1.1 (K1, Q5, Q9) and §15 scenario 4: canvas folders are real folders under `surfaces`, a canvas's place
 * is its entity path, and the canvas picker finds, opens and arranges hundreds of canvases — on a computer and on a
 * phone (an emulated Pixel 7, as mobile.spec.ts does). */

import { devices, type Page } from '@playwright/test'
import { core, expect, openWorkspacePage, test, type Api } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'
const L1 = ['日记', '项目', '归档']
const L2 = ['Q1', 'Q2', 'Q3', 'Q4']
const L3 = ['W1', 'W2', 'W3', 'W4', 'W5']

// The backend's JSON results are inspected ad hoc.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

/** The ids of the tree: folders `d1-a`, `d2-a-q`, `d3-a-q-w`; canvases `c-a-q-w-n` titled `第<k>页` (k = 1…300). */
const canvasId = (a: number, q: number, w: number, n: number) => `c-${a}-${q}-${w}-${n}`
const pageNo = (a: number, q: number, w: number, n: number) => a * 100 + q * 25 + w * 5 + n + 1

/** 300 canvases three folders deep (3 × 4 × 5 folders, five flow pages in each of the deepest), one Commit. */
async function bigTree(api: Api, title: string): Promise<{ workspace_id: string; epoch: string }> {
  const ws = await api.rpc(ALICE, 'ws.create', { title }) as { workspace_id: string; epoch: string }
  let k: string | undefined
  const key = () => (k = core().order_key_between(k, undefined))
  const folder = (id: string, parent: string, t: string) => ({ op: 'entity.create', entity_id: id, type_id: 'buckyos.container', parent_id: parent, order_key: key(), payload: { kind: 'folder', title: t } })
  const ops: Json[] = []
  L1.forEach((t1, a) => {
    ops.push(folder(`d1-${a}`, 'surfaces', t1))
    L2.forEach((t2, q) => {
      ops.push(folder(`d2-${a}-${q}`, `d1-${a}`, t2))
      L3.forEach((t3, w) => {
        ops.push(folder(`d3-${a}-${q}-${w}`, `d2-${a}-${q}`, t3))
        for (let n = 0; n < 5; n += 1) {
          const id = canvasId(a, q, w, n)
          const t = `第${pageNo(a, q, w, n)}页`
          ops.push({ op: 'entity.create', entity_id: `${id}-content`, type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: key(), payload: { kind: 'folder', system: 'surface_content', surface_id: id, title: t } })
          ops.push({ op: 'entity.create', entity_id: id, type_id: 'buckyos.container', parent_id: `d3-${a}-${q}-${w}`, order_key: key(), payload: { kind: 'surface', layout: { mode: 'flow' }, title: t, content_folder_id: `${id}-content` } })
        }
      })
    })
  })
  const made = await api.commit(ALICE, ws, ops)
  expect(made.status, JSON.stringify(made).slice(0, 400)).toBe('accepted')
  return ws
}

async function outlineEntry(api: Api, workspaceId: string, id: string): Promise<Json> {
  return (await api.outline(ALICE, workspaceId)).find((e: Json) => e.entity_id === id)
}

/** The canvas a window shows: a free canvas or a flow page. */
function shown(page: Page) {
  return page.locator('[data-testid="aiws-canvas"], [data-testid="aiws-flow"]')
}

async function openPicker(page: Page) {
  await page.getByTestId('aiws-surface-switch').click()
  const list = page.getByTestId('aiws-surface-list')
  await expect(list).toBeVisible()
  return list
}

/** The picker, opened unless it still is (Escape on a row's menu may close both). */
async function pickerShown(page: Page) {
  const list = page.getByTestId('aiws-surface-list')
  return (await list.isVisible()) ? list : openPicker(page)
}

test('§15-4 300 canvases three folders deep: the picker opens, searches and switches; a canvas moved to another folder keeps its link, takes the new path and the new folder\'s grant', async ({ page, api, browser }) => {
  const ws = await bigTree(api, `tree ${Date.now()}`)
  // bob reads the workspace and may edit what is in 归档 / Q4 only
  await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: 'bob', allow: ['read'] }, { subject: 'bob', resource: '/surfaces/归档/Q4', allow: ['read', 'update', 'structure'] }] })
  await openWorkspacePage(page, ALICE, ws.workspace_id)
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
  const canvas = shown(page)
  await expect(canvas).toHaveAttribute('data-surface-id', canvasId(0, 0, 0, 0))

  // opening: quick, and only what is on screen is drawn
  const opened = Date.now()
  const list = await openPicker(page)
  await expect(list.getByTestId(`aiws-surface-item-${canvasId(0, 0, 0, 0)}`)).toBeVisible()
  expect(Date.now() - opened).toBeLessThan(3000)
  await expect(list.getByTestId('aiws-surface-rows')).toHaveAttribute('data-count', String(2 + 1 + 3 + 12 + 60 + 300))
  expect(await list.locator('[data-testid^="aiws-surface-item-"], [data-testid^="aiws-surface-folder-d"]').count()).toBeLessThan(60)
  // more than 20 canvases: the recent ones come first; the current one is marked in the tree
  await expect(list.getByTestId(`aiws-surface-recent-${canvasId(0, 0, 0, 0)}`)).toBeVisible()
  await expect(list.getByTestId(`aiws-surface-item-${canvasId(0, 0, 0, 0)}`)).toHaveClass(/is-active/)
  await expect(list.getByTestId(`aiws-surface-item-${canvasId(0, 0, 0, 0)}`).getByTestId('aiws-surface-mode')).toHaveText('流式')

  // searching the whole tree, whatever is collapsed: flat hits with their folders; Enter opens the first
  await list.getByTestId('aiws-surface-folder-toggle-d1-0').click()
  await expect(list.getByTestId('aiws-surface-folder-d2-0-0')).toHaveCount(0)
  await expect(list.getByTestId('aiws-surface-folder-d1-1')).toBeVisible()
  await list.getByTestId('aiws-surface-search').fill('第3页')
  await expect(list.getByTestId(`aiws-surface-item-${canvasId(0, 0, 0, 2)}`).getByTestId('aiws-surface-trail')).toHaveText('日记 / Q1 / W1')
  await expect(list.locator('[data-testid^="aiws-surface-item-"]')).toHaveCount(1)
  const typed = Date.now()
  await list.getByTestId('aiws-surface-search').fill('第300页')
  const hit = list.getByTestId(`aiws-surface-item-${canvasId(2, 3, 4, 4)}`)
  await expect(hit).toBeVisible()
  expect(Date.now() - typed).toBeLessThan(2000)
  await expect(list.locator('[data-testid^="aiws-surface-item-"]')).toHaveCount(1)
  await expect(hit.getByTestId('aiws-surface-trail')).toHaveText('归档 / Q4 / W5')
  await list.getByTestId('aiws-surface-search').press('Enter')
  await expect(canvas).toHaveAttribute('data-surface-id', canvasId(2, 3, 4, 4))
  await expect(list).toHaveCount(0)
  // no case, folders too (after the canvases); a folder found is revealed in the tree, opening what was collapsed
  const again = await openPicker(page)
  await again.getByTestId('aiws-surface-search').fill('w5')
  await expect(again.getByTestId(`aiws-surface-item-${canvasId(0, 0, 4, 0)}`)).toHaveCount(0)
  await again.getByTestId('aiws-surface-search').fill('归档')
  await expect(again.getByTestId('aiws-surface-folder-d1-2')).toBeVisible()
  await again.getByTestId('aiws-surface-search').press('Enter')
  await expect(again.getByTestId('aiws-surface-search')).toHaveValue('')
  await expect(again.getByTestId('aiws-surface-folder-d2-2-0')).toBeVisible()
  // switching by a row of the tree; the recent list follows
  await again.getByTestId(`aiws-surface-item-${canvasId(2, 0, 0, 1)}`).getByRole('menuitem').click()
  await expect(canvas).toHaveAttribute('data-surface-id', canvasId(2, 0, 0, 1))
  const third = await openPicker(page)
  await expect(third.locator('[data-testid^="aiws-surface-recent-"]').first()).toHaveAttribute('data-testid', `aiws-surface-recent-${canvasId(2, 0, 0, 1)}`)

  // the canvas to move, its link, and bob's rights on it before
  const moved = canvasId(1, 0, 0, 0)
  const before = await api.rpc(BOB, 'doc.read', { workspace_id: ws.workspace_id, entity_id: moved })
  expect(before.ok).toBe(true)
  expect(before.capabilities).not.toContain('update')
  await third.getByTestId('aiws-surface-search').fill(`第${pageNo(1, 0, 0, 0)}页`)
  await third.getByTestId(`aiws-surface-more-${moved}`).click()
  await page.getByTestId(`aiws-surface-move-${moved}`).click()
  await expect(third.getByTestId('aiws-surface-move-view')).toBeVisible()
  await expect(third.getByTestId('aiws-surface-move-to-d3-1-0-0')).toBeDisabled()
  await third.getByTestId('aiws-surface-move-to-d2-2-3').click()
  await expect.poll(async () => (await outlineEntry(api, ws.workspace_id, moved))?.parent_id).toBe('d2-2-3')
  // its path is the new folder's, its id unchanged
  const items = await api.children(ALICE, ws.workspace_id, '/surfaces/归档/Q4')
  expect(items.find((i: Json) => i.entity_id === moved)?.path).toBe(`/surfaces/归档/Q4/第${pageNo(1, 0, 0, 0)}页`)
  // the grant on the new folder now covers it
  const after = await api.rpc(BOB, 'doc.read', { workspace_id: ws.workspace_id, entity_id: moved })
  expect(after.capabilities).toEqual(expect.arrayContaining(['read', 'update', 'structure']))
  // the link sent before the move opens the same canvas, for alice and for bob
  for (const who of [ALICE, BOB]) {
    const other = await (await browser.newContext()).newPage()
    await openWorkspacePage(other, who, ws.workspace_id, moved)
    await expect(shown(other)).toHaveAttribute('data-surface-id', moved)
    await other.context().close()
  }
})

/** `/surfaces/部门/项目A` (canvases 看板 `c-board`, 周报 `c-week`), `/surfaces/部门/其他` (秘密 `c-secret`), `/surfaces/外面` (`c-out`). */
async function scopedTree(api: Api, title: string): Promise<{ workspace_id: string; epoch: string }> {
  const ws = await api.rpc(ALICE, 'ws.create', { title }) as { workspace_id: string; epoch: string }
  const folder = (id: string, parent: string, key: string, t: string) => ({ op: 'entity.create', entity_id: id, type_id: 'buckyos.container', parent_id: parent, order_key: key, payload: { kind: 'folder', title: t } })
  let n = 0
  const canvas = (id: string, parent: string, key: string, t: string) => [
    { op: 'entity.create', entity_id: `${id}-content`, type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: `m${(n += 1)}`, payload: { kind: 'folder', system: 'surface_content', surface_id: id, title: t } },
    { op: 'entity.create', entity_id: id, type_id: 'buckyos.container', parent_id: parent, order_key: key, payload: { kind: 'surface', layout: { mode: 'flow' }, title: t, content_folder_id: `${id}-content` } },
  ]
  const made = await api.commit(ALICE, ws, [
    folder('dept', 'surfaces', 'a', '部门'), folder('proj', 'dept', 'a', '项目A'), folder('other', 'dept', 'b', '其他'),
    ...canvas('c-board', 'proj', 'a', '看板'), ...canvas('c-week', 'proj', 'b', '周报'), ...canvas('c-secret', 'other', 'a', '秘密'), ...canvas('c-out', 'surfaces', 'b', '外面'),
  ])
  expect(made.status, JSON.stringify(made).slice(0, 400)).toBe('accepted')
  return ws
}

test('§15-4 Q5 a grant on one canvas folder only: the picker shows the folder and its canvases under the folders above it (fold only, no name, no menu), opens them, nothing else; a canvas moved in follows; a grant on one canvas the same', async ({ page, api }) => {
  const ws = await scopedTree(api, `scoped ${Date.now()}`)
  await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: 'bob', resource: '/surfaces/部门/项目A', allow: ['read'] }] })
  // the outline: 部门 as a path node — its place only
  const outline = await api.outline(BOB, ws.workspace_id)
  expect(outline.find((e: Json) => e.entity_id === 'dept')).toEqual({ entity_id: 'dept', type_id: 'buckyos.container', kind: 'folder', parent_id: 'surfaces', order_key: 'a', path_only: true })
  expect(outline.map((e: Json) => e.entity_id)).toEqual(['root', 'surfaces', 'dept', 'proj', 'c-board', 'c-week'])

  await openWorkspacePage(page, BOB, ws.workspace_id)
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
  const canvas = shown(page)
  await expect(canvas).toHaveAttribute('data-surface-id', 'c-board')
  let list = await openPicker(page)
  const dept = list.getByTestId('aiws-surface-folder-dept')
  await expect(dept).toHaveAttribute('data-path-only', 'true')
  await expect(dept).toContainText('无权访问的目录')
  await expect(list.getByTestId('aiws-surface-more-dept')).toHaveCount(0)
  await expect(list.getByTestId('aiws-surface-folder-proj')).toContainText('项目A')
  await expect(list.getByTestId('aiws-surface-folder-proj')).toHaveAttribute('data-depth', '1')
  await expect(list.getByTestId('aiws-surface-item-c-week')).toHaveAttribute('data-depth', '2')
  for (const id of ['aiws-surface-item-c-secret', 'aiws-surface-folder-other', 'aiws-surface-item-c-out', 'aiws-surface-new', 'aiws-surface-new-folder']) await expect(list.getByTestId(id)).toHaveCount(0)
  await expect(list).not.toContainText('部门')
  // the path node folds, by its toggle and by its row
  await dept.getByRole('menuitem').click()
  await expect(list.getByTestId('aiws-surface-item-c-board')).toHaveCount(0)
  await list.getByTestId('aiws-surface-folder-toggle-dept').click()
  await expect(list.getByTestId('aiws-surface-item-c-board')).toBeVisible()
  // a right click on it opens no menu
  await dept.click({ button: 'right' })
  await expect(page.getByTestId('aiws-surface-rename-dept')).toHaveCount(0)
  // the folder bob reads has its menu: nothing to create there without `structure`
  await list.getByTestId('aiws-surface-more-proj').click()
  await expect(page.getByTestId('aiws-surface-new-here-proj')).toBeDisabled()
  await page.keyboard.press('Escape')
  // opening a canvas
  list = await pickerShown(page)
  await list.getByTestId('aiws-surface-item-c-week').getByRole('menuitem').click()
  await expect(canvas).toHaveAttribute('data-surface-id', 'c-week')
  // the search finds bob's canvases (the trail runs through the path node), never the path node itself
  list = await pickerShown(page)
  await list.getByTestId('aiws-surface-search').fill('周报')
  await expect(list.getByTestId('aiws-surface-item-c-week').getByTestId('aiws-surface-trail')).toHaveText('无权访问的目录 / 项目A')
  await list.getByTestId('aiws-surface-search').fill('无权')
  await expect(list.locator('[data-testid^="aiws-surface-item-"], [data-testid^="aiws-surface-folder-"]')).toHaveCount(0)
  await list.getByTestId('aiws-surface-search').fill('')

  // a canvas alice moves into 项目A takes the folder's grant: it shows up for bob (§15 #4)
  const out = (await api.outline(ALICE, ws.workspace_id)).find((e: Json) => e.entity_id === 'c-out')
  expect((await api.commit(ALICE, ws, [{ op: 'tree.move', entity_id: 'c-out', new_parent_id: 'proj', order_key: 'c', expect: { rev: out.struct_rev } }])).status).toBe('accepted')
  await expect(list.getByTestId('aiws-surface-item-c-out')).toBeVisible()
  await expect(list.getByTestId('aiws-surface-item-c-out')).toHaveAttribute('data-depth', '2')

  // a grant on one canvas instead (Q19: someone who reads only the canvas a show plays): both folders above are path nodes
  await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: 'bob', resource: '/surfaces/部门/项目A' }, { subject: 'bob', resource: '/surfaces/部门/项目A/周报', allow: ['read'] }] })
  await page.reload()
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
  await expect(canvas).toHaveAttribute('data-surface-id', 'c-week')
  list = await openPicker(page)
  for (const id of ['dept', 'proj']) {
    await expect(list.getByTestId(`aiws-surface-folder-${id}`)).toHaveAttribute('data-path-only', 'true')
    await expect(list.getByTestId(`aiws-surface-more-${id}`)).toHaveCount(0)
  }
  await expect(list).not.toContainText('项目A')
  await expect(list.locator('[data-testid^="aiws-surface-item-"]')).toHaveCount(1)
  await expect(list.getByTestId('aiws-surface-item-c-week')).toBeVisible()
})

test('PRD §4.1.1 canvas folders from the picker: new folder, new canvas here, rename, drag into a folder, copy path, only an empty folder deleted, undo', async ({ page, api, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])
  const ws = await api.freeCanvas(ALICE, `folders ${Date.now()}`)
  await openWorkspacePage(page, ALICE, ws.workspace_id)
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
  let list = await openPicker(page)
  // a new folder at the top level: its name follows its title
  await list.getByTestId('aiws-surface-new-folder').click()
  await list.getByTestId('aiws-surface-folder-input').fill('周报')
  await list.getByTestId('aiws-surface-folder-create').click()
  const folderRow = list.locator('[data-testid^="aiws-surface-folder-sd"]')
  await expect(folderRow).toHaveCount(1)
  const folderId = (await folderRow.getAttribute('data-entity-id'))!
  await expect.poll(async () => (await outlineEntry(api, ws.workspace_id, folderId))?.name).toBe('周报')
  // a second one of the same title: `-2`
  await list.getByTestId('aiws-surface-new-folder').click()
  await list.getByTestId('aiws-surface-folder-input').fill('周报')
  await list.getByTestId('aiws-surface-folder-create').click()
  await expect(list.locator('[data-testid^="aiws-surface-folder-sd"]')).toHaveCount(2)
  const second = (await list.locator('[data-testid^="aiws-surface-folder-sd"]').nth(1).getAttribute('data-entity-id'))!
  await expect.poll(async () => (await outlineEntry(api, ws.workspace_id, second))?.name).toBe('周报-2')
  // rename it: the name follows again
  await list.getByTestId(`aiws-surface-more-${second}`).click()
  await page.getByTestId(`aiws-surface-rename-${second}`).click()
  await list.getByTestId('aiws-surface-rename-input').fill('Archive')
  await list.getByTestId('aiws-surface-rename-input').press('Enter')
  await expect.poll(async () => (await outlineEntry(api, ws.workspace_id, second))?.name).toBe('Archive')

  // "new canvas here": the dialog starts in that folder
  await list.getByTestId(`aiws-surface-more-${folderId}`).click()
  await page.getByTestId(`aiws-surface-new-here-${folderId}`).click()
  await expect(page.getByTestId('aiws-new-canvas-folder')).toHaveValue(folderId)
  await page.getByLabel('新画布标题').fill('第一周')
  await page.getByTestId('aiws-new-flow').check()
  await page.getByTestId('aiws-surface-create').click()
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).find((e: Json) => e.kind === 'surface' && e.title === '第一周')?.parent_id).toBe(folderId)
  const week = (await api.outline(ALICE, ws.workspace_id)).find((e: Json) => e.kind === 'surface' && e.title === '第一周').entity_id as string
  await expect(shown(page)).toHaveAttribute('data-surface-id', week)

  // drag the free canvas into Archive (a mouse; touch uses "move to…")
  list = await openPicker(page)
  await list.getByTestId('aiws-surface-item-free').dragTo(list.getByTestId(`aiws-surface-folder-${second}`))
  await expect.poll(async () => (await outlineEntry(api, ws.workspace_id, 'free'))?.parent_id).toBe(second)
  await expect(list.getByTestId('aiws-surface-item-free')).toHaveAttribute('data-depth', '1')
  // copy path
  await list.getByTestId('aiws-surface-more-free').click()
  await page.getByTestId('aiws-surface-copy-path-free').click()
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe('/surfaces/Archive/Free')
  // a folder that holds something cannot be deleted; empty, it can — and undo brings it back
  await list.getByTestId(`aiws-surface-more-${second}`).click()
  await expect(page.getByTestId(`aiws-surface-delete-${second}`)).toBeDisabled()
  await page.keyboard.press('Escape')
  const refused = await api.commit(ALICE, ws, [{ op: 'entity.delete', entity_id: second, expect: { rev: (await outlineEntry(api, ws.workspace_id, second)).life_rev } }])
  expect(refused.status).toBe('rejected')
  expect(JSON.stringify(refused)).toContain('FOLDER_NOT_EMPTY')
  list = await openPicker(page)
  await list.getByTestId('aiws-surface-more-free').click()
  await page.getByTestId('aiws-surface-move-free').click()
  await list.getByTestId('aiws-surface-move-to-surfaces').click()
  await expect.poll(async () => (await outlineEntry(api, ws.workspace_id, 'free'))?.parent_id).toBe('surfaces')
  await list.getByTestId(`aiws-surface-more-${second}`).click()
  await page.getByTestId(`aiws-surface-delete-${second}`).click()
  await expect.poll(async () => Boolean(await outlineEntry(api, ws.workspace_id, second))).toBe(false)
  await page.keyboard.press('Escape')
  await page.getByTestId('aiws-undo').click()
  await expect.poll(async () => Boolean(await outlineEntry(api, ws.workspace_id, second))).toBe(true)
  // sorting by name is personal: the shared order stays
  list = await openPicker(page)
  await list.getByTestId('aiws-surface-sort').click()
  await expect(list.getByTestId('aiws-surface-sort')).toHaveAttribute('aria-pressed', 'true')
  const order = await list.locator('[data-entity-id][data-depth]').evaluateAll((rows) => rows.map((r) => r.getAttribute('data-entity-id')))
  expect(order.slice(0, 2)).toEqual([second, folderId])
  const shared = (await api.children(ALICE, ws.workspace_id, '/surfaces')).map((i: Json) => i.entity_id)
  expect(shared.indexOf(folderId)).toBeLessThan(shared.indexOf(second))
})

test.describe('on a phone', () => {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars -- the browser comes from the project, the rest is the phone
  const { defaultBrowserType, ...pixel } = devices['Pixel 7']
  test.use(pixel)

  test('PRD §4.1.1, §6.5 (Q9) the whole title opens the picker over the screen; a phone makes flow pages and folders, never a free canvas; rows have their menu', async ({ page, api }) => {
    const ws = await api.freeCanvas(ALICE, `phone tree ${Date.now()}`)
    await openWorkspacePage(page, ALICE, ws.workspace_id)
    await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
    const viewport = page.viewportSize()!
    await page.getByTestId('aiws-surface-switch').tap()
    const list = page.getByTestId('aiws-surface-list')
    const box = (await list.boundingBox())!
    expect(box.height).toBeGreaterThan(viewport.height - 4)
    expect(box.width).toBeGreaterThan(viewport.width - 4)
    await expect(list.getByTestId('aiws-surface-search')).toBeVisible()
    await expect(list.getByTestId('aiws-surface-more-free')).toBeVisible()
    // a folder, then a flow page in it
    await list.getByTestId('aiws-surface-new-folder').tap()
    await list.getByTestId('aiws-surface-folder-input').fill('日记')
    await list.getByTestId('aiws-surface-folder-create').tap()
    const folderRow = list.locator('[data-testid^="aiws-surface-folder-sd"]')
    await expect(folderRow).toHaveCount(1)
    const folderId = (await folderRow.getAttribute('data-entity-id'))!
    await list.getByTestId(`aiws-surface-more-${folderId}`).tap()
    await expect(page.getByTestId(`aiws-surface-new-here-${folderId}`)).toHaveText(/新建流式页/)
    await page.getByTestId(`aiws-surface-new-here-${folderId}`).tap()
    await expect(page.getByTestId('aiws-new-free')).toBeDisabled()
    await expect(page.getByTestId('aiws-new-flow')).toBeChecked()
    await page.getByLabel('新画布标题').fill('11-13')
    await page.getByTestId('aiws-surface-create').tap()
    await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).find((e: Json) => e.kind === 'surface' && e.title === '11-13')).toMatchObject({ parent_id: folderId, layout: { mode: 'flow' } })
    // move by menu (no dragging on touch), then close with ×
    await page.getByTestId('aiws-surface-switch').tap()
    await expect(list.getByTestId('aiws-surface-item-free')).not.toHaveAttribute('draggable', 'true')
    await list.getByTestId('aiws-surface-more-free').tap()
    await page.getByTestId('aiws-surface-move-free').tap()
    await list.getByTestId(`aiws-surface-move-to-${folderId}`).tap()
    await expect.poll(async () => (await outlineEntry(api, ws.workspace_id, 'free'))?.parent_id).toBe(folderId)
    await list.getByTestId('aiws-surface-close').tap()
    await expect(list).toHaveCount(0)
  })
})
