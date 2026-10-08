/* Presentation (doc/workspace/BuckyOS AI Workspace 第三期规划.md §15 P3-01…P3-19): presentation paths and their editor,
 * the stage of a non-public show (transitions, keys, free browsing, marks, operable Blocks in the clone), the show lock,
 * the prompter link and the guide — against the real backend. The workspace is the M0 fixture
 * (src/frame/aiworkspace/fixtures/presentation/commits.json): two free Surfaces, frame1 → frame2 → viewport1 →
 * viewport2 → frame3 and a guide path. */

import type { Page } from '@playwright/test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { mainMenu, openCanvas, setCanvasMode, test, expect, type Api } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'
const SHOTS = 'test-results/aiworkspace-presentation'
const FIXTURE = resolve('../aiworkspace/fixtures/presentation/commits.json')

type Json = Record<string, unknown>

/** The M0 fixture replayed through the public interface; `extra` grants follow. */
async function showWorkspace(api: Api, title: string, grants: { subject: string; capabilities: string[] }[] = []) {
  const ws = await api.rpc(ALICE, 'ws.create', { title }) as { workspace_id: string; epoch: string }
  const fixture = JSON.parse(readFileSync(FIXTURE, 'utf8')) as { commits: { operations: Json[] }[] }
  for (const [index, commit] of fixture.commits.entries()) {
    const result = await api.commit(ALICE, ws, commit.operations, `presentation/${index + 1}`)
    expect(result.status, JSON.stringify(result)).toBe('accepted')
  }
  for (const g of grants) expect((await api.rpc(ALICE, 'ws.grant', { workspace_id: ws.workspace_id, ...g })).ok).toBe(true)
  return ws
}

async function startShow(page: Page, stepTitle?: string) {
  await mainMenu(page, 'aiws-top-play')
  await expect(page.getByTestId('aiws-start-show')).toBeVisible()
  if (stepTitle) await page.getByTestId('aiws-start-step').selectOption({ label: stepTitle })
  await page.getByTestId('aiws-start-show-go').click()
  await expect(page.getByTestId('aiws-show')).toBeVisible({ timeout: 30_000 })
}

/** The step on stage once its transition finished. */
async function onStep(page: Page, stepId: string) {
  await expect(page.getByTestId('aiws-show')).toHaveAttribute('data-step', stepId)
  await expect.poll(() => page.evaluate((id) => {
    const t = window.__aiwsTestHooks?.stage?.transitions.at(-1)
    return t && t.step === id && t.arrivedAt !== null
  }, stepId), { timeout: 10_000 }).toBe(true)
}

/** The screen rectangle of the stage mask's hole (the cropped Frame or the stage). */
async function hole(page: Page): Promise<{ x: number; y: number; w: number; h: number }> {
  const raw = await page.getByTestId('aiws-stage-mask').getAttribute('data-hole')
  const [x, y, w, h] = (raw ?? '').split(',').map(Number)
  return { x, y, w, h }
}

async function shot(page: Page, name: string) {
  await page.screenshot({ path: `${SHOTS}/${name}.png` })
}

test('P3-01 the five-step mixed path plays in order from the real UI', async ({ page, api }) => {
  const ws = await showWorkspace(api, `p3-01 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await setCanvasMode(page, 'presentation_edit')
  await expect(page.getByTestId('aiws-path-editor')).toBeVisible()
  await expect(page.getByTestId('aiws-path-steps').getByRole('option')).toHaveCount(5)
  await shot(page, 'p3-01-editor')
  await startShow(page)
  await onStep(page, 's1')
  await shot(page, 'p3-01-s1')
  for (const id of ['s2', 's3', 's4', 's5']) {
    await page.keyboard.press('ArrowRight')
    await onStep(page, id)
    await shot(page, `p3-01-${id}`)
  }
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('aiws-show')).toHaveCount(0)
  await expect(page.getByTestId('aiws-canvas-view')).toBeVisible()
})

// ---- helpers over the stage's test hooks

/** The world rectangle the stage shows inside the stage rectangle S (letterbox excluded). */
async function stageWorld(page: Page, stage = { w: 1920, h: 1080 }) {
  return page.evaluate((st) => {
    const el = document.querySelector('[data-testid="aiws-stage-canvas"]') as HTMLElement
    const v = window.__aiwsTestHooks!.stage!.view()
    const k = Math.min(el.clientWidth / st.w, el.clientHeight / st.h)
    const s = { x: (el.clientWidth - st.w * k) / 2, y: (el.clientHeight - st.h * k) / 2, w: st.w * k, h: st.h * k }
    return { x: v.x + s.x / v.zoom, y: v.y + s.y / v.zoom, w: s.w / v.zoom, h: s.h / v.zoom, surfaceId: v.surfaceId }
  }, stage)
}

function closeTo(actual: { x: number; y: number; w: number; h: number }, expected: { x: number; y: number; w: number; h: number }, tolerance = 1) {
  for (const k of ['x', 'y', 'w', 'h'] as const) expect(Math.abs(actual[k] - expected[k]), `${k}: ${actual[k]} vs ${expected[k]}`).toBeLessThanOrEqual(tolerance)
}

/** Press a stage key from inside the page and sample the camera zoom every frame until the step arrived. */
async function zoomsDuring(page: Page, key: string): Promise<{ zooms: number[]; kind: string }> {
  return page.evaluate((k) => new Promise((resolveZooms) => {
    const stage = window.__aiwsTestHooks!.stage!
    const before = stage.transitions.length
    const zooms: number[] = []
    window.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true }))
    const tick = () => {
      zooms.push(stage.view().zoom)
      const t = stage.transitions.at(-1)
      if (stage.transitions.length > before && t && t.arrivedAt !== null) resolveZooms({ zooms, kind: t.kind })
      else requestAnimationFrame(tick)
    }
    requestAnimationFrame(tick)
  }), key)
}

function viewportRect(center: { x: number; y: number }, zoom: number, stage = { w: 1920, h: 1080 }) {
  return { x: center.x - stage.w / zoom / 2, y: center.y - stage.h / zoom / 2, w: stage.w / zoom, h: stage.h / zoom }
}

/** The active show of the stage page (kept for a reload in sessionStorage). */
async function activeShow(page: Page, workspaceId: string): Promise<{ start: { show_id: string; prompter_token: string; clone_workspace_id: string | null } }> {
  return page.evaluate((id) => JSON.parse(window.sessionStorage.getItem(`aiworkspace.active-show:${id}`) ?? 'null'), workspaceId)
}

async function pathOf(api: Api, workspaceId: string, pathId = 'path-intro'): Promise<{ steps: { id: string; target: { kind: string; entity_id: string }; transition?: string; enabled?: boolean }[]; stage: { w: number; h: number } }> {
  return (await api.read(ALICE, workspaceId, pathId)).content.payload
}

async function setKey(api: Api, ws: { workspace_id: string; epoch: string }, entity: string, key: string, value: unknown, token = ALICE) {
  const read = await api.read(token, ws.workspace_id, entity)
  const result = await api.commit(token, ws, [{ op: 'entity.set_keys', entity_id: entity, keys: [{ key, value, expect: { rev: read.content.key_revs[key] ?? 0 } }] }])
  expect(result.status, JSON.stringify(result)).toBe('accepted')
  return result
}

test('P3-01b the editor builds a path from the UI: new path, Frames, the current view; it survives a reload', async ({ page, api }) => {
  const ws = await showWorkspace(api, `p3-01b ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await setCanvasMode(page, 'presentation_edit')
  await page.getByTestId('aiws-path-new').click()
  await expect(page.getByTestId('aiws-path-select')).toHaveValue(/.+/)
  const pathId = await page.getByTestId('aiws-path-select').inputValue()
  expect(pathId).not.toBe('path-intro')
  await page.getByTestId('aiws-path-add-frame').selectOption('frame2')
  await page.getByTestId('aiws-path-add-frame').selectOption('frame1')
  await page.getByTestId('aiws-path-add-view').click()
  await expect(page.getByTestId('aiws-viewfinder')).toBeVisible()
  await shot(page, 'p3-01b-viewfinder')
  await page.getByTestId('aiws-viewfinder-confirm').click()
  await expect(page.getByTestId('aiws-viewfinder')).toHaveCount(0)
  await expect(page.getByTestId('aiws-path-steps').getByRole('option')).toHaveCount(3)
  const path = await pathOf(api, ws.workspace_id, pathId)
  expect(path.steps.map((s) => s.target.kind)).toEqual(['frame', 'frame', 'viewport'])
  expect(path.steps.slice(0, 2).map((s) => s.target.entity_id)).toEqual(['frame2', 'frame1'])
  const vp = (await api.read(ALICE, ws.workspace_id, path.steps[2].target.entity_id)).content.payload
  expect(vp.surface_ref.entity_id).toBe('s-launch')
  expect(vp.zoom).toBeGreaterThan(0)
  // each action was one commit and one undo step
  await page.keyboard.press('Control+z')
  await expect(page.getByTestId('aiws-path-steps').getByRole('option')).toHaveCount(2)
  await page.keyboard.press('Control+Shift+z')
  await expect(page.getByTestId('aiws-path-steps').getByRole('option')).toHaveCount(3)
  // reopened: the sub-mode and the chosen path are the user's work state
  await openCanvas(page, ALICE, ws.workspace_id)
  await expect(page.getByTestId('aiws-path-editor')).toBeVisible({ timeout: 30_000 })
  await expect(page.getByTestId('aiws-path-select')).toHaveValue(pathId)
  await expect(page.getByTestId('aiws-path-steps').getByRole('option')).toHaveCount(3)
})

test('P3-02 changing the stage aspect brings the Frames of the path to it in one undoable commit', async ({ page, api }) => {
  const ws = await showWorkspace(api, `p3-02 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await setCanvasMode(page, 'presentation_edit')
  const head = await api.headSeq(ALICE, ws.workspace_id)
  await page.getByTestId('aiws-path-stage').selectOption('4:3')
  await expect(page.getByTestId('aiws-path-stage-confirm')).toBeVisible()
  await page.getByTestId('aiws-path-stage-apply').click()
  await expect.poll(async () => (await pathOf(api, ws.workspace_id)).stage).toEqual({ w: 1440, h: 1080 })
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
  for (const id of ['frame1', 'frame2', 'frame3']) {
    const p = (await api.read(ALICE, ws.workspace_id, id)).placement
    expect(p.w, id).toBe(1600)
    expect(p.h, id).toBe(1200)
    expect(p.y + p.h / 2, `${id} keeps its centre`).toBe(450)
  }
  // frame1 is also in the 16:9 guide: the editor says it can only fit one of them
  await page.getByTestId('aiws-path-step-s1').click()
  await expect(page.getByTestId('aiws-step-props')).toContainText('舞台比例不同的路径')
  await page.getByTestId('aiws-path-editor').getByRole('heading', { name: '演讲路径' }).click()
  await page.keyboard.press('Control+z')
  await expect.poll(async () => (await pathOf(api, ws.workspace_id)).stage).toEqual({ w: 1920, h: 1080 })
  expect((await api.read(ALICE, ws.workspace_id, 'frame2')).placement.h).toBe(900)
})

test('P3-03 a saved view shows the same content in any window size, only the letterbox differs', async ({ page, api }) => {
  const ws = await showWorkspace(api, `p3-03 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await startShow(page, '3. 工作流全貌')
  await onStep(page, 's3')
  const expected = viewportRect({ x: 800, y: 1600 }, 1)
  closeTo(await stageWorld(page), expected)
  await page.setViewportSize({ width: 1100, height: 950 })
  await expect.poll(async () => Math.abs((await stageWorld(page)).w - expected.w)).toBeLessThan(1)
  closeTo(await stageWorld(page), expected)
  await shot(page, 'p3-03-narrow')
  // a Frame step of the stage's aspect fills the stage exactly (no crop of the stage itself)
  await page.keyboard.press('Home')
  await onStep(page, 's1')
  closeTo(await stageWorld(page), { x: 0, y: 0, w: 1600, h: 900 })
})

test('P3-04 Viewport to Viewport: far apart it zooms out first, close by it does not; it lands exactly', async ({ page, api }) => {
  const ws = await showWorkspace(api, `p3-04 ${Date.now()}`)
  await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'v-near', type_id: 'buckyos.viewport', parent_id: 'shows', order_key: 'n', payload: { title: '近处', surface_ref: { entity_id: 's-launch' }, center: { x: 1000, y: 1700 }, zoom: 1.25 } },
    { op: 'entity.create', entity_id: 'v-far', type_id: 'buckyos.viewport', parent_id: 'shows', order_key: 'o', payload: { title: '远处', surface_ref: { entity_id: 's-launch' }, center: { x: 30000, y: 1600 }, zoom: 1 } },
    { op: 'entity.create', entity_id: 'path-fly', type_id: 'buckyos.show-path', parent_id: 'shows', order_key: 'p', name: '飞行', payload: { title: '飞行', purpose: 'presentation', stage: { w: 1920, h: 1080 }, steps: [
      { id: 'a', target: { kind: 'viewport', entity_id: 'viewport1' } }, { id: 'b', target: { kind: 'viewport', entity_id: 'v-near' } }, { id: 'c', target: { kind: 'viewport', entity_id: 'v-far' } }] } },
  ])
  await openCanvas(page, ALICE, ws.workspace_id)
  await mainMenu(page, 'aiws-top-play')
  await page.getByTestId('aiws-start-path').selectOption({ label: '飞行' })
  await page.getByTestId('aiws-start-show-go').click()
  await onStep(page, 'a')
  const near = await zoomsDuring(page, 'ArrowRight')
  expect(near.kind).toBe('fly')
  // a near target: never further out than the nearer of the two ends
  expect(Math.min(...near.zooms), JSON.stringify(near.zooms)).toBeGreaterThanOrEqual(0.98 * Math.min(near.zooms[0], near.zooms.at(-1)!))
  closeTo(await stageWorld(page), viewportRect({ x: 1000, y: 1700 }, 1.25))
  const far = await zoomsDuring(page, 'ArrowRight')
  expect(far.kind).toBe('fly')
  // a far one: well out first, then in
  expect(Math.min(...far.zooms), JSON.stringify(far.zooms)).toBeLessThan(0.5 * Math.min(far.zooms[0], far.zooms.at(-1)!))
  closeTo(await stageWorld(page), viewportRect({ x: 30000, y: 1600 }, 1))
})

test('P3-05 commands during a transition: the last one wins; free browsing and back to the step', async ({ page, api }) => {
  const ws = await showWorkspace(api, `p3-05 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await startShow(page)
  await onStep(page, 's1')
  await page.keyboard.press('ArrowRight')
  await page.keyboard.press('ArrowRight')
  await page.keyboard.press('ArrowRight')
  await onStep(page, 's4')
  const landed = await stageWorld(page)
  closeTo(landed, viewportRect({ x: 350, y: 1450 }, 2))
  await page.waitForTimeout(1500)
  closeTo(await stageWorld(page), landed)
  // a hand drag browses freely; the step stays and the crop goes away
  await page.mouse.move(800, 500)
  await page.mouse.down()
  await page.mouse.move(600, 400, { steps: 6 })
  await page.mouse.up()
  await expect(page.getByTestId('aiws-show')).toHaveAttribute('data-free', 'true')
  await expect(page.getByTestId('aiws-show')).toHaveAttribute('data-step', 's4')
  await page.getByTestId('aiws-show-back').click()
  await expect(page.getByTestId('aiws-show')).not.toHaveAttribute('data-free', 'true')
  await expect.poll(async () => Math.abs((await stageWorld(page)).x - landed.x)).toBeLessThan(1)
  // Esc out of free browsing goes back, not out of the show
  await page.mouse.wheel(0, 300)
  await expect(page.getByTestId('aiws-show')).toHaveAttribute('data-free', 'true')
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('aiws-show')).not.toHaveAttribute('data-free', 'true')
  await expect(page.getByTestId('aiws-show')).toBeVisible()
})

test('P3-06 mixed steps across Surfaces: same Surface by the default rules, a fade across; no crop left over', async ({ page, api }) => {
  const ws = await showWorkspace(api, `p3-06 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await startShow(page, '2. 问题')
  await onStep(page, 's2')
  const kinds: string[] = []
  for (const id of ['s3', 's4', 's5']) {
    await page.keyboard.press('ArrowRight')
    await onStep(page, id)
    kinds.push(await page.evaluate(() => window.__aiwsTestHooks!.stage!.transitions.at(-1)!.kind))
  }
  // frame → viewport flies; s4 asked for a fade; s5 is on another Surface
  expect(kinds).toEqual(['fly', 'fade', 'fade'])
  await expect(page.getByTestId('aiws-stage-canvas')).toHaveAttribute('data-surface', 's-work')
  const el = page.getByTestId('aiws-stage-canvas')
  const box = (await el.boundingBox())!
  const k = Math.min(box.width / 1920, box.height / 1080)
  closeTo(await hole(page), { x: (box.width - 1920 * k) / 2, y: (box.height - 1080 * k) / 2, w: 1920 * k, h: 1080 * k }, 1.5)
  await expect(page.getByTestId('aiws-stage-veil')).toHaveCSS('opacity', '0')
  await expect(page.getByTestId('aiws-stage-mask')).toHaveCSS('opacity', '1')
  // the hidden Block of s5 is not drawn, the Frame chrome never is
  await expect(page.getByTestId('aiws-canvas-block-shape-extra')).toBeHidden()
  await expect(page.getByTestId('aiws-canvas-block-frame3')).toBeHidden()
})

test('P3-07 the same Frame twice: two steps, reordered by dragging; the canvas stacking stays', async ({ page, api }) => {
  const ws = await showWorkspace(api, `p3-07 ${Date.now()}`)
  const orderBefore = (await api.read(ALICE, ws.workspace_id, 'frame1')).order_key
  await openCanvas(page, ALICE, ws.workspace_id)
  await setCanvasMode(page, 'presentation_edit')
  await page.getByTestId('aiws-path-step-s4').click()
  await page.getByTestId('aiws-path-add-frame').selectOption('frame1')
  await expect(page.getByTestId('aiws-path-steps').getByRole('option')).toHaveCount(6)
  let path = await pathOf(api, ws.workspace_id)
  // inserted after the selected step
  const again = path.steps[4]
  expect(again.target.entity_id).toBe('frame1')
  expect(again.id).not.toBe('s1')
  await page.getByTestId(`aiws-path-step-${again.id}`).dragTo(page.getByTestId('aiws-path-step-s2'))
  await expect.poll(async () => (await pathOf(api, ws.workspace_id)).steps.map((s) => s.id)).toEqual(['s1', again.id, 's2', 's3', 's4', 's5'])
  await shot(page, 'p3-07-reordered')
  path = await pathOf(api, ws.workspace_id)
  expect(path.steps.filter((s) => s.target.entity_id === 'frame1').length).toBe(2)
  expect((await api.read(ALICE, ws.workspace_id, 'frame1')).order_key).toBe(orderBefore)
  // keyboard reordering is the same command
  await page.getByTestId(`aiws-path-step-${again.id}`).press('Alt+ArrowUp')
  await expect.poll(async () => (await pathOf(api, ws.workspace_id)).steps[0].id).toBe(again.id)
})

test('P3-08 deleted targets become dangling steps: listed, skipped by the show, removable; nothing else goes', async ({ page, api }) => {
  const ws = await showWorkspace(api, `p3-08 ${Date.now()}`)
  for (const id of ['viewport2', 'frame3']) {
    const read = await api.read(ALICE, ws.workspace_id, id)
    const r = await api.commit(ALICE, ws, [{ op: 'entity.delete', entity_id: id, expect: { rev: read.life_rev } }])
    expect(r.status, JSON.stringify(r)).toBe('accepted')
  }
  await openCanvas(page, ALICE, ws.workspace_id)
  await setCanvasMode(page, 'presentation_edit')
  await expect(page.getByTestId('aiws-path-step-s4')).toHaveAttribute('data-problem', 'missing')
  await expect(page.getByTestId('aiws-path-step-s5')).toHaveAttribute('data-problem', 'missing')
  await expect(page.getByTestId('aiws-path-editor')).toContainText('2 个失效项')
  await mainMenu(page, 'aiws-top-play')
  await expect(page.getByTestId('aiws-start-summary')).toContainText('3 步')
  await page.getByTestId('aiws-start-show-go').click()
  await expect(page.getByTestId('aiws-show')).toHaveAttribute('data-count', '3')
  await page.keyboard.press('Escape')
  await page.getByTestId('aiws-path-step-s5').click()
  await expect(page.getByTestId('aiws-step-problem')).toBeVisible()
  await page.getByTestId('aiws-path-step-remove').click()
  await expect.poll(async () => (await pathOf(api, ws.workspace_id)).steps.map((s) => s.id)).toEqual(['s1', 's2', 's3', 's4'])
  // the Viewport and Frame stayed deleted; the other targets are untouched
  for (const id of ['frame1', 'frame2', 'viewport1']) expect((await api.read(ALICE, ws.workspace_id, id)).deleted).toBe(false)
})

test('P3-09 two writers on one path: commands replay on the newer steps; a deleted anchor needs attention', async ({ page, api }) => {
  const ws = await showWorkspace(api, `p3-09 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await setCanvasMode(page, 'presentation_edit')
  // the first commit of the editor is held back while another writer reorders the steps
  let interfered = false
  await page.route('**/kapi/aiworkspace', async (route) => {
    const body = route.request().postDataJSON() as { method: string; params: { operations?: { op: string; keys?: { key: string }[] }[] } }
    if (!interfered && body.method === 'doc.commit' && body.params.operations?.some((op) => op.keys?.some((k) => k.key === 'steps'))) {
      interfered = true
      const path = await pathOf(api, ws.workspace_id)
      const steps = [...path.steps].reverse()
      await setKey(api, ws, 'path-intro', 'steps', steps)
    }
    await route.continue()
  })
  await page.getByTestId('aiws-path-step-s2').click()
  await page.getByTestId('aiws-step-transition').selectOption('cut')
  await expect.poll(async () => (await pathOf(api, ws.workspace_id)).steps.find((s) => s.id === 's2')?.transition).toBe('cut')
  // both writes are there: the reversed order and the transition
  expect((await pathOf(api, ws.workspace_id)).steps.map((s) => s.id)).toEqual(['s5', 's4', 's3', 's2', 's1'])
  expect(interfered).toBe(true)
  // an anchor deleted meanwhile cannot be replayed: the edit stays to be handled
  interfered = false
  await page.unroute('**/kapi/aiworkspace')
  await page.route('**/kapi/aiworkspace', async (route) => {
    const body = route.request().postDataJSON() as { method: string; params: { operations?: { op: string; keys?: { key: string }[] }[] } }
    if (!interfered && body.method === 'doc.commit' && body.params.operations?.some((op) => op.keys?.some((k) => k.key === 'steps'))) {
      interfered = true
      const path = await pathOf(api, ws.workspace_id)
      await setKey(api, ws, 'path-intro', 'steps', path.steps.filter((s) => s.id !== 's3'))
    }
    await route.continue()
  })
  await page.getByTestId('aiws-path-step-s3').click()
  await page.getByTestId('aiws-step-transition').selectOption('fade')
  await expect(page.getByTestId('aiws-notice').filter({ hasText: '无法应用到最新的路径上' })).toBeVisible()
  await page.unroute('**/kapi/aiworkspace')
})

test('P3-10 a share export leaves the speaker notes out; captions and paths travel; no one\'s progress does', async ({ page, api }) => {
  const ws = await showWorkspace(api, `p3-10 ${Date.now()}`)
  await api.rpc(ALICE, 'ws.set_user_state', { workspace_id: ws.workspace_id, entries: { 'guide:path-guide': { index: 2, done: false } } })
  // the dialog: a share package defaults to no notes, a personal backup to notes
  await openCanvas(page, ALICE, ws.workspace_id)
  await mainMenu(page, 'aiws-menu-export')
  await expect(page.getByTestId('aiws-export-notes')).not.toBeChecked()
  await page.getByTestId('aiws-export-dialog').getByLabel('导出方式').selectOption('personal_backup')
  await expect(page.getByTestId('aiws-export-notes')).toBeChecked()
  // the package itself, through the API
  const exported = await api.rpc(ALICE, 'doc.export', { workspace_id: ws.workspace_id, mode: 'share', self_contained: true })
  expect(exported.manifest.notes_included).toBe(false)
  const zip = await fetch(`${api.base}/export/${ws.workspace_id}/${exported.export_id}`, { headers: { authorization: `Bearer ${ALICE}` } })
  const begin = await api.rpc(ALICE, 'ws.begin_import', {})
  await fetch(`${api.base}/upload/${begin.upload_id}`, { method: 'PUT', headers: { authorization: `Bearer ${ALICE}` }, body: Buffer.from(await zip.arrayBuffer()) })
  const imported = await api.rpc(ALICE, 'ws.import', { upload_id: begin.upload_id, semantics: 'new' })
  expect(imported.ok, JSON.stringify(imported)).toBe(true)
  const copy = imported.workspace_id
  expect((await pathOf(api, copy)).steps.length).toBe(5)
  const frame1 = (await api.read(ALICE, copy, 'frame1')).content.payload.presentation
  expect(frame1.notes).toBeUndefined()
  expect(frame1.caption).toBe('欢迎：这是演示的第一页。')
  const vp1 = (await api.read(ALICE, copy, 'viewport1')).content.payload
  expect(vp1.notes).toBeUndefined()
  expect(vp1.caption).toBe('在这里调整生成参数')
  expect((await api.rpc(ALICE, 'ws.get_user_state', { workspace_id: copy })).entries['guide:path-guide']).toBeUndefined()
})

test('P3-11 the guide: the rest is dimmed and inert, it ends where it is, resumes, and is offered once', async ({ page, api }) => {
  const ws = await showWorkspace(api, `p3-11 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await expect(page.getByTestId('aiws-notice').filter({ hasText: '使用引导「首次使用」' })).toBeVisible()
  await page.getByTestId('aiws-guide-start').click()
  await expect(page.getByTestId('aiws-guide')).toHaveAttribute('data-step', 'g1')
  await expect(page.getByTestId('aiws-guide-caption')).toHaveText('欢迎：这是演示的第一页。')
  // everything but the step is dimmed; the step (a 16:9 Frame) is fitted into the canvas
  await expect(page.getByTestId('aiws-guide-shade')).toHaveAttribute('data-hole', /^\d+,\d+,\d+,\d+$/)
  const [, , hw, hh] = (await page.getByTestId('aiws-guide-shade').getAttribute('data-hole'))!.split(',').map(Number)
  expect(Math.abs(hw / hh - 16 / 9)).toBeLessThan(0.02)
  await shot(page, 'p3-11-guide')
  const head = await api.headSeq(ALICE, ws.workspace_id)
  // the step was fitted; wait for the camera to settle there
  const settled = page.getByTestId('aiws-canvas')
  await expect.poll(async () => { const a = await settled.getAttribute('data-zoom'); await page.waitForTimeout(300); return a === await settled.getAttribute('data-zoom') ? a : null }).not.toBeNull()
  const zoom = await settled.getAttribute('data-zoom')
  // nothing on the canvas takes input: no drag, no wheel, no shortcut
  await page.mouse.move(700, 500)
  await page.mouse.down()
  await page.mouse.move(500, 300, { steps: 5 })
  await page.mouse.up()
  await page.mouse.wheel(0, 400)
  await page.keyboard.press('Delete')
  await page.waitForTimeout(400)
  expect(await page.getByTestId('aiws-canvas').getAttribute('data-zoom')).toBe(zoom)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head)
  await page.keyboard.press('ArrowRight')
  await expect(page.getByTestId('aiws-guide')).toHaveAttribute('data-step', 'g2')
  await expect(page.getByTestId('aiws-guide-caption')).toHaveText('在这里调整生成参数')
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('aiws-guide')).toHaveCount(0)
  await expect.poll(async () => (await api.rpc(ALICE, 'ws.get_user_state', { workspace_id: ws.workspace_id })).entries['guide:path-guide']?.index).toBe(1)
  // it resumes where it ended
  await page.getByTestId('aiws-guide-start').click()
  await expect(page.getByTestId('aiws-guide')).toHaveAttribute('data-step', 'g2')
  await page.getByTestId('aiws-guide-next').click()
  await expect(page.getByTestId('aiws-guide-count')).toHaveText('3 / 3')
  await page.getByTestId('aiws-guide-done').click()
  await expect(page.getByTestId('aiws-guide')).toHaveCount(0)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head)
  // offered once: not again on the next open
  await openCanvas(page, ALICE, ws.workspace_id)
  await page.waitForTimeout(1000)
  await expect(page.getByTestId('aiws-notice').filter({ hasText: '使用引导「首次使用」' })).toHaveCount(0)
})

test('P3-12 the show lock: other writes are refused and name the presenter; a crashed stage loses it; the owner can end a show', async ({ page, browser, api }) => {
  const ws = await showWorkspace(api, `p3-12 ${Date.now()}`, [{ subject: 'bob', capabilities: ['read', 'update', 'structure', 'comment'] }])
  // bob's window
  const bobContext = await browser.newContext({ viewport: { width: 1600, height: 1000 } })
  const bobPage = await bobContext.newPage()
  await openCanvas(bobPage, BOB, ws.workspace_id)
  // alice presents
  await openCanvas(page, ALICE, ws.workspace_id)
  await startShow(page)
  await onStep(page, 's1')
  const show = await activeShow(page, ws.workspace_id)
  // bob writes through the API: refused, with the presenter
  const read = await api.read(BOB, ws.workspace_id, 'shape-cover')
  const refused = await api.commit(BOB, ws, [{ op: 'entity.set_keys', entity_id: 'shape-cover', keys: [{ key: 'title', value: 'x', expect: { rev: read.content.key_revs.title } }] }])
  expect(refused.status).toBe('rejected')
  expect(refused.code).toBe('SHOW_LOCKED')
  expect(refused.errors[0].data.presenter).toBe('alice')
  // bob's window learns it from its change stream and goes read-only
  await expect(bobPage.getByTestId('aiws-show-locked')).toBeVisible({ timeout: 40_000 })
  await expect(bobPage.getByTestId('aiws-show-locked')).toHaveAttribute('data-presenter', 'alice')
  await bobPage.screenshot({ path: `${SHOTS}/p3-12-locked.png` })
  // the stage crashes (the tab goes away without ending the show): the lease runs out, the clone goes
  await page.close()
  await expect.poll(async () => (await api.rpc(BOB, 'ws.get_info', { workspace_id: ws.workspace_id })).show_lock, { timeout: 20_000 }).toBeNull()
  if (show.start.clone_workspace_id) expect((await api.rpc(ALICE, 'ws.get_info', { workspace_id: show.start.clone_workspace_id })).error?.code).toBe('NOT_FOUND')
  await expect(bobPage.getByTestId('aiws-show-locked')).toHaveCount(0, { timeout: 40_000 })
  // bob presents; alice (the owner) ends it from her window's alert
  const bobShow = await api.rpc(BOB, 'show.start', { workspace_id: ws.workspace_id, path_id: 'path-intro', live: false })
  expect(bobShow.locked).toBe(true)
  const alicePage = await page.context().newPage()
  await openCanvas(alicePage, ALICE, ws.workspace_id)
  await expect(alicePage.getByTestId('aiws-show-locked')).toBeVisible({ timeout: 40_000 })
  await alicePage.getByTestId('aiws-show-force-end').click()
  await expect.poll(async () => (await api.rpc(ALICE, 'ws.get_info', { workspace_id: ws.workspace_id })).show_lock).toBeNull()
  await expect(alicePage.getByTestId('aiws-show-locked')).toHaveCount(0)
  await bobContext.close()
})

test('P3-13 an operable Block takes input on stage; the writes stay in the clone, which is gone afterwards', async ({ page, api }) => {
  const ws = await showWorkspace(api, `p3-13 ${Date.now()}`)
  const head = await api.headSeq(ALICE, ws.workspace_id)
  await openCanvas(page, ALICE, ws.workspace_id)
  await startShow(page, '4. 参数区特写')
  await onStep(page, 's4')
  await expect(page.getByTestId('aiws-show')).toHaveAttribute('data-live', 'true')
  const show = await activeShow(page, ws.workspace_id)
  const clone = show.start.clone_workspace_id!
  expect(clone).toBeTruthy()
  // a Block that is not operable stays static on a double-click
  const note = page.getByTestId('aiws-canvas-block-note-live')
  await note.dblclick()
  await expect(note.locator('textarea')).toBeVisible()
  await note.locator('textarea').fill('现场写的')
  // keys go to the Block while it is active: an arrow key does not turn the page
  await note.locator('textarea').press('ArrowRight')
  await expect(page.getByTestId('aiws-show')).toHaveAttribute('data-step', 's4')
  await note.locator('textarea').press('Escape')
  await expect.poll(async () => (await api.read(ALICE, clone, 'd-note-live')).content.payload.body, { timeout: 15_000 }).toBe('现场写的')
  expect((await api.read(ALICE, ws.workspace_id, 'd-note-live')).content.payload.body).toBe('放映时可以在这里写')
  await page.keyboard.press('Escape')
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('aiws-show')).toHaveCount(0)
  await expect.poll(async () => (await api.rpc(ALICE, 'ws.get_info', { workspace_id: clone })).error?.code).toBe('NOT_FOUND')
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head)
})

test('P3-14 a read-only show: turning pages, zoom, pointer and ink write nothing and leave the editing view as it was', async ({ page, api }) => {
  const ws = await showWorkspace(api, `p3-14 ${Date.now()}`)
  await setKey(api, ws, 'note-live', 'presentation', null)
  await openCanvas(page, ALICE, ws.workspace_id)
  await page.waitForTimeout(1200)
  const stateBefore = (await api.rpc(ALICE, 'ws.get_user_state', { workspace_id: ws.workspace_id })).entries
  const head = await api.headSeq(ALICE, ws.workspace_id)
  await startShow(page)
  await onStep(page, 's1')
  await expect(page.getByTestId('aiws-show')).not.toHaveAttribute('data-live', 'true')
  await expect(page.getByTestId('aiws-show-status')).toHaveText('只读放映 · 已关闭写入')
  await page.keyboard.press('ArrowRight')
  await onStep(page, 's2')
  await page.mouse.wheel(0, -300)
  await page.getByTestId('aiws-show-back').click()
  await page.getByTestId('aiws-show-pointer').click()
  await page.mouse.move(700, 400)
  await page.mouse.move(760, 430, { steps: 4 })
  await page.getByTestId('aiws-show-pen').click()
  await page.mouse.move(600, 500)
  await page.mouse.down()
  await page.mouse.move(700, 560, { steps: 6 })
  await page.mouse.up()
  await expect(page.getByTestId('aiws-stage-ink')).toHaveAttribute('data-strokes', '1')
  await shot(page, 'p3-14-ink')
  await page.getByTestId('aiws-show-ink-undo').click()
  await expect(page.getByTestId('aiws-stage-ink')).toHaveAttribute('data-strokes', '0')
  await page.getByTestId('aiws-show-exit').click()
  await expect(page.getByTestId('aiws-show')).toHaveCount(0)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head)
  await page.waitForTimeout(1200)
  const stateAfter = (await api.rpc(ALICE, 'ws.get_user_state', { workspace_id: ws.workspace_id })).entries
  for (const key of ['canvas:mode', 'surface:active', 'viewport:s-launch']) expect(stateAfter[key], key).toEqual(stateBefore[key])
})

test('P3-15 the prompter link: no login, notes, page turns, several prompters, a reloaded stage, dead after the show', async ({ page, browser, api }) => {
  const ws = await showWorkspace(api, `p3-15 ${Date.now()}`)
  // the stage runs in a tab of its own (its address survives a reload)
  await page.addInitScript((value) => window.localStorage.setItem('aiworkspace.dev', JSON.stringify({ token: value })), ALICE)
  await page.goto(`/workspace/${ws.workspace_id}`)
  await expect(page.getByTestId('aiws-workspace')).toBeVisible({ timeout: 30_000 })
  await startShow(page)
  await onStep(page, 's1')
  await page.getByTestId('aiws-show-link').click()
  const link = await page.getByTestId('aiws-show-link-input').inputValue()
  expect(link).toMatch(/\/workspace\/.+\/show\/sh_.+#k=pt_/)
  await page.keyboard.press('Escape')
  // a phone that is not signed in
  const phone = await browser.newContext({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true })
  const prompter = await phone.newPage()
  await prompter.goto(link.replace(/^https?:\/\/[^/]+/, ''))
  await expect(prompter.getByTestId('aiws-prompter-title')).toHaveText('封面')
  await expect(prompter.getByTestId('aiws-prompter-notes')).toContainText('开场')
  await expect(prompter.getByTestId('aiws-prompter-next')).toHaveText('下一步：问题')
  await expect(prompter.getByTestId('aiws-prompter-stage')).toHaveCount(0)
  await prompter.screenshot({ path: `${SHOTS}/p3-15-prompter.png` })
  await prompter.getByTestId('aiws-prompter-next-button').click()
  await onStep(page, 's2')
  await expect(prompter.getByTestId('aiws-prompter-title')).toHaveText('问题')
  // a second prompter, on a large screen of a signed-in browser: the picture behind the panel
  const desk = await browser.newContext({ viewport: { width: 1600, height: 1000 } })
  await desk.addInitScript((value) => window.localStorage.setItem('aiworkspace.dev', JSON.stringify({ token: value })), ALICE)
  const second = await desk.newPage()
  await second.goto(link.replace(/^https?:\/\/[^/]+/, ''))
  await expect(second.getByTestId('aiws-prompter-title')).toHaveText('问题')
  await expect(second.getByTestId('aiws-prompter-stage')).toBeVisible({ timeout: 30_000 })
  await second.screenshot({ path: `${SHOTS}/p3-15-prompter-desk.png` })
  await second.getByTestId('aiws-prompter-black').click()
  await expect(page.getByTestId('aiws-stage-black')).toBeVisible()
  await second.getByTestId('aiws-prompter-black').click()
  await expect(page.getByTestId('aiws-stage-black')).toHaveCount(0)
  await second.close()
  // the stage reloads and goes on with the same show
  await page.reload()
  await expect(page.getByTestId('aiws-show')).toBeVisible({ timeout: 30_000 })
  await expect(page.getByTestId('aiws-show')).toHaveAttribute('data-step', 's2')
  await prompter.getByTestId('aiws-prompter-next-button').click()
  await onStep(page, 's3')
  // the end of the show kills the link
  await page.getByTestId('aiws-show-exit').click()
  await expect(prompter.getByTestId('aiws-prompter-ended')).toBeVisible({ timeout: 40_000 })
  await phone.close()
  await desk.close()
})

test('P3-16 keys: PageDown / PageUp / B / Home / End drive the stage', async ({ page, api }) => {
  const ws = await showWorkspace(api, `p3-16 ${Date.now()}`)
  await setKey(api, ws, 'note-live', 'presentation', null)
  await openCanvas(page, ALICE, ws.workspace_id)
  await startShow(page)
  await onStep(page, 's1')
  await page.keyboard.press('PageDown')
  await onStep(page, 's2')
  await page.keyboard.press('PageUp')
  await onStep(page, 's1')
  await page.keyboard.press('b')
  await expect(page.getByTestId('aiws-stage-black')).toBeVisible()
  await page.keyboard.press('.')
  await expect(page.getByTestId('aiws-stage-black')).toHaveCount(0)
  await page.keyboard.press('End')
  await onStep(page, 's5')
  await page.keyboard.press('Home')
  await onStep(page, 's1')
})

test('P3-17 with reduced motion a flight becomes a fade', async ({ page, api }) => {
  const ws = await showWorkspace(api, `p3-17 ${Date.now()}`)
  await setKey(api, ws, 'note-live', 'presentation', null)
  await page.emulateMedia({ reducedMotion: 'reduce' })
  await openCanvas(page, ALICE, ws.workspace_id)
  await startShow(page, '2. 问题')
  await onStep(page, 's2')
  await page.keyboard.press('ArrowRight')
  await onStep(page, 's3')
  expect(await page.evaluate(() => window.__aiwsTestHooks!.stage!.transitions.at(-1)!.kind)).toBe('fade')
})

test('P3 dark theme: the path editor, the start dialog, the stage controls, the guide and the prompter read on the dark theme', async ({ page, browser, api }) => {
  const ws = await showWorkspace(api, `p3-dark ${Date.now()}`)
  await page.addInitScript(() => { window.localStorage.setItem('buckyos.prototype.theme.v1', 'dark') })
  await openCanvas(page, ALICE, ws.workspace_id)
  await setCanvasMode(page, 'presentation_edit')
  await page.getByTestId('aiws-path-step-s1').click()
  await expect(page.getByTestId('aiws-step-props')).toBeVisible()
  await shot(page, 'dark-editor')
  await mainMenu(page, 'aiws-top-play')
  await expect(page.getByTestId('aiws-start-show')).toBeVisible()
  await shot(page, 'dark-start')
  await page.getByTestId('aiws-start-show-go').click()
  await onStep(page, 's1')
  await page.getByTestId('aiws-show-notes').click()
  await shot(page, 'dark-stage')
  await page.getByTestId('aiws-show-link').click()
  const link = await page.getByTestId('aiws-show-link-input').inputValue()
  const phone = await browser.newContext({ viewport: { width: 390, height: 844 }, colorScheme: 'dark' })
  await phone.addInitScript(() => { window.localStorage.setItem('buckyos.prototype.theme.v1', 'dark') })
  const prompter = await phone.newPage()
  await prompter.goto(link.replace(/^https?:\/\/[^/]+/, ''))
  await expect(prompter.getByTestId('aiws-prompter-title')).toHaveText('封面')
  await prompter.screenshot({ path: `${SHOTS}/dark-prompter.png` })
  await phone.close()
  await page.getByTestId('aiws-show-exit').click()
  await setCanvasMode(page, 'view')
  await page.getByTestId('aiws-guide-start').click()
  await expect(page.getByTestId('aiws-guide')).toBeVisible()
  await shot(page, 'dark-guide')
})
