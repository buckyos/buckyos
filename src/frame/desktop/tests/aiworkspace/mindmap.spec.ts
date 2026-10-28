/* Mind maps (doc/workspace/脑图对象方案讨论.md; notepads/aiworkspace-mindmap-todo.md M1–M2): the M0 fixture
 * (src/frame/aiworkspace/fixtures/mindmap/commits.json) drawn by the shared layout — four directions, a pinned node,
 * a nested control, a non-text node — folding as the reader's own state, building a map with the keyboard, dragging
 * nodes (re-hang, pin, refused outside), copy and paste of branches, a map on a flow page and an unknown control kind
 * drawn as a group, a canvas Block dragged onto a node and any object inserted under a node from its toolbar. */

import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import type { Page } from '@playwright/test'
import { blockCenter, fitAll, openCanvas, test, expect, type Api } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'
const SHOTS = 'test-results/aiworkspace-mindmap'
const FIXTURE = resolve('../aiworkspace/fixtures/mindmap/commits.json')

type Json = Record<string, unknown>

/** The M0 fixture replayed (one free canvas "脑图" with the maps), shared with bob. */
async function mindWorkspace(api: Api, title: string) {
  const ws = await api.rpc(ALICE, 'ws.create', { title }) as { workspace_id: string; epoch: string }
  const fixture = JSON.parse(readFileSync(FIXTURE, 'utf8')) as { commits: { operations: Json[] }[] }
  for (const commit of fixture.commits) {
    const result = await api.commit(ALICE, ws, commit.operations) as { status: string }
    expect(result.status, JSON.stringify(result)).toBe('accepted')
  }
  await api.rpc(ALICE, 'ws.grant', { workspace_id: ws.workspace_id, subject: 'bob', capabilities: ['read', 'update', 'append', 'structure', 'delete', 'comment'] })
  return ws
}

async function shot(page: Page, name: string) {
  await page.getByTestId('aiws-canvas-view').screenshot({ path: `${SHOTS}/${name}.png` })
}

async function box(page: Page, id: string) {
  await expect(page.getByTestId(`aiws-canvas-block-${id}`)).toBeVisible()
  const b = await page.getByTestId(`aiws-canvas-block-${id}`).boundingBox()
  if (!b) throw new Error(`${id} is not on screen`)
  return b
}

async function entity(api: Api, workspaceId: string, id: string): Promise<Json | undefined> {
  return (await api.outline(ALICE, workspaceId)).find((e: Json) => e.entity_id === id)
}

async function drag(page: Page, from: { x: number; y: number }, to: { x: number; y: number }, steps = 12) {
  await page.mouse.move(from.x, from.y)
  await page.mouse.down()
  await page.mouse.move(to.x, to.y, { steps })
  await page.mouse.up()
}

test('MM01 the fixture is drawn by the control: four directions, edges, a pin, a nested map, a non-text node', async ({ page, api }) => {
  const ws = await mindWorkspace(api, `mm-fixture ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  // every map draws one edge per parent → child pair
  await expect(page.getByTestId('aiws-mindmap-edges-mm-right')).toHaveAttribute('data-edges', '10')
  await expect(page.getByTestId('aiws-mindmap-edges-mm-down')).toHaveAttribute('data-edges', '10')
  await expect(page.getByTestId('aiws-mindmap-edges-min-inner')).toHaveAttribute('data-edges', '2')
  // right: children right of the root; left: left of it; down: below it
  const root = await box(page, 'right-root')
  expect((await box(page, 'right-1')).x).toBeGreaterThan(root.x + root.width)
  const lroot = await box(page, 'left-root')
  expect((await box(page, 'left-1')).x + (await box(page, 'left-1')).width).toBeLessThan(lroot.x)
  const droot = await box(page, 'down-root')
  expect((await box(page, 'down-1')).y).toBeGreaterThan(droot.y + droot.height)
  // two-sided: the explicit left branch, the pinned one marked
  const broot = await box(page, 'both-root')
  expect((await box(page, 'both-1')).x).toBeLessThan(broot.x)
  await expect(page.getByTestId('aiws-pin-both-3')).toBeVisible()
  // the non-text node keeps its size; the nested map sits inside its parent's branch
  const frame = await box(page, 'min-frame')
  expect(Math.round(frame.width / frame.height * 10) / 10).toBe(Math.round(160 / 90 * 10) / 10)
  await expect(page.getByTestId('aiws-mindnode-inner-root')).toBeVisible()
  await shot(page, 'mm01-fixture')
})

test('MM02 folding is the reader\'s own state: the subtree leaves the layout, another reader still sees it', async ({ page, browser, api }) => {
  // two readers in two browser contexts
  test.setTimeout(180_000)
  const ws = await mindWorkspace(api, `mm-fold ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  const before = await box(page, 'mm-right')
  await page.getByTestId('aiws-canvas-block-right-3').click()
  await page.getByTestId('aiws-fold-right-3').click()
  await expect(page.getByTestId('aiws-canvas-block-right-3-1')).toHaveCount(0)
  await expect(page.getByTestId('aiws-fold-right-3')).toHaveText('3')
  expect((await box(page, 'mm-right')).height).toBeLessThan(before.height)
  // nothing was committed: the outline still has the children
  expect(await entity(api, ws.workspace_id, 'right-3-1')).toBeTruthy()
  // bob sees the whole tree
  const other = await browser.newPage()
  await openCanvas(other, BOB, ws.workspace_id)
  await fitAll(other)
  await expect(other.getByTestId('aiws-canvas-block-right-3-1')).toBeVisible()
  await other.close()
  // Ctrl+/ unfolds it again
  await page.getByTestId('aiws-canvas-block-right-3').click()
  await page.keyboard.press('Control+/')
  await expect(page.getByTestId('aiws-canvas-block-right-3-1')).toBeVisible()
})

test('MM03 a map is built with the keyboard: insert, Tab for a child, Enter for a sibling, arrows, Delete, Ctrl+D', async ({ page, api }) => {
  const ws = await mindWorkspace(api, `mm-keys ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-tool-insert-mindmap').click()
  const view = await page.getByTestId('aiws-canvas').boundingBox()
  await page.mouse.click(view!.x + view!.width - 300, view!.y + 200)
  // the root is edited at once
  const edit = page.locator('[data-testid^="aiws-mindnode-edit-"]')
  await expect(edit).toBeVisible()
  await page.keyboard.press('Control+a')
  await page.keyboard.type('Launch')
  await page.keyboard.press('Tab')
  await expect(edit).toBeVisible()
  await page.keyboard.type('Market')
  await page.keyboard.press('Enter')
  // a selected node: Enter adds a sibling and edits it
  await page.keyboard.press('Enter')
  await expect(edit).toBeVisible()
  await page.keyboard.type('Product')
  await page.keyboard.press('Enter')
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).filter((e: Json) => e.view_type === 'mindnode' && ['Launch', 'Market', 'Product'].includes(e.title as string)).length).toBe(3)
  const outline = await api.outline(ALICE, ws.workspace_id)
  const launch = outline.find((e: Json) => e.title === 'Launch')!
  const market = outline.find((e: Json) => e.title === 'Market')!
  const product = outline.find((e: Json) => e.title === 'Product')!
  expect(market.parent_id).toBe(launch.entity_id)
  expect(product.parent_id).toBe(launch.entity_id)
  expect(market.placement).toEqual({ x: 'auto', y: 'auto', w: 'auto', h: 'auto' })
  // arrows walk the tree
  await page.keyboard.press('ArrowUp')
  await expect(page.getByTestId(`aiws-selection-${market.entity_id}`)).toBeVisible()
  await page.keyboard.press('ArrowLeft')
  await expect(page.getByTestId(`aiws-selection-${launch.entity_id}`)).toBeVisible()
  await page.keyboard.press('ArrowRight')
  await expect(page.getByTestId(`aiws-selection-${market.entity_id}`)).toBeVisible()
  // Ctrl+D copies the branch after it
  await page.keyboard.press('Control+d')
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).filter((e: Json) => e.title === 'Market').length).toBe(2)
  // Delete removes the selected copy
  await page.keyboard.press('Delete')
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).filter((e: Json) => e.title === 'Market').length).toBe(1)
  await shot(page, 'mm03-keyboard')
})

test('MM04 dragging a node: onto another node re-hangs it, onto blank space pins it, outside the map is refused', async ({ page, api }) => {
  const ws = await mindWorkspace(api, `mm-drag ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  // re-hang: 一.2 onto 三
  const from = await blockCenter(page, 'right-1-2')
  const to = await blockCenter(page, 'right-3')
  await page.mouse.move(from.x, from.y)
  await page.mouse.down()
  await page.mouse.move(to.x, to.y, { steps: 12 })
  await expect(page.getByTestId('aiws-drop-child-right-3')).toBeVisible()
  await shot(page, 'mm04-preview')
  await page.mouse.up()
  await expect.poll(async () => (await entity(api, ws.workspace_id, 'right-1-2'))?.parent_id).toBe('right-3')
  expect((await entity(api, ws.workspace_id, 'right-1-2'))?.placement).toEqual({ x: 'auto', y: 'auto', w: 'auto', h: 'auto' })
  // pin: 四 dragged to the blank space below the map's last branch
  const four = await box(page, 'right-4')
  const map = await box(page, 'mm-right')
  await drag(page, { x: four.x + four.width / 2, y: four.y + four.height / 2 }, { x: four.x + four.width / 2 + 60, y: map.y + map.height - 4 })
  await expect.poll(async () => typeof ((await entity(api, ws.workspace_id, 'right-4'))?.placement as Json | undefined)?.x).toBe('number')
  await expect(page.getByTestId('aiws-pin-right-4')).toBeVisible()
  // back to the automatic layout
  await page.getByTestId('aiws-canvas-block-right-4').click()
  await page.getByTestId('aiws-near-mindmap-auto').click()
  await expect.poll(async () => ((await entity(api, ws.workspace_id, 'right-4'))?.placement as Json | undefined)?.x).toBe('auto')
  // refused outside: dropped far away, nothing changes
  const seq = await api.headSeq(ALICE, ws.workspace_id)
  const two = await blockCenter(page, 'right-2')
  const canvas = await page.getByTestId('aiws-canvas').boundingBox()
  await page.mouse.move(two.x, two.y)
  await page.mouse.down()
  await page.mouse.move(canvas!.x + canvas!.width - 20, canvas!.y + canvas!.height - 60, { steps: 12 })
  await expect(page.getByTestId('aiws-drop-refused')).toBeVisible()
  await page.mouse.up()
  await page.waitForTimeout(400)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(seq)
})

test('MM05 copy and paste: a branch pasted under a selected node, nodes pasted on the canvas become a new map', async ({ page, api }) => {
  const ws = await mindWorkspace(api, `mm-paste ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await page.getByTestId('aiws-canvas-block-right-3').click()
  await page.keyboard.press('Control+c')
  await expect(page.getByTestId('aiws-notice').filter({ hasText: '已复制 1 个对象' }).last()).toBeVisible()
  // under the root of the left map
  await page.getByTestId('aiws-canvas-block-left-root').click()
  await page.keyboard.press('Control+v')
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).filter((e: Json) => e.title === '三' && e.parent_id === 'left-root').length).toBe(2)
  const outline = await api.outline(ALICE, ws.workspace_id)
  const copy = outline.find((e: Json) => e.title === '三' && e.parent_id === 'left-root' && e.entity_id !== 'left-3')!
  expect(copy.placement).toEqual({ x: 'auto', y: 'auto', w: 'auto', h: 'auto' })
  expect(outline.filter((e: Json) => e.parent_id === copy.entity_id).length).toBe(3)
  // on the canvas (nothing selected): a new map around the branch
  await page.keyboard.press('Escape')
  await page.keyboard.press('Control+v')
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).filter((e: Json) => e.kind === 'mindmap' && e.parent_id === 's-mind').length).toBe(7)
  await shot(page, 'mm05-paste')
})

test('MM06 a map on a flow page is one self-contained Block', async ({ page, api }) => {
  const ws = await mindWorkspace(api, `mm-flow ${Date.now()}`)
  const result = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'page-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'z1', name: '页', payload: { kind: 'folder', title: '页', system: 'surface_content', surface_id: 'page' } },
    { op: 'entity.create', entity_id: 'page', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'z1', name: '页', payload: { kind: 'surface', layout: { mode: 'flow' }, title: '页', content_folder_id: 'page-content' } },
    { op: 'entity.create', entity_id: 'fm', type_id: 'buckyos.container', parent_id: 'page', order_key: 'a', payload: { kind: 'mindmap', title: '页里的脑图' } },
    { op: 'entity.create', entity_id: 'fm-root', type_id: 'buckyos.cell', parent_id: 'fm', order_key: 'a', payload: { view: { type: 'mindnode' }, title: '根' } },
    { op: 'entity.create', entity_id: 'fm-1', type_id: 'buckyos.cell', parent_id: 'fm-root', order_key: 'a', payload: { view: { type: 'mindnode' }, title: '一' } },
  ]) as { status: string }
  expect(result.status, JSON.stringify(result)).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-surface-switch').click()
  await page.getByTestId('aiws-surface-item-page').getByRole('menuitem').click()
  await expect(page.getByTestId('aiws-flow-mindmap-fm')).toBeVisible()
  await expect(page.getByTestId('aiws-flow-mindmap-fm').locator('svg text')).toHaveCount(2)
  await page.getByTestId('aiws-flow-mindmap-fit-fm').click()
})

test('MM07 a control kind this client does not know is drawn as a group', async ({ page, api }) => {
  const ws = await mindWorkspace(api, `mm-unknown ${Date.now()}`)
  // a newer client's control, as an older reader would get it from the outline
  await page.route('**/kapi/aiworkspace', async (route) => {
    const body = route.request().postDataJSON() as { method?: string } | null
    if (body?.method !== 'doc.outline') { await route.continue(); return }
    const response = await route.fetch()
    const json = await response.json() as { result: { entities: Json[] } }
    json.result.entities.push({ ...json.result.entities.find((e: Json) => e.entity_id === 'mm-min')!, entity_id: 'future', kind: 'carousel', title: '未来的控件', order_key: 'zz', placement: { x: 3000, y: 0, w: 300, h: 200 } })
    await route.fulfill({ response, json })
  })
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await expect(page.getByTestId('aiws-unknown-kind-future')).toBeVisible()
})

test('MM08 a locked node is not re-hung or pinned by a drag', async ({ page, api }) => {
  const ws = await mindWorkspace(api, `mm-lock ${Date.now()}`)
  const read = await api.read(ALICE, ws.workspace_id, 'right-2') as { content: { key_revs: Record<string, number> } }
  const locked = await api.commit(ALICE, ws, [{ op: 'entity.set_keys', entity_id: 'right-2', keys: [{ key: 'locked', value: true, expect: { rev: read.content.key_revs.locked ?? 0 } }] }]) as { status: string }
  expect(locked.status).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  const seq = await api.headSeq(ALICE, ws.workspace_id)
  await drag(page, await blockCenter(page, 'right-2'), await blockCenter(page, 'right-3'))
  await page.waitForTimeout(400)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(seq)
  expect((await entity(api, ws.workspace_id, 'right-2'))?.parent_id).toBe('right-root')
})

test('MM09 a Block of the canvas dragged onto a node hangs under it: laid out by the map, its size kept', async ({ page, api }) => {
  const ws = await mindWorkspace(api, `mm-drag-in ${Date.now()}`)
  const result = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'note-free', type_id: 'buckyos.annotation', parent_id: 's-mind-content', order_key: 'z', payload: { kind: 'note', body: '便签' } },
    { op: 'entity.create', entity_id: 'blk-free', type_id: 'buckyos.cell', parent_id: 's-mind', order_key: 'z', placement: { x: 1500, y: 0, w: 220, h: 160 }, payload: { view: { type: 'note' }, source_ref: { entity_id: 'note-free' } } },
  ]) as { status: string }
  expect(result.status, JSON.stringify(result)).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  const from = await blockCenter(page, 'blk-free')
  const to = await blockCenter(page, 'right-3')
  await page.mouse.move(from.x, from.y)
  await page.mouse.down()
  await page.mouse.move(to.x, to.y, { steps: 14 })
  await expect(page.getByTestId('aiws-drop-child-right-3')).toBeVisible()
  // the dashed preview hangs it next to 三, where the drop puts it (not where it stood on the canvas)
  const three = await box(page, 'right-3')
  const hang = await page.getByTestId('aiws-mindmap-hang').boundingBox()
  expect(hang!.x).toBeGreaterThanOrEqual(three.x + three.width - 2)
  expect(hang!.x + hang!.width).toBeLessThan(three.x + three.width + 100)
  await shot(page, 'mm09-drag-in')
  await page.mouse.up()
  await expect.poll(async () => (await entity(api, ws.workspace_id, 'blk-free'))?.parent_id).toBe('right-3')
  expect((await entity(api, ws.workspace_id, 'blk-free'))?.placement).toEqual({ x: 'auto', y: 'auto', w: 220, h: 160 })
  await expect(page.getByTestId('aiws-mindmap-edges-mm-right')).toHaveAttribute('data-edges', '11')
  const parent = await box(page, 'right-3')
  expect((await box(page, 'blk-free')).x).toBeGreaterThan(parent.x + parent.width)
})

test('MM10 the node toolbar inserts any object from the catalog under the node: a note, a nested map', async ({ page, api }) => {
  const ws = await mindWorkspace(api, `mm-insert ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await page.getByTestId('aiws-canvas-block-right-2').click()
  await page.getByTestId('aiws-near-mindmap-insert').click()
  await expect(page.getByTestId('aiws-catalog')).toBeVisible()
  // the entry hangs under the node: no placing by click, no imports onto the canvas
  await expect(page.getByTestId('aiws-catalog-place')).toHaveCount(0)
  await expect(page.getByTestId('aiws-catalog-import-drawio')).toHaveCount(0)
  await page.getByTestId('aiws-catalog-entry-block-note').click()
  await page.getByTestId('aiws-catalog-insert').click()
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).filter((e: Json) => e.view_type === 'note' && e.parent_id === 'right-2').length).toBe(1)
  const note = (await api.outline(ALICE, ws.workspace_id)).find((e: Json) => e.view_type === 'note' && e.parent_id === 'right-2')!
  const placement = note.placement as Json
  expect([placement.x, placement.y, typeof placement.w, typeof placement.h]).toEqual(['auto', 'auto', 'number', 'number'])
  await page.keyboard.press('Escape')
  await page.keyboard.press('Escape')
  // a control is a node too: a nested map sizes itself
  await page.getByTestId('aiws-canvas-block-right-4').click()
  await page.getByTestId('aiws-near-mindmap-insert').click()
  await page.getByTestId('aiws-catalog-entry-control-mindmap').click()
  await page.getByTestId('aiws-catalog-insert').click()
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).filter((e: Json) => e.kind === 'mindmap' && e.parent_id === 'right-4').length).toBe(1)
  const nested = (await api.outline(ALICE, ws.workspace_id)).find((e: Json) => e.kind === 'mindmap' && e.parent_id === 'right-4')!
  expect(nested.placement).toEqual({ x: 'auto', y: 'auto', w: 'auto', h: 'auto' })
  await expect(page.getByTestId(`aiws-canvas-block-${note.entity_id}`)).toBeVisible()
  await shot(page, 'mm10-insert')
})
