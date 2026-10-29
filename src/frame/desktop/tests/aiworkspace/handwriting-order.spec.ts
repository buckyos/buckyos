import type { Page } from '@playwright/test'
import { test, expect, openCanvas, expectAllCommitted, type Api } from './fixtures'

const ALICE = 'tok-alice'

async function workspace(api: Api, lastKey: string) {
  const ws = await api.rpc(ALICE, 'ws.create', { title: 'Handwriting order regression' }) as { workspace_id: string; epoch: string }
  const outcome = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'notes-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'a1', payload: { kind: 'folder', title: 'Notes', system: 'surface_content', surface_id: 'notes' } },
    { op: 'entity.create', entity_id: 'notes', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'a1', payload: { kind: 'surface', layout: { mode: 'free' }, title: 'Notes', content_folder_id: 'notes-content' } },
    ...['first', 'last'].map((id, i) => ({ op: 'entity.create', entity_id: id, type_id: 'buckyos.cell', parent_id: 'notes', order_key: i ? lastKey : 'a', placement: { x: 100 + i * 200, y: 100, w: 100, h: 100 }, payload: { view: { type: 'shape' }, title: id, config: { shape: 'rect', fill: '#e8f0fe' } } })),
  ])
  expect(outcome.status, JSON.stringify(outcome)).toBe('accepted')
  return ws
}

async function writeDots(page: Page, count: number) {
  await page.getByTestId('aiws-tool-pen').click()
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-tool', 'pen')
  const box = (await page.getByTestId('aiws-canvas').boundingBox())!
  const cdp = await page.context().newCDPSession(page)
  for (let i = 0; i < count; i++) {
    const x = box.x + box.width * 0.35 + (i % 20) * 8
    const y = box.y + box.height * 0.55 + Math.floor(i / 20) * 6
    await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', buttons: 1, clickCount: 1, pointerType: 'pen', force: 0.5 })
    await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', buttons: 0, clickCount: 1, pointerType: 'pen', force: 0 })
  }
  await cdp.detach()
}

test('400 consecutive pen strokes save through the real key allocator', async ({ page, api }) => {
  test.setTimeout(180_000)
  const ws = await workspace(api, 'z')
  const errors: string[] = []
  page.on('pageerror', (error) => errors.push(error.message))
  await openCanvas(page, ALICE, ws.workspace_id)
  await writeDots(page, 400)
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).filter((e) => e.view_type === 'ink' && !e.deleted).length, { timeout: 60_000 }).toBe(400)
  await expectAllCommitted(page)
  const ink = (await api.outline(ALICE, ws.workspace_id)).filter((e) => e.view_type === 'ink' && !e.deleted)
  expect(new Set(ink.map((e) => e.order_key)).size).toBe(400)
  expect(Math.max(...ink.map((e) => e.order_key.length))).toBeLessThanOrEqual(8)
  expect(errors).toEqual([])
})

test('a saturated canvas rebalances and keeps existing content and layer order', async ({ page, api }) => {
  const ws = await workspace(api, 'z'.repeat(64))
  const before = await Promise.all(['first', 'last'].map((id) => api.read(ALICE, ws.workspace_id, id)))
  const errors: string[] = []
  page.on('pageerror', (error) => errors.push(error.message))
  await openCanvas(page, ALICE, ws.workspace_id)
  await writeDots(page, 8)
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).filter((e) => e.view_type === 'ink' && !e.deleted).length).toBe(8)
  await expectAllCommitted(page)
  const children = (await api.outline(ALICE, ws.workspace_id)).filter((e) => e.parent_id === 'notes' && !e.deleted).sort((a, b) => a.order_key < b.order_key ? -1 : a.order_key > b.order_key ? 1 : a.entity_id.localeCompare(b.entity_id))
  expect(children.slice(0, 2).map((e) => e.entity_id)).toEqual(['first', 'last'])
  expect(Math.max(...children.map((e) => e.order_key.length))).toBeLessThanOrEqual(8)
  for (const [i, id] of ['first', 'last'].entries()) {
    const after = await api.read(ALICE, ws.workspace_id, id)
    expect(after.content.payload).toEqual(before[i].content.payload)
    expect(after.placement).toEqual(before[i].placement)
    expect(after.content_rev).toBe(before[i].content_rev)
  }
  expect(errors).toEqual([])
})

test('undoing the first stroke after rebalancing preserves a collaborators later layer', async ({ page, api }) => {
  const ws = await workspace(api, 'z'.repeat(64))
  await openCanvas(page, ALICE, ws.workspace_id)
  await writeDots(page, 1)
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).filter((e) => e.view_type === 'ink' && !e.deleted).length).toBe(1)
  await expectAllCommitted(page)
  const result = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'collaborator', type_id: 'buckyos.cell', parent_id: 'notes', order_key: 'y', placement: { x: 150, y: 150, w: 100, h: 100 }, payload: { view: { type: 'shape' }, title: 'Later layer', config: { shape: 'rect', fill: '#00ff00' } } },
  ])
  expect(result.status, JSON.stringify(result)).toBe('accepted')
  await page.getByTestId('aiws-undo').click()
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).filter((e) => e.view_type === 'ink' && !e.deleted).length).toBe(0)
  const children = (await api.outline(ALICE, ws.workspace_id)).filter((e) => e.parent_id === 'notes' && !e.deleted).sort((a, b) => a.order_key < b.order_key ? -1 : a.order_key > b.order_key ? 1 : 0)
  expect(children.map((e) => e.entity_id)).toEqual(['first', 'last', 'collaborator'])
  expect(children.every((e) => e.order_key.length <= 8)).toBe(true)
})
