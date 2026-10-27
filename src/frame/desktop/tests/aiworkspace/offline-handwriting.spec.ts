/* Handwriting offline (notepads/aiworkspace-handwriting-todo.md H10): in a window holding a prepared replica, with the
 * network really cut, strokes are written, one is undone and one erased; a cold start through the service worker still
 * shows what is left with its queue; reconnected, the queue goes out once and the backend converges on the same strokes.
 * Production build behind a TCP relay (offline-fixtures.ts); the pen is a CDP pen with pressure. */

import type { CDPSession, Page } from '@playwright/test'
import { backToList } from './fixtures'
import { expect, openCard, prepareOffline, saveSummary, serviceWorkerReady, test, type Api } from './offline-fixtures'

const ALICE = 'tok-alice'
type Json = Record<string, unknown>
interface Point { x: number; y: number }

async function inkWorkspace(api: Api, title: string) {
  const ws = await api.rpc(ALICE, 'ws.create', { title }) as { workspace_id: string; epoch: string }
  const result = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'notes-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'a1', name: '笔记', payload: { kind: 'folder', title: '笔记', system: 'surface_content', surface_id: 'notes' } },
    { op: 'entity.create', entity_id: 'notes', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'a1', name: '笔记', payload: { kind: 'surface', layout: { mode: 'free' }, title: '笔记', content_folder_id: 'notes-content' } },
  ]) as { status: string }
  expect(result.status).toBe('accepted')
  return ws
}

async function penOf(page: Page) {
  const cdp: CDPSession = await page.context().newCDPSession(page)
  return async (points: Point[], force = 0.5) => {
    await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: points[0].x, y: points[0].y, button: 'left', buttons: 1, clickCount: 1, pointerType: 'pen', force })
    for (const p of points.slice(1)) await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: p.x, y: p.y, button: 'left', buttons: 1, pointerType: 'pen', force })
    const last = points.at(-1)!
    await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: last.x, y: last.y, button: 'left', buttons: 0, clickCount: 1, pointerType: 'pen', force: 0 })
  }
}

const line = (y: number, x0 = 400, x1 = 700) => Array.from({ length: 16 }, (_, i) => ({ x: x0 + ((x1 - x0) * i) / 15, y: y + Math.sin(i / 2) * 8 }))

async function inkOn(api: Api, workspaceId: string): Promise<Json[]> {
  return (await api.outline(ALICE, workspaceId)).filter((e: Json) => e.view_type === 'ink' && !e.deleted)
}

test('H10 offline: write, undo and erase on the replica; a cold start keeps them queued; reconnected, they go out once', async ({ page, context, api, net }) => {
  const ws = await inkWorkspace(api, `h10 ${Date.now()}`)
  await prepareOffline(page, net, ALICE, ws.workspace_id)
  await serviceWorkerReady(page)
  const head = await api.headSeq(ALICE, ws.workspace_id)
  await net.down()
  await context.setOffline(true)
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'offline', { timeout: 40_000 })
  await page.getByTestId('aiws-tool-pen').click()
  const write = await penOf(page)
  for (const y of [300, 420, 540]) await write(line(y), 0.6)
  await expect(page.locator('svg.aiws-ink')).toHaveCount(3)
  await expect.poll(async () => (await saveSummary(page)).pending).toBe(3)
  // undo the last stroke: its queued submission goes
  await page.getByTestId('aiws-undo').click()
  await expect(page.locator('svg.aiws-ink')).toHaveCount(2)
  await expect.poll(async () => (await saveSummary(page)).pending).toBe(2)
  // erase the first one: a queued delete
  await page.getByTestId('aiws-ink-eraser').click()
  await write([{ x: 550, y: 280 }, { x: 550, y: 320 }])
  await expect(page.locator('svg.aiws-ink')).toHaveCount(1)
  await expect.poll(async () => (await saveSummary(page)).pending).toBe(3)
  expect(await api.headSeq(ALICE, ws.workspace_id), 'nothing reached the backend').toBe(head)
  const kept = await page.locator('svg.aiws-ink').getAttribute('data-stroke')
  // a cold start, still offline: the replica draws what is left and keeps the queue
  await page.close()
  const cold = await context.newPage()
  await cold.goto(`${net.origin}/?scenario=normal`)
  await cold.getByTestId('desktop-app-aiworkspace').click()
  await backToList(cold)
  await openCard(cold, ws.workspace_id)
  await expect(cold.getByTestId('aiws-mode')).toHaveAttribute('data-mode', 'replica')
  await expect(cold.locator('svg.aiws-ink')).toHaveCount(1, { timeout: 30_000 })
  await expect(cold.locator('svg.aiws-ink')).toHaveAttribute('data-stroke', kept!)
  expect((await saveSummary(cold)).pending).toBe(3)
  // reconnected: the queue goes out once, the backend has the one stroke left
  await context.setOffline(false)
  await net.up()
  await expect(cold.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live', { timeout: 40_000 })
  await expect.poll(async () => (await saveSummary(cold)).pending, { timeout: 30_000 }).toBe(0)
  const cells = await inkOn(api, ws.workspace_id)
  expect(cells.map((c) => c.ink_stroke)).toEqual([kept])
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 3)
  await expect(cold.locator('svg.aiws-ink')).toHaveCount(1)
})
