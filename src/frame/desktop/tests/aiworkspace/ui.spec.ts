/* UI improvement §14 (UI-P01…P13): what opens and when, the floating layout, the main toolbar (name,
 * icon, Surfaces, New), insertion and one-shot placement, tools and view navigation, the object
 * clipboard, sub-modes, the collaboration / share / AI entries and keyboard behaviour. Persistence and
 * commit counts are asserted through kRPC and the dev-override hooks, not only through the DOM. */

import type { Page } from '@playwright/test'
import { closeWorkspace, expect, fitAll, hooks, mainMenu, openCanvas, test, type Api } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'

/** A workspace with one empty free Surface `sf-p` (content folder `sf-p-content`). */
async function blankCanvas(api: Api, title: string) {
  const ws = await api.rpc(ALICE, 'ws.create', { title })
  const r = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'sf-p-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'a', name: '空白', payload: { kind: 'folder', title: '空白', system: 'surface_content', surface_id: 'sf-p' } },
    { op: 'entity.create', entity_id: 'sf-p', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'a', name: '空白', payload: { kind: 'surface', layout: { mode: 'free' }, title: '空白', content_folder_id: 'sf-p-content' } },
  ])
  expect(r.status).toBe('accepted')
  return { workspace_id: ws.workspace_id as string, epoch: ws.epoch as string }
}

/** Screen point of a world point on the canvas (the camera transform of the world layer). */
async function screenOf(page: Page, world: { x: number; y: number }) {
  const box = (await page.getByTestId('aiws-canvas').boundingBox())!
  const m = await page.getByTestId('aiws-world').evaluate((el) => new DOMMatrix(getComputedStyle(el).transform))
  return { x: box.x + m.e + world.x * m.a, y: box.y + m.f + world.y * m.d }
}

async function cells(api: Api, workspaceId: string, parentId: string) {
  return (await api.outline(ALICE, workspaceId)).filter((e: { type_id: string; parent_id: string }) => e.type_id === 'buckyos.cell' && e.parent_id === parentId)
}

function overlap(a: { x: number; y: number; width: number; height: number }, b: { x: number; y: number; width: number; height: number }) {
  return a.x < b.x + b.width && a.x + a.width > b.x && a.y < b.y + b.height && a.y + a.height > b.y
}

function pathOf(page: Page) {
  const url = new URL(page.url())
  return url.pathname + url.search
}

test('UI-P01 a normal start reopens the last workspace and Surface; a link wins over it; gone or forbidden targets fall back', async ({ page, api, browser }) => {
  const a = await api.demo(ALICE, 'quarterly', `p01a ${Date.now()}`)
  const b = await api.demo(ALICE, 'quarterly', `p01b ${Date.now()}`)
  await openCanvas(page, ALICE, a.workspace_id)
  await page.getByTestId('aiws-surface-switch').click()
  await page.getByTestId('aiws-surface-item-sf-detail').getByRole('menuitem').click()
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-surface-id', 'sf-detail')
  // a reload restores the workspace and its Surface without going through the list
  await page.reload()
  await page.getByTestId('desktop-app-aiworkspace').click()
  await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-workspace-id', a.workspace_id, { timeout: 30_000 })
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-surface-id', 'sf-detail')
  // closing goes back to the list and pauses the restore for this app session
  await closeWorkspace(page)
  await expect(page.getByTestId('aiws-list')).toBeVisible()
  // an access link opens its own workspace and Surface in a tab of its own; the target leaves the address
  await page.goto(`/workspace/${b.workspace_id}?surface=sf-detail`)
  await expect(page.getByTestId('aiws-tab').getByTestId('aiws-workspace')).toHaveAttribute('data-workspace-id', b.workspace_id, { timeout: 30_000 })
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-surface-id', 'sf-detail')
  await expect.poll(() => pathOf(page)).toBe(`/workspace/${b.workspace_id}`)
  // a link to a Surface that is gone: the first readable Surface, and it says so
  await page.goto(`/workspace/${a.workspace_id}?surface=sf-gone`)
  await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-workspace-id', a.workspace_id, { timeout: 30_000 })
  await expect(page.getByTestId('aiws-notice').filter({ hasText: '链接指向的画布' })).toBeVisible()
  // another identity: no access to the link's workspace, nothing of alice's recent state; the list stays usable
  const context = await browser.newContext({ viewport: { width: 1600, height: 1000 } })
  const bob = await context.newPage()
  await bob.addInitScript((value) => window.localStorage.setItem('aiworkspace.dev', JSON.stringify({ token: value })), BOB)
  await bob.goto(`/workspace/${a.workspace_id}`)
  await expect(bob.getByTestId('aiws-open-error')).toContainText('不存在，或你没有访问权限', { timeout: 30_000 })
  await expect(bob.getByTestId('aiws-list')).toBeVisible()
  expect(await bob.evaluate(() => window.localStorage.getItem('aiworkspace.recent') ?? '')).not.toContain('|bob')
  await context.close()
})

test('UI-P02 three floating toolbars and the bottom-right status area, no navigation there; nothing overlaps from wide to narrow; hit testing matches', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `p02 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await expect(page.getByTestId('aiws-canvas-tools')).toHaveCount(0)
  for (const width of [0, 1024, 768, 390]) {
    if (width) await page.getByTestId('aiws-root').evaluate((root, w) => { root.style.width = `${w}px`; root.style.flex = 'none' }, width)
    await page.waitForTimeout(400)
    await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-size', width === 0 || width >= 1100 ? 'wide' : width >= 760 ? 'medium' : 'narrow')
    const ids = ['aiws-main-toolbar', 'aiws-presenter-toolbar', 'aiws-object-toolbar', 'aiws-status']
    const boxes = await Promise.all(ids.map(async (id) => (await page.getByTestId(id).boundingBox())!))
    for (let i = 0; i < boxes.length; i++) for (let j = i + 1; j < boxes.length; j++) expect(overlap(boxes[i], boxes[j]), `${ids[i]} × ${ids[j]} at ${width}`).toBe(false)
    // the status area is the bottom-right corner of the work area
    const area = (await page.getByTestId('aiws-canvas').boundingBox())!
    const status = boxes[3]
    expect(area.x + area.width - (status.x + status.width)).toBeLessThan(20)
    expect(area.y + area.height - (status.y + status.height)).toBeLessThan(20)
    // the right panel: layout width when wide, a drawer otherwise; it always closes
    await page.getByTestId('aiws-collab').click()
    await expect(page.getByTestId('aiws-side-panel')).toBeVisible()
    await expect(page.getByTestId('aiws-side-panel')).toHaveClass(width === 0 || width >= 1100 ? /aiws-side(?! is-drawer)/ : /is-drawer/)
    await page.getByTestId('aiws-side-close').click()
    await expect(page.getByTestId('aiws-side-panel')).toHaveCount(0)
  }
  // a click on a Block selects that Block: the effective viewport and the hit coordinates agree
  await fitAll(page)
  const target = await page.getByTestId('aiws-canvas-block-blk-kpi').boundingBox()
  await page.mouse.click(target!.x + target!.width / 2, target!.y + target!.height / 2)
  await expect(page.getByTestId('aiws-selection-blk-kpi')).toBeVisible()
})

test('UI-P03 rename in place: Enter one commit, Esc none, a concurrent rename keeps my input; the preset icon is shared', async ({ page, api, browser }) => {
  const ws = await blankCanvas(api, `p03 ${Date.now()}`)
  await api.rpc(ALICE, 'ws.grant', { workspace_id: ws.workspace_id, subject: 'bob', capabilities: ['read'] })
  await openCanvas(page, ALICE, ws.workspace_id)
  const head = await api.headSeq(ALICE, ws.workspace_id)
  // Esc: nothing is written, the blur that follows saves nothing
  await page.getByTestId('aiws-surface-name').click()
  await page.getByTestId('aiws-surface-name-input').fill('不要保存')
  await page.keyboard.press('Escape')
  await page.waitForTimeout(400)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head)
  await expect(page.getByTestId('aiws-surface-name')).toHaveText('空白')
  // Enter: one commit, title and name together
  await page.getByTestId('aiws-surface-name').click()
  await page.getByTestId('aiws-surface-name-input').fill('季度复盘')
  await page.keyboard.press('Enter')
  await expect.poll(async () => (await api.read(ALICE, ws.workspace_id, 'sf-p')).name).toBe('季度复盘')
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
  expect((await api.read(ALICE, ws.workspace_id, 'sf-p')).content.payload.title).toBe('季度复盘')
  // an empty name restores the old one
  await page.getByTestId('aiws-surface-name').click()
  await page.getByTestId('aiws-surface-name-input').fill('  ')
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('aiws-surface-name')).toHaveText('季度复盘')
  // someone renames while I edit: my input stays and the refusal is explained
  await page.getByTestId('aiws-surface-name').click()
  await page.getByTestId('aiws-surface-name-input').fill('我的名字')
  const read = await api.read(ALICE, ws.workspace_id, 'sf-p')
  expect((await api.commit(ALICE, ws, [
    { op: 'entity.set_keys', entity_id: 'sf-p', keys: [{ key: 'title', value: '别人的名字', expect: { rev: read.content.key_revs.title } }] },
    { op: 'entity.rename', entity_id: 'sf-p', name: '别人的名字', expect: { rev: read.meta_rev } },
  ])).status).toBe('accepted')
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('aiws-surface-name-error')).toContainText('其他人刚改过')
  await expect(page.getByTestId('aiws-surface-name-input')).toHaveValue('我的名字')
  expect((await api.read(ALICE, ws.workspace_id, 'sf-p')).name).toBe('别人的名字')
  await page.keyboard.press('Escape')
  // the preset icon: a shared Surface property, seen by another client and kept by an export / import
  await page.getByTestId('aiws-surface-icon').click()
  await page.getByTestId('aiws-icon-rocket').click()
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).find((e: { entity_id: string }) => e.entity_id === 'sf-p')?.icon).toBe('rocket')
  const context = await browser.newContext({ viewport: { width: 1600, height: 1000 } })
  const bob = await context.newPage()
  await openCanvas(bob, BOB, ws.workspace_id)
  await expect(bob.getByTestId('aiws-surface-icon')).toHaveAttribute('data-icon', 'rocket')
  await expect(bob.getByTestId('aiws-surface-icon')).toBeDisabled()
  await context.close()
  const exported = await api.rpc(ALICE, 'doc.export', { workspace_id: ws.workspace_id, mode: 'share', self_contained: true })
  const pkg = await (await fetch(`${api.base}/export/${ws.workspace_id}/${exported.export_id}`, { headers: { authorization: `Bearer ${ALICE}` } })).arrayBuffer()
  const begin = await api.rpc(ALICE, 'ws.begin_import', {})
  await fetch(`${api.base}/upload/${begin.upload_id}`, { method: 'PUT', headers: { authorization: `Bearer ${ALICE}` }, body: Buffer.from(pkg) })
  const imported = await api.rpc(ALICE, 'ws.import', { upload_id: begin.upload_id, semantics: 'new' })
  expect((await api.outline(ALICE, imported.workspace_id)).find((e: { entity_id: string }) => e.entity_id === 'sf-p')?.icon).toBe('rocket')
})

test('UI-P04 New: a canvas with its content folder in one commit; a template creates a new workspace and leaves this one alone', async ({ page, api }) => {
  const ws = await blankCanvas(api, `p04 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  const head = await api.headSeq(ALICE, ws.workspace_id)
  await page.getByTestId('aiws-surface-switch').click()
  await page.getByTestId('aiws-surface-new').click()
  await page.getByLabel('新画布标题').fill('流式草稿')
  await page.getByTestId('aiws-new-flow').check()
  await page.getByRole('option', { name: '看板' }).click()
  await page.getByTestId('aiws-surface-create').click()
  await expect(page.getByTestId('aiws-surface-name')).toHaveText('流式草稿')
  await expect(page.getByTestId('aiws-flow')).toBeVisible()
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
  const outline = await api.outline(ALICE, ws.workspace_id)
  const created = outline.find((e: { title?: string; kind?: string }) => e.kind === 'surface' && e.title === '流式草稿')
  expect(created.layout.mode).toBe('flow')
  expect(created.icon).toBe('kanban')
  expect(outline.find((e: { entity_id: string }) => e.entity_id === created.content_folder_id)?.parent_id).toBe('canvas-content')
  // a template: always a new workspace, opened at once; the simulated content is labelled
  await mainMenu(page, 'aiws-menu-new')
  await page.getByTestId('aiws-new-tab-template').click()
  await page.getByTestId('aiws-template-film').click()
  await expect(page.getByTestId('aiws-template-film')).toContainText('含模拟内容')
  await page.getByLabel('工作区标题', { exact: true }).fill(`p04 模板 ${Date.now()}`)
  await page.getByTestId('aiws-create-template').click()
  await expect(page.getByTestId('aiws-workspace')).not.toHaveAttribute('data-workspace-id', ws.workspace_id, { timeout: 60_000 })
  await expect(page.getByTestId('aiws-canvas-block-blk-wish-1')).toBeVisible()
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
})

test('UI-P05 insertion: one-shot placement (Esc writes nothing), the main menu, the context menu and existing data as a view only', async ({ page, api }) => {
  const ws = await blankCanvas(api, `p05 ${Date.now()}`)
  const data = await api.commit(ALICE, ws, [{ op: 'entity.create', entity_id: 'tbl', type_id: 'buckyos.table-source', parent_id: 'data', order_key: 'b', name: '清单', payload: { fields: [{ field_id: 'title', name: '标题', type: 'text' }] } }])
  expect(data.status).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  const head = await api.headSeq(ALICE, ws.workspace_id)
  // pick a note, then Esc: zero writes
  await page.getByTestId('aiws-tool-insert-note').click()
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-tool', 'place')
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-tool', 'select')
  // pick a note and click: one commit at the clicked point, then typing goes into the note at once
  await page.getByTestId('aiws-tool-insert-note').click()
  const at = await screenOf(page, { x: 300, y: 260 })
  await page.mouse.move(at.x - 10, at.y - 10)
  await page.mouse.move(at.x, at.y)
  await expect(page.getByTestId('aiws-place-ghost')).toBeVisible()
  await page.mouse.click(at.x, at.y)
  await expect.poll(async () => (await cells(api, ws.workspace_id, 'sf-p')).length).toBe(1)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
  const note = (await cells(api, ws.workspace_id, 'sf-p'))[0]
  expect(Math.abs(note.placement.x - 300)).toBeLessThan(3)
  expect(Math.abs(note.placement.y - 260)).toBeLessThan(3)
  await expect(page.getByLabel('便签内容')).toBeFocused()
  await page.keyboard.type('放置后直接输入')
  await page.getByTestId('aiws-canvas').click({ position: { x: 900, y: 700 } })
  await expect.poll(async () => (await api.read(ALICE, ws.workspace_id, note.source_id)).content.payload.body).toBe('放置后直接输入')
  // main menu → 插入 → 布局 → 框: at the centre of the view, without asking for a title
  await mainMenu(page, 'aiws-menu-insert', 'aiws-menu-insert-group-layout', 'aiws-main-insert-frame')
  await expect.poll(async () => (await cells(api, ws.workspace_id, 'sf-p')).some((c: { view_type: string }) => c.view_type === 'frame')).toBe(true)
  // context menu on a blank spot: the shape lands where the menu was opened
  const spot = await screenOf(page, { x: 100, y: 700 })
  await page.mouse.click(spot.x, spot.y, { button: 'right' })
  await page.getByTestId('aiws-menu-insert-shape').click()
  await expect.poll(async () => (await cells(api, ws.workspace_id, 'sf-p')).some((c: { view_type: string }) => c.view_type === 'shape')).toBe(true)
  const shape = (await cells(api, ws.workspace_id, 'sf-p')).find((c: { view_type: string }) => c.view_type === 'shape')
  expect(Math.abs(shape.placement.x - 100)).toBeLessThan(3)
  expect(Math.abs(shape.placement.y - 700)).toBeLessThan(3)
  // existing data: a view of the same table, no new data entity
  const tables = async () => (await api.outline(ALICE, ws.workspace_id)).filter((e: { type_id: string }) => e.type_id === 'buckyos.table-source').length
  expect(await tables()).toBe(1)
  await mainMenu(page, 'aiws-menu-add-data')
  await expect(page.getByTestId('aiws-catalog-existing')).toContainText('与原视图共享数据')
  await page.getByTestId('aiws-picker-data').selectOption('tbl')
  await page.getByTestId('aiws-picker-confirm').click()
  await expect.poll(async () => (await cells(api, ws.workspace_id, 'sf-p')).some((c: { source_id: string }) => c.source_id === 'tbl')).toBe(true)
  expect(await tables()).toBe(1)
  // a sample renderer needs its data first: the catalog does not insert it empty
  await page.getByTestId('aiws-tool-catalog').click()
  await page.getByTestId('aiws-catalog-tab-sample').click()
  await page.getByTestId('aiws-catalog-entry-block-sample.metric').click()
  await expect(page.getByTestId('aiws-catalog-insert')).toBeDisabled()
  await expect(page.getByTestId('aiws-catalog-missing')).toContainText('先选择要显示的数据')
  await page.keyboard.press('Escape')
})

test('UI-P06 hand tool and space pan, zoom from the presenter toolbar and the main menu; the viewport never writes the document', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `p06 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  const head = await api.headSeq(ALICE, ws.workspace_id)
  const before = (await page.getByTestId('aiws-canvas-block-blk-kpi').boundingBox())!
  // the hand tool drags the view, even over a Block, and never selects
  await page.keyboard.press('h')
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-tool', 'hand')
  await page.mouse.move(before.x + 40, before.y + 40)
  await page.mouse.down()
  await page.mouse.move(before.x - 60, before.y + 90, { steps: 6 })
  await page.mouse.up()
  const after = (await page.getByTestId('aiws-canvas-block-blk-kpi').boundingBox())!
  expect(Math.round(after.x - before.x)).toBe(-100)
  await expect(page.locator('[data-testid^="aiws-selection-"]')).toHaveCount(0)
  await page.keyboard.press('v')
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-tool', 'select')
  // zoom: the panel's preset and the main menu act on the same camera
  await page.getByTestId('aiws-zoom-menu').click()
  await page.getByTestId('aiws-zoom-150').click()
  await expect(page.getByTestId('aiws-zoom')).toHaveText('150%')
  await page.keyboard.press('Escape')
  await mainMenu(page, 'aiws-menu-zoom', 'aiws-menu-zoom-50')
  await expect(page.getByTestId('aiws-zoom')).toHaveText('50%')
  // the presenter toolbar can be hidden; the main menu keeps view navigation reachable
  await mainMenu(page, 'aiws-menu-view', 'aiws-menu-pref-presenter-toolbar')
  await expect(page.getByTestId('aiws-presenter-toolbar')).toHaveCount(0)
  await mainMenu(page, 'aiws-menu-zoom', 'aiws-menu-fit-all')
  await expect(page.getByTestId('aiws-canvas')).not.toHaveAttribute('data-zoom', '0.50')
  await mainMenu(page, 'aiws-menu-view', 'aiws-menu-reset-layout')
  await expect(page.getByTestId('aiws-presenter-toolbar')).toBeVisible()
  await page.waitForTimeout(500)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head)
  expect((await hooks(page)).commits).toBe(0)
})

test('UI-P07 object clipboard: copy a group (new ids, shared data), cut across Surfaces (same identity), editors keep their text clipboard', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `p07 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  // group the KPI card and the wish, then copy and paste the group: one commit, new identities, same sources
  await page.getByTestId('aiws-canvas-block-blk-kpi').click()
  await page.getByTestId('aiws-canvas-block-blk-wish').click({ modifiers: ['Shift'] })
  await page.keyboard.press('Control+g')
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).find((e: { entity_id: string }) => e.entity_id === 'blk-kpi')?.parent_id).not.toBe('sf-analysis')
  const groupId = (await api.outline(ALICE, ws.workspace_id)).find((e: { entity_id: string }) => e.entity_id === 'blk-kpi')?.parent_id as string
  await expect(page.getByTestId(`aiws-selection-${groupId}`)).toBeVisible()
  const head = await api.headSeq(ALICE, ws.workspace_id)
  await page.keyboard.press('Control+c')
  await expect(page.getByTestId('aiws-notice').filter({ hasText: '已复制 1 个对象' })).toBeVisible()
  await page.keyboard.press('Control+v')
  await expect.poll(async () => api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
  const outline = await api.outline(ALICE, ws.workspace_id)
  const groups = outline.filter((e: { kind?: string; parent_id: string }) => e.kind === 'group' && e.parent_id === 'sf-analysis')
  expect(groups).toHaveLength(2)
  const copy = groups.find((g: { entity_id: string }) => g.entity_id !== groupId)
  const kids = outline.filter((e: { parent_id: string }) => e.parent_id === copy.entity_id)
  expect(kids.map((k: { source_id: string }) => k.source_id).sort()).toEqual(['sales', 'wish-analysis'])
  expect(kids.some((k: { entity_id: string }) => k.entity_id === 'blk-kpi' || k.entity_id === 'blk-wish')).toBe(false)
  await page.getByTestId('aiws-undo').click()
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).filter((e: { kind?: string; parent_id: string }) => e.kind === 'group' && e.parent_id === 'sf-analysis')).toHaveLength(1)
  // cut the intro and paste it on the other Surface: moved in one commit, identity and binding kept
  await page.getByTestId('aiws-canvas').click({ position: { x: 600, y: 750 } })
  await page.getByTestId('aiws-canvas-block-blk-intro').click({ position: { x: 20, y: 10 } })
  await page.keyboard.press('Control+x')
  await page.getByTestId('aiws-surface-switch').click()
  await page.getByTestId('aiws-surface-item-sf-detail').getByRole('menuitem').click()
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-surface-id', 'sf-detail')
  const beforeCut = await api.headSeq(ALICE, ws.workspace_id)
  await page.getByTestId('aiws-canvas').click({ position: { x: 600, y: 750 } })
  await page.keyboard.press('Control+v')
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).find((e: { entity_id: string }) => e.entity_id === 'blk-intro')?.parent_id).toBe('sf-detail')
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(beforeCut + 1)
  expect((await api.outline(ALICE, ws.workspace_id)).find((e: { entity_id: string }) => e.entity_id === 'blk-intro')?.source_id).toBe('intro')
  // inside the rich text editor, copy and paste are the editor's: no Block is created
  await page.getByTestId('aiws-canvas-block-blk-intro').dblclick()
  const prose = page.getByTestId('aiws-canvas-block-blk-intro').locator('.aiws-prose[contenteditable="true"]')
  await expect(prose).toBeVisible()
  await prose.click()
  const copied = await page.getByTestId('aiws-notice').filter({ hasText: '已复制' }).count()
  await page.keyboard.press('Control+a')
  await page.keyboard.press('Control+c')
  const blocks = (await cells(api, ws.workspace_id, 'sf-detail')).length
  await page.keyboard.press('End')
  await page.keyboard.press('Control+v')
  await page.waitForTimeout(600)
  expect((await cells(api, ws.workspace_id, 'sf-detail')).length).toBe(blocks)
  expect(await page.getByTestId('aiws-notice').filter({ hasText: '已复制' }).count()).toBeLessThanOrEqual(copied)
})

test('UI-P08 view mode hides creation, the data-source view keeps the session; presentation edit is a labelled placeholder', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `p08 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  const session = await page.getByTestId('aiws-workspace').getAttribute('data-session-id')
  await expect(page.getByTestId('aiws-tool-insert-richtext')).toBeVisible()
  await mainMenu(page, 'aiws-menu-canvas-mode', 'aiws-mode-view')
  await expect(page.getByTestId('aiws-mode-note')).toHaveText('查看')
  await expect(page.getByTestId('aiws-tool-insert-richtext')).toHaveCount(0)
  await expect(page.getByTestId('aiws-undo')).toBeVisible()
  await page.getByTestId('aiws-main-menu').click()
  await expect(page.getByTestId('aiws-menu-insert')).toBeDisabled()
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('aiws-main-menu')).toBeFocused()
  // annotations stay available in view mode
  await expect(page.getByTestId('aiws-annotate-start')).toBeEnabled()
  await mainMenu(page, 'aiws-menu-canvas-mode', 'aiws-mode-presentation_edit')
  await expect(page.getByTestId('aiws-presentation-placeholder')).toContainText('尚未实现')
  await page.getByTestId('aiws-main-menu').click()
  await expect(page.getByTestId('aiws-top-play')).toBeDisabled()
  await expect(page.getByTestId('aiws-top-play')).toContainText('当前不可用')
  await page.keyboard.press('Escape')
  // the presentation-edit placeholder is never where a workspace reopens
  await closeWorkspace(page)
  await page.locator(`[data-testid="aiws-workspace-card"][data-workspace-id="${ws.workspace_id}"]`).getByTestId('aiws-open').click()
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-mode', 'view', { timeout: 30_000 })
  await expect(page.getByTestId('aiws-notice').filter({ hasText: '已切换到查看模式' })).toBeVisible()
  // data source and back: same session, the toolbar says where it leads
  const reopened = await page.getByTestId('aiws-workspace').getAttribute('data-session-id')
  await page.getByTestId('aiws-top-sources').click()
  await expect(page.getByTestId('aiws-sources')).toBeVisible()
  await expect(page.getByTestId('aiws-top-canvas')).toHaveText('返回画布')
  await page.getByTestId('aiws-top-canvas').click()
  await expect(page.getByTestId('aiws-canvas')).toBeVisible()
  expect(await page.getByTestId('aiws-workspace').getAttribute('data-session-id')).toBe(reopened)
  expect(reopened).not.toBe(session)
})

test('UI-P10/P11 collaboration opens the permissions; the share link points at the Surface, carries no token and grants nothing', async ({ page, api, browser }) => {
  const ws = await api.demo(ALICE, 'quarterly', `p10 ${Date.now()}`)
  await api.rpc(ALICE, 'ws.grant', { workspace_id: ws.workspace_id, subject: 'bob', capabilities: ['read'] })
  await openCanvas(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-collab').click()
  await expect(page.getByTestId('aiws-side-panel').getByTestId('aiws-permissions')).toContainText('全部授权')
  await expect(page.locator('[data-testid="aiws-grant"][data-subject="bob"]')).toBeVisible()
  await page.getByTestId('aiws-share').click()
  const link = await page.getByTestId('aiws-share-link').inputValue()
  expect(new URL(link).pathname).toBe(`/workspace/${ws.workspace_id}`)
  expect(new URL(link).searchParams.get('surface')).toBe('sf-analysis')
  expect(link).not.toContain('tok-')
  await expect(page.getByTestId('aiws-share-panel')).toContainText('不会授予权限')
  // bob (read only) opens the link: the right Surface, his own grants only, no way to write
  const context = await browser.newContext({ viewport: { width: 1600, height: 1000 } })
  const bob = await context.newPage()
  await bob.addInitScript((value) => window.localStorage.setItem('aiworkspace.dev', JSON.stringify({ token: value })), BOB)
  await bob.goto(link.replace(/^https?:\/\/[^/]+/, ''))
  await expect(bob.getByTestId('aiws-workspace')).toHaveAttribute('data-workspace-id', ws.workspace_id, { timeout: 30_000 })
  await expect(bob.getByTestId('aiws-canvas')).toHaveAttribute('data-surface-id', 'sf-analysis')
  await bob.getByTestId('aiws-collab').click()
  await expect(bob.getByTestId('aiws-permissions')).toContainText('只显示对我生效的授权')
  await expect(bob.getByTestId('aiws-grant-form')).toHaveCount(0)
  await expect(bob.getByTestId('aiws-tool-insert-note')).toHaveCount(0)
  await expect(bob.getByTestId('aiws-presence-unavailable')).toHaveCount(0)
  await bob.getByTestId('aiws-principal').click()
  await expect(bob.getByTestId('aiws-presence-unavailable')).toContainText('授权名单不等于在线成员')
  await expect(bob.getByTestId('aiws-logout')).toBeDisabled()
  await context.close()
})

test('UI-P12 the AI entry creates a wish where it is placed and opens its task; with a wish selected it opens that one', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `p12 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  // a selected wish: the AI entry opens its existing task flow, nothing is created
  const head = await api.headSeq(ALICE, ws.workspace_id)
  await page.getByTestId('aiws-canvas-block-blk-wish').click()
  await page.getByTestId('aiws-tool-insert-wish').click()
  // on a free canvas the task opens in the right panel (标准对象的交互改进 R6)
  const wishPanel = page.locator('[data-testid="aiws-side-panel"][data-tab="wish"]')
  await expect(wishPanel.getByTestId('aiws-wish-prompt')).toHaveValue(/经营分析/)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head)
  await page.getByTestId('aiws-side-close').click()
  await page.keyboard.press('Escape')
  await page.getByTestId('aiws-canvas').click({ position: { x: 600, y: 750 } })
  // nothing selected: place a new wish; its panel opens for the task, nothing runs by itself
  await page.getByTestId('aiws-tool-insert-wish').click()
  const at = await screenOf(page, { x: 40, y: 700 })
  await page.mouse.move(at.x, at.y)
  await page.mouse.click(at.x, at.y)
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).filter((e: { type_id: string }) => e.type_id === 'buckyos.wish').length).toBe(2)
  const created = (await api.outline(ALICE, ws.workspace_id)).find((e: { type_id: string; entity_id: string }) => e.type_id === 'buckyos.wish' && e.entity_id !== 'wish-analysis')
  const block = (await cells(api, ws.workspace_id, 'sf-analysis')).find((c: { source_id: string }) => c.source_id === created.entity_id)
  await expect(page.getByTestId(`aiws-canvas-block-${block.entity_id}`)).toBeVisible()
  await expect(wishPanel.getByTestId(`aiws-wish-${created.entity_id}`)).toBeVisible()
  await expect(wishPanel.getByTestId('aiws-wish-prompt')).toBeVisible()
  expect((await api.read(ALICE, ws.workspace_id, created.entity_id)).content.payload.last_run ?? null).toBeNull()
})

test('UI-P13 keyboard: menus by arrows and Esc with focus back on the trigger; V/H only with the canvas focused; help lists the shortcuts', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `p13 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-main-menu').focus()
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('aiws-main-menu-list')).toBeVisible()
  await expect(page.getByTestId('aiws-menu-new-canvas')).toBeFocused()
  await page.keyboard.press('ArrowDown')
  await expect(page.getByTestId('aiws-menu-new')).toBeFocused()
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('aiws-main-menu-list')).toHaveCount(0)
  await expect(page.getByTestId('aiws-main-menu')).toBeFocused()
  // help through the menu, closed by Esc
  await mainMenu(page, 'aiws-menu-help', 'aiws-menu-shortcuts')
  await expect(page.getByTestId('aiws-help-dialog')).toContainText('V / H')
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('aiws-help-dialog')).toHaveCount(0)
  // typing in an editor never switches the tool
  await page.getByTestId('aiws-canvas-block-blk-intro').dblclick()
  const prose = page.getByTestId('aiws-canvas-block-blk-intro').locator('.aiws-prose[contenteditable="true"]')
  await prose.click()
  await page.keyboard.type('vh')
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-tool', 'select')
  await page.keyboard.press('Escape')
  await page.getByTestId('aiws-canvas').click({ position: { x: 600, y: 750 } })
  await page.keyboard.press('h')
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-tool', 'hand')
  await page.keyboard.press('v')
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-tool', 'select')
})

test('UI-P01b closing with nothing unsaved asks nothing; the list keeps a New entry for blank workspaces and templates', async ({ page, api }) => {
  const ws = await blankCanvas(api, `p01b ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await closeWorkspace(page)
  await expect(page.getByTestId('aiws-leave-dialog')).toHaveCount(0)
  await expect(page.getByTestId('aiws-list')).toBeVisible()
  await page.getByTestId('aiws-new').click()
  await expect(page.getByTestId('aiws-new-dialog')).toBeVisible()
  await expect(page.getByTestId('aiws-new-tab-canvas')).toHaveCount(0)
  await page.getByTestId('aiws-new-tab-template').click()
  await expect(page.getByTestId('aiws-create-template')).toHaveText('从模板创建工作区')
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('aiws-new-dialog')).toHaveCount(0)
})

test('UI-P15 a workspace in a browser tab of its own: the address names what is open, back / forward / reload follow it, the Desktop window opens it in a new tab', async ({ page, api, context }) => {
  const a = await api.demo(ALICE, 'quarterly', `p14a ${Date.now()}`)
  const b = await api.demo(ALICE, 'quarterly', `p14b ${Date.now()}`)
  await page.addInitScript((value) => window.localStorage.setItem('aiworkspace.dev', JSON.stringify({ token: value })), ALICE)
  const card = (id: string) => page.locator(`[data-testid="aiws-workspace-card"][data-workspace-id="${id}"]`)
  // the list: no automatic restore in a tab
  await page.goto('/workspace')
  await expect(page.getByTestId('aiws-tab').getByTestId('aiws-list')).toBeVisible({ timeout: 30_000 })
  await expect(page).toHaveTitle('AI Workspace')
  await card(a.workspace_id).getByTestId('aiws-open').click()
  await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-workspace-id', a.workspace_id, { timeout: 30_000 })
  await expect.poll(() => pathOf(page)).toBe(`/workspace/${a.workspace_id}`)
  await expect(page).toHaveTitle(/p14a .* - AI Workspace/)
  // the workspace fills the tab
  const box = (await page.getByTestId('aiws-root').boundingBox())!
  const viewport = page.viewportSize()!
  expect(Math.round(box.width)).toBe(viewport.width)
  expect(Math.round(box.height)).toBe(viewport.height)
  // closing goes to the list address; opening another one is a new history entry
  await closeWorkspace(page)
  await expect(page.getByTestId('aiws-list')).toBeVisible()
  await expect.poll(() => pathOf(page)).toBe('/workspace')
  await expect(page).toHaveTitle('AI Workspace')
  await card(b.workspace_id).getByTestId('aiws-open').click()
  await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-workspace-id', b.workspace_id, { timeout: 30_000 })
  await expect.poll(() => pathOf(page)).toBe(`/workspace/${b.workspace_id}`)
  // back and forward walk the same entries
  await page.goBack()
  await expect(page.getByTestId('aiws-list')).toBeVisible({ timeout: 30_000 })
  await page.goBack()
  await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-workspace-id', a.workspace_id, { timeout: 30_000 })
  await page.goForward()
  await page.goForward()
  await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-workspace-id', b.workspace_id, { timeout: 30_000 })
  // switching from the main menu's New dialog / fork lands in the address too: here, the menu's "open workspace" + list
  await mainMenu(page, 'aiws-menu-open-workspace')
  await expect(page.getByTestId('aiws-list')).toBeVisible()
  await card(a.workspace_id).getByTestId('aiws-open').click()
  await expect.poll(() => pathOf(page)).toBe(`/workspace/${a.workspace_id}`)
  // a reload reopens what the address names, without the list in between
  await page.reload()
  await expect(page.getByTestId('aiws-opening').or(page.getByTestId('aiws-workspace'))).toBeVisible({ timeout: 30_000 })
  await expect(page.getByTestId('aiws-list')).toHaveCount(0)
  await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-workspace-id', a.workspace_id, { timeout: 30_000 })
  // back to the Desktop from the main menu
  await mainMenu(page, 'aiws-menu-home')
  await expect.poll(() => pathOf(page)).toBe('/')
  await expect(page.getByTestId('desktop-app-aiworkspace')).toBeVisible({ timeout: 30_000 })
  await expect(page).toHaveTitle('BuckyOS')

  // in a Desktop window, the main menu opens the workspace (and its Surface) in a new tab
  await openCanvas(page, ALICE, b.workspace_id)
  await page.getByTestId('aiws-surface-switch').click()
  await page.getByTestId('aiws-surface-item-sf-detail').getByRole('menuitem').click()
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-surface-id', 'sf-detail')
  await expect(page.getByTestId('aiws-menu-home')).toHaveCount(0)
  const opened = context.waitForEvent('page')
  await mainMenu(page, 'aiws-menu-new-tab')
  const tab = await opened
  await expect(tab.getByTestId('aiws-tab').getByTestId('aiws-workspace')).toHaveAttribute('data-workspace-id', b.workspace_id, { timeout: 30_000 })
  await expect(tab.getByTestId('aiws-canvas')).toHaveAttribute('data-surface-id', 'sf-detail')
  await expect.poll(() => pathOf(tab)).toBe(`/workspace/${b.workspace_id}`)
  await tab.close()
})

test('P0 stacking (connector plan §9.3): painting and hit testing follow the BlockTree order; front / back change both and move no frame', async ({ page, api }) => {
  const ws = await blankCanvas(api, `stack ${Date.now()}`)
  const shape = (id: string, parent: string, key: string, placement: { x: number; y: number; w: number; h: number }, fill: string) =>
    ({ op: 'entity.create', entity_id: id, type_id: 'buckyos.cell', parent_id: parent, order_key: key, placement, payload: { view: { type: 'shape' }, title: id, config: { shape: 'rect', fill, stroke: '#4f8df7' } } })
  const r = await api.commit(ALICE, ws, [
    shape('shp-a', 'sf-p', 'b', { x: 200, y: 200, w: 200, h: 140 }, '#fde68a'),
    shape('shp-b', 'sf-p', 'c', { x: 300, y: 260, w: 200, h: 140 }, '#bfdbfe'),
    // a group first in sibling order: its child sits above the group, not above the group's later siblings
    { op: 'entity.create', entity_id: 'grp-g', type_id: 'buckyos.container', parent_id: 'sf-p', order_key: 'a', placement: { x: 650, y: 150, w: 320, h: 260 }, payload: { kind: 'group', layout: { mode: 'free' }, title: '分组' } },
    shape('shp-c', 'grp-g', 'a', { x: 20, y: 40, w: 200, h: 140 }, '#bbf7d0'),
    shape('shp-d', 'sf-p', 'd', { x: 780, y: 260, w: 200, h: 140 }, '#fecaca'),
  ])
  expect(r.status, JSON.stringify(r)).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  await expect(page.getByTestId('aiws-canvas-block-shp-d')).toBeVisible()
  /** The Block drawn on top at a world point (what the eye sees). */
  const paintedAt = async (world: { x: number; y: number }) => {
    const p = await screenOf(page, world)
    return page.evaluate(({ x, y }) => document.elementFromPoint(x, y)?.closest('[data-block-id]')?.getAttribute('data-block-id') ?? null, p)
  }
  /** Click a world point and return what got selected (what the hit test picked). */
  const selectedBy = async (world: { x: number; y: number }) => {
    const p = await screenOf(page, world)
    await page.mouse.click(p.x, p.y)
    await expect(page.locator('[data-testid^="aiws-selection-"]')).toHaveCount(1)
    return (await page.locator('[data-testid^="aiws-selection-"]').getAttribute('data-testid'))!.replace('aiws-selection-', '')
  }
  const frameOrder = () => page.locator('[data-testid="aiws-world"] > [data-block-id]').evaluateAll((els) => els.map((el) => el.getAttribute('data-block-id')))
  const ab = { x: 350, y: 300 }
  const cd = { x: 825, y: 295 }
  // siblings: the later order_key is drawn and hit on top
  expect(await paintedAt(ab)).toBe('shp-b')
  expect(await selectedBy(ab)).toBe('shp-b')
  // across depths: a later sibling of the group is above the group's child
  expect(await paintedAt(cd)).toBe('shp-d')
  expect(await selectedBy(cd)).toBe('shp-d')
  // bring A to front: one commit; drawing and hit testing both follow, and no frame moves in the DOM
  const before = await frameOrder()
  const head = await api.headSeq(ALICE, ws.workspace_id)
  const aOnly = await screenOf(page, { x: 240, y: 300 })
  await page.mouse.click(aOnly.x, aOnly.y, { button: 'right' })
  await page.getByTestId('aiws-menu-front').click()
  await expect.poll(async () => api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
  await expect.poll(() => paintedAt(ab)).toBe('shp-a')
  expect(await selectedBy(ab)).toBe('shp-a')
  expect(await frameOrder()).toEqual(before)
  // and send it back
  await page.mouse.click(aOnly.x, aOnly.y, { button: 'right' })
  await page.getByTestId('aiws-menu-back').click()
  await expect.poll(() => paintedAt(ab)).toBe('shp-b')
  expect(await selectedBy(ab)).toBe('shp-b')
})
