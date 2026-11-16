/* The Agent's panel and the data source request (notepads/aiws-ai-native-remaining-todo.md C2-1, B4-6) against the
 * real backend, MessageHub on its mock (the Desktop's mock runtime): the panel opens the one conversation with the
 * viewer's Agent marked as started in the AI Workspace (`ui.source = aiworkspace`) and hands over the workspace, the
 * canvas and the selection with the next message — once; on a phone it is a sheet from the bottom. A collaborator's
 * request reaches the owner as a message, the owner's own goes to their Agent. */

import { devices, type Page } from '@playwright/test'
import { expect, mainMenu, openWorkspacePage, test, wishCard, type Api } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'
/** The MessageHub mock: the signed-in viewer and their own Agent ("CodeAssistant"). */
const SELF = 'did:buckyos:user:self'
const CODER = 'did:buckyos:agent:codeassistant'
const OWN = { viewerDid: SELF, ownerDid: SELF, mode: 'self' as const }
const MARKS = 'buckyos.conversation.mock.sources.v1'

interface HubMock {
  initialize(): Promise<void>
  sessions(context: typeof OWN, entityId?: string, lifecycle?: string): Array<{ id: string }>
  reader(context: typeof OWN, sessionId: string): { timeline?: Array<{ from: string; content: { content?: string } }> }
}
type MockWindow = { __messageHubMock: HubMock }

/** What the viewer sent in a conversation, oldest first. */
const sent = (page: Page, session: string) =>
  page.evaluate(([context, s, self]) => ((window as unknown as MockWindow).__messageHubMock.reader(context, s).timeline ?? []).filter((m) => m.from === self).map((m) => m.content.content ?? ''), [OWN, session, SELF] as const)
/** The viewer's sessions marked as started in the AI Workspace. */
const marked = (page: Page) => page.evaluate((key) => Object.entries(JSON.parse(window.localStorage.getItem(key) ?? '{}') as Record<string, string>).filter(([, source]) => source === 'aiworkspace').map(([entry]) => entry.split('\n')[1]), MARKS)
const sessionsWith = (page: Page, entity: string) =>
  page.evaluate(async ([context, e]) => { const hub = (window as unknown as MockWindow).__messageHubMock; await hub.initialize(); return hub.sessions(context, e, 'active').map((s) => s.id) }, [OWN, entity] as const)

/** A free canvas "Free" with a wish (`wish-j`, its Block `blk-wish`) and a shape (`blk-shape`). */
async function canvasWithWish(api: Api, title: string) {
  const ws = await api.freeCanvas(ALICE, title)
  const made = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'wish-j', type_id: 'buckyos.wish', parent_id: 'free-content', order_key: 'a', name: '月度汇总', payload: {
      title: '月度汇总', prompt: '按月汇总销售额', executor: 'xllm', output_mode: 'overwrite', output: { container_id: 'free-content', surface_id: 'free', name: '月度汇总结果' } } },
    { op: 'entity.create', entity_id: 'blk-wish', type_id: 'buckyos.cell', parent_id: 'free', order_key: 'a', placement: { x: 40, y: 40, w: 420, h: 300 }, payload: { view: { type: 'wish' }, source_ref: { entity_id: 'wish-j' } } },
    { op: 'entity.create', entity_id: 'blk-shape', type_id: 'buckyos.cell', parent_id: 'free', order_key: 'b', placement: { x: 520, y: 40, w: 200, h: 140 }, payload: { view: { type: 'shape' }, config: { shape: 'rect', fill: '#fff2cc' } } },
  ])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  return ws
}

async function open(page: Page, token: string, workspaceId: string) {
  await openWorkspacePage(page, token, workspaceId, 'free')
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
}

const composer = (page: Page) => page.getByTestId('aiws-jarvis-conversation').getByTestId('message-composer').getByRole('textbox')

test('C2-1 the Agent panel opens beside the canvas in its own conversation and hands the selection over once', async ({ page, api }) => {
  const ws = await canvasWithWish(api, `jarvis ${Date.now()}`)
  await open(page, ALICE, ws.workspace_id)
  await wishCard(page, 'wish-j').getByTestId('aiws-wish-title').click()
  await page.getByTestId('aiws-view-toolbar').getByTestId('aiws-collab-jarvis').click()

  const panel = page.getByTestId('aiws-jarvis')
  await expect(panel).toHaveAttribute('data-layout', 'side')
  await expect(panel.getByTestId('aiws-jarvis-conversation')).toBeVisible({ timeout: 20_000 })
  await expect(panel.getByRole('heading')).toContainText('CodeAssistant')
  // at the side of the window, its whole height
  const viewport = page.viewportSize()!
  const box = (await panel.boundingBox())!
  expect(box.x + box.width).toBeGreaterThanOrEqual(viewport.width - 2)
  expect(box.width).toBeLessThanOrEqual(422)
  expect(box.height).toBeGreaterThanOrEqual(viewport.height - 2)
  // one conversation with the Agent, marked as the AI Workspace's
  const session = (await panel.getAttribute('data-session'))!
  expect(await marked(page)).toEqual([session])
  expect(await sessionsWith(page, CODER)).toContain(session)

  // what goes along: the canvas and the selected wish
  const chip = panel.getByTestId('aiws-jarvis-context')
  await expect(chip).toContainText('Free')
  await expect(chip).toContainText('选中 1 项')
  await page.screenshot({ path: 'test-results/aiworkspace-jarvis/panel.png' })
  await composer(page).fill('帮我分析这个许愿格')
  await composer(page).press('Enter')
  await expect.poll(() => sent(page, session).then((list) => list.length)).toBe(1)
  const [first] = await sent(page, session)
  expect(first.startsWith('帮我分析这个许愿格\n\n[aiworkspace context]\n'), first).toBe(true)
  expect(first).toMatch(new RegExp(`\\nworkspace: jarvis \\d+ obj://aiworkspace/${ws.workspace_id}\\n`))
  expect(first).toContain(`canvas: Free (surface) obj://aiworkspace/${ws.workspace_id}/entities/free`)
  expect(first).toMatch(new RegExp(`selected: .+ \\(wish block\\) obj://aiworkspace/${ws.workspace_id}/entities/blk-wish .+; shows 月度汇总 \\(buckyos\\.wish\\) obj://aiworkspace/${ws.workspace_id}/entities/wish-j`))
  await expect(chip).toHaveCount(0)

  // the same place again: the message goes as it is
  await composer(page).fill('还有一件事')
  await composer(page).press('Enter')
  await expect.poll(() => sent(page, session).then((list) => list.at(-1))).toBe('还有一件事')

  // another selection is offered again; put aside, it stays out
  await page.getByTestId('aiws-canvas-block-blk-shape').click()
  await expect(chip).toBeVisible()
  await chip.getByTestId('aiws-jarvis-context-dismiss').click()
  await expect(chip).toHaveCount(0)
  await composer(page).fill('第三条')
  await composer(page).press('Enter')
  await expect.poll(() => sent(page, session).then((list) => list.at(-1))).toBe('第三条')

  // closed and opened again from the main menu: the same conversation
  await panel.getByTestId('aiws-jarvis-close').click()
  await expect(panel).toHaveCount(0)
  await mainMenu(page, 'aiws-menu-collab', 'aiws-menu-jarvis')
  await expect(page.getByTestId('aiws-jarvis')).toHaveAttribute('data-session', session)
  expect(await marked(page)).toEqual([session])
})

test('B4-6 a collaborator asks the owner for a data source: a message to the owner with the wish and the workspace', async ({ page, api }) => {
  const ws = await canvasWithWish(api, `request ${Date.now()}`)
  expect((await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: 'bob', allow: ['read', 'update', 'comment'] }] })).ok).toBe(true)
  await open(page, BOB, ws.workspace_id)
  await mainMenu(page, 'aiws-menu-collab', 'aiws-menu-request-data')
  const dialog = page.getByTestId('aiws-source-request')
  await expect(dialog.getByTestId('aiws-source-request-to')).toHaveAttribute('data-to', 'owner')
  await expect(dialog.getByTestId('aiws-source-request-to')).toContainText('alice')
  await expect(dialog.getByTestId('aiws-source-request-send')).toBeDisabled()
  await dialog.getByTestId('aiws-source-request-meaning').fill('自选股每天的收盘价')
  await dialog.getByTestId('aiws-source-request-origin').fill('交易所网站')
  await dialog.getByTestId('aiws-source-request-freshness').fill('每个交易日')
  await dialog.getByTestId('aiws-source-request-send').click()
  await expect(dialog).toHaveCount(0)
  const [session] = await sessionsWith(page, 'did:bns:alice')
  expect(session).toBeTruthy()
  const [message] = await sent(page, session)
  expect(message).toContain('数据：自选股每天的收盘价')
  expect(message).toContain('来源：交易所网站')
  expect(message).toContain('时效：每个交易日')
  expect(message).toContain(`工作区：obj://aiworkspace/${ws.workspace_id}`)
})

test('B4-6 the owner hands a data source request to their Agent: it lands in the panel’s conversation', async ({ page, api }) => {
  const ws = await canvasWithWish(api, `request owner ${Date.now()}`)
  await open(page, ALICE, ws.workspace_id)
  await wishCard(page, 'wish-j').getByTestId('aiws-wish-title').click()
  await mainMenu(page, 'aiws-menu-collab', 'aiws-menu-request-data')
  const dialog = page.getByTestId('aiws-source-request')
  await expect(dialog.getByTestId('aiws-source-request-to')).toHaveAttribute('data-to', 'agent')
  await dialog.getByTestId('aiws-source-request-meaning').fill('汇率')
  await dialog.getByTestId('aiws-source-request-send').click()
  await expect(dialog).toHaveCount(0)
  const panel = page.getByTestId('aiws-jarvis')
  await expect(panel.getByTestId('aiws-jarvis-conversation')).toBeVisible({ timeout: 20_000 })
  const session = (await panel.getAttribute('data-session'))!
  await expect.poll(() => sent(page, session).then((list) => list.at(-1) ?? '')).toContain('数据：汇率')
})

test.describe('on a phone', () => {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars -- the browser comes from the project, the rest is the phone
  const { defaultBrowserType, ...pixel } = devices['Pixel 7']
  test.use(pixel)

  test('C2-1 the panel is a sheet from the bottom over the canvas, the request dialog fits the screen', async ({ page, api }) => {
    const ws = await canvasWithWish(api, `jarvis phone ${Date.now()}`)
    await open(page, ALICE, ws.workspace_id)
    await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-phone', 'true')
    await page.getByTestId('aiws-main-menu').tap()
    const menu = page.getByTestId('aiws-main-menu-list')
    await menu.getByTestId('aiws-menu-collab').tap()
    await menu.getByTestId('aiws-menu-jarvis').tap()
    const panel = page.getByTestId('aiws-jarvis')
    await expect(panel).toHaveAttribute('data-layout', 'sheet')
    await expect(panel.getByTestId('aiws-jarvis-conversation')).toBeVisible({ timeout: 20_000 })
    const viewport = page.viewportSize()!
    const box = (await panel.boundingBox())!
    expect(box.x).toBeLessThanOrEqual(1)
    expect(box.width).toBeGreaterThanOrEqual(viewport.width - 2)
    expect(box.y + box.height).toBeGreaterThanOrEqual(viewport.height - 2)
    expect(box.height).toBeLessThan(viewport.height)
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
    // the composer's text is 16px: a phone does not zoom in on it
    expect(await composer(page).evaluate((el) => getComputedStyle(el).fontSize)).toBe('16px')
    await page.screenshot({ path: 'test-results/aiworkspace-jarvis/phone-panel.png' })
    await panel.getByTestId('aiws-jarvis-close').tap()
    await expect(panel).toHaveCount(0)

    await page.getByTestId('aiws-main-menu').tap()
    await menu.getByTestId('aiws-menu-collab').tap()
    await menu.getByTestId('aiws-menu-request-data').tap()
    const dialog = page.locator('.aiws-modal-backdrop > .aiws-dialog').first()
    await expect(dialog).toBeVisible()
    const dialogBox = (await dialog.boundingBox())!
    expect(dialogBox.x).toBeGreaterThanOrEqual(0)
    expect(dialogBox.x + dialogBox.width).toBeLessThanOrEqual(viewport.width + 1)
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
    await page.screenshot({ path: 'test-results/aiworkspace-jarvis/phone-request.png' })
  })
})
