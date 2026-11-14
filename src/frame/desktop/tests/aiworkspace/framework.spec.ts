/* The UI framework (UI 框架 PRD §3.3, §4): actions registered once and shown by the tool zones and the main menu;
 * the main menu's height rules; the collaboration zone's 播放 and 宏 (wishes holding a program, Q27; its runs in the work
 * log, K3); the status bar's panel; the canvas layer of the toolbar configuration. Acceptance scenarios of §15 by
 * number. */

import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import type { Page } from '@playwright/test'
import { core, expect, fitAll, mainMenu, openCanvas, openStatus, test, type Api } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'
const PRESENTATION = resolve('../aiworkspace/fixtures/presentation/commits.json')

/** The presentation fixture (a presentation path `path-intro` and a guide `path-guide`), without the entities named. */
async function showWorkspace(api: Api, title: string, without: string[] = []) {
  const ws = await api.rpc(ALICE, 'ws.create', { title }) as { workspace_id: string; epoch: string }
  const fixture = JSON.parse(readFileSync(PRESENTATION, 'utf8')) as { commits: { operations: Record<string, unknown>[] }[] }
  for (const [index, commit] of fixture.commits.entries()) {
    const operations = commit.operations.filter((op) => !without.includes(op.entity_id as string))
    if (operations.length === 0) continue
    const result = await api.commit(ALICE, ws, operations, `presentation/${index + 1}`)
    expect(result.status, JSON.stringify(result)).toBe('accepted')
  }
  return ws
}

async function openAs(browser: import('@playwright/test').Browser, token: string, workspaceId: string): Promise<{ page: Page; close: () => Promise<void> }> {
  const context = await browser.newContext({ viewport: { width: 1600, height: 1000 } })
  const page = await context.newPage()
  await openCanvas(page, token, workspaceId)
  return { page, close: () => context.close() }
}

test('§15-1 an action registered for the collaboration zone shows there and in the main menu "协作" with one name, icon and reason', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `f01 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  // an extension's action: declared for the collaboration zone and the collaboration group, nothing else
  await page.evaluate(() => {
    const w = window as unknown as { __aiwsTestHooks: { actions: { register(def: object): () => void } }; __pinged?: number; __blocked?: boolean }
    w.__blocked = true
    w.__pinged = 0
    w.__aiwsTestHooks.actions.register({
      id: 'test.ping', icon: 'flask', phone: true, label: () => '测试动作',
      menu: { group: 'collab', order: 99, id: 'test-ping' }, zone: { zone: 'collab', order: 25, testId: 'aiws-collab-test-ping' },
      reason: () => (w.__blocked ? '测试用的不可用原因' : null),
      run: () => { w.__pinged = (w.__pinged ?? 0) + 1 },
    })
  })
  const tool = page.getByTestId('aiws-view-toolbar').getByTestId('aiws-collab-test-ping')
  await expect(tool).toBeVisible()
  await expect(tool).toHaveAttribute('aria-label', '测试动作')
  await expect(tool).toHaveAttribute('data-icon', 'flask')
  await expect(tool).toHaveAttribute('data-action', 'test.ping')
  await expect(tool).toBeDisabled()
  await expect(tool).toHaveAttribute('title', '测试用的不可用原因')
  await mainMenu(page, 'aiws-menu-collab')
  const entry = page.getByTestId('aiws-menu-test-ping')
  await expect(entry).toContainText('测试动作')
  await expect(entry).toBeDisabled()
  await expect(entry).toHaveAttribute('title', '测试用的不可用原因')
  await page.keyboard.press('Escape')
  // available: both entries run the same action
  await page.evaluate(() => { (window as unknown as { __blocked: boolean }).__blocked = false })
  await mainMenu(page, 'aiws-menu-collab', 'aiws-menu-test-ping')
  expect(await page.evaluate(() => (window as unknown as { __pinged: number }).__pinged)).toBe(1)
  await expect(tool).toBeEnabled()
  await tool.click()
  expect(await page.evaluate(() => (window as unknown as { __pinged: number }).__pinged)).toBe(2)
  // the zone keeps its order: play, macro, the new action, comment …
  const order = await page.getByTestId('aiws-view-toolbar').locator('[data-action]').evaluateAll((els) => els.map((el) => el.getAttribute('data-action')))
  expect(order.indexOf('test.ping')).toBeLessThan(order.indexOf('collab.comment'))
  // gone with its registration
  await page.evaluate(() => {
    const w = window as unknown as { __aiwsTestHooks: { actions: { get(id: string): object; register(def: object): () => void } } }
    const def = w.__aiwsTestHooks.actions.get('test.ping')
    w.__aiwsTestHooks.actions.register(def)()
  })
  await expect(tool).toHaveCount(0)
})

test('§15-2 the main menu: groups only on its first level (at most nine, no scrolling in a 480px-high window), at most twelve entries below', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `f02 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-root').evaluate((root) => { root.style.height = '480px'; root.style.flex = 'none' })
  await page.waitForTimeout(300)
  await page.getByTestId('aiws-main-menu').click()
  const list = page.getByTestId('aiws-main-menu-list')
  const rows = list.locator(':scope > .aiws-menu-entry > button')
  const ids = await rows.evaluateAll((els) => els.map((el) => el.getAttribute('data-testid')))
  expect(ids).toEqual(['aiws-menu-workspace', 'aiws-menu-surface', 'aiws-menu-insert', 'aiws-menu-edit', 'aiws-menu-view', 'aiws-menu-collab', 'aiws-menu-data-mode', 'aiws-menu-help'])
  expect(ids.length).toBeLessThanOrEqual(9)
  const root = (await page.getByTestId('aiws-root').boundingBox())!
  const box = (await list.boundingBox())!
  expect(box.y + box.height).toBeLessThanOrEqual(root.y + root.height)
  expect(await list.evaluate((el) => el.scrollHeight <= el.clientHeight + 1)).toBe(true)
  // every group but data mode opens a second level of at most twelve entries
  for (const id of ids.filter((id) => id !== 'aiws-menu-data-mode')) {
    await page.getByTestId(id!).click()
    const sub = list.locator(':scope > .aiws-menu-entry > .aiws-submenu')
    await expect(sub).toBeVisible()
    expect(await sub.locator(':scope > .aiws-menu-entry').count(), id!).toBeLessThanOrEqual(12)
  }
  await page.keyboard.press('Escape')
  // data mode is a row of its own, and leads back
  await mainMenu(page, 'aiws-menu-data-mode')
  await expect(page.getByTestId('aiws-sources')).toBeVisible()
  await mainMenu(page, 'aiws-menu-data-mode')
  await expect(page.getByTestId('aiws-canvas-view')).toBeVisible()
})

test('§4.1.2 in a narrow window the third level of the menu stays reachable (to the left, or moved in over the menus above)', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `f02n ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  for (const width of [768, 390]) {
    await page.getByTestId('aiws-root').evaluate((root, w) => { root.style.width = `${w}px`; root.style.flex = 'none' }, width)
    await page.waitForTimeout(400)
    await mainMenu(page, 'aiws-menu-view', 'aiws-menu-panels', 'aiws-menu-side-annotations')
    await expect(page.locator('[data-testid="aiws-side-panel"][data-tab="annotations"]')).toBeVisible()
    await page.getByTestId('aiws-side-close').click()
    await mainMenu(page, 'aiws-menu-insert', 'aiws-menu-import-file')
    const root = (await page.getByTestId('aiws-root').boundingBox())!
    const pdf = (await page.getByTestId('aiws-menu-import-pdf').boundingBox())!
    expect(pdf.x).toBeGreaterThanOrEqual(root.x)
    expect(pdf.x + pdf.width).toBeLessThanOrEqual(root.x + root.width)
    await page.keyboard.press('Escape')
  }
})

test('§4.3 the tool selection zone: design / run in the pointer group (on a flow page alone); in a low window the object group gives way from the bottom, pointer and history stay', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `f43 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  const bar = page.getByTestId('aiws-object-toolbar')
  const pointerGroup = bar.locator(':scope > .aiws-tool-group').first()
  await expect(pointerGroup.getByTestId('aiws-run-toggle')).toBeVisible()
  await expect(page.getByTestId('aiws-view-toolbar').getByTestId('aiws-run-toggle')).toHaveCount(0)
  await page.getByTestId('aiws-root').evaluate((root) => { root.style.height = '480px'; root.style.flex = 'none' })
  await expect(bar).toHaveAttribute('data-hidden', /^[1-9]/)
  expect(await bar.evaluate((el) => el.scrollHeight - el.clientHeight)).toBeLessThanOrEqual(0)
  for (const id of ['aiws-tool-pointer', 'aiws-run-toggle', 'aiws-tool-pen', 'aiws-tool-catalog', 'aiws-undo', 'aiws-redo']) await expect(bar.getByTestId(id)).toBeVisible()
  await expect(bar.getByTestId('aiws-tool-insert-note')).toHaveCount(0)
  await page.getByTestId('aiws-root').evaluate((root) => { root.style.height = '' })
  await expect(bar.getByTestId('aiws-tool-insert-note')).toBeVisible()
  await expect(bar).not.toHaveAttribute('data-hidden')
})

test('§15-5 播放: one path starts it at once; a path and a guide drop down; no path: path editing for structure, nothing for a reader', async ({ page, api, browser }) => {
  // one presentation path, no guide: the click opens "开始放映" for that path
  const one = await showWorkspace(api, `f05a ${Date.now()}`, ['path-guide'])
  await openCanvas(page, ALICE, one.workspace_id)
  const play = page.getByTestId('aiws-collab-play')
  await expect(play).not.toHaveAttribute('aria-haspopup', 'menu')
  await expect(play).toHaveAttribute('title', '放映：五分钟介绍')
  await play.click()
  await expect(page.getByTestId('aiws-start-show')).toBeVisible()
  await expect(page.getByTestId('aiws-start-path')).toHaveValue('path-intro')
  await page.keyboard.press('Escape')
  // a path and a guide: a drop-down of both, then a public presentation
  const two = await showWorkspace(api, `f05b ${Date.now()}`)
  await openCanvas(page, ALICE, two.workspace_id)
  await expect(play).toHaveAttribute('aria-haspopup', 'menu')
  await play.click()
  const list = page.getByTestId('aiws-collab-play-list')
  await expect(list.getByTestId('aiws-play-path-path-intro')).toBeVisible()
  await expect(list.getByTestId('aiws-play-guide-path-guide')).toBeVisible()
  await expect(list.getByTestId('aiws-play-public')).toBeVisible()
  await list.getByTestId('aiws-play-guide-path-guide').click()
  await expect(page.getByTestId('aiws-guide')).toHaveAttribute('data-step', 'g1')
  await page.keyboard.press('Escape')
  // no path: whoever may structure the canvas is led to path editing, a reader is shown nothing
  const none = await api.demo(ALICE, 'quarterly', `f05c ${Date.now()}`)
  await api.rpc(ALICE, 'acl.set', { workspace_id: none.workspace_id, changes: [{ subject: 'bob', allow: ['read'] }] })
  await openCanvas(page, ALICE, none.workspace_id)
  await expect(play).toHaveAttribute('title', '还没有演示路径：进入路径编辑创建')
  await play.click()
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-mode', 'presentation_edit')
  const bob = await openAs(browser, BOB, none.workspace_id)
  await expect(bob.page.getByTestId('aiws-view-toolbar')).toBeVisible()
  await expect(bob.page.getByTestId('aiws-collab-play')).toHaveCount(0)
  await bob.close()
})

test('§15-6 宏: two macros on the canvas drop down — wishes holding a program, never the Mock executor\'s; running one is the wish panel\'s run, its start and end are in the work log, and it comes first next time', async ({ page, api }) => {
  test.slow()
  // the scripted model (mock-llm.mjs) analyses and codes the real executor's wish in this browser
  const llm = `http://127.0.0.1:${process.env.AIWS_E2E_LLM_PORT}/v1`
  await page.addInitScript((baseUrl) => window.localStorage.setItem('aiworkspace.dev', JSON.stringify({ llm: { baseUrl } })), llm)
  const ws = await api.demo(ALICE, 'quarterly', `f06 ${Date.now()}`)
  const wish = await api.read(ALICE, ws.workspace_id, 'wish-analysis')
  // the demo's wish on the real executor; a Mock wish beside it, on the same canvas
  const outline = await api.outline(ALICE, ws.workspace_id) as { entity_id: string; parent_id?: string; order_key?: string }[]
  const last = (parent: string) => outline.filter((e) => e.parent_id === parent).map((e) => e.order_key ?? '').sort().at(-1)
  const result = await api.commit(ALICE, ws, [
    { op: 'entity.set_keys', entity_id: 'wish-analysis', keys: [
      { key: 'executor', value: 'xllm', expect: { rev: wish.content.key_revs.executor } },
      { key: 'prompt', value: '按区域汇总「原始销售数据」的销售额，并写一段解读', expect: { rev: wish.content.key_revs.prompt } }] },
    { op: 'entity.create', entity_id: 'wish-mock', type_id: 'buckyos.wish', parent_id: 'sf-analysis-content', order_key: core().order_key_between(last('sf-analysis-content'), undefined), name: '模拟的许愿格',
      payload: { title: '模拟的许愿格', prompt: '按产品汇总「原始销售数据」的销售额', executor: 'mock', output_mode: 'overwrite', inputs: [{ entity_id: 'sales', version: { mode: 'follow' } }],
        output: { container_id: 'sf-analysis-content', surface_id: 'sf-analysis', name: '模拟的结果' } } },
    { op: 'entity.create', entity_id: 'blk-wish-mock', type_id: 'buckyos.cell', parent_id: 'sf-analysis', order_key: core().order_key_between(last('sf-analysis'), undefined),
      placement: { x: 1460, y: 190, w: 420, h: 300 }, payload: { view: { type: 'wish' }, source_ref: { entity_id: 'wish-mock' } } },
  ])
  expect(result.status, JSON.stringify(result)).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  // no analysis yet: no macro can run, the entry is not shown
  await expect(page.getByTestId('aiws-view-toolbar')).toBeVisible()
  const macro = page.getByTestId('aiws-collab-macro')
  await expect(macro).toHaveCount(0)
  const panelOf = async (block: string) => { await fitAll(page); await page.getByTestId(`aiws-canvas-block-${block}`).dblclick() }
  // the Mock wish analysed: it can run, but it holds no macro (Q27) — still no entry
  await panelOf('blk-wish-mock')
  await page.getByTestId('aiws-wish-analyze').click()
  await expect(page.getByTestId('aiws-wish-analysis')).toBeVisible({ timeout: 30_000 })
  await expect(page.getByTestId('aiws-wish-main')).toHaveAttribute('data-next', 'run')
  await page.getByTestId('aiws-side-close').click()
  await page.waitForTimeout(1000) // the freshness of both wishes is in
  await expect(macro).toHaveCount(0)
  // the real one: analysed, coded (its macro and trial run), applied — now it holds a macro
  await panelOf('blk-wish')
  await page.getByTestId('aiws-wish-analyze').click()
  await expect(page.getByTestId('aiws-wish-contract')).toContainText('summary', { timeout: 30_000 })
  await expect(page.getByTestId('aiws-wish-main')).toHaveAttribute('data-main', 'needs_coding')
  await page.getByTestId('aiws-wish-next').click()
  await expect(page.getByTestId('aiws-wish-candidate')).toHaveAttribute('data-stage', 'code', { timeout: 60_000 })
  await page.getByTestId('aiws-wish-apply').click()
  await expect(page.getByTestId('aiws-wish-main')).toHaveAttribute('data-main', 'current', { timeout: 30_000 })
  await page.getByTestId('aiws-side-close').click()
  // a second wish holding the same macro (its task contract and program), on the same canvas
  const coded = (await api.read(ALICE, ws.workspace_id, 'wish-analysis')).content.payload
  expect(coded.program.api_version).toBe(3)
  const now = await api.outline(ALICE, ws.workspace_id) as { entity_id: string; parent_id?: string; order_key?: string }[]
  const lastNow = (parent: string) => now.filter((e) => e.parent_id === parent).map((e) => e.order_key ?? '').sort().at(-1)
  const second = await api.commit(ALICE, { workspace_id: ws.workspace_id, epoch: ws.epoch }, [
    { op: 'entity.create', entity_id: 'wish-second', type_id: 'buckyos.wish', parent_id: 'sf-analysis-content', order_key: core().order_key_between(lastNow('sf-analysis-content'), undefined), name: '第二个宏',
      payload: { title: '第二个宏', prompt: coded.prompt, executor: 'xllm', output_mode: 'overwrite', inputs: coded.inputs, analysis: coded.analysis, program: coded.program,
        output: { container_id: 'sf-analysis-content', surface_id: 'sf-analysis', name: '第二个宏的结果' } } },
    { op: 'entity.create', entity_id: 'blk-wish-second', type_id: 'buckyos.cell', parent_id: 'sf-analysis', order_key: core().order_key_between(lastNow('sf-analysis'), undefined),
      placement: { x: 1460, y: 560, w: 420, h: 300 }, payload: { view: { type: 'wish' }, source_ref: { entity_id: 'wish-second' } } },
  ])
  expect(second.status, JSON.stringify(second)).toBe('accepted')
  // two macros: a drop-down of the two, without the Mock wish
  await expect(macro).toHaveAttribute('aria-haspopup', 'menu', { timeout: 15_000 })
  await macro.click()
  const list = page.getByTestId('aiws-collab-macro-list')
  await expect(list.locator('[data-testid^="aiws-macro-"]')).toHaveCount(2)
  await expect(list.getByTestId('aiws-macro-wish-analysis')).toBeVisible()
  await expect(list.getByTestId('aiws-macro-wish-mock')).toHaveCount(0)
  await list.getByTestId('aiws-macro-wish-second').click()
  // the same run as the wish panel's: its candidate waits in the wish; its start and its end are entries of the status
  // bar's work log (K3), not notices
  await expect.poll(async () => ((await api.runs(ALICE, ws.workspace_id, 'wish-second')) as { stage: string }[])[0]?.stage, { timeout: 30_000 }).toBe('run')
  await openStatus(page)
  const log = page.getByTestId('aiws-status-log')
  const entry = (phase: string) => log.locator(`[data-testid="aiws-status-log-entry"][data-kind="run"][data-phase="${phase}"][data-target="wish-second"]`)
  await expect(entry('end')).toBeVisible({ timeout: 30_000 })
  await expect(entry('start')).toBeVisible()
  await expect(entry('start').getByTestId('aiws-status-log-text')).toHaveText('开始运行宏「第二个宏」')
  await expect(page.getByTestId('aiws-notice').filter({ hasText: '第二个宏' })).toHaveCount(0)
  // the one used last comes first, per workspace (user state)
  await macro.click()
  await expect(list.locator('[data-testid^="aiws-macro-"]').first()).toHaveAttribute('data-testid', 'aiws-macro-wish-second')
  await page.keyboard.press('Escape')
  await expect.poll(async () => (await api.rpc(ALICE, 'ws.get_user_state', { workspace_id: ws.workspace_id })).entries['ui:recent-macro']).toBe('wish-second')
  // the main menu lists the same macros under 协作 ▸ 运行宏, the Mock wish not among them
  await mainMenu(page, 'aiws-menu-collab', 'aiws-menu-macros')
  await expect(page.getByTestId('aiws-macro-wish-second')).toBeVisible()
  await expect(page.getByTestId('aiws-macro-wish-mock')).toHaveCount(0)
  await page.keyboard.press('Escape')
})

test('§15-8 the status bar: the online count and the commit state collapsed; in place it opens what needs attention, the connection and the work log, by itself on a new problem; alerts stay; a canvas configuration hides zones, never the alerts', async ({ page, api }) => {
  const ws = await showWorkspace(api, `f08 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  const status = page.getByTestId('aiws-status')
  await expect(page.getByTestId('aiws-save-summary')).toBeVisible()
  // presence (K2): the window is registered from the start; alone, one online
  await expect(page.getByTestId('aiws-status-online')).toHaveText('在线 1', { timeout: 20_000 })
  // the guide offer is a notice above the status bar: not part of the panel, not folded with it
  const offer = page.getByTestId('aiws-notice').filter({ hasText: '使用引导「首次使用」' })
  await expect(offer).toBeVisible()
  await expect(page.getByTestId('aiws-status-detail')).toHaveCount(0)
  await status.click()
  const panel = page.getByTestId('aiws-status-detail')
  await expect(panel).toBeVisible()
  await expect(status).toHaveAttribute('aria-expanded', 'true')
  // nothing needs attention: the connection part, folded, and the work log (K3) with the fixture's Commits
  await expect(page.getByTestId('aiws-status-attention')).toHaveCount(0)
  expect(await page.getByTestId('aiws-status-connection').evaluate((el) => (el as HTMLDetailsElement).open)).toBe(false)
  await expect(page.getByTestId('aiws-status-log')).toBeVisible()
  await expect(page.getByTestId('aiws-status-log-entry').first()).toBeVisible({ timeout: 20_000 })
  // at most half the work area
  const area = (await page.getByTestId('aiws-canvas').boundingBox())!
  expect((await panel.boundingBox())!.height).toBeLessThanOrEqual(area.height / 2 + 1)
  await expect(offer).toBeVisible()
  // open or closed is the person's preference, across Workspaces (K4)
  await expect.poll(async () => (await api.rpc(ALICE, 'user.get_prefs', {})).prefs['status.expanded']).toBe(true)
  await status.click()
  await expect(panel).toHaveCount(0)
  await expect(offer).toBeVisible()
  // a new problem opens it by itself: someone renames the canvas while I do
  await page.getByTestId('aiws-surface-name').click()
  await page.getByTestId('aiws-surface-name-input').fill('我的名字')
  const read = await api.read(ALICE, ws.workspace_id, 's-launch')
  expect((await api.commit(ALICE, ws, [
    { op: 'entity.set_keys', entity_id: 's-launch', keys: [{ key: 'title', value: '别人的名字', expect: { rev: read.content.key_revs.title } }] },
    { op: 'entity.rename', entity_id: 's-launch', name: '别人的名字', expect: { rev: read.meta_rev } },
  ])).status).toBe('accepted')
  await page.keyboard.press('Enter')
  await expect(panel).toBeVisible()
  await expect(page.getByTestId('aiws-status-attention').getByTestId('aiws-edit-entry')).toHaveAttribute('data-state', 'needs_attention')
  await page.keyboard.press('Escape')
  // the canvas layer of the toolbar configuration (§4.6): it hides the collaboration zone and an object of the tool
  // selection zone for everyone; the alerts are not configurable, and a status bar hidden by it still shows while
  // something needs attention
  const surface = await api.read(ALICE, ws.workspace_id, 's-launch')
  expect((await api.commit(ALICE, ws, [{ op: 'entity.set_keys', entity_id: 's-launch', keys: [{ key: 'config', expect: { rev: surface.content.key_revs.config ?? 0 },
    value: { ...(surface.content.payload.config ?? {}), toolbar: { zones: { collab: { hidden: true }, tools: { items: [{ id: 'insert:wish', action: 'block.insert', args: { entry: 'wish' }, hidden: true }] }, status: { hidden: true } } } } }] }])).status).toBe('accepted')
  await expect(page.getByTestId('aiws-view-toolbar')).toHaveCount(0)
  await expect(page.getByTestId('aiws-tool-insert-wish')).toHaveCount(0)
  await expect(page.getByTestId('aiws-tool-insert-note')).toBeVisible()
  await expect(page.getByTestId('aiws-status-dock')).toHaveAttribute('data-zone-hidden', 'true')
  await expect(status).toBeVisible()
  await expect(page.getByTestId('aiws-status-online')).toHaveCount(0)
  await expect(offer).toBeVisible()
})
