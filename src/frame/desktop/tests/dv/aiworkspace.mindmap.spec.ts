/* Mind maps on a real Zone (notepads/aiworkspace-mindmap-todo.md, doc/workspace/脑图实施记录与验收报告.md): the M0
 * fixture committed with protocol 1.1 through the gateway and drawn by the Desktop, folding as each reader's own
 * state across two real accounts (and across a reload), a re-hang seen live by the other reader, a map built with
 * the keyboard, control.import / control.export, and a wish the real model answers with a mind map. */

import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import type { Page } from '@playwright/test'
import type { CommitResult, EntityEnvelope, Operation, WorkspaceSummary } from '../../src/app/aiworkspace/api/types'
import { expect, openWorkspace, screenshot, test, type Actor } from './aiworkspace.fixtures'

type Workspace = Pick<WorkspaceSummary, 'workspace_id' | 'epoch'>

test.use({ viewport: { width: 1600, height: 1000 } })

const FIXTURE = resolve('../aiworkspace/fixtures/mindmap/commits.json')
const AUTO = { x: 'auto', y: 'auto', w: 'auto', h: 'auto' }

async function create(actor: Actor): Promise<Workspace> {
  const ws = await actor.client.ok<WorkspaceSummary>('ws.create', { title: actor.client.prefix })
  actor.client.owned.add(ws.workspace_id)
  return { workspace_id: ws.workspace_id, epoch: ws.epoch }
}

async function commit(actor: Actor, ws: Workspace, operations: Operation[]) {
  const result = await actor.client.commit(ws, operations)
  expect(result.status, JSON.stringify(result)).toBe('accepted')
  return result as CommitResult
}

async function outline(actor: Actor, ws: Workspace) {
  return (await actor.client.ok<{ entities: EntityEnvelope[] }>('doc.outline', { workspace_id: ws.workspace_id })).entities
}

async function entity(actor: Actor, ws: Workspace, id: string) {
  return (await outline(actor, ws)).find((e) => e.entity_id === id)
}

async function openCanvas(actor: Actor, ws: Workspace) {
  await openWorkspace(actor, ws)
  const top = actor.page.getByTestId('aiws-top-canvas')
  if (await top.isVisible()) await top.click()
  await expect(actor.page.getByTestId('aiws-canvas-view')).toBeVisible()
}

async function fitAll(page: Page) {
  await page.getByTestId('aiws-zoom-menu').click()
  await page.getByTestId('aiws-fit-all').click()
  await page.waitForTimeout(400)
}

async function box(page: Page, id: string) {
  const block = page.getByTestId(`aiws-canvas-block-${id}`)
  await expect(block).toBeVisible()
  const b = await block.boundingBox()
  if (!b) throw new Error(`${id} is not on screen`)
  return b
}

const center = (b: { x: number; y: number; width: number; height: number }) => ({ x: b.x + b.width / 2, y: b.y + b.height / 2 })

test('[mindmap] the fixture on a real Zone: drawn, folded per reader, re-hung live, built with the keyboard', async ({ owner, peer }, info) => {
  test.setTimeout(300_000)
  const ws = await create(owner)
  const fixture = JSON.parse(readFileSync(FIXTURE, 'utf8')) as { commits: { operations: Operation[] }[] }
  for (const c of fixture.commits) await commit(owner, ws, c.operations)
  await owner.client.ok('ws.grant', { workspace_id: ws.workspace_id, subject: peer.userId, capabilities: ['read', 'update', 'append', 'structure', 'delete', 'comment'] })
  const right3 = await entity(owner, ws, 'right-3')
  expect(right3?.placement).toEqual(AUTO)

  // drawn by the control: one edge per parent → child pair, children on the side of the direction
  const page = owner.page
  await openCanvas(owner, ws)
  await fitAll(page)
  await expect(page.getByTestId('aiws-mindmap-edges-mm-right')).toHaveAttribute('data-edges', '10')
  await expect(page.getByTestId('aiws-mindmap-edges-mm-down')).toHaveAttribute('data-edges', '10')
  await expect(page.getByTestId('aiws-mindmap-edges-min-inner')).toHaveAttribute('data-edges', '2')
  const root = await box(page, 'right-root')
  expect((await box(page, 'right-1')).x).toBeGreaterThan(root.x + root.width)
  await expect(page.getByTestId('aiws-pin-both-3')).toBeVisible()
  await screenshot(page, info, 'mindmap-fixture')

  // folding: the owner's own state — kept over a reload, not committed, not seen by the peer
  const head = (await owner.client.info(ws)).head_seq
  await page.getByTestId('aiws-canvas-block-right-3').click()
  await page.getByTestId('aiws-fold-right-3').click()
  await expect(page.getByTestId('aiws-canvas-block-right-3-1')).toHaveCount(0)
  await expect(page.getByTestId('aiws-fold-right-3')).toHaveText('3')
  await page.waitForTimeout(1500)
  expect((await owner.client.info(ws)).head_seq).toBe(head)
  await page.reload()
  await openCanvas(owner, ws)
  await fitAll(page)
  await expect(page.getByTestId('aiws-canvas-block-right-3')).toBeVisible()
  await expect(page.getByTestId('aiws-canvas-block-right-3-1')).toHaveCount(0)
  await openCanvas(peer, ws)
  await fitAll(peer.page)
  await expect(peer.page.getByTestId('aiws-canvas-block-right-3-1')).toBeVisible()
  await screenshot(peer.page, info, 'mindmap-peer-unfolded')

  // unfold, then re-hang 一.2 under 三 by a drag; the peer's map moves it without a reload
  await page.getByTestId('aiws-canvas-block-right-3').click()
  await page.keyboard.press('Control+/')
  await expect(page.getByTestId('aiws-canvas-block-right-3-1')).toBeVisible()
  await page.keyboard.press('Escape')
  const from = center(await box(page, 'right-1-2'))
  const to = center(await box(page, 'right-3'))
  await page.mouse.move(from.x, from.y)
  await page.mouse.down()
  await page.mouse.move(to.x, to.y, { steps: 14 })
  await expect(page.getByTestId('aiws-drop-child-right-3')).toBeVisible()
  await page.mouse.up()
  await expect.poll(async () => (await entity(owner, ws, 'right-1-2'))?.parent_id).toBe('right-3')
  expect((await entity(owner, ws, 'right-1-2'))?.placement).toEqual(AUTO)
  await expect.poll(async () => {
    const node = await peer.page.getByTestId('aiws-canvas-block-right-1-2').boundingBox()
    const three = await peer.page.getByTestId('aiws-canvas-block-right-3').boundingBox()
    return Boolean(node && three && node.x > three.x + three.width)
  }, { timeout: 30_000 }).toBe(true)
  await screenshot(peer.page, info, 'mindmap-peer-rehung')

  // a new map from the toolbar, built with the keyboard
  await page.getByTestId('aiws-tool-insert-mindmap').click()
  const view = await page.getByTestId('aiws-canvas').boundingBox()
  await page.mouse.click(view!.x + view!.width - 260, view!.y + 160)
  const edit = page.locator('[data-testid^="aiws-mindnode-edit-"]')
  await expect(edit).toBeVisible()
  await page.keyboard.press('Control+a')
  await page.keyboard.type('Launch')
  await page.keyboard.press('Tab')
  await expect(edit).toBeVisible()
  await page.keyboard.type('Market')
  await page.keyboard.press('Enter')
  await page.keyboard.press('Enter')
  await expect(edit).toBeVisible()
  await page.keyboard.type('Product')
  await page.keyboard.press('Enter')
  await expect.poll(async () => (await outline(owner, ws)).filter((e) => e.view_type === 'mindnode' && ['Launch', 'Market', 'Product'].includes(e.title ?? '')).length).toBe(3)
  const all = await outline(owner, ws)
  const launch = all.find((e) => e.title === 'Launch')!
  const map = all.find((e) => e.entity_id === launch.parent_id)!
  expect(map.kind).toBe('mindmap')
  for (const title of ['Market', 'Product']) {
    const node = all.find((e) => e.title === title)!
    expect(node.parent_id).toBe(launch.entity_id)
    expect(node.placement).toEqual(AUTO)
  }
  await expect(peer.page.getByTestId(`aiws-mindnode-${launch.entity_id}`)).toBeAttached({ timeout: 30_000 })
  await screenshot(page, info, 'mindmap-keyboard')
})

test('[mindmap] control.list / describe / import / export through the Zone gateway, drawn by the Desktop', async ({ owner }, info) => {
  const ws = await create(owner)
  const fixture = JSON.parse(readFileSync(FIXTURE, 'utf8')) as { commits: { operations: Operation[] }[] }
  await commit(owner, ws, fixture.commits[0].operations)
  const listed = await owner.client.ok<{ controls: { kind: string }[] }>('control.list', {})
  expect(listed.controls.map((c) => c.kind)).toContain('mindmap')
  const described = await owner.client.ok<{ control: { agent: string } }>('control.describe', { kind: 'mindmap' })
  expect(described.control.agent).toContain('"auto"')

  const md = '- Launch\n  - **Market**\n    - interviews\n  - Product\n'
  // control.import answers like doc.commit (the commit outcome), plus the new control's id and node count
  const imported = await owner.client.call<{ status: string; container_id: string; nodes: number }>('control.import', {
    workspace_id: ws.workspace_id, parent_id: 's-mind', format: 'markdown', text: md, at: { x: 100, y: 80 },
  })
  expect(imported).toMatchObject({ status: 'accepted', nodes: 4 })
  const map = await entity(owner, ws, imported.container_id)
  expect(map).toMatchObject({ kind: 'mindmap', title: 'Launch', placement: { x: 100, y: 80, w: 'auto', h: 'auto' } })
  expect((await outline(owner, ws)).find((e) => e.title === '**Market**')).toMatchObject({ view_type: 'mindnode', placement: AUTO })

  const exported = await owner.client.ok<{ text: string }>('control.export', { workspace_id: ws.workspace_id, entity_id: imported.container_id })
  expect(exported.text).toBe(md)
  const opml = await owner.client.ok<{ text: string }>('control.export', { workspace_id: ws.workspace_id, entity_id: imported.container_id, format: 'opml' })
  expect(opml.text).toContain('<outline text="Product"')
  const again = await owner.client.call<{ status: string; container_id: string }>('control.import', { workspace_id: ws.workspace_id, parent_id: 's-mind', format: 'opml', text: opml.text, at: { x: 100, y: 400 } })
  expect(again.status).toBe('accepted')
  expect((await owner.client.ok<{ text: string }>('control.export', { workspace_id: ws.workspace_id, entity_id: again.container_id })).text).toBe(md)
  await expect(owner.client.ok('control.export', { workspace_id: ws.workspace_id, entity_id: 's-mind' })).rejects.toThrow()

  await openCanvas(owner, ws)
  await fitAll(owner.page)
  await expect(owner.page.getByTestId(`aiws-mindmap-edges-${imported.container_id}`)).toHaveAttribute('data-edges', '3')
  await expect(owner.page.getByTestId(`aiws-mindmap-edges-${again.container_id}`)).toHaveAttribute('data-edges', '3')
  // inline Markdown is drawn formatted: **Market** heavier than its plain sibling, without the asterisks
  const nodes = (await outline(owner, ws)).filter((e) => e.view_type === 'mindnode')
  const weight = async (title: string) => {
    const node = owner.page.getByTestId(`aiws-mindnode-${nodes.find((e) => e.title === title)!.entity_id}`).first()
    await expect(node).toHaveText(title.replaceAll('*', ''))
    return Number(await node.locator('span').first().evaluate((el) => getComputedStyle(el).fontWeight))
  }
  expect(await weight('**Market**')).toBeGreaterThan(await weight('Product'))
  await screenshot(owner.page, info, 'mindmap-imported')
})

test('[mindmap] a wish answered by the real model with a mind map', async ({ owner }, info) => {
  test.setTimeout(600_000)
  const ws = await create(owner)
  const fixture = JSON.parse(readFileSync(FIXTURE, 'utf8')) as { commits: { operations: Operation[] }[] }
  await commit(owner, ws, fixture.commits[0].operations)
  await commit(owner, ws, [
    { op: 'entity.create', entity_id: 'wish-map', type_id: 'buckyos.wish', parent_id: 's-mind-content', order_key: 'w', name: '产品发布脑图',
      payload: { title: '产品发布脑图', executor: 'xllm', output_mode: 'overwrite', inputs: [], output: { container_id: 's-mind-content', surface_id: 's-mind', name: '产品发布脑图结果' },
        prompt: '做一张脑图：一款家用 NAS 产品发布计划。根节点是产品发布，下面分市场、产品、渠道、风险四个分支，每个分支 2 到 4 个要点。' } },
    { op: 'entity.create', entity_id: 'blk-wish-map', type_id: 'buckyos.cell', parent_id: 's-mind', order_key: 'w',
      placement: { x: 0, y: 900, w: 360, h: 240 }, payload: { view: { type: 'wish' }, source_ref: { entity_id: 'wish-map' }, title: '产品发布脑图' } },
  ] as Operation[])
  const page = owner.page
  await openCanvas(owner, ws)
  await fitAll(page)
  await page.getByTestId('aiws-canvas-block-blk-wish-map').dblclick()
  await expect(page.getByTestId('aiws-wish-executor')).toHaveValue('xllm')
  await page.getByTestId('aiws-wish-analyze').click()
  await expect(page.getByTestId('aiws-wish-contract')).toContainText('control', { timeout: 180_000 })
  await screenshot(page, info, 'mindmap-wish-analysis')
  await page.getByTestId('aiws-wish-execute').click()
  await expect(page.getByTestId('aiws-wish-candidate')).toBeVisible({ timeout: 300_000 })
  await expect(page.getByTestId('aiws-wish-control-preview')).toBeVisible()
  await screenshot(page, info, 'mindmap-wish-candidate')
  await page.getByTestId('aiws-wish-apply').click()
  await expect(page.getByTestId('aiws-wish-message')).toBeVisible({ timeout: 60_000 })
  // a control carries no dependency record (only data entities do): the wish's last run binds it
  const wish = await owner.client.ok<{ content: { payload: { last_run: { result_bindings: { results: Record<string, { type: string; entity_id: string }> } } } } }>('doc.read', { workspace_id: ws.workspace_id, entity_id: 'wish-map' })
  const bound = Object.values(wish.content.payload.last_run.result_bindings.results).filter((b) => b.type === 'control')
  expect(bound).toHaveLength(1)
  const all = await outline(owner, ws)
  const maps = all.filter((e) => e.entity_id === bound[0].entity_id)
  expect(maps).toHaveLength(1)
  expect(maps[0]).toMatchObject({ kind: 'mindmap', parent_id: 'wish-map-out' }) // in the wish's output group on the canvas
  const nodes = all.filter((e) => e.view_type === 'mindnode')
  const under = (id: string): EntityEnvelope[] => nodes.filter((n) => n.parent_id === id).flatMap((n) => [n, ...under(n.entity_id)])
  const tree = under(maps[0].entity_id)
  expect(tree.length).toBeGreaterThanOrEqual(9)
  await info.attach('wish-map-outline', { body: Buffer.from(JSON.stringify(tree.map((n) => ({ id: n.entity_id, parent: n.parent_id, title: n.title, placement: n.placement })), null, 1)), contentType: 'application/json' })
  expect(tree.map((n) => n.placement)).toEqual(tree.map(() => AUTO))
  await fitAll(page)
  await expect(page.getByTestId(`aiws-mindmap-edges-${maps[0].entity_id}`)).toHaveAttribute('data-edges', String(tree.length - 1))
  await screenshot(page, info, 'mindmap-wish-applied')
})
