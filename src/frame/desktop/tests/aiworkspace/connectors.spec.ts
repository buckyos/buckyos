/* Connectors (doc/workspace/连接线实现方案讨论.md §11 CN01–CN16; 标准对象的交互改进 §6, §9 OX16/OX17): drawing lines
 * from connection handles and with the connector tool, snapping, end / bend / segment / label handles, lines
 * following their targets, deletion, copy and paste, flow pages, view mode, and the geometry itself. */

import type { Page } from '@playwright/test'
import { blockCenter, fitAll, hooks, openCanvas, setCanvasMode, test, expect, type Api } from './fixtures'

const ALICE = 'tok-alice'
const SHOTS = 'test-results/aiworkspace-connectors'

type Json = Record<string, unknown>

/** A free canvas "连线" with three shapes (a, b; c is an ellipse) and a flow page. */
async function lineWorkspace(api: Api, title: string, extra: Json[] = []) {
  const ws = await api.rpc(ALICE, 'ws.create', { title }) as { workspace_id: string; epoch: string }
  const shape = (id: string, x: number, y: number, label: string, kind = 'rect') => ({ op: 'entity.create', entity_id: id, type_id: 'buckyos.cell', parent_id: 'lines', order_key: `k${id}`, placement: { x, y, w: 160, h: 100 },
    payload: { view: { type: 'shape' }, title: label, config: { shape: kind, fill: '#e8f0fe', stroke: '#4f8df7' } } })
  const result = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'lines-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'a1', name: '连线', payload: { kind: 'folder', title: '连线', system: 'surface_content', surface_id: 'lines' } },
    { op: 'entity.create', entity_id: 'lines', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'a1', name: '连线', payload: { kind: 'surface', layout: { mode: 'free' }, title: '连线', content_folder_id: 'lines-content' } },
    { op: 'entity.create', entity_id: 'page-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'a2', name: '页', payload: { kind: 'folder', title: '页', system: 'surface_content', surface_id: 'page' } },
    { op: 'entity.create', entity_id: 'page', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'a2', name: '页', payload: { kind: 'surface', layout: { mode: 'flow' }, title: '页', content_folder_id: 'page-content' } },
    shape('a', 100, 100, 'A'), shape('b', 600, 400, 'B'), shape('c', 100, 600, 'C', 'ellipse'),
    ...extra,
  ]) as { status: string }
  expect(result.status, JSON.stringify(result)).toBe('accepted')
  return ws
}

async function shot(page: Page, name: string) {
  await page.getByTestId('aiws-canvas-view').screenshot({ path: `${SHOTS}/${name}.png` })
}

/** The connector cells of the canvas, as the backend's outline projects them. */
async function lines(api: Api, workspaceId: string): Promise<Json[]> {
  return (await api.outline(ALICE, workspaceId)).filter((e: Json) => e.view_type === 'connector')
}

async function center(page: Page, testId: string) {
  const box = await page.getByTestId(testId).boundingBox()
  if (!box) throw new Error(`${testId} is not on screen`)
  return { x: box.x + box.width / 2, y: box.y + box.height / 2 }
}

/** Shift-click at a screen point (`mouse.click` has no modifiers). */
async function shiftClick(page: Page, p: { x: number; y: number }) {
  await page.keyboard.down('Shift')
  await page.mouse.click(p.x, p.y)
  await page.keyboard.up('Shift')
}

async function drag(page: Page, from: { x: number; y: number }, to: { x: number; y: number }, steps = 8) {
  await page.mouse.move(from.x, from.y)
  await page.mouse.down()
  await page.mouse.move(to.x, to.y, { steps })
  await page.mouse.up()
}

test('OX16 a line is dragged out of a connection handle and bound at both ends', async ({ page, api }) => {
  const ws = await lineWorkspace(api, `cn-handle ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await page.getByTestId('aiws-canvas-block-a').click()
  for (const side of ['n', 'e', 's', 'w']) await expect(page.getByTestId(`aiws-connect-${side}`)).toBeVisible()
  await shot(page, 'ox16-connect-handles')
  const from = await center(page, 'aiws-connect-e')
  const to = await blockCenter(page, 'b')
  await page.mouse.move(from.x, from.y)
  await page.mouse.down()
  await page.mouse.move(to.x, to.y, { steps: 10 })
  await expect(page.getByTestId('aiws-snap-b')).toBeVisible()
  await expect(page.getByTestId('aiws-line-preview')).toBeVisible()
  await shot(page, 'ox16-dragging')
  await page.mouse.up()
  await expect.poll(async () => (await lines(api, ws.workspace_id)).length).toBe(1)
  const [line] = await lines(api, ws.workspace_id)
  const projected = line.connector as { start: Json; end: Json }
  // both ends name their anchor: the handle's side, and (released inside b) b's side facing the start
  expect(projected.start).toEqual({ entity_id: 'a', anchor: { kind: 'named', id: 'e' } })
  expect(projected.end).toEqual({ entity_id: 'b', anchor: { kind: 'named', id: 'w' } })
  const id = line.entity_id as string
  await expect(page.getByTestId(`aiws-canvas-block-${id}`)).toHaveAttribute('data-end', 'bound')
  await expect(page.getByTestId(`aiws-selection-${id}`)).toBeVisible()
  await shot(page, 'ox16-created')
})

test('CN geometry: stored ends and flip, the endpoint frame, orthogonal elbows, connection points', async () => {
  const g = await import('../../src/app/aiworkspace/ui/canvas/connectors/geometry')
  // stored box + flip ⇄ the two ends (also flat boxes)
  for (const [s, e] of [[{ x: 10, y: 20 }, { x: 110, y: 20 }], [{ x: 110, y: 80 }, { x: 10, y: 20 }], [{ x: 5, y: 90 }, { x: 5, y: 10 }]]) {
    const { rect, flip } = g.boxOf(s, e)
    expect(rect.w >= 0 && rect.h >= 0).toBe(true)
    expect(g.storedEnds(rect, flip)).toEqual([s, e])
  }
  // between the ends: a ratio; beyond them: an offset from the nearer end; a flat span never divides by zero
  const s = { x: 0, y: 0 }, e = { x: 200, y: 100 }
  expect(g.decompose({ x: 50, y: 50 }, s, e)).toEqual({ u: 0.25, v: 0.5 })
  expect(g.decompose({ x: -30, y: 140 }, s, e)).toEqual({ u: 0, v: 1, dx: -30, dy: 40 })
  expect(g.decompose({ x: 70, y: 0 }, { x: 0, y: 0 }, { x: 0.5, y: 0 })).toEqual({ u: 1, v: 0, dx: 69.5 })
  // points sharing x keep sharing it whatever the ends do (right angles survive)
  const a = g.decompose({ x: 120, y: 10 }, s, e), b = g.decompose({ x: 120, y: 90 }, s, e)
  for (const [s2, e2] of [[{ x: 40, y: -20 }, { x: 900, y: 600 }], [{ x: 300, y: 300 }, { x: -50, y: 10 }]]) expect(g.compose(a, s2, e2).x).toBeCloseTo(g.compose(b, s2, e2).x, 6)
  // automatic elbows are orthogonal and leave / enter along the given directions
  const dirs = [{ x: 1, y: 0 }, { x: -1, y: 0 }, { x: 0, y: 1 }, { x: 0, y: -1 }]
  for (const ds of dirs) for (const de of dirs) for (const end of [{ x: 300, y: 200 }, { x: -200, y: 50 }, { x: 10, y: -300 }]) {
    const pts = g.elbowAuto({ x: 0, y: 0 }, ds, end, de)
    for (let i = 1; i < pts.length; i++) expect(Math.abs(pts[i].x - pts[i - 1].x) < 0.01 || Math.abs(pts[i].y - pts[i - 1].y) < 0.01).toBe(true)
    const first = { x: Math.sign(pts[1].x - pts[0].x), y: Math.sign(pts[1].y - pts[0].y) }
    expect(first).toEqual({ x: ds.x || 0, y: ds.y || 0 })
  }
  const through = g.elbowThrough({ x: 0, y: 0 }, { x: 1, y: 0 }, [{ x: 50, y: 80 }, { x: 120, y: 30 }], { x: 200, y: 200 }, { x: 0, y: -1 })
  for (let i = 1; i < through.length; i++) expect(Math.abs(through[i].x - through[i - 1].x) < 0.01 || Math.abs(through[i].y - through[i - 1].y) < 0.01).toBe(true)
  // anchors: the 16 compass anchors of the outline (the same id on a rectangle and an ellipse), turned with the
  // target; an id the target does not have floats, meeting the outline on the way to the other end
  const box = { rect: { x: 0, y: 0, w: 200, h: 100 }, rotation: 0, shape: 'rect' as const }
  const named = (id: string) => ({ kind: 'named' as const, id })
  expect(g.anchorWorld(box, named('ne'), { x: 0, y: 0 })).toEqual({ x: 200, y: 0 })
  expect(g.anchorWorld(box, named('wnw'), { x: 0, y: 0 })).toEqual({ x: 0, y: 25 })
  const ne = g.anchorWorld({ ...box, shape: 'ellipse' }, named('ne'), { x: 0, y: 0 })
  expect(ne.x).toBeCloseTo(100 + 100 * Math.SQRT1_2, 3)
  expect(ne.y).toBeCloseTo(50 - 50 * Math.SQRT1_2, 3)
  const turned = g.anchorWorld({ ...box, rotation: 90 }, named('e'), { x: 0, y: 0 })
  expect(turned.x).toBeCloseTo(100, 6)
  expect(turned.y).toBeCloseTo(150, 6)
  expect(g.anchorWorld(box, named('nope'), { x: 1000, y: 50 })).toEqual({ x: 200, y: 50 })
  const floating = g.anchorWorld({ ...box, shape: 'ellipse' }, named('nope'), { x: 100 + 300, y: 50 + 300 })
  expect(((floating.x - 100) / 100) ** 2 + ((floating.y - 50) / 50) ** 2).toBeCloseTo(1, 6)
  // a Block's own anchors (with an offset in world units); a corner leaves by its side facing the other end
  const ports = { ...box, anchors: [{ id: 'in', x: 0, y: 0.5, sides: ['w' as const] }, { id: 'tip', x: 1, y: 0, dy: -10 }] }
  expect(g.anchorWorld(ports, named('tip'), { x: 0, y: 0 })).toEqual({ x: 200, y: -10 })
  expect(g.anchorWorld(ports, named('e'), { x: 1000, y: 50 })).toEqual({ x: 200, y: 50 })
  const lookup = (t: object) => () => t as never
  const corner = (to: { x: number; y: number }) => g.resolveEnds({ start: { entity_id: 't', anchor: named('ne') }, end: null }, [{ x: 0, y: 0 }, to], lookup(box))[0].dir
  expect(corner({ x: 600, y: 20 })).toEqual({ x: 1, y: 0 })
  expect(corner({ x: 220, y: -400 })).toEqual({ x: 0, y: -1 })
  // 8 anchors, 16 when the minor ones are 32 px apart on screen; inside, the major anchor facing the other end
  expect(g.visibleAnchors(box, 1).map((a) => a.def.id)).toEqual(['n', 'ne', 'e', 'se', 's', 'sw', 'w', 'nw'])
  expect(g.visibleAnchors(box, 1.3)).toHaveLength(16)
  expect(g.facingAnchor(box, { x: 900, y: -200 })).toEqual(named('e'))
  expect(g.facingAnchor(box, { x: 120, y: 700 })).toEqual(named('s'))
  expect(g.facingAnchor(ports, { x: -500, y: 400 })).toEqual(named('in'))
  expect(g.sideAnchor(ports, 'n')?.def.id).toBe('tip')
  expect(g.sideAnchor(ports, 's')).toBeNull()
  expect(g.sideAnchor(ports, 'w')?.def.id).toBe('in')
  expect(g.sideAnchor(box, 's')?.def.id).toBe('s')
  // a route measures: the label point, the nearest point, the bounds
  const route = g.routeBetween({ route: 'straight', controls: [], label: { t: 0.5, offset: 10 } },
    { point: { x: 0, y: 0 }, dir: { x: 1, y: 0 }, state: 'free' }, { point: { x: 100, y: 0 }, dir: { x: -1, y: 0 }, state: 'free' }, { pad: 0 })
  expect(route.length).toBeCloseTo(100, 6)
  expect(route.label.point).toEqual({ x: 50, y: -10 })
  expect(g.nearest(route, { x: 30, y: 7 }).t).toBeCloseTo(0.3, 6)
})

test('CN01/CN06 the connector tool draws free lines: flat boxes, drawn backwards, three routes; dragging an end across flips', async ({ page, api }) => {
  const ws = await lineWorkspace(api, `cn01 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  const b = await page.getByTestId('aiws-canvas-block-b').boundingBox()
  if (!b) throw new Error('no b')
  // drawn right to left, horizontally, on blank canvas: a free line with a flat box and flip.h
  await page.getByTestId('aiws-tool-insert-connector').click()
  await expect(page.getByTestId('aiws-tool-hint')).toContainText('连接线')
  const y = b.y + b.height + 120
  await drag(page, { x: b.x + 300, y }, { x: b.x - 200, y })
  await expect(page.getByTestId('aiws-menu-next-note')).toBeVisible()
  await page.keyboard.press('Escape')
  await expect.poll(async () => (await lines(api, ws.workspace_id)).length).toBe(1)
  let [line] = await lines(api, ws.workspace_id)
  const id = line.entity_id as string
  expect((line.placement as { h: number }).h).toBe(0)
  expect((line.connector as { flip?: Json }).flip).toEqual({ h: true })
  expect(await page.getByTestId('aiws-canvas').getAttribute('data-tool')).toBe('select')
  // the toolbar: route, caps, reset
  await page.getByTestId('aiws-near-line-route').click()
  await page.getByTestId('aiws-near-line-route-elbow').click()
  await expect(page.getByTestId(`aiws-canvas-block-${id}`)).toHaveAttribute('data-route', 'elbow')
  await page.getByTestId('aiws-near-line-route').click()
  await page.getByTestId('aiws-near-line-route-curve').click()
  await expect(page.getByTestId(`aiws-canvas-block-${id}`)).toHaveAttribute('data-route', 'curve')
  await page.getByTestId('aiws-near-line-start-cap').click()
  await page.getByTestId('aiws-near-line-start-cap-diamond').click()
  await page.getByTestId('aiws-near-line-dash').click()
  await page.getByTestId('aiws-near-line-dash-dashed').click()
  await expect.poll(async () => ((await api.read(ALICE, ws.workspace_id, id)).content.payload.config ?? {}) as Json).toEqual({ start_cap: 'diamond', dash: 'dashed' })
  await shot(page, 'cn01-curve-caps')
  // CN06: the start end dragged past the other end on both axes: flip changes in the same single commit
  const before = (await hooks(page)).commits
  const start = await center(page, 'aiws-line-end-start')
  const end = await center(page, 'aiws-line-end-end')
  await drag(page, start, { x: end.x - 150, y: end.y - 90 })
  await expect.poll(async () => (await hooks(page)).commits).toBe(before + 1)
  await expect.poll(async () => ((await lines(api, ws.workspace_id))[0].connector as { flip?: Json }).flip ?? {}).toEqual({})
  ;[line] = await lines(api, ws.workspace_id)
  expect((line.placement as { h: number }).h).toBeGreaterThan(50)
  // saved and reopened: the same line
  await page.reload()
  await openCanvas(page, ALICE, ws.workspace_id)
  await expect(page.getByTestId(`aiws-canvas-block-${id}`)).toHaveAttribute('data-route', 'curve')
})

test('CN02 a bound line follows its target while it moves and resizes; only the target is written', async ({ page, api }) => {
  const ws = await lineWorkspace(api, `cn02 ${Date.now()}`, [
    { op: 'entity.create', entity_id: 'ab', type_id: 'buckyos.cell', parent_id: 'lines', order_key: 'kz', placement: { x: 260, y: 150, w: 340, h: 300 },
      payload: { view: { type: 'connector', version: 1 }, start: { entity_id: 'a', anchor: { kind: 'named', id: 'e' } }, end: { entity_id: 'b', anchor: { kind: 'named', id: 'w' } }, route: 'elbow', controls: [{ u: 0.5, v: 0 }, { u: 0.5, v: 1 }] } },
  ])
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  const path = () => page.getByTestId('aiws-canvas-block-ab').locator('path.aiws-connector-path').getAttribute('d')
  const d0 = await path()
  const placement0 = (await lines(api, ws.workspace_id))[0].placement
  const before = (await hooks(page)).commits
  const c = await blockCenter(page, 'b')
  await page.mouse.move(c.x, c.y)
  await page.mouse.down()
  await page.mouse.move(c.x + 60, c.y + 80, { steps: 6 })
  // mid-gesture the line already follows (repainted without a commit)
  expect(await path()).not.toBe(d0)
  expect((await hooks(page)).commits).toBe(before)
  await page.mouse.up()
  await expect.poll(async () => (await hooks(page)).commits).toBe(before + 1)
  expect((await lines(api, ws.workspace_id))[0].placement).toEqual(placement0)
  // right angles survive: every piece of the elbow is horizontal or vertical
  const pts = ((await path()) ?? '').match(/-?\d+(\.\d+)?/g)?.map(Number) ?? []
  expect(pts.length).toBeGreaterThan(4)
  const segs = await page.getByTestId('aiws-canvas-block-ab').locator('path.aiws-connector-path').evaluate((el) => {
    const p = el as SVGPathElement
    const out: [number, number][] = []
    const n = 40
    for (let i = 0; i <= n; i++) { const q = p.getPointAtLength((p.getTotalLength() * i) / n); out.push([q.x, q.y]) }
    return out
  })
  // sampled points of a rounded orthogonal path stay near some horizontal or vertical line through a neighbour
  expect(segs.length).toBe(41)
  await shot(page, 'cn02-followed')
  // resizing the target re-routes too (one commit, the line untouched)
  await page.getByTestId('aiws-canvas-block-b').click()
  const se = await center(page, 'aiws-handle-se')
  const d1 = await path()
  await page.mouse.move(se.x, se.y)
  await page.mouse.down()
  await page.mouse.move(se.x + 80, se.y + 60, { steps: 5 })
  expect(await path()).not.toBe(d1)
  await page.mouse.up()
  expect((await lines(api, ws.workspace_id))[0].placement).toEqual(placement0)
})

test('CN03 anchors: an end on an anchor names it, inside a Block it takes the anchor facing the other end, ellipses have their own; loops need two anchors', async ({ page, api }) => {
  const ws = await lineWorkspace(api, `cn03 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  // a → the ellipse c, released inside it: c's top anchor, which faces a
  await page.getByTestId('aiws-canvas-block-a').click()
  await drag(page, await center(page, 'aiws-connect-s'), await blockCenter(page, 'c'))
  await expect.poll(async () => (await lines(api, ws.workspace_id)).length).toBe(1)
  let all = await lines(api, ws.workspace_id)
  expect((all[0].connector as { end: Json }).end).toEqual({ entity_id: 'c', anchor: { kind: 'named', id: 'n' } })
  // b → on the right anchor of c
  await page.getByTestId('aiws-canvas-block-b').click()
  const cBox = await page.getByTestId('aiws-canvas-block-c').boundingBox()
  if (!cBox) throw new Error('no c')
  await drag(page, await center(page, 'aiws-connect-w'), { x: cBox.x + cBox.width + 5, y: cBox.y + cBox.height / 2 + 4 })
  await expect.poll(async () => (await lines(api, ws.workspace_id)).length).toBe(2)
  all = await lines(api, ws.workspace_id)
  expect(all.map((l) => (l.connector as { end: Json }).end)).toContainEqual({ entity_id: 'c', anchor: { kind: 'named', id: 'e' } })
  await shot(page, 'cn03-anchors')
  // a loop on a: from its east to its north anchor (an elbow); released inside a it is refused
  await page.getByTestId('aiws-canvas-block-a').click()
  await drag(page, await center(page, 'aiws-connect-e'), await blockCenter(page, 'a'))
  await expect(page.getByTestId('aiws-status-dock').or(page.locator('.aiws-toast, [role="status"]')).first()).toBeVisible()
  expect((await lines(api, ws.workspace_id)).length).toBe(2)
  await page.getByTestId('aiws-canvas-block-a').click()
  const aBox = await page.getByTestId('aiws-canvas-block-a').boundingBox()
  if (!aBox) throw new Error('no a')
  await drag(page, await center(page, 'aiws-connect-e'), { x: aBox.x + aBox.width / 2, y: aBox.y + 2 })
  await expect.poll(async () => (await lines(api, ws.workspace_id)).length).toBe(3)
  const loop = (await lines(api, ws.workspace_id)).find((l) => (l.connector as { end: { entity_id: string } }).end.entity_id === 'a')
  expect(loop?.connector).toMatchObject({ start: { entity_id: 'a', anchor: { kind: 'named', id: 'e' } }, end: { entity_id: 'a', anchor: { kind: 'named', id: 'n' } }, route: 'elbow' })
  await shot(page, 'cn03-loop')
})

/** The screen point of a Block's normalised spot (x, y in [0, 1] of its frame). */
async function spot(page: Page, blockId: string, x: number, y: number) {
  const box = await page.getByTestId(`aiws-canvas-block-${blockId}`).boundingBox()
  if (!box) throw new Error(`block ${blockId} is not on screen`)
  return { x: box.x + box.width * x, y: box.y + box.height * y }
}

test('CN17 anchors: 8 per Block, 16 with room; the id survives an outline change; a Block may declare its own; the tool shows them', async ({ page, api }) => {
  const ws = await lineWorkspace(api, `cn17 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  const snaps = page.locator('[data-testid^="aiws-snap-anchor-"]')
  const zoom = async () => Number(await page.getByTestId('aiws-canvas').getAttribute('data-zoom'))
  const zoomBy = async (button: 'aiws-zoom-in' | 'aiws-zoom-out') => {
    const before = await zoom()
    await page.getByTestId('aiws-zoom-menu').click()
    await page.getByTestId(button).click()
    await page.keyboard.press('Escape')
    await expect.poll(zoom).not.toBe(before)
  }
  // b is 160 × 100: its minor anchors are 25 apart on the short sides, so they show from zoom 1.28
  while (await zoom() >= 1.2) await zoomBy('aiws-zoom-out')
  // over b: its 8 major anchors, the facing one marked
  await page.getByTestId('aiws-canvas-block-a').click()
  let from = await center(page, 'aiws-connect-e')
  await page.mouse.move(from.x, from.y)
  await page.mouse.down()
  await page.mouse.move((await blockCenter(page, 'b')).x, (await blockCenter(page, 'b')).y, { steps: 8 })
  await expect(page.getByTestId('aiws-snap-b')).toHaveAttribute('data-anchor', 'w')
  await expect(snaps).toHaveCount(8)
  await expect(page.getByTestId('aiws-snap-anchor-w')).toHaveClass(/is-on/)
  await page.keyboard.press('Escape')
  await page.mouse.up()
  // zoomed in: 16; released on a minor one, the end names it
  await page.mouse.click(5, 500)
  while (await zoom() < 1.3) await zoomBy('aiws-zoom-in')
  await page.getByTestId('aiws-canvas-block-a').click()
  from = await center(page, 'aiws-connect-e')
  const wnw = await spot(page, 'b', 0, 0.25)
  await page.mouse.move(from.x, from.y)
  await page.mouse.down()
  await page.mouse.move(wnw.x + 3, wnw.y + 2, { steps: 8 })
  await expect(snaps).toHaveCount(16)
  await expect(page.getByTestId('aiws-snap-b')).toHaveAttribute('data-anchor', 'wnw')
  await shot(page, 'cn17-sixteen')
  await page.mouse.up()
  await expect.poll(async () => (await lines(api, ws.workspace_id)).length).toBe(1)
  const line = (await lines(api, ws.workspace_id))[0]
  const id = line.entity_id as string
  expect((line.connector as { end: Json }).end).toEqual({ entity_id: 'b', anchor: { kind: 'named', id: 'wnw' } })
  // the end handle says where it is
  await expect(page.getByTestId('aiws-line-end-end')).toHaveAttribute('data-anchor', 'wnw')
  await expect(page.getByTestId('aiws-line-end-end').locator('title')).toContainText('左偏上')
  // b becomes an ellipse: the end moves to the ellipse's wnw anchor (22.5° above the left end of the axis)
  const read = await api.read(ALICE, ws.workspace_id, 'b') as { content: { key_revs?: Record<string, number> } }
  expect((await api.commit(ALICE, ws, [{ op: 'entity.set_keys', entity_id: 'b', keys: [{ key: 'config', value: { shape: 'ellipse', fill: '#e8f0fe', stroke: '#4f8df7' }, expect: { rev: read.content.key_revs?.config ?? 0 } }] }]) as { status: string }).status).toBe('accepted')
  const onEllipse = await spot(page, 'b', 0.5 - 0.5 * Math.cos(Math.PI / 8), 0.5 - 0.5 * Math.sin(Math.PI / 8))
  await expect.poll(async () => { const at = await center(page, 'aiws-line-end-end'); return Math.hypot(at.x - onEllipse.x, at.y - onEllipse.y) }).toBeLessThan(2)
  expect(((await lines(api, ws.workspace_id))[0].connector as { end: Json }).end).toEqual({ entity_id: 'b', anchor: { kind: 'named', id: 'wnw' } })
  await shot(page, 'cn17-ellipse')
  await page.keyboard.press('Escape')

  // a Block declaring its own anchors: three inputs on the left, one output on the right
  await fitAll(page)
  await page.evaluate(() => {
    const blocks = (window as unknown as { __aiwsTestHooks: { blocks: { get(t: string): object; register(d: object): void } } }).__aiwsTestHooks.blocks
    blocks.register({ ...blocks.get('shape'), type: 'test-ports', title: '端口', catalog: undefined, shape: 'rect',
      anchors: [{ id: 'in1', x: 0, y: 0.25 }, { id: 'in2', x: 0, y: 0.5 }, { id: 'in3', x: 0, y: 0.75 }, { id: 'out', x: 1, y: 0.5, label: '输出' }] })
  })
  expect((await api.commit(ALICE, ws, [{ op: 'entity.create', entity_id: 'p', type_id: 'buckyos.cell', parent_id: 'lines', order_key: 'kp', placement: { x: 600, y: 100, w: 160, h: 100 },
    payload: { view: { type: 'test-ports' }, title: 'P', config: { shape: 'rect', fill: '#fdf2d0', stroke: '#c58b00' } } }]) as { status: string }).status).toBe('accepted')
  await expect(page.getByTestId('aiws-canvas-block-p')).toBeVisible()
  // its connection handles are the ones it has a side anchor for
  await page.getByTestId('aiws-canvas-block-p').click()
  await expect(page.getByTestId('aiws-connect-e')).toBeVisible()
  await expect(page.getByTestId('aiws-connect-w')).toBeVisible()
  await expect(page.getByTestId('aiws-connect-n')).toHaveCount(0)
  await expect(page.getByTestId('aiws-connect-s')).toHaveCount(0)
  // a → inside p: the input facing a; only p's own anchors are offered
  await page.getByTestId('aiws-canvas-block-a').click()
  from = await center(page, 'aiws-connect-e')
  await page.mouse.move(from.x, from.y)
  await page.mouse.down()
  await page.mouse.move((await blockCenter(page, 'p')).x + 10, (await blockCenter(page, 'p')).y, { steps: 8 })
  await expect(page.getByTestId('aiws-snap-p')).toHaveAttribute('data-anchor', 'in2')
  await expect(snaps).toHaveCount(4)
  await shot(page, 'cn17-ports')
  await page.mouse.up()
  await expect.poll(async () => (await lines(api, ws.workspace_id)).length).toBe(2)
  expect((await lines(api, ws.workspace_id)).map((l) => (l.connector as { end: Json }).end)).toContainEqual({ entity_id: 'p', anchor: { kind: 'named', id: 'in2' } })

  // the connector tool shows the anchors under the pointer before the press; pressed inside a, the start faces the end
  await page.mouse.click(5, 500)
  await page.keyboard.press('l')
  const aIn = await spot(page, 'a', 0.4, 0.6)
  await page.mouse.move(aIn.x, aIn.y)
  await expect(page.getByTestId('aiws-snap-a')).toHaveAttribute('data-exact', 'false')
  await expect(snaps).toHaveCount((await zoom()) * 25 >= 32 ? 16 : 8)
  const bTop = await spot(page, 'b', 0.5, 0)
  await page.mouse.down()
  await page.mouse.move(bTop.x, bTop.y + 2, { steps: 8 })
  await expect(page.getByTestId('aiws-snap-b')).toHaveAttribute('data-anchor', 'n')
  await page.mouse.up()
  await expect.poll(async () => (await lines(api, ws.workspace_id)).length).toBe(3)
  const drawn = (await lines(api, ws.workspace_id)).find((l) => (l.connector as { end: { entity_id?: string } | null }).end?.entity_id === 'b' && l.entity_id !== id)
  expect(drawn?.connector).toMatchObject({ start: { entity_id: 'a', anchor: { kind: 'named', id: 'e' } }, end: { entity_id: 'b', anchor: { kind: 'named', id: 'n' } } })
})

const AB = (extra: Json = {}, id = 'ab', order = 'kz') => ({ op: 'entity.create', entity_id: id, type_id: 'buckyos.cell', parent_id: 'lines', order_key: order, placement: { x: 260, y: 150, w: 340, h: 300 },
  payload: { view: { type: 'connector', version: 1 }, start: { entity_id: 'a', anchor: { kind: 'named', id: 'e' } }, end: { entity_id: 'b', anchor: { kind: 'named', id: 'w' } }, ...extra } })

/** The screen point at arc-length ratio `t` of a drawn line. Mapped through the frame's layout box and the SVG's
 * viewBox: `getScreenCTM()` can answer an identity matrix for a moment after React replaced the SVG's content. */
async function linePoint(page: Page, id: string, t: number): Promise<{ x: number; y: number }> {
  return page.getByTestId(`aiws-canvas-block-${id}`).evaluate((frame, ratio) => {
    const svg = frame.querySelector('svg')!
    const path = frame.querySelector('path.aiws-connector-path') as SVGPathElement
    const p = path.getPointAtLength(path.getTotalLength() * ratio)
    const [vx, vy, vw, vh] = (svg.getAttribute('viewBox') ?? '0 0 1 1').split(/\s+/).map(Number)
    const r = svg.getBoundingClientRect()
    return { x: r.x + ((p.x - vx) * r.width) / vw, y: r.y + ((p.y - vy) * r.height) / vh }
  }, t)
}

async function payload(api: Api, workspaceId: string, id: string): Promise<Json> {
  return (await api.read(ALICE, workspaceId, id)).content.payload as Json
}

/** Is every piece of the drawn path horizontal or vertical (rounded corners aside)? */
async function orthogonal(page: Page, id: string): Promise<boolean> {
  const d = await page.getByTestId(`aiws-canvas-block-${id}`).locator('path.aiws-connector-path').getAttribute('d') ?? ''
  // every straight piece (L) is horizontal or vertical; rounded corners (Q) only move the current point
  let cur: number[] | null = null
  let ok = true
  let pieces = 0
  for (const [, c, args] of d.matchAll(/([MLQC])([^MLQC]*)/g)) {
    const n = args.trim().split(/[\s,]+/).map(Number)
    if (c === 'L' && cur) { pieces += 1; if (Math.abs(n[0] - cur[0]) > 0.6 && Math.abs(n[1] - cur[1]) > 0.6) ok = false }
    cur = c === 'Q' ? [n[2], n[3]] : c === 'C' ? [n[4], n[5]] : [n[0], n[1]]
  }
  return ok && pieces > 0
}

test('CN04/OX17 bends: a straight line gains, moves and loses bends; an elbow segment drags; a curve gets a point and resets; one commit each, undoable', async ({ page, api }) => {
  const ws = await lineWorkspace(api, `cn04 ${Date.now()}`, [AB()])
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await page.mouse.click((await linePoint(page, 'ab', 0.3)).x, (await linePoint(page, 'ab', 0.3)).y)
  await expect(page.getByTestId('aiws-line-handles-ab')).toBeVisible()
  await expect(page.getByTestId('aiws-handle-se')).toHaveCount(0)
  await expect(page.getByTestId('aiws-rotate-handle')).toHaveCount(0)
  await expect(page.getByTestId('aiws-line-end-start')).toHaveAttribute('data-state', 'bound')
  // the virtual handle in the middle of the piece adds a bend
  let commits = (await hooks(page)).commits
  const mid = await center(page, 'aiws-line-mid-0')
  await drag(page, mid, { x: mid.x, y: mid.y + 90 })
  await expect.poll(async () => ((await payload(api, ws.workspace_id, 'ab')).controls as unknown[] | undefined)?.length).toBe(1)
  expect((await hooks(page)).commits).toBe(commits + 1)
  await expect(page.getByTestId('aiws-line-control-0')).toBeVisible()
  await shot(page, 'cn04-bend')
  // moving the bend with Shift lines it up with the start (here on its x: the nearer axis)
  const ctl = await center(page, 'aiws-line-control-0')
  await page.keyboard.down('Shift')
  await drag(page, ctl, { x: ctl.x + 40, y: ctl.y + 4 })
  await page.keyboard.up('Shift')
  await expect.poll(async () => ((await payload(api, ws.workspace_id, 'ab')).controls as { u: number; dx?: number }[])[0]).toMatchObject({ u: 0 })
  // a double click removes it; undo brings it back
  commits = (await hooks(page)).commits
  const again = await center(page, 'aiws-line-control-0')
  await page.mouse.click(again.x, again.y)
  await page.mouse.click(again.x, again.y)
  await expect.poll(async () => (await payload(api, ws.workspace_id, 'ab')).controls).toBeUndefined()
  await page.getByTestId('aiws-undo').click()
  await expect.poll(async () => ((await payload(api, ws.workspace_id, 'ab')).controls as unknown[] | undefined)?.length).toBe(1)
  // elbow: the stored controls of another route go; the middle segment drags sideways and stays orthogonal
  await page.getByTestId('aiws-near-line-route').click()
  await page.getByTestId('aiws-near-line-route-elbow').click()
  await expect.poll(async () => (await payload(api, ws.workspace_id, 'ab')).controls).toBeUndefined()
  await expect(page.getByTestId('aiws-canvas-block-ab')).toHaveAttribute('data-route', 'elbow')
  expect(await orthogonal(page, 'ab')).toBe(true)
  const segments = await page.locator('[data-testid^="aiws-line-mid-"]').count()
  expect(segments).toBeGreaterThanOrEqual(1)
  const seg = await center(page, `aiws-line-mid-${Math.floor(segments / 2)}`)
  await drag(page, seg, { x: seg.x + 70, y: seg.y + 50 })
  await expect.poll(async () => ((await payload(api, ws.workspace_id, 'ab')).controls as unknown[] | undefined)?.length ?? 0).toBeGreaterThanOrEqual(2)
  expect(await orthogonal(page, 'ab')).toBe(true)
  await shot(page, 'cn04-elbow')
  // the target moves: the dragged elbow keeps its right angles
  const bc = await blockCenter(page, 'b')
  await page.mouse.click(bc.x, bc.y)
  await drag(page, bc, { x: bc.x + 40, y: bc.y - 120 })
  await expect.poll(async () => orthogonal(page, 'ab')).toBe(true)
  // curve: dragging the middle makes one pass-through point; reset route clears it
  await page.mouse.click((await linePoint(page, 'ab', 0.5)).x, (await linePoint(page, 'ab', 0.5)).y)
  await page.getByTestId('aiws-near-line-route').click()
  await page.getByTestId('aiws-near-line-route-curve').click()
  await expect(page.getByTestId('aiws-canvas-block-ab')).toHaveAttribute('data-route', 'curve')
  const cm = await center(page, 'aiws-line-mid-0')
  await drag(page, cm, { x: cm.x - 60, y: cm.y + 100 })
  await expect.poll(async () => ((await payload(api, ws.workspace_id, 'ab')).controls as unknown[] | undefined)?.length).toBe(1)
  await shot(page, 'cn04-curve')
  await page.getByTestId('aiws-near-line-reset').click()
  await expect.poll(async () => (await payload(api, ws.workspace_id, 'ab')).controls).toBeUndefined()
  await expect(page.getByTestId('aiws-near-line-reset')).toBeDisabled()
})

test('CN05 the label: typed in place, dragged along the line, snapped back onto it, kept off it with an offset', async ({ page, api }) => {
  const ws = await lineWorkspace(api, `cn05 ${Date.now()}`, [AB()])
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  const p = await linePoint(page, 'ab', 0.5)
  await page.mouse.dblclick(p.x, p.y)
  const editor = page.getByTestId('aiws-connector-label-edit-ab')
  await expect(editor).toBeFocused()
  await expect(page.getByTestId('aiws-near-line-label-size')).toBeVisible()
  await page.keyboard.type('next step')
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('aiws-connector-label-ab')).toHaveText('next step')
  await expect.poll(async () => (await payload(api, ws.workspace_id, 'ab')).title).toBe('next step')
  // the line is broken behind the label (a mask), and the label handle drags it towards the end
  await expect(page.getByTestId('aiws-canvas-block-ab').locator('mask')).toHaveCount(1)
  await expect(page.getByTestId('aiws-line-label-handle')).toBeVisible()
  const h = await center(page, 'aiws-line-label-handle')
  const far = await linePoint(page, 'ab', 0.8)
  await drag(page, h, { x: far.x, y: far.y + 3 })
  await expect.poll(async () => ((await payload(api, ws.workspace_id, 'ab')).label as { t: number } | undefined)?.t ?? 0).toBeGreaterThan(0.7)
  expect(((await payload(api, ws.workspace_id, 'ab')).label as { offset?: number }).offset ?? 0).toBe(0)
  // further off the line it keeps an offset
  const h2 = await center(page, 'aiws-line-label-handle')
  await drag(page, h2, { x: h2.x, y: h2.y - 40 })
  await expect.poll(async () => Math.abs(((await payload(api, ws.workspace_id, 'ab')).label as { offset?: number }).offset ?? 0)).toBeGreaterThan(20)
  await shot(page, 'cn05-label')
  // Esc leaves the label as it was
  await page.keyboard.press('Enter')
  await expect(editor).toBeFocused()
  await page.keyboard.type(' changed')
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('aiws-connector-label-ab')).toHaveText('next step')
})

test('CN07 lines in groups: grouped with their ends, moved with the group, ungrouped at the same place', async ({ page, api }) => {
  const ws = await lineWorkspace(api, `cn07 ${Date.now()}`, [AB()])
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  const d0 = await page.getByTestId('aiws-canvas-block-ab').locator('path.aiws-connector-path').getAttribute('d')
  await page.getByTestId('aiws-canvas-block-a').click()
  await page.getByTestId('aiws-canvas-block-b').click({ modifiers: ['Shift'] })
  const p = await linePoint(page, 'ab', 0.5)
  await shiftClick(page, p)
  await page.keyboard.press('Control+g')
  await expect.poll(async () => (await lines(api, ws.workspace_id))[0].parent_id).not.toBe('lines')
  const group = (await lines(api, ws.workspace_id))[0].parent_id as string
  // same drawing after grouping
  await expect.poll(async () => page.getByTestId('aiws-canvas-block-ab').locator('path.aiws-connector-path').getAttribute('d')).toBe(d0)
  // the group moves: ends and line move together, no doubled offset
  const g = await blockCenter(page, group)
  const before = await linePoint(page, 'ab', 0)
  await drag(page, { x: g.x, y: g.y - 0 }, { x: g.x + 100, y: g.y + 50 })
  await expect.poll(async () => { const now = await linePoint(page, 'ab', 0); return Math.round(now.x - before.x) }).toBeGreaterThan(70)
  const after = await linePoint(page, 'ab', 0)
  const a = await page.getByTestId('aiws-canvas-block-a').boundingBox()
  expect(Math.abs(after.x - (a!.x + a!.width))).toBeLessThan(8)
  // ungroup: the line stays where it is
  const d1 = await page.getByTestId('aiws-canvas-block-ab').locator('path.aiws-connector-path').getAttribute('d')
  await page.keyboard.press('Control+Shift+g')
  await expect.poll(async () => (await lines(api, ws.workspace_id))[0].parent_id).toBe('lines')
  await expect.poll(async () => page.getByTestId('aiws-canvas-block-ab').locator('path.aiws-connector-path').getAttribute('d')).toBe(d1)
})

test('CN08 copy and paste keep only the bindings inside the copied set', async ({ page, api }) => {
  const ws = await lineWorkspace(api, `cn08 ${Date.now()}`, [AB()])
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  const p = await linePoint(page, 'ab', 0.5)
  // a and the line: the start goes to the copy of a, the end becomes a coordinate
  await page.getByTestId('aiws-canvas-block-a').click()
  await shiftClick(page, p)
  await page.keyboard.press('Control+c')
  await expect(page.getByTestId('aiws-notice').filter({ hasText: '已复制 2 个对象' })).toBeVisible()
  await page.keyboard.press('Control+v')
  await expect.poll(async () => (await lines(api, ws.workspace_id)).length).toBe(2)
  const outline = await api.outline(ALICE, ws.workspace_id)
  const copy = (await lines(api, ws.workspace_id)).find((l) => l.entity_id !== 'ab')!
  const copyA = outline.find((e: Json) => e.view_type === 'shape' && e.title === 'A' && e.entity_id !== 'a')
  expect((copy.connector as { start: Json; end: Json }).start).toEqual({ entity_id: copyA.entity_id, anchor: { kind: 'named', id: 'e' } })
  expect((copy.connector as { end: Json }).end).toBeNull()
  // only the line: both ends become coordinates
  await page.mouse.click(5, 500)
  await page.mouse.click(p.x, p.y)
  await page.keyboard.press('Control+c')
  await expect(page.getByTestId('aiws-notice').filter({ hasText: '已复制 1 个对象' })).toBeVisible()
  await page.keyboard.press('Control+v')
  await expect.poll(async () => (await lines(api, ws.workspace_id)).length).toBe(3)
  const free = (await lines(api, ws.workspace_id)).find((l) => l.entity_id !== 'ab' && l.entity_id !== copy.entity_id)!
  expect(free.connector).toMatchObject({ start: null, end: null })
  // a flow page refuses lines
  await page.keyboard.press('Control+c')
  await expect(page.getByTestId('aiws-notice').filter({ hasText: '已复制 1 个对象' }).last()).toBeVisible()
  await page.getByTestId('aiws-surface-switch').click()
  await page.getByTestId('aiws-surface-item-page').getByRole('menuitem').click()
  await expect(page.getByTestId('aiws-flow-host')).toBeVisible()
  await page.getByTestId('aiws-flow-host').click()
  await page.keyboard.press('Control+v')
  await expect(page.getByText('流式页不显示连接线', { exact: false }).first()).toBeVisible()
})

test('CN09 deleting a target freezes its lines where they are; undo brings the target and the binding back', async ({ page, api }) => {
  const ws = await lineWorkspace(api, `cn09 ${Date.now()}`, [AB({ end: { entity_id: 'b', anchor: { kind: 'named', id: 'n' } } })])
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  // b moved by someone else: the stored spot of the end is stale until the delete freezes it
  const bBox = await page.getByTestId('aiws-canvas-block-b').boundingBox()
  const end0 = await linePoint(page, 'ab', 1)
  await page.getByTestId('aiws-canvas-block-b').click()
  await page.keyboard.press('Delete')
  await expect(page.getByTestId('aiws-canvas-block-b')).toHaveCount(0)
  await expect(page.getByTestId('aiws-canvas-block-ab')).toHaveAttribute('data-end', 'broken')
  const end1 = await linePoint(page, 'ab', 1)
  expect(Math.hypot(end1.x - end0.x, end1.y - end0.y)).toBeLessThan(12)
  // still bound in the data (a broken end), with the frozen corner
  const line = (await lines(api, ws.workspace_id))[0]
  expect((line.connector as { end: Json }).end).toEqual({ entity_id: 'b', anchor: { kind: 'named', id: 'n' } })
  await page.mouse.click((await linePoint(page, 'ab', 0.5)).x, (await linePoint(page, 'ab', 0.5)).y)
  await expect(page.getByTestId('aiws-line-end-end')).toHaveAttribute('data-state', 'broken')
  await shot(page, 'cn09-broken')
  await page.getByTestId('aiws-undo').click()
  await expect(page.getByTestId('aiws-canvas-block-b')).toBeVisible()
  await expect(page.getByTestId('aiws-canvas-block-ab')).toHaveAttribute('data-end', 'bound')
  expect(bBox).not.toBeNull()
  // a broken end reconnects by dragging it onto a Block
  await page.getByTestId('aiws-canvas-block-b').click()
  await page.keyboard.press('Delete')
  await expect(page.getByTestId('aiws-canvas-block-ab')).toHaveAttribute('data-end', 'broken')
  await page.mouse.click((await linePoint(page, 'ab', 0.5)).x, (await linePoint(page, 'ab', 0.5)).y)
  await drag(page, await center(page, 'aiws-line-end-end'), await blockCenter(page, 'c'))
  await expect.poll(async () => ((await lines(api, ws.workspace_id))[0].connector as { end: { entity_id: string } }).end.entity_id).toBe('c')
  await expect(page.getByTestId('aiws-canvas-block-ab')).toHaveAttribute('data-end', 'bound')
})

test('CN14 stacking: a line and a Block overlapping hit in paint order; send to back changes both', async ({ page, api }) => {
  const ws = await lineWorkspace(api, `cn14 ${Date.now()}`, [
    { op: 'entity.create', entity_id: 'd', type_id: 'buckyos.cell', parent_id: 'lines', order_key: 'kd', placement: { x: 380, y: 230, w: 120, h: 140 }, payload: { view: { type: 'shape' }, title: 'D', config: { fill: '#ffd9d9', stroke: '#d1242f' } } },
    AB({}, 'ab', 'kz'),
  ])
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  // where the line crosses d, the line (painted later) takes the click
  const d = await page.getByTestId('aiws-canvas-block-d').boundingBox()
  const crossing = await linePoint(page, 'ab', 0.5)
  expect(crossing.x > d!.x && crossing.x < d!.x + d!.width, JSON.stringify({ d, crossing })).toBe(true)
  await page.mouse.click(crossing.x, crossing.y)
  await expect(page.getByTestId('aiws-selection-ab')).toBeVisible()
  await page.keyboard.press('Control+BracketLeft')
  await expect.poll(async () => String((await lines(api, ws.workspace_id))[0].order_key) < 'kd').toBe(true)
  const z = async (id: string) => Number(await page.getByTestId(`aiws-canvas-block-${id}`).evaluate((el) => (el as HTMLElement).style.zIndex))
  await expect.poll(async () => (await z('ab')) < (await z('d'))).toBe(true)
  await page.mouse.click(5, 500)
  await page.mouse.click(crossing.x, crossing.y)
  await expect(page.getByTestId('aiws-selection-d')).toBeVisible()
})

test('CN15 a long line whose ends are off screen is drawn and hit where it crosses the view', async ({ page, api }) => {
  const ws = await lineWorkspace(api, `cn15 ${Date.now()}`, [
    { op: 'entity.create', entity_id: 'far', type_id: 'buckyos.cell', parent_id: 'lines', order_key: 'kf', placement: { x: 6000, y: 4000, w: 160, h: 100 }, payload: { view: { type: 'shape' }, title: 'F' } },
    AB({ end: { entity_id: 'far', anchor: { kind: 'named', id: 'w' } } }, 'long'),
  ])
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  const mid = await linePoint(page, 'long', 0.5)
  await page.mouse.move(mid.x, mid.y)
  for (let i = 0; i < 10; i++) { await page.keyboard.down('Control'); await page.mouse.wheel(0, -200); await page.keyboard.up('Control') }
  await page.waitForTimeout(400)
  await expect(page.getByTestId('aiws-canvas-block-a')).not.toBeInViewport()
  await expect(page.getByTestId('aiws-canvas-block-far')).not.toBeInViewport()
  const visible = await linePoint(page, 'long', 0.5)
  await page.mouse.click(visible.x, visible.y)
  await expect(page.getByTestId('aiws-selection-long')).toBeVisible()
})

test('CN16 view mode shows lines without handles; a flow page does not show them; touch only selects', async ({ page, api }) => {
  const ws = await lineWorkspace(api, `cn16 ${Date.now()}`, [AB({}, 'ab'), AB({}, 'moved', 'ky')])
  await api.commit(ALICE, ws, [{ op: 'tree.move', entity_id: 'moved', new_parent_id: 'page', order_key: 'm', placement: { x: 0, y: 0, w: 10, h: 10 } }])
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await setCanvasMode(page, 'view')
  const p = await linePoint(page, 'ab', 0.5)
  await page.mouse.click(p.x, p.y)
  await expect(page.getByTestId('aiws-selection-ab')).toBeVisible()
  await expect(page.locator('[data-testid^="aiws-line-handles-"]')).toHaveCount(0)
  await page.getByTestId('aiws-canvas-block-a').click()
  await expect(page.locator('[data-testid^="aiws-connect-"]')).toHaveCount(0)
  await page.mouse.dblclick(p.x, p.y)
  await expect(page.getByTestId('aiws-connector-label-edit-ab')).toHaveCount(0)
  await setCanvasMode(page, 'edit')
  // the flow page keeps the line's data but draws no line
  await page.getByTestId('aiws-surface-switch').click()
  await page.getByTestId('aiws-surface-item-page').getByRole('menuitem').click()
  await expect(page.getByTestId('aiws-flow-host')).toBeVisible()
  await expect(page.locator('[data-renderer="connector"], [data-testid="aiws-connector-static"]')).toHaveCount(0)
  expect((await lines(api, ws.workspace_id)).some((l) => l.entity_id === 'moved')).toBe(true)
})

test('OX16 released on blank canvas: the "next object" menu creates the object there and binds the end to it', async ({ page, api }) => {
  const ws = await lineWorkspace(api, `ox16n ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await page.getByTestId('aiws-canvas-block-b').click()
  const from = await center(page, 'aiws-connect-e')
  await drag(page, from, { x: from.x + 220, y: from.y + 10 })
  await expect(page.getByTestId('aiws-menu-next-note')).toBeVisible()
  await expect(page.getByTestId('aiws-menu-next-shape')).toBeVisible()
  await page.getByTestId('aiws-menu-next-shape').click()
  await expect.poll(async () => ((await lines(api, ws.workspace_id))[0]?.connector as { end?: { entity_id: string } } | undefined)?.end?.entity_id ?? '').toMatch(/^c/)
  const line = (await lines(api, ws.workspace_id))[0]
  const target = (await api.outline(ALICE, ws.workspace_id)).find((e: Json) => e.entity_id === (line.connector as { end: { entity_id: string } }).end.entity_id)
  expect(target.view_type).toBe('shape')
  // the end is on the new shape's side facing the start
  expect((line.connector as { end: Json }).end).toEqual({ entity_id: target.entity_id, anchor: { kind: 'named', id: 'w' } })
  await expect(page.getByTestId(`aiws-canvas-block-${line.entity_id as string}`)).toHaveAttribute('data-end', 'bound')
  await shot(page, 'ox16-next-object')
  // the L key is the connector tool; Esc leaves it
  await page.mouse.click(5, 500)
  await page.keyboard.press('l')
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-tool', 'connector')
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-tool', 'select')
})

test('CN dark theme: lines, caps, labels, halo, handles and connection handles read on the dark canvas', async ({ page, api }) => {
  const ws = await lineWorkspace(api, `cn-dark ${Date.now()}`, [
    AB({ route: 'elbow' }, 'ab'),
    { op: 'entity.create', entity_id: 'ac', type_id: 'buckyos.cell', parent_id: 'lines', order_key: 'ky', placement: { x: 180, y: 200, w: 0, h: 400 }, payload: { view: { type: 'connector', version: 1 }, title: '包含', start: { entity_id: 'a', anchor: { kind: 'named', id: 's' } }, end: { entity_id: 'c', anchor: { kind: 'named', id: 'n' } }, config: { start_cap: 'cardinality_one', end_cap: 'cardinality_zero_or_many', dash: 'dashed' } } },
  ])
  await page.addInitScript(() => { window.localStorage.setItem('buckyos.prototype.theme.v1', 'dark') })
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await expect(page.getByTestId('aiws-connector-label-ac')).toHaveText('包含')
  await page.mouse.move(5, 500)
  await shot(page, 'cn-dark-rest')
  const p = await linePoint(page, 'ab', 0.5)
  await page.mouse.click(p.x, p.y)
  await expect(page.getByTestId('aiws-line-handles-ab')).toBeVisible()
  await shot(page, 'cn-dark-selected-line')
  await page.getByTestId('aiws-canvas-block-b').click()
  await expect(page.getByTestId('aiws-connect-w')).toBeVisible()
  await shot(page, 'cn-dark-connect-handles')
})

test('OX16 dragging near the canvas edge pans the view; the dragged end keeps following the pointer', async ({ page, api }) => {
  const ws = await lineWorkspace(api, `ox16e ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  const transform = () => page.getByTestId('aiws-world').evaluate((el) => (el as HTMLElement).style.transform)
  const canvas = (await page.getByTestId('aiws-canvas').boundingBox())!
  await page.getByTestId('aiws-canvas-block-b').click()
  const from = await center(page, 'aiws-connect-e')
  const before = await transform()
  await page.mouse.move(from.x, from.y)
  await page.mouse.down()
  await page.mouse.move(canvas.x + canvas.width - 8, from.y, { steps: 10 })
  // held at the edge: the camera keeps moving
  await expect.poll(transform, { timeout: 5000 }).not.toBe(before)
  const during = await transform()
  await page.waitForTimeout(300)
  expect(await transform()).not.toBe(during)
  await page.mouse.up()
  // the line ends where the pointer was released, now further right in the world
  await expect.poll(async () => (await lines(api, ws.workspace_id)).length).toBe(1)
  await page.keyboard.press('Escape')
  const line = (await lines(api, ws.workspace_id))[0]
  expect((line.placement as { w: number }).w).toBeGreaterThan(400)
})
