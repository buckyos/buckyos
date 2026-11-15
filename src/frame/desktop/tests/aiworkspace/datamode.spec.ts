/* The data mode (UI 框架 PRD §5; acceptance §15 #11–#15, #21): no row above it — "返回画布" and the main menu head the
 * object tree, or the tab row while the tree is put away; the object tree's default and filters, new data at a chosen
 * place through the tree's menu, tabs (preview, fixed, switching keeps what was typed, restored after a reload), the
 * "宏" folder showing a macro's code, the "工作区权限" row (always the tree's last) opening the permission editor, the right sidebar following the
 * current tab with a version restored and undone, a directory entered and left; the tree by keyboard (the WAI-ARIA tree
 * pattern); and the data mode on a phone (Pixel 7): from the main menu, the tree full screen, the editor full screen, the
 * sidebar as a bottom panel, and "移动到…" choosing the folder and the place in it (the way to reorder by touch). */

import { createHash } from 'node:crypto'
import { devices, type Page } from '@playwright/test'
import { expect, openWorkspace, openWorkspacePage, test, type Api } from './fixtures'

const ALICE = 'tok-alice'
// The backend's JSON results are inspected ad hoc.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

async function dataMode(page: Page) {
  if (await page.getByTestId('aiws-top-sources').isVisible()) await page.getByTestId('aiws-top-sources').click()
  await expect(page.getByTestId('aiws-sources')).toBeVisible()
}

/** The context menu of a tree row (a right click), then entries by test id (a submenu opens on hover). */
async function rowMenu(page: Page, entityId: string, ...testIds: string[]) {
  await page.getByTestId(`aiws-tree-${entityId}`).click({ button: 'right' })
  const menu = page.getByTestId('aiws-dm-menu')
  await expect(menu).toBeVisible()
  for (const id of testIds) {
    const item = page.getByTestId(id)
    if (await item.getAttribute('aria-haspopup') === 'menu') await item.hover()
    else await item.click()
  }
}

/** The workspace's own page (a reload opens it again, in the view it was left in). */
async function openOwnPage(page: Page, workspaceId: string) {
  await openWorkspacePage(page, ALICE, workspaceId)
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
}

const tabs = (page: Page) => page.getByTestId('aiws-dm-tab')
const tab = (page: Page, id: string) => page.locator(`[data-testid="aiws-dm-tab"][data-tab-id="${id}"]`)

async function workspaceWithExtras(api: Api) {
  const ws = await api.demo(ALICE, 'quarterly', `datamode ${Date.now()}`)
  const made = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'f-docs', type_id: 'buckyos.container', parent_id: 'data', order_key: 'zx', payload: { kind: 'folder', title: '资料' } },
    { op: 'entity.create', entity_id: 'note-x', type_id: 'buckyos.annotation', parent_id: 'data', order_key: 'zy', payload: { kind: 'note', body: '一张便签', style: { color: '#fff2cc' } } },
    { op: 'entity.create', entity_id: 'def-x', type_id: 'buckyos.block-def', parent_id: 'data', order_key: 'zz', payload: { def_id: 'test.dm', kind: 'html', title: '测试控件', allow_no_source: true, html: { html: '<div>x</div>', js: '', api_version: 2 } } },
    { op: 'entity.create', entity_id: 'own-note', type_id: 'buckyos.richtext', parent_id: 'sf-analysis-content', order_key: 'zz', name: '画布上的备注', payload: { content: { type: 'doc', content: [{ type: 'paragraph', attrs: { block_id: 'b-own' }, content: [{ type: 'text', text: '备注' }] }] } } },
  ])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  return ws
}

test('§15-11 the object tree lists only the data sources by default; the filter brings control definitions and comments', async ({ page, api }) => {
  const ws = await workspaceWithExtras(api)
  await openOwnPage(page, ws.workspace_id)
  await dataMode(page)
  const item = (id: string) => page.locator(`[data-testid="aiws-tree-item"][data-entity-id="${id}"]`)
  // data sources and the folders holding them; the canvas content folded; not the definition, the note, the presentations
  await expect(item('sales')).toBeVisible()
  await expect(item('f-docs')).toBeVisible()
  await expect(item('canvas-content')).toHaveAttribute('aria-expanded', 'false')
  for (const id of ['def-x', 'note-x', 'shows']) await expect(item(id)).toHaveCount(0)
  const types = await page.getByTestId('aiws-tree-item').evaluateAll((els) => els.map((el) => el.getAttribute('data-type')))
  for (const type of types) expect(['buckyos.table-source', 'buckyos.richtext', 'buckyos.record', 'buckyos.asset-ref', 'buckyos.dir', 'buckyos.container']).toContain(type)
  // each row: the type's icon and the name; "宏" is a folder of the tree and "工作区权限" its last top-level row
  await expect(item('sales').locator('.aiws-dm-type svg')).toBeVisible()
  const macros = page.getByTestId('aiws-dm-macros')
  await expect(macros).toHaveAttribute('role', 'treeitem')
  await expect(macros).toHaveAttribute('aria-level', '1')
  await expect(macros).toHaveAttribute('aria-expanded', 'true')
  await page.getByTestId('aiws-dm-macros-label').click()
  await expect(macros).toHaveAttribute('aria-expanded', 'false')
  await page.getByTestId('aiws-dm-macros-label').click()
  const topLevel = await page.locator('[role="tree"] > [role="treeitem"]').evaluateAll((els) => els.map((el) => el.getAttribute('data-entity-id')))
  expect(topLevel.slice(-2)).toEqual(['@macros', '@permissions'])
  await expect(page.getByTestId('aiws-dm-perm-node')).toBeVisible()
  // no row above the data mode (owner 11-14): "返回画布" and the main menu head the tree; the Workspace's name is on
  // the status bar's left, the filter ends the search row, the tree's fold starts the tab row
  for (const gone of ['aiws-view-toolbar', 'aiws-main-toolbar']) await expect(page.getByTestId(gone), gone).toHaveCount(0)
  const left = page.getByTestId('aiws-dm-left')
  await expect(left.getByTestId('aiws-top-canvas')).toBeVisible()
  await expect(left.getByTestId('aiws-main-menu')).toBeVisible()
  await expect(left.getByTestId('aiws-dm-workspace-title')).toHaveCount(0)
  await expect(page.getByTestId('aiws-status-row').getByTestId('aiws-dm-workspace-title')).toHaveText(/datamode/)
  await expect(page.getByTestId('aiws-dm-ws-more')).toContainText('所有者')
  const head = (await left.locator('.aiws-dm-tree-head').boundingBox())!
  const body = (await page.getByTestId('aiws-dm').boundingBox())!
  expect(head.y - body.y).toBeLessThan(8)
  const search = (await page.getByTestId('aiws-dm-search').boundingBox())!
  const filterButton = (await page.getByTestId('aiws-dm-filter').boundingBox())!
  expect(Math.abs((filterButton.y + filterButton.height / 2) - (search.y + search.height / 2))).toBeLessThan(6)
  const fold = (await page.getByTestId('aiws-dm-tree-hide').boundingBox())!
  const main = (await page.locator('.aiws-dm-main').boundingBox())!
  expect(fold.x).toBeGreaterThanOrEqual(main.x)
  expect(fold.x - main.x).toBeLessThan(12)
  // the tree put away: they start the tab row; the main menu's "添加评论" is about the current object
  await page.getByTestId('aiws-dm-tree-hide').click()
  await expect(left).toHaveCount(0)
  await expect(page.getByTestId('aiws-top-canvas')).toBeVisible()
  await page.getByTestId('aiws-main-menu').click()
  await page.getByTestId('aiws-menu-collab').hover()
  await expect(page.getByTestId('aiws-menu-annotate')).toBeDisabled()
  await page.keyboard.press('Escape')
  await page.getByTestId('aiws-dm-tree-show').click()
  await expect(left.getByTestId('aiws-top-canvas')).toBeVisible()
  await page.getByTestId('aiws-tree-sales').click()
  await left.getByTestId('aiws-main-menu').click()
  await page.getByTestId('aiws-menu-collab').hover()
  await expect(page.getByTestId('aiws-menu-annotate')).toBeEnabled()
  await page.keyboard.press('Escape')
  // the filter: control definitions and comments by type
  await page.getByTestId('aiws-dm-filter').click()
  await page.getByTestId('aiws-dm-filter-type-block-def').check()
  await page.getByTestId('aiws-dm-filter-type-annotation').check()
  await expect(item('def-x')).toBeVisible()
  await expect(item('note-x')).toBeVisible()
  await expect(item('sales')).toHaveCount(0)
  // the search keeps the levels: a match inside the canvas content shows with its folders
  await page.getByTestId('aiws-dm-filter-reset').click()
  await page.getByTestId('aiws-dm-search').fill('画布上的')
  const hit = page.locator('[data-testid="aiws-tree-item"][data-hit="true"]')
  await expect(hit).toHaveCount(1)
  await expect(hit).toHaveAttribute('data-entity-id', 'own-note')
  expect(await hit.evaluate((el) => [...new Set([el.parentElement?.closest('[data-entity-id]')?.getAttribute('data-entity-id'), el.parentElement?.closest('[data-entity-id]')?.parentElement?.closest('[data-entity-id]')?.getAttribute('data-entity-id')])])).toEqual(['sf-analysis-content', 'canvas-content'])
  await expect(item('sales')).toHaveCount(0)
  await page.getByTestId('aiws-dm-search').fill('')
  // "everything" shows the rest, the filter is the person's (it survives a reload)
  await page.getByTestId('aiws-dm-filter-all').check()
  await expect(item('note-x')).toBeVisible()
  await expect(item('shows')).toBeVisible()
  await page.reload()
  await expect(page.getByTestId('aiws-sources')).toBeVisible({ timeout: 30_000 })
  await expect(item('note-x')).toBeVisible()
})

test('§15-12 new data from the tree’s menu goes where the menu was opened: into a folder, or right after an object', async ({ page, api }) => {
  const ws = await workspaceWithExtras(api)
  await openWorkspace(page, ALICE, ws.workspace_id)
  await dataMode(page)
  // on a folder: a new table goes into it
  await rowMenu(page, 'f-docs', 'aiws-dm-menu-new', 'aiws-dm-new-table')
  const dialog = page.getByTestId('aiws-dm-new-dialog')
  await expect(dialog.getByTestId('aiws-dm-new-where')).toContainText('资料')
  await dialog.getByTestId('aiws-dm-new-title').fill('预算表')
  await dialog.getByTestId('aiws-new-data').click()
  await expect(dialog).toHaveCount(0)
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).find((e) => e.name === '预算表')?.parent_id).toBe('f-docs')
  const table = (await api.outline(ALICE, ws.workspace_id)).find((e) => e.name === '预算表')
  // it opens in a tab of its own, and its row is inside the folder
  await expect(tab(page, table.entity_id)).toHaveAttribute('data-preview', 'false')
  await expect(page.locator('[data-testid="aiws-tree-item"][data-entity-id="f-docs"]').locator(`[data-entity-id="${table.entity_id}"]`)).toBeVisible()
  // on an object: a new text becomes its next sibling
  await rowMenu(page, 'sales', 'aiws-dm-menu-new', 'aiws-dm-new-richtext')
  await expect(page.getByTestId('aiws-dm-new-where')).toContainText('原始销售数据')
  await page.getByTestId('aiws-dm-new-title').fill('销售备注')
  await page.getByTestId('aiws-new-data').click()
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).some((e) => e.name === '销售备注')).toBe(true)
  const outline = await api.outline(ALICE, ws.workspace_id)
  const siblings = outline.filter((e) => e.parent_id === 'data' && !e.deleted).sort((a, b) => (a.order_key < b.order_key ? -1 : a.order_key > b.order_key ? 1 : 0)).map((e) => e.name ?? e.entity_id)
  expect(siblings[siblings.indexOf(outline.find((e) => e.entity_id === 'sales').name ?? 'sales') + 1]).toBe('销售备注')
  // the inline form below the tree is gone; the tree's ＋ offers the same menu
  await expect(page.getByTestId('aiws-new-kind')).toHaveCount(0)
  await page.getByTestId('aiws-dm-tree-add').click()
  await expect(page.getByTestId('aiws-dm-menu').getByTestId('aiws-dm-menu-new')).toBeVisible()
  await page.keyboard.press('Escape')
})

test('§15-13 three objects in three tabs: a preview is reused, switching keeps what was typed, the tabs come back after a reload', async ({ page, api }) => {
  const ws = await workspaceWithExtras(api)
  await openOwnPage(page, ws.workspace_id)
  await dataMode(page)
  // a single click previews (italic, reused by the next single click)
  await page.getByTestId('aiws-tree-sales').click()
  await expect(tab(page, 'sales')).toHaveAttribute('data-preview', 'true')
  await page.getByTestId('aiws-tree-intro').click()
  await expect(tabs(page)).toHaveCount(1)
  await expect(tab(page, 'intro')).toHaveAttribute('data-preview', 'true')
  await expect(tab(page, 'intro').locator('.aiws-dm-tab-name')).toHaveCSS('font-style', 'italic')
  // a double click keeps it; two more objects make three tabs
  await page.getByTestId('aiws-tree-intro').dblclick()
  await expect(tab(page, 'intro')).toHaveAttribute('data-preview', 'false')
  await page.getByTestId('aiws-tree-sales').dblclick()
  await page.getByTestId('aiws-tree-f-docs').dblclick()
  await expect(tabs(page)).toHaveCount(3)
  // only the current tab's editor is mounted
  await expect(page.getByTestId('aiws-dm-editor')).toHaveCount(1)
  await expect(page.getByTestId('aiws-detail-f-docs')).toBeVisible()
  // type into the table without pressing Enter, switch away and back: nothing typed is lost
  await tab(page, 'sales').click()
  await page.getByTestId('aiws-detail-sales').getByTestId('aiws-cell-s-2-cost').getByRole('button').first().click()
  await page.getByLabel('成本 s-2', { exact: true }).fill('59000')
  await tab(page, 'intro').click()
  await expect(page.getByTestId('aiws-detail-intro')).toBeVisible()
  await expect(page.getByTestId('aiws-detail-sales')).toHaveCount(0)
  await tab(page, 'sales').click()
  await expect(page.getByTestId('aiws-detail-sales').getByTestId('aiws-cell-s-2-cost')).toContainText('59000')
  await expect.poll(async () => (await api.cell(ALICE, ws.workspace_id, 'sales', 's-2', 'cost')).value).toBe(59000)
  // drag to reorder: sales goes first
  await tab(page, 'sales').dragTo(tab(page, 'intro'))
  const order = async () => tabs(page).evaluateAll((els) => els.map((el) => el.getAttribute('data-tab-id')))
  await expect.poll(order).toEqual(['sales', 'intro', 'f-docs'])
  // the tab's menu
  await tab(page, 'f-docs').click({ button: 'right' })
  await expect(page.getByTestId('aiws-dm-tab-menu')).toBeVisible()
  await expect(page.getByTestId('aiws-dm-tab-copy-path')).toBeVisible()
  await page.keyboard.press('Escape')
  // a reload: the same tabs, the same current one
  await page.waitForTimeout(1200) // the user state goes out after 0.8 s
  await page.reload()
  await expect(page.getByTestId('aiws-sources')).toBeVisible({ timeout: 30_000 })
  await expect.poll(order).toEqual(['sales', 'intro', 'f-docs'])
  await expect(tab(page, 'sales')).toHaveAttribute('aria-selected', 'true')
  await expect(page.getByTestId('aiws-detail-sales')).toBeVisible()
  // an object deleted elsewhere: its tab says so instead of vanishing
  expect((await api.commit(ALICE, ws, [{ op: 'entity.delete', entity_id: 'f-docs', expect: { rev: (await api.read(ALICE, ws.workspace_id, 'f-docs')).life_rev } }])).status).toBe('accepted')
  await tab(page, 'f-docs').click()
  await expect(page.getByTestId('aiws-dm-gone')).toBeVisible()
})

test('§15-14 a macro in the “宏” folder opens on its code; “工作区权限” turns the editing area into the permission editor', async ({ page, api }) => {
  const ws = await workspaceWithExtras(api)
  // a macro: the wish's saved program (the demo's wish is on the Mock executor here)
  const source = "aiws.declare({ inputs: {}, results: {} })\nconst rows = aiws.input('sales')\naiws.result.table('summary', rows)\n"
  const uploaded = await api.uploadAsset(ALICE, ws.workspace_id, new TextEncoder().encode(source))
  const manifest = { inputs: {}, results: {} }
  const digest = createHash('sha256').update(`${source}\n${JSON.stringify(manifest)}`).digest('hex')
  const wish = await api.read(ALICE, ws.workspace_id, 'wish-analysis')
  const set = await api.commit(ALICE, ws, [{ op: 'entity.set_keys', entity_id: 'wish-analysis', keys: [{ key: 'program', value: { language: 'js', api_version: 3, source: uploaded.object_id, digest, manifest, produces: [], edited_by: 'alice' }, expect: { rev: wish.content.key_revs?.program ?? 0 } }] }])
  expect(set.status, JSON.stringify(set)).toBe('accepted')
  await openWorkspace(page, ALICE, ws.workspace_id)
  await dataMode(page)
  const macros = page.getByTestId('aiws-dm-macros')
  // Q27: a Mock wish is no macro, even with a program in it; on the real executor it is
  await expect(macros).toHaveAttribute('data-count', '0')
  await expect(macros.getByTestId('aiws-dm-macro-wish-analysis')).toHaveCount(0)
  const mock = await api.read(ALICE, ws.workspace_id, 'wish-analysis')
  const real = await api.commit(ALICE, ws, [{ op: 'entity.set_keys', entity_id: 'wish-analysis', keys: [{ key: 'executor', value: 'xllm', expect: { rev: mock.content.key_revs.executor } }] }])
  expect(real.status, JSON.stringify(real)).toBe('accepted')
  await expect(macros.getByTestId('aiws-dm-macro-wish-analysis')).toBeVisible()
  await macros.getByTestId('aiws-dm-macro-wish-analysis').click()
  const editor = page.getByTestId('aiws-dm-macro-editor-wish-analysis')
  await expect(editor).toHaveAttribute('data-page', 'code')
  const code = editor.getByTestId('aiws-wish-code')
  await expect(code).toContainText("aiws.result.table('summary'")
  await expect(code).toHaveAttribute('data-lines', String(source.split('\n').length))
  await expect(code.locator('.aiws-wish-code-no').first()).toHaveText('1')
  expect(await code.evaluate((el) => getComputedStyle(el).fontFamily)).toMatch(/mono|Menlo|Consolas/i)
  // the three pages
  await editor.getByTestId('aiws-dm-macro-page-task').click()
  await expect(page.getByTestId('aiws-wish-wish-analysis')).toBeVisible()
  await editor.getByTestId('aiws-dm-macro-page-runs').click()
  await expect(editor.getByTestId('aiws-dm-runs').or(editor.getByTestId('aiws-dm-runs-empty'))).toBeVisible()
  // the Workspace's permissions: the editor in the editing area, the operation record as the sidebar's only tab
  await page.getByTestId('aiws-dm-perm-node').click()
  await expect(page.getByTestId('aiws-detail-permissions').getByTestId('aiws-permissions')).toHaveAttribute('data-complete', 'true')
  await expect(page.getByTestId('aiws-dm-side-tab-history')).toBeVisible()
  await expect(page.getByTestId('aiws-dm-side-tab-properties')).toHaveCount(0)
  await expect(page.getByTestId('aiws-dm-side').getByTestId('aiws-perm-audit')).toBeVisible()
  // "成员与权限…" of the main menu leads to the same tab
  await tab(page, '@permissions').getByTestId('aiws-dm-tab-close').click()
  await page.getByTestId('aiws-top-canvas').click()
  await page.getByTestId('aiws-main-menu').click()
  await page.getByTestId('aiws-menu-collab').hover()
  await page.getByTestId('aiws-menu-members').click()
  await expect(page.getByTestId('aiws-detail-permissions')).toBeVisible()
  await expect(tab(page, '@permissions')).toHaveAttribute('aria-selected', 'true')
})

test('§15-15 the sidebar’s three tabs follow the current tab; a version restored from the history can be undone', async ({ page, api }) => {
  const ws = await workspaceWithExtras(api)
  const made = await api.commit(ALICE, ws, [{ op: 'entity.create', entity_id: 'rec-x', type_id: 'buckyos.record', parent_id: 'data', order_key: 'zw', name: '客户', payload: { schema: { properties: [{ key: 'name', name: '名称', type: 'text' }] }, props: { name: '旧名称' } } }])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  const nameOf = async () => { const r = await api.read(ALICE, ws.workspace_id, 'rec-x'); return r.content.payload?.props?.name ?? r.content.props?.name }
  expect((await api.rpc(ALICE, 'doc.checkpoint', { workspace_id: ws.workspace_id })).ok).toBe(true)
  const before = await api.read(ALICE, ws.workspace_id, 'rec-x')
  const revs = before.content.key_revs ?? before.content.payload?.key_revs ?? {}
  expect((await api.commit(ALICE, ws, [{ op: 'entity.set_keys', entity_id: 'rec-x', keys: [{ key: 'p:name', value: '新名称', expect: { rev: revs['p:name'] ?? 0 } }] }])).status).toBe('accepted')
  expect(await nameOf()).toBe('新名称')
  const versions = (await api.rpc(ALICE, 'doc.list_versions', { workspace_id: ws.workspace_id, entity_id: 'rec-x' })).versions as { content_rev: number }[]
  expect(versions.length).toBeGreaterThan(0)
  const old = versions[versions.length - 1].content_rev
  await openWorkspace(page, ALICE, ws.workspace_id)
  await dataMode(page)
  await page.getByTestId('aiws-tree-sales').dblclick()
  const side = page.getByTestId('aiws-dm-side')
  await side.getByTestId('aiws-dm-side-tab-properties').click()
  await expect(side.getByTestId('aiws-properties')).toHaveAttribute('data-entity-id', 'sales')
  await expect(side.getByTestId('aiws-dm-prop-path')).toContainText('/data/')
  await expect(side.getByTestId('aiws-dm-prop-size')).toContainText('12')
  await expect(page.getByTestId('aiws-dm-info')).toContainText('12')
  // the sections fold, and stay folded for the type
  await side.getByTestId('aiws-dm-section-toggle-publish').click()
  await expect(side.getByTestId('aiws-dm-section-publish')).toHaveAttribute('data-open', 'false')
  // another tab: the sidebar follows
  await page.getByTestId('aiws-tree-rec-x').dblclick()
  await expect(side.getByTestId('aiws-properties')).toHaveAttribute('data-entity-id', 'rec-x')
  await side.getByTestId('aiws-dm-side-tab-permissions').click()
  await expect(side.locator('[data-testid="aiws-perm-entity"][data-entity-id="rec-x"]')).toBeVisible()
  await tab(page, 'sales').click()
  await expect(side.locator('[data-testid="aiws-perm-entity"][data-entity-id="sales"]')).toBeVisible()
  // the history of the record: restore the checkpoint, one ordinary undoable Commit
  await tab(page, 'rec-x').click()
  await side.getByTestId('aiws-dm-side-tab-history').click()
  await expect(side.getByTestId('aiws-version').first()).toBeVisible()
  await side.getByTestId(`aiws-restore-${old}`).click()
  await expect.poll(nameOf).toBe('旧名称')
  await page.keyboard.press('Control+z')
  await expect.poll(nameOf).toBe('新名称')
})

test('§5.5 the status bar shows the row a table is on: the current cell’s row, the record open in its form', async ({ page, api }) => {
  const ws = await workspaceWithExtras(api)
  await openWorkspace(page, ALICE, ws.workspace_id)
  await dataMode(page)
  await page.getByTestId('aiws-tree-sales').dblclick()
  const info = page.getByTestId('aiws-dm-info-text')
  // on no row yet: the table's size
  await expect(info).toHaveText('表 · 12 行')
  const grid = page.getByTestId('aiws-detail-sales')
  const third = grid.getByTestId('aiws-row').nth(2)
  const thirdId = await third.getAttribute('data-record-id')
  // a cell of the third row edited, then left: the bar says where the table is
  await third.locator('button.aiws-cell-value').first().click()
  await page.keyboard.press('Escape')
  await expect(info).toHaveText('表 · 第 3 行 / 共 12 行')
  await expect(info).toHaveAttribute('data-record-id', thirdId!)
  // arrows move the current cell, and the row with it
  await page.keyboard.press('ArrowDown')
  await expect(info).toHaveText('表 · 第 4 行 / 共 12 行')
  // the record open in its form; its next record moves the table along, and it stays the current one when closed
  const fifth = await grid.getByTestId('aiws-row').nth(4).getAttribute('data-record-id')
  await grid.getByTestId(`aiws-row-open-${fifth}`).click()
  await expect(info).toHaveText('表 · 第 5 行 / 共 12 行')
  await page.getByTestId('aiws-table-record-next').click()
  await expect(info).toHaveText('表 · 第 6 行 / 共 12 行')
  await expect(info).toHaveAttribute('title', /当前记录：/)
  await page.getByTestId('aiws-table-record-close').click()
  await expect(info).toHaveText('表 · 第 6 行 / 共 12 行')
  // a cell clicked again is the current one
  await third.locator('button.aiws-cell-value').first().click()
  await page.keyboard.press('Escape')
  await expect(info).toHaveText('表 · 第 3 行 / 共 12 行')
  // another tab has no row; the table's tab opened again starts on none
  await page.getByTestId('aiws-tree-intro').dblclick()
  await expect(info).not.toContainText('第')
  await tab(page, 'sales').click()
  await expect(info).toHaveText('表 · 12 行')
})

test('§15-21 a directory opened from the object tree: into a subdirectory and back', async ({ page, api }) => {
  const ws = await api.freeCanvas(ALICE, `dm dir ${Date.now()}`)
  const photo = await api.uploadAsset(ALICE, ws.workspace_id, Buffer.from('iVBORw0KGgoAAAANSUhEUgAAABAAAAAJCAIAAAC0SDtlAAAAFElEQVR42mPQiDpBEmIY1TAoNAAAk1q5oVLxFXwAAAAASUVORK5CYII=', 'base64'))
  const made = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'inner-dir', type_id: 'buckyos.dir', parent_id: 'data', order_key: 'a', name: 'Inner', payload: { members: { 'a.png': { kind: 'file', object_id: photo.object_id } } } },
    { op: 'entity.create', entity_id: 'outer-dir', type_id: 'buckyos.dir', parent_id: 'data', order_key: 'b', name: 'Outer', payload: { members: { 'photo.png': { kind: 'file', object_id: photo.object_id }, sub: { kind: 'dir', entity_id: 'inner-dir' } } } },
  ])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  await openWorkspace(page, ALICE, ws.workspace_id)
  await dataMode(page)
  await page.getByTestId('aiws-tree-outer-dir').click()
  const editor = page.getByTestId('aiws-detail-outer-dir').getByTestId('aiws-directory')
  await expect(editor.locator('[data-testid="aiws-directory-member"][data-name="photo.png"]')).toBeVisible()
  await editor.locator('[data-testid="aiws-directory-member"][data-name="sub"]').getByTestId('aiws-directory-open').click()
  await expect(editor.getByTestId('aiws-directory-crumb')).toHaveCount(2)
  await expect(editor.locator('[data-testid="aiws-directory-member"][data-name="a.png"]')).toBeVisible()
  // the same tab: the subdirectory opens inside the editor, not as another object
  await expect(tabs(page)).toHaveCount(1)
  await editor.locator('[data-testid="aiws-directory-crumb"][data-depth="0"]').click()
  await expect(editor.locator('[data-testid="aiws-directory-member"][data-name="photo.png"]')).toBeVisible()
  await expect(page.getByTestId('aiws-dm-info')).toContainText('2')
})

/** An empty Workspace with data in a row and a folder: `x` (Xray), the folder `f-docs` (资料) holding `r-a` (Alpha),
 * `r-b` (Bravo) and `r-c` (Charlie). */
async function rowsWorkspace(api: Api, title: string) {
  const ws = await api.rpc(ALICE, 'ws.create', { title }) as { workspace_id: string; epoch: string }
  const text = (id: string, parent: string, order: string, name: string) => ({ op: 'entity.create', entity_id: id, type_id: 'buckyos.richtext', parent_id: parent, order_key: order, name, payload: {} })
  const made = await api.commit(ALICE, ws, [
    text('x', 'data', 'a', 'Xray'),
    { op: 'entity.create', entity_id: 'f-docs', type_id: 'buckyos.container', parent_id: 'data', order_key: 'b', payload: { kind: 'folder', title: '资料' } },
    text('r-a', 'f-docs', 'a', 'Alpha'), text('r-b', 'f-docs', 'b', 'Bravo'), text('r-c', 'f-docs', 'c', 'Charlie'),
  ])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  return ws
}

/** The children of `parent` in their order (by order key). */
async function order(api: Api, workspaceId: string, parent: string): Promise<string[]> {
  return (await api.outline(ALICE, workspaceId)).filter((e) => e.parent_id === parent && !e.deleted)
    .sort((a, b) => (a.order_key < b.order_key ? -1 : a.order_key > b.order_key ? 1 : 0)).map((e) => e.entity_id as string)
}

test('§5.2 the object tree by keyboard (WAI-ARIA tree): one tab stop; arrows move, unfold and fold; Home / End; type-ahead; Space previews, Enter opens; Shift+F10 and the menu key open the row menu, focus comes back', async ({ page, api }) => {
  const ws = await rowsWorkspace(api, `keys ${Date.now()}`)
  await openOwnPage(page, ws.workspace_id)
  await dataMode(page)
  const item = (id: string) => page.locator(`[role="treeitem"][data-entity-id="${id}"]`)
  const focused = () => page.evaluate(() => document.activeElement?.closest('[role="treeitem"]')?.getAttribute('data-entity-id') ?? null)
  const press = async (key: string, id: string) => { await page.keyboard.press(key); await expect.poll(focused).toBe(id) }
  await expect(item('r-c')).toBeVisible()
  const rows = await page.locator('[role="treeitem"]').evaluateAll((els) => els.map((el) => el.getAttribute('data-entity-id')))
  expect(rows.slice(0, 5)).toEqual(['x', 'f-docs', 'r-a', 'r-b', 'r-c'])
  // one tab stop: the first row while nothing is open
  await expect(page.locator('[role="treeitem"][tabindex="0"]')).toHaveCount(1)
  await expect(item('x')).toHaveAttribute('tabindex', '0')
  // the filter ends the search row: Tab from the search reaches it, then the tree
  await page.getByTestId('aiws-dm-search').focus()
  await page.keyboard.press('Tab')
  await expect(page.getByTestId('aiws-dm-filter')).toBeFocused()
  await page.keyboard.press('Tab')
  await expect.poll(focused).toBe('x')
  // ↓ ↑, → into the folder's first row, ← back to it, ← folds, → unfolds
  await press('ArrowDown', 'f-docs')
  await expect(item('f-docs')).toHaveAttribute('aria-expanded', 'true')
  await press('ArrowRight', 'r-a')
  await press('ArrowDown', 'r-b')
  await press('ArrowLeft', 'f-docs')
  await page.keyboard.press('ArrowLeft')
  await expect(item('f-docs')).toHaveAttribute('aria-expanded', 'false')
  await expect(item('r-a')).toHaveCount(0)
  await press('ArrowDown', rows[5] ?? 'f-docs')
  await press('ArrowUp', 'f-docs')
  await page.keyboard.press('ArrowRight')
  await expect(item('f-docs')).toHaveAttribute('aria-expanded', 'true')
  await expect.poll(focused).toBe('f-docs')
  // Home / End: "工作区权限" is the last row; "宏" folds and unfolds like a folder (Enter too: it does not open)
  expect(rows.slice(-2)).toEqual(['@macros', '@permissions'])
  await press('End', '@permissions')
  await press('ArrowUp', '@macros')
  await page.keyboard.press('ArrowLeft')
  await expect(page.getByTestId('aiws-dm-macros')).toHaveAttribute('aria-expanded', 'false')
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('aiws-dm-macros')).toHaveAttribute('aria-expanded', 'true')
  await expect(tabs(page)).toHaveCount(0)
  await press('Home', 'x')
  // type-ahead: the next row whose name starts with what is typed
  await page.keyboard.type('br')
  await expect.poll(focused).toBe('r-b')
  // Space: the preview tab, the focus stays in the tree; Enter: a tab of its own
  await page.keyboard.press(' ')
  await expect(tab(page, 'r-b')).toHaveAttribute('data-preview', 'true')
  await expect.poll(focused).toBe('r-b')
  await press('ArrowUp', 'r-a')
  await page.keyboard.press('Enter')
  await expect(tab(page, 'r-a')).toHaveAttribute('data-preview', 'false')
  // the open object is the tab stop once the tree has no focus of its own (a reload)
  await page.reload()
  await expect(page.getByTestId('aiws-sources')).toBeVisible({ timeout: 30_000 })
  await expect(item('r-a')).toHaveAttribute('tabindex', '0')
  await item('r-a').focus()
  // Shift+F10: the row's menu, its first entry focused; Esc closes it and the focus is on the row again
  await page.keyboard.press('Shift+F10')
  const menu = page.getByTestId('aiws-dm-menu')
  await expect(menu.getByTestId('aiws-dm-menu-rename')).toBeVisible()
  await expect.poll(() => page.evaluate(() => document.activeElement?.getAttribute('role'))).toBe('menuitem')
  await page.keyboard.press('Escape')
  await expect(menu).toHaveCount(0)
  await expect.poll(focused).toBe('r-a')
  // the menu key does the same; a click on a row moves the tab stop there
  await press('ArrowDown', 'r-b')
  await page.keyboard.press('ContextMenu')
  await expect(menu.getByTestId('aiws-dm-menu-rename')).toBeVisible()
  await page.keyboard.press('Escape')
  await expect.poll(focused).toBe('r-b')
  await page.getByTestId('aiws-tree-x').click()
  await expect(item('x')).toHaveAttribute('tabindex', '0')
  await expect(page.locator('[role="treeitem"][tabindex="0"]')).toHaveCount(1)
})

test('§5.2 §15-24 Q5 a grant on /data/销售 only: the object tree has the folder and what is in it, nothing else of the data; a grant on a folder inside shows 销售 as a row that only folds', async ({ page, api }) => {
  const BOB = 'tok-bob'
  const ws = await api.freeCanvas(ALICE, `scoped data ${Date.now()}`)
  const folder = (id: string, parent: string, key: string, title: string) => ({ op: 'entity.create', entity_id: id, type_id: 'buckyos.container', parent_id: parent, order_key: key, payload: { kind: 'folder', title } })
  const table = (id: string, parent: string, name: string) => ({ op: 'entity.create', entity_id: id, type_id: 'buckyos.table-source', parent_id: parent, order_key: 'a', name, payload: { fields: [{ field_id: 'c', name: '客户', type: 'text' }] } })
  const made = await api.commit(ALICE, ws, [folder('f-sales', 'data', 'a', '销售'), table('t-orders', 'f-sales', '订单'), folder('f-east', 'f-sales', 'b', '华东'), table('t-east', 'f-east', '华东订单'), folder('f-hr', 'data', 'b', '人事'), table('t-pay', 'f-hr', '工资')])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: 'bob', resource: '/data/销售', allow: ['read'] }] })
  expect((await api.outline(BOB, ws.workspace_id)).map((e: Json) => [e.entity_id, e.path_only === true])).toEqual([['root', true], ['data', true], ['f-sales', false], ['t-orders', false], ['f-east', false], ['t-east', false]])

  await openWorkspacePage(page, BOB, ws.workspace_id)
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
  await dataMode(page)
  const item = (id: string) => page.locator(`[data-testid="aiws-tree-item"][data-entity-id="${id}"]`)
  for (const id of ['f-sales', 't-orders', 'f-east', 't-east']) await expect(item(id)).toBeVisible()
  for (const id of ['f-hr', 't-pay', 'canvas-content', 'free-content']) await expect(item(id)).toHaveCount(0)
  await expect(page.locator('[data-testid="aiws-tree-item"][data-path-only="true"]')).toHaveCount(0)
  await page.getByTestId('aiws-tree-t-orders').dblclick()
  await expect(tab(page, 't-orders')).toBeVisible()

  // only 华东 inside it: 销售 stays as the way there, unnamed, no menu, not opened — it folds
  await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: 'bob', resource: '/data/销售' }, { subject: 'bob', resource: '/data/销售/华东', allow: ['read'] }] })
  await page.reload()
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
  await dataMode(page)
  await expect(item('f-sales')).toHaveAttribute('data-path-only', 'true')
  await expect(page.getByTestId('aiws-tree-f-sales')).toHaveText('无权访问的文件夹')
  await expect(page.getByTestId('aiws-tree-more-f-sales')).toHaveCount(0)
  await expect(item('t-orders')).toHaveCount(0)
  await expect(item('t-east')).toBeVisible()
  await page.getByTestId('aiws-tree-f-sales').click()
  await expect(item('f-sales')).toHaveAttribute('aria-expanded', 'false')
  await expect(item('f-east')).toHaveCount(0)
  await expect(tab(page, 'f-sales')).toHaveCount(0)
  await page.getByTestId('aiws-tree-f-sales').click({ button: 'right' })
  await expect(page.getByTestId('aiws-dm-menu')).toHaveCount(0)
  await page.getByTestId('aiws-tree-f-sales').click()
  await page.getByTestId('aiws-tree-t-east').dblclick()
  await expect(tab(page, 't-east')).toBeVisible()
  // the search finds what bob reads, never the path node
  await page.getByTestId('aiws-dm-search').fill('无权')
  await expect(page.getByTestId('aiws-tree-item')).toHaveCount(0)
  await page.getByTestId('aiws-dm-search').fill('华东订单')
  await expect(page.locator('[data-testid="aiws-tree-item"][data-hit="true"]')).toHaveAttribute('data-entity-id', 't-east')
})

test.describe('on a phone', () => {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars -- the browser comes from the project, the rest is the phone
  const { defaultBrowserType, ...pixel } = devices['Pixel 7']
  test.use(pixel)

  test('§5.6 the data mode on a phone: from the main menu, the tree full screen, the editor full screen, the sidebar as a bottom panel', async ({ page, api }) => {
    const ws = await workspaceWithExtras(api)
    await openWorkspacePage(page, ALICE, ws.workspace_id)
    await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
    await expect(page.getByTestId('aiws-canvas')).toBeVisible()
    // the main menu's "数据模式"
    await page.getByTestId('aiws-main-menu').tap()
    await page.getByTestId('aiws-main-menu-list').getByTestId('aiws-menu-data-mode').tap()
    const mode = page.getByTestId('aiws-dm')
    await expect(mode).toHaveAttribute('data-layout', 'phone')
    await expect(mode).toHaveAttribute('data-layer', 'tree')
    // the tree's head is the top bar: "返回画布", the main menu, ＋ — one row; the Workspace's name in the status bar
    await expect(page.getByTestId('aiws-dm-phone-bar')).toHaveCount(0)
    const left = page.getByTestId('aiws-dm-left')
    for (const id of ['aiws-top-canvas', 'aiws-main-menu', 'aiws-dm-tree-add', 'aiws-dm-filter']) await expect(left.getByTestId(id), id).toBeVisible()
    await expect(page.getByTestId('aiws-status-row').getByTestId('aiws-dm-workspace-title')).toBeVisible()
    const headBox = (await left.locator('.aiws-dm-tree-head').boundingBox())!
    expect(headBox.height).toBeLessThan(60)
    // the tree: the whole screen, rows of at least 48 px with their ⋯ always shown
    const viewport = page.viewportSize()!
    const tree = (await page.getByTestId('aiws-dm-left').boundingBox())!
    expect(tree.width).toBeGreaterThan(viewport.width - 2)
    const row = page.locator('[data-testid="aiws-tree-item"][data-entity-id="sales"] > .aiws-tree-row')
    expect((await row.boundingBox())!.height).toBeGreaterThanOrEqual(48)
    await expect(page.getByTestId('aiws-tree-more-sales')).toBeVisible()
    // its ⋯: the object's menu as a bottom panel
    await page.getByTestId('aiws-tree-more-sales').tap()
    await expect(page.getByTestId('aiws-dm-menu').getByTestId('aiws-dm-menu-rename')).toBeVisible()
    await page.getByTestId('aiws-dm-sheet-close').tap()
    // a table: its editor full screen, a top bar with back, the name, the open count and ⋯
    await page.getByTestId('aiws-tree-sales').tap()
    await expect(mode).toHaveAttribute('data-layer', 'editor')
    await expect(page.getByTestId('aiws-dm-phone-title')).toContainText('原始销售数据')
    await expect(page.getByTestId('aiws-dm-phone-tabs')).toContainText('1')
    await expect(page.getByTestId('aiws-detail-sales')).toBeVisible()
    const editor = (await page.getByTestId('aiws-dm-editor').boundingBox())!
    expect(editor.width).toBeGreaterThan(viewport.width - 2)
    // the sidebar's tabs as a bottom panel, from ⋯
    await page.getByTestId('aiws-dm-phone-more').tap()
    await page.getByTestId('aiws-dm-phone-side-properties').tap()
    const panel = page.getByTestId('aiws-dm-phone-panel')
    await expect(panel.getByTestId('aiws-properties')).toHaveAttribute('data-entity-id', 'sales')
    await panel.getByTestId('aiws-dm-side-tab-history').tap()
    await expect(panel.getByTestId('aiws-versions')).toBeVisible()
    await panel.getByTestId('aiws-dm-phone-panel-close').tap()
    // from the status bar's object line too
    await page.getByTestId('aiws-dm-info').tap()
    await expect(page.getByTestId('aiws-dm-phone-panel')).toBeVisible()
    await page.getByTestId('aiws-dm-phone-panel-close').tap()
    // a second object; the count switches between them
    await page.getByTestId('aiws-dm-phone-back').tap()
    await expect(mode).toHaveAttribute('data-layer', 'tree')
    await page.getByTestId('aiws-tree-intro').tap()
    await expect(page.getByTestId('aiws-dm-phone-tabs')).toContainText('2')
    await page.getByTestId('aiws-dm-phone-tabs').tap()
    await page.locator('[data-testid="aiws-dm-phone-switch"][data-tab-id="sales"] button').first().tap()
    await expect(page.getByTestId('aiws-detail-sales')).toBeVisible()
    // back to the canvas
    await page.getByTestId('aiws-dm-phone-back').tap()
    await page.getByTestId('aiws-top-canvas').tap()
    await expect(page.getByTestId('aiws-canvas')).toBeVisible()
  })

  test('§5.2 “移动到…” on a phone: a folder, then the place in it — first or after a row; its own folder too, which is how a finger reorders', async ({ page, api }) => {
    const ws = await rowsWorkspace(api, `move ${Date.now()}`)
    await openWorkspacePage(page, ALICE, ws.workspace_id)
    await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
    await page.getByTestId('aiws-main-menu').tap()
    await page.getByTestId('aiws-main-menu-list').getByTestId('aiws-menu-data-mode').tap()
    await expect(page.getByTestId('aiws-dm')).toHaveAttribute('data-layer', 'tree')
    const dialog = page.getByTestId('aiws-dm-move-dialog')
    const step = dialog.getByTestId('aiws-dm-move-step')
    const moveTo = async (id: string) => {
      await page.getByTestId(`aiws-tree-more-${id}`).tap()
      await page.getByTestId('aiws-dm-menu').getByTestId('aiws-dm-menu-move').tap()
      await expect(step).toHaveAttribute('data-step', 'folder')
    }
    // Charlie, last in 资料: its own folder is offered (marked), its place there is said and cannot be chosen
    await moveTo('r-c')
    const viewport = page.viewportSize()!
    expect((await dialog.boundingBox())!.width).toBeGreaterThan(viewport.width - 2)
    await expect(dialog.getByTestId('aiws-dm-move-f-docs')).toHaveAttribute('data-current', 'true')
    await dialog.getByTestId('aiws-dm-move-f-docs').tap()
    await expect(step).toHaveAttribute('data-step', 'place')
    await expect(step).toContainText('资料')
    await expect(dialog.getByTestId('aiws-dm-move-after-r-b')).toBeDisabled()
    await expect(dialog.getByTestId('aiws-dm-move-after-r-b')).toHaveAttribute('data-current', 'true')
    await expect(dialog.getByTestId('aiws-dm-move-after-r-c')).toHaveCount(0)
    expect((await dialog.getByTestId('aiws-dm-move-first').boundingBox())!.height).toBeGreaterThanOrEqual(44)
    // to the top: reordered in one Commit
    await dialog.getByTestId('aiws-dm-move-first').tap()
    await expect(dialog).toHaveCount(0)
    await expect.poll(() => order(api, ws.workspace_id, 'f-docs')).toEqual(['r-c', 'r-a', 'r-b'])
    const shown = () => page.locator('[role="treeitem"][data-entity-id="f-docs"] [role="treeitem"]').evaluateAll((els) => els.map((el) => el.getAttribute('data-entity-id')))
    await expect.poll(shown).toEqual(['r-c', 'r-a', 'r-b'])
    // Alpha into another folder, after a row there; "其他文件夹" goes back a step
    await moveTo('r-a')
    await dialog.getByTestId('aiws-dm-move-data').tap()
    await expect(step).toHaveAttribute('data-folder', 'data')
    await dialog.getByTestId('aiws-dm-move-back').tap()
    await expect(step).toHaveAttribute('data-step', 'folder')
    await dialog.getByTestId('aiws-dm-move-data').tap()
    await expect(dialog.getByTestId('aiws-dm-move-after-f-docs')).toBeEnabled()
    await dialog.getByTestId('aiws-dm-move-after-x').tap()
    await expect(dialog).toHaveCount(0)
    // (the system folders follow the data)
    await expect.poll(async () => (await order(api, ws.workspace_id, 'data')).slice(0, 3)).toEqual(['x', 'r-a', 'f-docs'])
    await expect.poll(() => order(api, ws.workspace_id, 'f-docs')).toEqual(['r-c', 'r-b'])
  })
})
