import { expect, test, type Page } from '@playwright/test'
import { openFromAllApps } from '../fixtures/desktop'
import { closeSelect, pickLocal, pickZone, selectDialog, selectGoto } from '../fixtures/selectDialog'

/**
 * The Agent browser on the mock Agent Service (src/app/agent-browser/mock): the App speaks the wire format of
 * doc/opendan/protocol/Agent Browser Protocol.md to it, and the specs read what the service received from
 * `window.__agentBrowserMock`, which also plays the Agent's side (lease, release, handover request).
 */

const AGENT = 'codeassistant'
const MOCK_CONTROL_PANEL = 'buckyos.mock.control-panel.v1'

type MockElement = 'input' | 'select' | 'alert' | 'confirm' | 'prompt' | 'upload' | 'download' | 'location' | 'newTab' | 'signIn'
interface MockRect { x: number; y: number; width: number; height: number }
/** A message the mock service received, as far as the specs look at it. */
interface Message {
  [field: string]: unknown
  type: string
  kind?: string
  action?: string
  phase?: string
  x?: number
  y?: number
  button?: string
  url?: string
  viewport?: { width: number; height: number; mobile: boolean }
}
interface Received { agentId: string; groupId: string; dropped: boolean; message: Message }
interface MockControl {
  log: Received[]
  rpc: Array<{ agentId: string; method: string; params: Record<string, unknown> }>
  uploads: Array<{ requestId: string; name: string; source: 'zone' | 'local'; done: boolean }>
  rects(width: number): Record<MockElement, MockRect>
  page(agentId: string, groupId: string): { text: string; composition: string; selected: number[]; dialog: string; files: string[]; scrollY: number; signedIn: boolean; url: string } | null
  agentAcquire(agentId: string, groupId: string): boolean
  agentOpen(agentId: string, groupId: string, url: string): boolean
  agentRelease(agentId: string, groupId: string, state?: 'done' | 'stuck', note?: string): void
  agentRequestHandover(agentId: string, groupId: string, reason: 'password' | 'captcha' | 'other', note?: string): void
  agentAsk(agentId: string, groupId: string, waiting: boolean): void
  setAvailability(agentId: string, patch: { enabled?: boolean; available?: boolean; unavailable?: string | null }): void
  dropConnections(agentId: string): void
}
type MockWindow = { __agentBrowserMock: MockControl }

test.use({ viewport: { width: 1280, height: 900 } })

const received = (page: Page, type: string, group = 'default') =>
  page.evaluate(([t, g]) => (window as unknown as MockWindow).__agentBrowserMock.log.filter((entry) => entry.message.type === t && entry.groupId === g), [type, group] as const)
const inputs = async (page: Page, kind: string) => (await received(page, 'input')).filter((entry) => entry.message.kind === kind)
const rpcCalls = (page: Page, method: string) =>
  page.evaluate((m) => (window as unknown as MockWindow).__agentBrowserMock.rpc.filter((call) => call.method === m), method)
const mockPage = (page: Page, group = 'default') =>
  page.evaluate(([a, g]) => (window as unknown as MockWindow).__agentBrowserMock.page(a, g), [AGENT, group] as const)
/** The Agent's side, on the default group: `agentAcquire`, `agentRelease(state, note)`, `agentRequestHandover(reason, note)`, `agentAsk(waiting)`. */
const agentSide = (page: Page, method: 'agentAcquire' | 'agentRelease' | 'agentRequestHandover' | 'agentAsk', ...args: unknown[]) =>
  page.evaluate(([m, list]) => ((window as unknown as MockWindow).__agentBrowserMock[m] as (...values: unknown[]) => unknown)(...list), [method, [AGENT, 'default', ...args]] as const)

/** Opens the App and, with several Agents, stops at the question whose browser to use. */
async function openApp(page: Page) {
  await page.goto('/?scenario=normal')
  await openFromAllApps(page, 'agent-browser')
  await expect(page.getByTestId('agent-browser')).toBeVisible()
}

async function enterAgent(page: Page, agentId = AGENT) {
  await page.getByTestId(`ab-agent-${agentId}`).click()
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-agent', agentId)
  await expect(page.getByTestId('ab-screen')).toHaveAttribute('data-status', 'open')
}

async function browse(page: Page, address: string) {
  const input = page.getByTestId('ab-address-input')
  await input.click()
  await input.fill(address)
  await input.press('Enter')
  await expect(page.getByTestId('ab-screen-canvas')).toHaveAttribute('data-seq', /\d+/)
  await expect(page.getByTestId('ab-screen-canvas')).toBeVisible()
}

async function openBrowser(page: Page, address = 'example.com') {
  await openApp(page)
  await enterAgent(page)
  await browse(page, address)
}

/** Where an element of the mock page is: on the screen (to press) and in the page's own pixels (what the service must receive). */
async function elementAt(page: Page, element: MockElement) {
  const viewport = await page.getByTestId('ab-screen-surface').getAttribute('data-viewport')
  const width = Number(viewport?.split('x')[0])
  const box = await page.getByTestId('ab-screen-canvas').boundingBox()
  if (!box || !width) throw new Error('the screen shows no frame')
  const rect = await page.evaluate(([w, name]) => (window as unknown as MockWindow).__agentBrowserMock.rects(w)[name], [width, element] as const)
  const at = { x: Math.round(rect.x + rect.width / 2), y: Math.round(rect.y + rect.height / 2) }
  const scale = box.width / width
  return { screen: { x: box.x + at.x * scale, y: box.y + at.y * scale }, page: at }
}

async function press(page: Page, element: MockElement) {
  const at = await elementAt(page, element)
  await page.mouse.click(at.screen.x, at.screen.y)
  return at
}

test('opens from All apps, asks whose browser, and shows the page of the address entered', async ({ page }) => {
  const errors: string[] = []
  page.on('console', (message) => { if (message.type() === 'error') errors.push(message.text()) })
  await openApp(page)
  const picker = page.getByTestId('ab-agent-picker')
  await expect(picker.getByRole('button')).toHaveCount(2)
  await expect(picker.getByTestId('ab-agent-assistant.alice')).toContainText('BuckyOS Assistant')
  await enterAgent(page)

  // The default entry: the default thing, original page mode, an empty address, no tab yet.
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-group', 'default')
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'page')
  await expect(page.getByTestId('ab-thing-title')).toHaveText('General browsing')
  await expect(page.getByTestId('ab-address-input')).toHaveValue('')
  await expect(page.getByTestId('ab-screen-empty')).toContainText('CodeAssistant')
  await expect(page.getByTestId('ab-status')).toHaveAttribute('data-controller', 'user')
  const hello = await received(page, 'hello')
  expect(hello).toHaveLength(1)
  expect(hello[0].message).toMatchObject({ token: 'mock-session-token', group_id: 'default', viewport: { mobile: false } })
  expect(await rpcCalls(page, 'browser.ensure')).toHaveLength(1)

  await browse(page, 'example.com')
  await expect.poll(async () => (await received(page, 'tab')).map((entry) => entry.message)).toEqual([{ type: 'tab', action: 'open', url: 'https://example.com' }])
  await expect(page.getByTestId('ab-address-input')).toHaveValue('example.com')
  await expect(page.getByTestId('ab-tabs-count')).toHaveText('1')
  // Frames keep coming and each one is acknowledged: the service only sends the next one after the `ack`.
  const first = Number(await page.getByTestId('ab-screen-canvas').getAttribute('data-seq'))
  await expect.poll(async () => Number(await page.getByTestId('ab-screen-canvas').getAttribute('data-seq'))).toBeGreaterThan(first)

  // An address in the same tab is a navigation; back and forward follow the history.
  await browse(page, 'https://news.example.org/today')
  await expect.poll(async () => (await received(page, 'nav')).at(-1)?.message).toEqual({ type: 'nav', action: 'url', url: 'https://news.example.org/today' })
  await expect(page.getByTestId('ab-address-input')).toHaveValue('news.example.org/today')
  await page.getByTestId('ab-back').click()
  await expect(page.getByTestId('ab-address-input')).toHaveValue('example.com')
  await page.getByTestId('ab-forward').click()
  await expect(page.getByTestId('ab-address-input')).toHaveValue('news.example.org/today')
  await expect(page.getByTestId('ab-reload')).toHaveAccessibleName('Reload')
  await page.getByTestId('ab-reload').click()
  await expect.poll(async () => (await received(page, 'nav')).map((entry) => entry.message.action)).toEqual(['url', 'back', 'forward', 'reload'])
  // Words that are not an address become a search.
  await browse(page, 'weather in kyoto')
  await expect.poll(async () => (await received(page, 'nav')).at(-1)?.message.url).toBe('https://www.bing.com/search?q=weather%20in%20kyoto')
  expect(errors).toEqual([])
})

test('a single Agent opens at once, and the Agent last used is remembered', async ({ page }) => {
  await page.addInitScript(([key]) => {
    if (!window.sessionStorage.getItem('ab-seeded')) {
      window.localStorage.setItem(key, JSON.stringify({ agents: [] }))
      window.sessionStorage.setItem('ab-seeded', '1')
    }
  }, [MOCK_CONTROL_PANEL] as const)
  await openApp(page)
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-agent', AGENT)
  await expect(page.getByTestId('ab-agent-picker')).toHaveCount(0)
  // With one Agent there is nothing to switch to.
  await page.getByTestId('ab-thing-button').click()
  await expect(page.getByTestId('ab-things-agent')).toContainText('CodeAssistant')
  await expect(page.getByTestId('ab-switch-agent')).toHaveCount(0)

  // Two Agents again: the one used last opens without asking, and can be switched.
  await page.evaluate((key) => window.localStorage.removeItem(key), MOCK_CONTROL_PANEL)
  await openApp(page)
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-agent', AGENT)
  await page.getByTestId('ab-thing-button').click()
  await page.getByTestId('ab-switch-agent').click()
  await page.getByTestId('ab-agent-assistant.alice').click()
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-agent', 'assistant.alice')
  await expect(page.getByTestId('ab-agent-name')).toHaveText('BuckyOS Assistant')
})

test('pointer, wheel and keyboard reach the page in the page’s own pixels', async ({ page }) => {
  await openBrowser(page)
  const field = await press(page, 'input')
  await expect.poll(async () => (await inputs(page, 'mouse')).filter((entry) => entry.message.action !== 'move').map((entry) => entry.message)).toEqual([
    { type: 'input', kind: 'mouse', action: 'down', ...field.page, button: 'left', click_count: 1, modifiers: 0 },
    { type: 'input', kind: 'mouse', action: 'up', ...field.page, button: 'left', click_count: 1, modifiers: 0 },
  ])
  await expect(page.getByTestId('ab-screen-input')).toBeFocused()

  // Keys of a hardware keyboard go as `key`, a printable one with its text.
  await page.keyboard.type('Hi')
  await page.keyboard.press('Backspace')
  await page.keyboard.press('Shift+Tab')
  await page.keyboard.press('Control+a')
  await expect.poll(async () => (await inputs(page, 'key')).map((entry) => `${entry.message.action}:${entry.message.key}:${entry.message.text}:${entry.message.modifiers}`)).toEqual([
    'down:H:H:0', 'up:H::0', 'down:i:i:0', 'up:i::0', 'down:Backspace::0', 'up:Backspace::0',
    'down:Shift::8', 'down:Tab::8', 'up:Tab::8', 'up:Shift::0',
    'down:Control::2', 'down:a::2', 'up:a::2', 'up:Control::0',
  ])
  expect((await inputs(page, 'key'))[0].message).toEqual({ type: 'input', kind: 'key', action: 'down', key: 'H', code: 'KeyH', modifiers: 0, text: 'H' })
  await expect(page.getByTestId('ab-screen-input')).toBeFocused()
  await expect.poll(async () => (await mockPage(page))?.text).toBe('H')

  // Text that arrives without key events (an on-screen keyboard, a paste) goes as `text`.
  await page.keyboard.insertText('你好')
  await expect.poll(async () => (await inputs(page, 'text')).map((entry) => entry.message.text)).toEqual(['你好'])

  // An input method: what is being composed goes as `composition`, the committed text as `text`.
  const cdp = await page.context().newCDPSession(page)
  await cdp.send('Input.imeSetComposition', { text: 'shi', selectionStart: 3, selectionEnd: 3 })
  await cdp.send('Input.imeSetComposition', { text: 'shijie', selectionStart: 6, selectionEnd: 6 })
  await expect.poll(async () => (await mockPage(page))?.composition).toBe('shijie')
  await cdp.send('Input.insertText', { text: '世界' })
  await expect.poll(async () => (await inputs(page, 'composition')).map((entry) => entry.message.text)).toEqual(['shi', 'shijie', '世界'])
  await expect.poll(async () => (await inputs(page, 'text')).map((entry) => entry.message.text)).toEqual(['你好', '世界'])
  await expect.poll(async () => (await mockPage(page))?.text).toBe('H你好世界')
  expect((await mockPage(page))?.composition).toBe('')

  // The wheel scrolls the page, not the App.
  await page.mouse.move(field.screen.x, field.screen.y)
  await page.mouse.wheel(0, 300)
  await expect.poll(async () => (await mockPage(page))?.scrollY).toBe(300)
  const wheel = (await inputs(page, 'wheel')).at(-1)?.message
  expect(wheel).toMatchObject({ x: field.page.x, y: field.page.y, delta_x: 0 })
})

test('tabs: a page’s new window joins the thing, tabs switch and close', async ({ page }) => {
  await openBrowser(page)
  await press(page, 'newTab')
  await expect(page.getByTestId('ab-tabs-count')).toHaveText('2')
  await expect(page.getByTestId('ab-address-input')).toHaveValue('example.com#popup')

  await page.getByTestId('ab-tabs-button').click()
  const tabs = page.getByTestId('ab-tabs').getByTestId('ab-tab')
  await expect(tabs).toHaveCount(2)
  await expect(tabs.nth(1)).toHaveAttribute('data-active', 'true')
  await tabs.nth(0).getByRole('button').first().click()
  await expect(page.getByTestId('ab-tabs')).toHaveCount(0)
  await expect(page.getByTestId('ab-address-input')).toHaveValue('example.com')
  expect((await received(page, 'tab')).at(-1)?.message).toMatchObject({ action: 'activate' })

  await page.getByTestId('ab-tabs-button').click()
  await page.getByTestId('ab-tab-new').click()
  await expect(page.getByTestId('ab-tabs-count')).toHaveText('3')
  await expect(page.getByTestId('ab-address-input')).toHaveValue('')
  expect((await received(page, 'tab')).at(-1)?.message).toEqual({ type: 'tab', action: 'open' })

  await page.getByTestId('ab-tabs-button').click()
  await expect(tabs).toHaveCount(3)
  await tabs.nth(2).getByRole('button', { name: 'Close tab' }).click()
  await expect(tabs).toHaveCount(2)
  await tabs.nth(1).getByRole('button', { name: 'Close tab' }).click()
  await tabs.nth(0).getByRole('button', { name: 'Close tab' }).click()
  await expect(page.getByTestId('ab-tabs-empty')).toBeVisible()
  await page.getByTestId('ab-tabs-close').click()
  await expect(page.getByTestId('ab-screen-empty')).toBeVisible()
  await expect(page.getByTestId('ab-tabs-count')).toHaveText('0')
})

test('a select opens the App’s own option panel', async ({ page }) => {
  await openBrowser(page)
  await press(page, 'select')
  const panel = page.getByTestId('ab-select')
  await expect(panel.getByRole('option')).toHaveText(['Apple', 'Banana', 'Cherry', 'Durian'])
  await expect(panel.getByRole('option', { name: 'Apple' })).toHaveAttribute('aria-selected', 'true')
  await expect(panel.getByRole('option', { name: 'Durian' })).toBeDisabled()
  await panel.getByRole('option', { name: 'Cherry' }).click()
  await expect(panel).toHaveCount(0)
  await expect.poll(async () => (await mockPage(page))?.selected).toEqual([2])
  expect((await received(page, 'respond')).at(-1)?.message.result).toEqual({ selected: [2] })

  // Closing the panel cancels: the page keeps its value.
  await press(page, 'select')
  await expect(panel.getByRole('option', { name: 'Cherry' })).toHaveAttribute('aria-selected', 'true')
  await page.keyboard.press('Escape')
  await expect(panel).toHaveCount(0)
  await expect.poll(async () => (await received(page, 'respond')).at(-1)?.message.result).toEqual({ cancel: true })
  expect((await mockPage(page))?.selected).toEqual([2])
})

test('JS dialogs are shown by the App and answered', async ({ page }) => {
  await openBrowser(page)
  const dialog = page.getByTestId('ab-dialog')

  await press(page, 'alert')
  await expect(dialog).toHaveAccessibleName('example.com says')
  await expect(dialog.getByTestId('ab-dialog-message')).toHaveText('This is an alert from the page.')
  await expect(dialog.getByTestId('ab-dialog-cancel')).toHaveCount(0)
  await dialog.getByTestId('ab-dialog-accept').click()
  await expect(dialog).toHaveCount(0)
  await expect.poll(async () => (await mockPage(page))?.dialog).toBe('alert:ok')

  await press(page, 'confirm')
  await dialog.getByTestId('ab-dialog-cancel').click()
  await expect.poll(async () => (await mockPage(page))?.dialog).toBe('confirm:cancel')

  await press(page, 'prompt')
  await expect(dialog.getByTestId('ab-dialog-input')).toHaveValue('Alice')
  await dialog.getByTestId('ab-dialog-input').fill('Bob')
  await dialog.getByTestId('ab-dialog-input').press('Enter')
  await expect.poll(async () => (await mockPage(page))?.dialog).toBe('prompt:ok:Bob')
  expect((await received(page, 'respond')).map((entry) => entry.message.result)).toEqual([{ accept: true }, { accept: false }, { accept: true, prompt_text: 'Bob' }])
})

test('who operates: the page is view only while the Agent operates; take over, hand back, hand to the Agent', async ({ page }) => {
  await openBrowser(page)
  const status = page.getByTestId('ab-status')
  await expect(status).toHaveAttribute('data-agent-state', 'idle')
  await expect(status.getByTestId('ab-status-operator')).toHaveText('You are operating')
  await expect(status.getByTestId('ab-status-agent')).toHaveText('CodeAssistant is idle')
  await expect(status.getByTestId('ab-hand-to-agent')).toBeVisible()

  // The Agent takes the lease: input is refused before it leaves, and the page says so.
  await agentSide(page, 'agentAcquire')
  await expect(status).toHaveAttribute('data-controller', 'agent')
  await expect(status).toHaveAttribute('data-agent-state', 'working')
  await expect(status.getByTestId('ab-status-operator')).toHaveText('CodeAssistant is operating')
  await expect(page.getByTestId('ab-screen-readonly')).toHaveText('CodeAssistant is operating · view only')
  await expect(page.getByTestId('ab-screen')).toHaveAttribute('data-readonly', 'true')
  const before = (await received(page, 'input')).length
  await press(page, 'alert')
  await expect(page.getByTestId('ab-toast')).toHaveText('CodeAssistant is operating. Take over to use the page.')
  await page.keyboard.type('x')
  await page.mouse.wheel(0, 200)
  await expect(page.getByTestId('ab-address-input')).toHaveAttribute('readonly', '')
  await expect(page.getByTestId('ab-back')).toBeDisabled()
  await expect(page.getByTestId('ab-reload')).toBeDisabled()
  await page.waitForTimeout(200)
  expect((await received(page, 'input')).length).toBe(before)
  await expect(page.getByTestId('ab-dialog')).toHaveCount(0)

  // Take over: the lease is gone, the user operates, and the Agent cannot come back until the page is handed back.
  await status.getByTestId('ab-take-over').click()
  await expect(status).toHaveAttribute('data-controller', 'user')
  await expect(status).toHaveAttribute('data-user-hold', 'true')
  await expect(status.getByTestId('ab-status-operator')).toHaveText('You have taken over')
  await expect(page.getByTestId('ab-screen-readonly')).toHaveCount(0)
  expect((await rpcCalls(page, 'browser.take_over')).map((call) => call.params)).toEqual([{ group_id: 'default' }])
  expect(await agentSide(page, 'agentAcquire')).toBe(false)
  await press(page, 'alert')
  await page.getByTestId('ab-dialog-accept').click()
  await expect.poll(async () => (await mockPage(page))?.dialog).toBe('alert:ok')

  // Handing the page back or to the Agent first shows the message that wakes it (agent-browser.conversation.spec.ts).
  await status.getByTestId('ab-hand-back').click()
  await page.getByTestId('ab-handover-confirm').click()
  await expect(status).toHaveAttribute('data-user-hold', 'false')
  expect((await rpcCalls(page, 'browser.hand_back')).map((call) => call.params)).toEqual([{ group_id: 'default' }])

  // The Agent asks for the user (a password): the strip says why, and "Hand back" returns the page.
  await agentSide(page, 'agentAcquire')
  await agentSide(page, 'agentRequestHandover', 'password', 'the bank login')
  await expect(status).toHaveAttribute('data-agent-state', 'waiting_takeover')
  await expect(status.getByTestId('ab-status-operator')).toHaveText('CodeAssistant asks you to take over')
  await expect(status.getByTestId('ab-status-agent')).toHaveText('It needs you to enter a password · the bank login')
  await status.getByTestId('ab-hand-back').click()
  await page.getByTestId('ab-handover-confirm').click()
  await expect(status).toHaveAttribute('data-agent-state', 'idle')
  expect(await rpcCalls(page, 'browser.hand_back')).toHaveLength(2)

  // The other states of the Agent each have their own text.
  await agentSide(page, 'agentAcquire')
  await agentSide(page, 'agentRelease', 'done', 'ordered 2 tickets')
  await expect(status).toHaveAttribute('data-agent-state', 'done')
  await expect(status.getByTestId('ab-status-agent')).toHaveText('CodeAssistant has finished · ordered 2 tickets')
  await agentSide(page, 'agentAcquire')
  await agentSide(page, 'agentRelease', 'stuck', 'the form rejects the date')
  await expect(status.getByTestId('ab-status-agent')).toHaveText('CodeAssistant is stuck · the form rejects the date')
  await status.getByTestId('ab-hand-to-agent').click()
  await page.getByTestId('ab-handover-confirm').click()
  await expect(status).toHaveAttribute('data-agent-state', 'idle')
  expect((await rpcCalls(page, 'browser.hand_to_agent')).map((call) => call.params)).toEqual([{ group_id: 'default' }])
  // A question of the Agent is answered in the conversation, which comes up over the page.
  await agentSide(page, 'agentAsk', true)
  await expect(status).toHaveAttribute('data-agent-state', 'waiting_answer')
  await expect(status.getByTestId('ab-status-agent')).toHaveText('CodeAssistant is waiting for your answer · answer in the conversation')
  await status.getByTestId('ab-answer').click()
  await expect(page.getByTestId('ab-chat-sheet')).toBeVisible()
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'page')
})

test('things are the conversations with the Agent; each has its own tabs', async ({ page }) => {
  await openBrowser(page)
  // The Agent used the browser for one of its conversations: that conversation is a thing beside the default one.
  await page.evaluate(([agent, group]) => {
    const mock = (window as unknown as MockWindow).__agentBrowserMock
    mock.agentAcquire(agent, group)
    mock.agentOpen(agent, group, 'https://api.example.org')
    mock.agentRelease(agent, group)
  }, [AGENT, 'session-coder-2'] as const)
  await page.getByTestId('ab-thing-button').click()
  const things = page.getByTestId('ab-thing-list').getByRole('button')
  // The Agent's other conversations are not things (DG1); new ones are started here.
  await expect(things).toHaveCount(2)
  await expect(things.first()).toContainText('General browsing')
  await expect(things.first()).toContainText('1 tab')
  await expect(things.first()).toHaveAttribute('aria-current', 'true')
  await expect(page.getByTestId('ab-thing-session-coder-2')).toContainText('API Documentation')
  await expect(page.getByTestId('ab-thing-session-coder-2')).toContainText('1 tab')
  await expect(page.getByTestId('ab-new-thing')).toBeVisible()

  await page.getByTestId('ab-thing-session-coder-2').click()
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-group', 'session-coder-2')
  await expect(page.getByTestId('ab-thing-title')).toHaveText('API Documentation')
  await expect(page.getByTestId('ab-screen')).toHaveAttribute('data-status', 'open')
  await expect(page.getByTestId('ab-address-input')).toHaveValue('api.example.org')
  await expect(page.getByTestId('ab-tabs-count')).toHaveText('1')
  expect((await received(page, 'hello', 'session-coder-2')).map((entry) => entry.message.group_id)).toEqual(['session-coder-2'])
  await browse(page, 'docs.example.com')
  await expect.poll(async () => (await mockPage(page, 'session-coder-2'))?.url).toBe('https://docs.example.com')

  // Back to the first thing: its tab is still there.
  await page.getByTestId('ab-thing-button').click()
  await page.getByTestId('ab-thing-default').click()
  await expect(page.getByTestId('ab-address-input')).toHaveValue('example.com')
  await expect(page.getByTestId('ab-screen-canvas')).toBeVisible()
  await expect(page.getByTestId('ab-tabs-count')).toHaveText('1')
})

test('the three modes switch; reflow and chat leave the stage to their panels', async ({ page }) => {
  await openBrowser(page)
  await expect(page.getByRole('tab', { name: 'Page' })).toHaveAttribute('aria-selected', 'true')
  await page.getByTestId('ab-mode-reflow').click()
  await expect(page.getByTestId('ab-reflow-panel')).toBeVisible()
  await expect(page.getByTestId('ab-screen')).toHaveCount(0)
  // Reflow does not need the screen; who operates still shows, from `browser.status`.
  await expect(page.getByTestId('ab-status')).toHaveAttribute('data-controller', 'user')
  await agentSide(page, 'agentAcquire')
  await expect(page.getByTestId('ab-status')).toHaveAttribute('data-controller', 'agent', { timeout: 10_000 })

  await page.getByTestId('ab-mode-chat').click()
  await expect(page.getByTestId('ab-chat-panel')).toBeVisible()
  await expect(page.getByTestId('ab-address')).toHaveCount(0)
  await page.getByTestId('ab-mode-page').click()
  await expect(page.getByTestId('ab-screen-canvas')).toBeVisible()
  await expect(page.getByTestId('ab-screen-readonly')).toBeVisible()
  await expect(page.getByTestId('ab-address-input')).toHaveValue('example.com')
})

test('a file for the page comes from the Zone or this device; a download is saved to the Zone', async ({ page }) => {
  await openBrowser(page)
  // The picker opens at once on the page's own upload control, on the Zone.
  await press(page, 'upload')
  const picker = selectDialog(page)
  await expect(picker).toHaveAttribute('data-mode', 'open')
  await pickZone(picker, '/home/Documents/Friend Contract Draft.pdf')
  await expect.poll(async () => (await rpcCalls(page, 'browser.upload')).map((call) => call.params)).toEqual([
    { request_id: expect.any(String), zone_path: '/home/Documents/Friend Contract Draft.pdf', done: true },
  ])
  await expect.poll(async () => (await mockPage(page))?.files).toEqual(['Friend Contract Draft.pdf'])
  await expect(page.getByTestId('ab-file-chooser')).toHaveCount(0)

  // A file of this device is uploaded as bytes.
  await press(page, 'upload')
  await pickLocal(page, { name: 'photo.png', mimeType: 'image/png', buffer: Buffer.from('png') }, picker)
  await expect.poll(async () => (await mockPage(page))?.files).toEqual(['Friend Contract Draft.pdf', 'photo.png'])
  expect(await page.evaluate(() => (window as unknown as MockWindow).__agentBrowserMock.uploads.map((upload) => `${upload.source}:${upload.name}:${upload.done}`))).toEqual(['zone:Friend Contract Draft.pdf:true', 'local:photo.png:true'])

  // Cancelling the picker tells the page nothing was chosen.
  await press(page, 'upload')
  await closeSelect(page)
  await expect(page.getByTestId('ab-file-chooser')).toHaveCount(0)
  await expect.poll(async () => (await received(page, 'respond')).at(-1)?.message.result).toEqual({ cancel: true })

  await press(page, 'download')
  const note = page.getByTestId('ab-download')
  await expect(note).toContainText('Downloaded report.txt (2.0 KB)')
  await note.getByTestId('ab-download-save').click()
  await expect(picker).toHaveAttribute('data-mode', 'save')
  await expect(picker.getByTestId('select-name')).toHaveValue('report.txt')
  await selectGoto(picker, '/home/Downloads')
  await picker.getByTestId('select-confirm').click()
  await expect(note).toHaveAttribute('data-state', 'saved')
  await expect(note).toContainText('Saved to /home/Downloads/report.txt')
  expect((await rpcCalls(page, 'browser.download_export')).map((call) => call.params)).toEqual([{ download_id: expect.any(String) }])
  await note.getByTestId('ab-download-dismiss').click()
  await expect(note).toHaveCount(0)

  // A permission the service refused is only announced.
  await press(page, 'location')
  await expect(page.getByTestId('ab-permission')).toHaveText(/example\.com asked for your location\. The request was refused\./)
  await page.getByTestId('ab-permission-dismiss').click()
  await expect(page.getByTestId('ab-permission')).toHaveCount(0)
})

test('settings: clearing the sign-ins asks first; the desktop layout is scaled to the window', async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 900 })
  await openBrowser(page)
  await press(page, 'signIn')
  await expect.poll(async () => (await mockPage(page))?.signedIn).toBe(true)

  await page.getByTestId('ab-menu-button').click()
  await expect(page.getByTestId('ab-version')).toHaveText('Chromium 159.0.8154.6 · Playwright 1.65.1')
  await page.getByTestId('ab-clear-profile').click()
  const confirm = page.getByTestId('ab-clear-confirm')
  await expect(confirm).toContainText('signed out of every website')
  await confirm.getByTestId('ab-clear-cancel').click()
  expect(await rpcCalls(page, 'browser.clear_profile')).toHaveLength(0)
  await page.getByTestId('ab-clear-profile').click()
  await confirm.getByTestId('ab-clear-accept').click()
  await expect(page.getByTestId('ab-toast')).toHaveText('Sign-ins cleared.')
  expect(await rpcCalls(page, 'browser.clear_profile')).toHaveLength(1)
  expect((await mockPage(page))?.signedIn).toBe(false)
  // Clearing stops the browser: the App asks for it again.
  await expect.poll(async () => {
    const methods = await page.evaluate(() => (window as unknown as MockWindow).__agentBrowserMock.rpc.map((call) => call.method))
    return methods.lastIndexOf('browser.ensure') > methods.lastIndexOf('browser.clear_profile')
  }).toBe(true)
  await expect(page.getByTestId('ab-menu')).toHaveCount(0)

  // Desktop site: a 1280 px layout in a narrower window, shown scaled; pointer positions stay in page pixels.
  const surface = page.getByTestId('ab-screen-surface')
  const plain = await surface.getAttribute('data-viewport')
  expect(Number(plain?.split('x')[0])).toBeLessThan(1280)
  await page.getByTestId('ab-menu-button').click()
  await page.getByTestId('ab-desktop-site').click()
  await expect(page.getByTestId('ab-desktop-site')).toHaveAttribute('aria-checked', 'true')
  await page.getByTestId('ab-menu-close').click()
  await expect(surface).toHaveAttribute('data-viewport', /^1280x\d+$/)
  expect((await received(page, 'viewport')).at(-1)?.message).toMatchObject({ width: 1280, mobile: false })
  const canvas = await page.getByTestId('ab-screen-canvas').boundingBox()
  const host = await surface.boundingBox()
  expect(Math.round(canvas?.width ?? 0)).toBe(Math.round(host?.width ?? -1))
  const field = await press(page, 'input')
  expect(field.page.x).toBe(640)
  await expect.poll(async () => (await inputs(page, 'mouse')).filter((entry) => entry.message.action === 'down')).toHaveLength(2)
  const down = (await inputs(page, 'mouse')).filter((entry) => entry.message.action === 'down')[1].message
  expect(Math.abs((down.x ?? 0) - field.page.x)).toBeLessThanOrEqual(2)
  expect(Math.abs((down.y ?? 0) - field.page.y)).toBeLessThanOrEqual(2)
})

test('a browser that is turned off or missing is explained, and comes back without reopening', async ({ page }) => {
  await openApp(page)
  await page.evaluate((agent) => (window as unknown as MockWindow).__agentBrowserMock.setAvailability(agent, { available: false, unavailable: 'chromium was not found in /opt/buckyos/tools' }), AGENT)
  await page.getByTestId(`ab-agent-${AGENT}`).click()
  const notice = page.getByTestId('ab-unavailable')
  await expect(notice).toHaveAttribute('data-reason', 'no_package')
  await expect(notice).toContainText('The browser is not available on CodeAssistant')
  await expect(notice).toContainText('ext-core')
  await expect(page.getByTestId('ab-unavailable-detail')).toHaveText('chromium was not found in /opt/buckyos/tools')
  expect(await received(page, 'hello')).toHaveLength(0)

  await page.evaluate((agent) => (window as unknown as MockWindow).__agentBrowserMock.setAvailability(agent, { available: true, enabled: false, unavailable: null }), AGENT)
  await page.getByTestId('ab-status-retry').click()
  await expect(notice).toHaveAttribute('data-reason', 'disabled')

  await page.evaluate((agent) => (window as unknown as MockWindow).__agentBrowserMock.setAvailability(agent, { enabled: true }), AGENT)
  await page.getByTestId('ab-status-retry').click()
  await expect(page.getByTestId('ab-screen')).toHaveAttribute('data-status', 'open')
  await browse(page, 'example.com')
})

test('a lost connection reconnects and the screen goes on', async ({ page }) => {
  await openBrowser(page)
  await page.evaluate((agent) => (window as unknown as MockWindow).__agentBrowserMock.dropConnections(agent), AGENT)
  await expect(page.getByTestId('ab-screen')).toHaveAttribute('data-status', 'reconnecting')
  await expect(page.getByTestId('ab-screen-reconnecting')).toBeVisible()
  await expect(page.getByTestId('ab-screen')).toHaveAttribute('data-status', 'open')
  expect(await received(page, 'hello')).toHaveLength(2)
  const seq = Number(await page.getByTestId('ab-screen-canvas').getAttribute('data-seq'))
  await press(page, 'input')
  await page.keyboard.type('ok')
  await expect.poll(async () => (await mockPage(page))?.text).toBe('ok')
  await expect.poll(async () => Number(await page.getByTestId('ab-screen-canvas').getAttribute('data-seq'))).not.toBe(seq)
})

test('the App’s text follows the language', async ({ page }) => {
  await page.addInitScript(() => window.localStorage.setItem('buckyos.prototype.locale.v1', 'zh-CN'))
  await openBrowser(page)
  await expect(page.getByTestId('window-agent-browser')).toContainText('Agent 浏览器')
  await expect(page.getByRole('tab')).toHaveText(['原始页面', '重排', '对话'])
  await expect(page.getByTestId('ab-thing-title')).toHaveText('日常浏览')
  await expect(page.getByTestId('ab-status-operator')).toHaveText('你在操作')
  await expect(page.getByTestId('ab-hand-to-agent')).toHaveText('交给 Agent')
  await agentSide(page, 'agentAcquire')
  await expect(page.getByTestId('ab-take-over')).toHaveText('接手')
  await expect(page.getByTestId('ab-screen-readonly')).toHaveText('CodeAssistant 正在操作 · 只读')
})
