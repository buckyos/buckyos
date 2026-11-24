import { expect, test, type Page } from '@playwright/test'
import { openFromAllApps } from '../fixtures/desktop'

/**
 * The conversation side of the Agent browser (APP7, APP8, DG2–DG4, D35) on the mock Agent Service and the MessageHub
 * mock: the browser conversation is one MessageHub session with the Agent, started in the App with the source mark
 * the first time it is needed and bound with `browser.bind_default`; handing the page over and asking for a reflowed
 * page wake the Agent with a message there. The Agent's side is played through `window.__agentBrowserMock` and
 * `window.__messageHubMock`.
 */

const AGENT = 'codeassistant'
const CODER = 'did:buckyos:agent:codeassistant'
const SELF = 'did:buckyos:user:self'
const OWN = { viewerDid: SELF, ownerDid: SELF, mode: 'self' as const }
/** "API Documentation": an ordinary conversation with the Agent in the MessageHub mock. */
const DOCS = 'session-coder-2'
/** "Docs Cleanup": the Agent waits there for the answer to a question. */
const ASKING = 'session-coder-ask'
const MARKS = 'buckyos.conversation.mock.sources.v1'

type MockElement = 'input' | 'select' | 'alert' | 'signIn'
interface Message { [field: string]: unknown; type: string; kind?: string; phase?: string }
interface BrowserMock {
  log: Array<{ agentId: string; dropped: boolean; message: Message }>
  rpc: Array<{ agentId: string; method: string; params: Record<string, unknown> }>
  rects(width: number): Record<MockElement, { x: number; y: number; width: number; height: number }>
  page(agentId: string): { text: string; url: string; focused: boolean } | null
  agentAcquire(agentId: string): boolean
  agentOpen(agentId: string, url: string): boolean
  agentRelease(agentId: string, state?: 'done' | 'stuck', note?: string): void
  agentRequestHandover(agentId: string, reason: 'password' | 'captcha' | 'other', note?: string): void
  agentAsk(agentId: string, waiting: boolean): void
  pageFocus(agentId: string, focused: boolean): void
}
interface HubSession { id: string; entityId: string; lifecycle: string; shared: { title: string } }
interface HubMock {
  initialize(): Promise<void>
  simulateIncoming(sessionId: string, message: { content: { format: string; content: string } }): Promise<void>
  sessions(context: typeof OWN, entityId?: string, lifecycle?: string): HubSession[]
  reader(context: typeof OWN, sessionId: string): { timeline?: Array<{ from: string; content: { content?: string } }> }
  manage(context: typeof OWN, sessionId: string, action: 'archive' | 'restore' | 'delete'): Promise<void>
}
type MockWindow = { __agentBrowserMock: BrowserMock; __messageHubMock: HubMock }

const rpcCalls = (page: Page, method: string) =>
  page.evaluate((m) => (window as unknown as MockWindow).__agentBrowserMock.rpc.filter((call) => call.method === m).map((call) => call.params), method)
/** The sessions `browser.bind_default` was called with, in order. */
const bound = async (page: Page) => (await rpcCalls(page, 'browser.bind_default')).map((params) => String(params.session_id))
const touches = (page: Page) =>
  page.evaluate(() => (window as unknown as MockWindow).__agentBrowserMock.log.filter((entry) => entry.message.type === 'input' && entry.message.kind === 'touch').length)
const mockPage = (page: Page) =>
  page.evaluate((a) => (window as unknown as MockWindow).__agentBrowserMock.page(a), AGENT)
/** The Agent's side of its browser. */
const agentSide = (page: Page, method: 'agentAcquire' | 'agentOpen' | 'agentRelease' | 'agentRequestHandover' | 'agentAsk' | 'pageFocus', ...args: unknown[]) =>
  page.evaluate(([m, agent, list]) => ((window as unknown as MockWindow).__agentBrowserMock[m] as (...values: unknown[]) => unknown)(agent, ...list), [method, AGENT, args] as const)

/** The Agent writes in a conversation: that conversation becomes the owner's latest one with it. */
const agentSays = (page: Page, session: string, text: string) =>
  page.evaluate(async ([s, t]) => {
    const hub = (window as unknown as MockWindow).__messageHubMock
    await hub.initialize()
    await hub.simulateIncoming(s, { content: { format: 'text/plain', content: t } })
  }, [session, text] as const)
/** What the owner sent in a conversation, oldest first. */
const sent = (page: Page, session: string) =>
  page.evaluate(([context, s, self]) => ((window as unknown as MockWindow).__messageHubMock.reader(context, s).timeline ?? []).filter((message) => message.from === self).map((message) => message.content.content ?? ''), [OWN, session, SELF] as const)
const sessionsOfAgent = (page: Page) =>
  page.evaluate(([context, agent]) => (window as unknown as MockWindow).__messageHubMock.sessions(context, agent, 'active').map((session) => ({ id: session.id, title: session.shared.title })), [OWN, CODER] as const)
/** The sessions the mock keeps as started in the App (`<owner>\n<session>`), in the order they were marked. */
const marks = (page: Page) => page.evaluate((key) => Object.entries(JSON.parse(window.localStorage.getItem(key) ?? '{}') as Record<string, string>).filter(([, source]) => source === 'agent-browser').map(([entry]) => entry), MARKS)
/** Sessions marked as started in the App before the first page load. */
const seedMarks = (page: Page, sessions: string[]) =>
  page.addInitScript(([key, value]) => { if (window.localStorage.getItem(key) === null) window.localStorage.setItem(key, value) }, [MARKS, JSON.stringify(Object.fromEntries(sessions.map((session) => [`${SELF}\n${session}`, 'agent-browser'])))] as const)

async function enterAgent(page: Page) {
  await openFromAllApps(page, 'agent-browser')
  await page.getByTestId(`ab-agent-${AGENT}`).click()
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-agent', AGENT)
  await expect(page.getByTestId('ab-screen')).toHaveAttribute('data-status', 'open')
}

async function browse(page: Page, address: string) {
  const input = page.getByTestId('ab-address-input')
  await input.click()
  await input.fill(address)
  await input.press('Enter')
  await expect(input).toHaveValue(address)
  await expect(page.getByTestId('ab-screen-canvas')).toBeVisible()
}

async function openBrowser(page: Page) {
  await page.goto('/?scenario=normal')
  await enterAgent(page)
  await browse(page, 'example.com')
}

/** Where an element of the mock page is on the screen. */
async function elementAt(page: Page, element: MockElement) {
  const viewport = await page.getByTestId('ab-screen-surface').getAttribute('data-viewport')
  const width = Number(viewport?.split('x')[0])
  const box = await page.getByTestId('ab-screen-canvas').boundingBox()
  if (!box || !width) throw new Error('the screen shows no frame')
  const rect = await page.evaluate(([w, name]) => (window as unknown as MockWindow).__agentBrowserMock.rects(w)[name], [width, element] as const)
  const scale = box.width / width
  return { x: box.x + (rect.x + rect.width / 2) * scale, y: box.y + (rect.y + rect.height / 2) * scale }
}

const composer = (page: Page) => page.getByTestId('message-composer').getByRole('textbox', { name: 'Message CodeAssistant…' })

test.describe('in a window', () => {
  test.use({ viewport: { width: 1280, height: 900 } })

  test('the browser conversation is started, marked and bound the first time it is opened; it is the same one after that', async ({ page }) => {
    const errors: string[] = []
    page.on('console', (message) => { if (message.type() === 'error') errors.push(message.text()) })
    await openBrowser(page)
    await agentSays(page, DOCS, 'The API docs are ready for review.')
    const before = (await sessionsOfAgent(page)).map((session) => session.id)

    // Closed by default, and nothing is started or bound for browsing alone.
    await expect(page.getByTestId('ab-chat-sheet')).toHaveCount(0)
    expect(await bound(page)).toEqual([])
    const toggle = page.getByTestId('ab-chat-toggle')
    await expect(toggle).toHaveAttribute('aria-expanded', 'false')
    await toggle.click()
    const sheet = page.getByTestId('ab-chat-sheet')
    await expect(sheet).toHaveAccessibleName('Conversation with CodeAssistant')
    // A new session with the Agent, named for the browser and marked as started here; not the latest conversation.
    await expect.poll(() => bound(page)).toHaveLength(1)
    const [browser] = await bound(page)
    expect(browser).not.toBe(DOCS)
    expect((await sessionsOfAgent(page)).filter((session) => !before.includes(session.id))).toEqual([{ id: browser, title: 'Browser' }])
    expect(await marks(page)).toEqual([`${SELF}\n${browser}`])
    await expect(sheet.getByTestId('ab-conversation')).toHaveAttribute('data-session', browser)
    await expect(sheet.getByTestId('conversation-history')).toContainText('Tell CodeAssistant what you want done.')
    await expect(toggle).toHaveAttribute('aria-expanded', 'true')
    // The page stays on the stage beside it, in the size it had.
    await expect(page.getByTestId('ab-screen-canvas')).toBeVisible()
    expect(await page.evaluate(() => (window as unknown as MockWindow).__agentBrowserMock.log.filter((entry) => entry.message.type === 'viewport').length)).toBe(0)

    await composer(page).fill('Find the changelog page')
    await composer(page).press('Enter')
    await expect(sheet.getByTestId('conversation-history')).toContainText('Find the changelog page')
    await expect.poll(async () => (await sent(page, browser)).at(-1)).toBe('Find the changelog page')
    await agentSays(page, browser, 'On it.')
    await expect(sheet.getByTestId('conversation-history')).toContainText('On it.')

    await page.getByTestId('ab-chat-sheet-close').click()
    await expect(sheet).toHaveCount(0)
    // A message that arrives while it is closed shows on the way in.
    await agentSays(page, browser, 'The changelog is at /changes.')
    await expect(page.getByTestId('ab-chat-unread')).toBeVisible()

    // Conversation mode: the same conversation is the stage; nothing is started or bound again.
    await page.getByTestId('ab-mode-chat').click()
    const panel = page.getByTestId('ab-chat-panel')
    await expect(panel.getByTestId('ab-conversation')).toHaveAttribute('data-session', browser)
    await expect(panel.getByTestId('conversation-history')).toContainText('The changelog is at /changes.')
    await expect(toggle).toHaveCount(0)
    expect(await bound(page)).toEqual([browser])
    expect((await sessionsOfAgent(page)).filter((session) => !before.includes(session.id))).toHaveLength(1)

    // The browser conversation is archived: the next use starts another one.
    await page.evaluate(([context, session]) => (window as unknown as MockWindow).__messageHubMock.manage(context, session, 'archive'), [OWN, browser] as const)
    await expect.poll(() => bound(page)).toHaveLength(2)
    const again = (await bound(page))[1]
    expect(again).not.toBe(browser)
    await expect(panel.getByTestId('ab-conversation')).toHaveAttribute('data-session', again)
    expect(await marks(page)).toEqual([`${SELF}\n${browser}`, `${SELF}\n${again}`])
    expect(errors).toEqual([])
  })

  test('of the sessions started here, the bound one stays the browser conversation, else the latest one is', async ({ page }) => {
    await seedMarks(page, [DOCS, ASKING])
    await openBrowser(page)
    // Two sessions carry the mark: the latest active one is taken.
    await agentSays(page, DOCS, 'Older.')
    await agentSays(page, ASKING, 'Newer.')
    await page.getByTestId('ab-chat-toggle').click()
    await expect(page.getByTestId('ab-chat-sheet').getByTestId('ab-conversation')).toHaveAttribute('data-session', ASKING)
    expect(await bound(page)).toEqual([ASKING])
    // Bound, it stays the one even when the other is more recent; nothing new is started.
    await agentSays(page, DOCS, 'Newest.')
    await page.getByTestId('ab-chat-sheet-close').click()
    await page.getByTestId('ab-mode-chat').click()
    await expect(page.getByTestId('ab-chat-panel').getByTestId('ab-conversation')).toHaveAttribute('data-session', ASKING)
    expect(await bound(page)).toEqual([ASKING])
    expect((await sessionsOfAgent(page)).some((session) => session.title === 'Browser')).toBe(false)
  })

  test('handing the page to the Agent tells it in the browser conversation where to go on and what to do', async ({ page }) => {
    await openBrowser(page)
    await agentSays(page, DOCS, 'What can I do for you?')

    await page.getByTestId('ab-hand-to-agent').click()
    const ask = page.getByTestId('ab-hand-to-agent-sheet')
    await expect(ask).toHaveAccessibleName('Hand to CodeAssistant')
    await expect(ask.getByTestId('ab-handover-page')).toHaveText('example.comexample.com')
    // Leaving the question leaves the page with the user.
    await ask.getByTestId('ab-handover-cancel').click()
    await expect(ask).toHaveCount(0)
    expect(await rpcCalls(page, 'browser.hand_to_agent')).toEqual([])

    await page.getByTestId('ab-hand-to-agent').click()
    await ask.getByTestId('ab-handover-note').fill('Book two tickets for Friday')
    await ask.getByTestId('ab-handover-confirm').click()
    await expect(ask).toHaveCount(0)
    await expect.poll(() => rpcCalls(page, 'browser.hand_to_agent')).toEqual([{ group_id: 'default' }])
    // There was no browser conversation: it is started for the message, after the page changed hands.
    await expect.poll(() => bound(page)).toHaveLength(1)
    const [browser] = await bound(page)
    expect(browser).not.toBe(DOCS)
    await expect.poll(() => sent(page, browser)).toEqual(['Please continue from the current page.\nPage: example.com\nURL: https://example.com\n\nBook two tickets for Friday'])
    const order = await page.evaluate(() => (window as unknown as MockWindow).__agentBrowserMock.rpc.map((call) => call.method).filter((method) => method === 'browser.hand_to_agent' || method === 'browser.bind_default'))
    expect(order).toEqual(['browser.hand_to_agent', 'browser.bind_default'])
    expect(await sent(page, DOCS)).toEqual([])

    // The owner sees the message in the conversation.
    await page.getByTestId('ab-chat-toggle').click()
    await expect(page.getByTestId('ab-chat-sheet').getByTestId('conversation-history')).toContainText('Book two tickets for Friday')
    await page.getByTestId('ab-chat-sheet-close').click()

    // Nothing added: the Agent is still told which page to go on from, in the same conversation.
    await page.getByTestId('ab-hand-to-agent').click()
    await ask.getByTestId('ab-handover-confirm').click()
    await expect.poll(async () => (await sent(page, browser)).at(-1)).toBe('Please continue from the current page.\nPage: example.com\nURL: https://example.com')
    expect(await bound(page)).toEqual([browser])
  })

  test('a password: the Agent waits for the user to take over; handing back sends “Done, please continue.” to the browser conversation', async ({ page }) => {
    await openBrowser(page)
    await page.getByTestId('ab-chat-toggle').click()
    await expect.poll(() => bound(page)).toHaveLength(1)
    const [browser] = await bound(page)
    await page.getByTestId('ab-chat-sheet-close').click()
    await agentSays(page, browser, 'I am logging in to the bank.')
    const status = page.getByTestId('ab-status')

    await agentSide(page, 'agentAcquire')
    await agentSide(page, 'agentRequestHandover', 'password', 'the bank login')
    await expect(status).toHaveAttribute('data-agent-state', 'waiting_takeover')
    await expect(status.getByTestId('ab-status-operator')).toHaveText('CodeAssistant asks you to take over')
    await expect(status.getByTestId('ab-status-agent')).toHaveText('It needs you to enter a password · the bank login')
    // The page is the user's until it is handed back.
    await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-operable', 'true')
    expect(await agentSide(page, 'agentAcquire')).toBe(false)

    await status.getByTestId('ab-hand-back').click()
    const ask = page.getByTestId('ab-hand-back-sheet')
    await expect(ask).toHaveAccessibleName('Hand back to CodeAssistant')
    await expect(ask.getByTestId('ab-handover-note')).toHaveValue('Done, please continue.')
    expect(await rpcCalls(page, 'browser.hand_back')).toEqual([])
    await ask.getByTestId('ab-handover-confirm').click()
    await expect(status).toHaveAttribute('data-agent-state', 'idle')
    expect(await rpcCalls(page, 'browser.hand_back')).toEqual([{ group_id: 'default' }])
    await expect.poll(() => sent(page, browser)).toEqual(['Done, please continue.'])
    expect(await bound(page)).toEqual([browser])

    // The message can be changed before it is sent.
    await agentSide(page, 'agentAcquire')
    await agentSide(page, 'agentRequestHandover', 'captcha')
    await expect(status.getByTestId('ab-status-agent')).toHaveText('It needs you to pass a verification')
    await status.getByTestId('ab-hand-back').click()
    await ask.getByTestId('ab-handover-note').fill('Solved it, the code was 4711')
    await ask.getByTestId('ab-handover-confirm').click()
    await expect.poll(() => sent(page, browser)).toEqual(['Done, please continue.', 'Solved it, the code was 4711'])

    // After taking over on one's own the default differs; an emptied message hands back without waking the Agent.
    await agentSide(page, 'agentAcquire')
    await status.getByTestId('ab-take-over').click()
    await expect(status).toHaveAttribute('data-user-hold', 'true')
    await status.getByTestId('ab-hand-back').click()
    await expect(ask.getByTestId('ab-handover-note')).toHaveValue('I am done here, please continue.')
    await ask.getByTestId('ab-handover-note').fill('')
    await expect(ask.getByTestId('ab-handover-empty')).toHaveText('Without a message CodeAssistant is not told to go on.')
    await ask.getByTestId('ab-handover-confirm').click()
    await expect(status).toHaveAttribute('data-user-hold', 'false')
    expect(await rpcCalls(page, 'browser.hand_back')).toHaveLength(3)
    await page.waitForTimeout(200)
    expect(await sent(page, browser)).toHaveLength(2)
  })

  test('the handover messages and the browser conversation’s name follow the language', async ({ page }) => {
    await page.addInitScript(() => window.localStorage.setItem('buckyos.prototype.locale.v1', 'zh-CN'))
    await openBrowser(page)
    await agentSays(page, DOCS, '我去登录银行。')
    const status = page.getByTestId('ab-status')

    await page.getByTestId('ab-hand-to-agent').click()
    const hand = page.getByTestId('ab-hand-to-agent-sheet')
    await expect(hand).toHaveAccessibleName('交给 CodeAssistant')
    await hand.getByTestId('ab-handover-note').fill('订两张周五的票')
    await hand.getByTestId('ab-handover-confirm').click()
    await expect.poll(() => bound(page)).toHaveLength(1)
    const [browser] = await bound(page)
    await expect.poll(() => sent(page, browser)).toEqual(['从当前页面接着做。\n页面：example.com\n网址：https://example.com\n\n订两张周五的票'])
    expect((await sessionsOfAgent(page)).find((session) => session.id === browser)?.title).toBe('浏览器')

    await agentSide(page, 'agentAcquire')
    await agentSide(page, 'agentRequestHandover', 'password', '网银登录')
    await expect(status.getByTestId('ab-status-operator')).toHaveText('CodeAssistant 请你接手')
    await expect(status.getByTestId('ab-status-agent')).toHaveText('需要你输入密码 · 网银登录')
    await status.getByTestId('ab-hand-back').click()
    const back = page.getByTestId('ab-hand-back-sheet')
    await expect(back.getByTestId('ab-handover-note')).toHaveValue('填好了，继续')
    await back.getByTestId('ab-handover-confirm').click()
    await expect.poll(async () => (await sent(page, browser)).at(-1)).toBe('填好了，继续')

    await agentSide(page, 'agentAsk', true)
    await expect(status.getByTestId('ab-status-agent')).toHaveText('CodeAssistant 在等你回答 · 到对话里回答')
    await status.getByTestId('ab-answer').click()
    await expect(page.getByTestId('ab-chat-sheet')).toHaveAccessibleName('和 CodeAssistant 的对话')
  })

  test('a link opens the App on an Agent; a question of the Agent is answered in the browser conversation', async ({ page }) => {
    await seedMarks(page, [ASKING])
    await page.goto(`/?scenario=normal&agent_browser=${AGENT}`)
    const app = page.getByTestId('agent-browser')
    await expect(app).toHaveAttribute('data-agent', AGENT)
    await expect(app).toHaveAttribute('data-mode', 'page')
    await expect(page.getByTestId('ab-agent-picker')).toHaveCount(0)
    await expect(page.getByTestId('ab-screen')).toHaveAttribute('data-status', 'open')
    // The link is consumed: a reload does not open the App again.
    expect(new URL(page.url()).search).toBe('?scenario=normal')

    // The service reports the open question of the browser conversation (DG4): the strip sends the user there.
    await agentSide(page, 'agentAsk', true)
    const status = page.getByTestId('ab-status')
    await expect(status).toHaveAttribute('data-agent-state', 'waiting_answer')
    await expect(status.getByTestId('ab-status-agent')).toHaveText('CodeAssistant is waiting for your answer · answer in the conversation')
    await status.getByTestId('ab-answer').click()
    const sheet = page.getByTestId('ab-chat-sheet')
    // "Docs Cleanup" was started here: it is the browser conversation.
    await expect(sheet.getByTestId('ab-conversation')).toHaveAttribute('data-session', ASKING)
    expect(await bound(page)).toEqual([ASKING])
    // `ask_user` is answered in the composer, as in MessageHub.
    const answer = sheet.getByTestId('composer-answer')
    await expect(answer).toHaveAttribute('data-kind', 'choice')
    await expect(answer.getByTestId('answer-question')).toHaveText('Which docs should I tidy?')
    await answer.getByTestId('answer-option').filter({ hasText: 'docs/ only' }).click()
    await expect(sheet.getByTestId('composer-answer')).toHaveCount(0)
    await expect.poll(async () => (await sent(page, ASKING)).at(-1)).toBe('docs/ only')
    await agentSide(page, 'agentAsk', false)
    await expect(status).toHaveAttribute('data-agent-state', 'idle')
    await expect(status.getByTestId('ab-hand-to-agent')).toBeVisible()
  })

  test('a link with the Agent’s DID opens conversation mode on the browser conversation; a session it names is ignored', async ({ page }) => {
    await page.goto(`/?scenario=normal&agent_browser=${encodeURIComponent(CODER)}&group=${DOCS}&mode=chat`)
    const app = page.getByTestId('agent-browser')
    await expect(app).toHaveAttribute('data-agent', AGENT)
    await expect(app).toHaveAttribute('data-mode', 'chat')
    await expect.poll(() => bound(page)).toHaveLength(1)
    const [browser] = await bound(page)
    expect(browser).not.toBe(DOCS)
    await expect(page.getByTestId('ab-chat-panel').getByTestId('ab-conversation')).toHaveAttribute('data-session', browser)
    await expect(page.getByTestId('ab-chat-toggle')).toHaveCount(0)
    expect(new URL(page.url()).search).toBe('?scenario=normal')
  })

  test('without the conversations the page still changes hands; the conversation says so and can be tried again', async ({ page }) => {
    await page.goto('/?scenario=normal')
    // The owner's messages cannot be read when the App asks for them.
    await openFromAllApps(page, 'messagehub')
    await page.waitForFunction(() => !!(window as unknown as Partial<MockWindow>).__messageHubMock)
    await page.evaluate((owner) => (window as unknown as { __messageHubMock: { denyOwner(did: string): void } }).__messageHubMock.denyOwner(owner), SELF)
    await enterAgent(page)
    await browse(page, 'example.com')

    // No question about a message that cannot be sent: the page is handed over at once, and the App says the Agent was not told.
    await page.getByTestId('ab-hand-to-agent').click()
    await expect(page.getByTestId('ab-hand-to-agent-sheet')).toHaveCount(0)
    await expect.poll(() => rpcCalls(page, 'browser.hand_to_agent')).toEqual([{ group_id: 'default' }])
    await expect(page.getByTestId('ab-toast')).toHaveText('The page was handed over, but the message to the Agent was not sent. Tell it in the conversation.')
    await agentSide(page, 'agentAcquire')
    await page.getByTestId('ab-take-over').click()
    await page.getByTestId('ab-hand-back').click()
    await expect(page.getByTestId('ab-status')).toHaveAttribute('data-user-hold', 'false')
    expect(await bound(page)).toEqual([])

    await page.getByTestId('ab-chat-toggle').click()
    const unavailable = page.getByTestId('ab-conversation-unavailable')
    await expect(unavailable).toContainText('Your conversations cannot be reached right now.')
    await page.evaluate((owner) => (window as unknown as { __messageHubMock: { revokedOwners: Set<string> } }).__messageHubMock.revokedOwners.delete(owner), SELF)
    await unavailable.getByTestId('ab-conversation-unavailable-retry').click()
    await expect(page.getByTestId('ab-chat-sheet').getByTestId('ab-conversation')).toBeVisible()
    expect(await bound(page)).toHaveLength(1)
    // With the conversations back, handing over asks for the message again.
    await page.getByTestId('ab-chat-sheet-close').click()
    await page.getByTestId('ab-hand-to-agent').click()
    await expect(page.getByTestId('ab-hand-to-agent-sheet')).toBeVisible()
  })

  test('MessageHub’s Agent panel: the browser conversation always opens the browser, another conversation only while the Agent operates it', async ({ page }) => {
    await page.goto('/?scenario=normal')
    await enterAgent(page)
    await page.getByTestId('ab-chat-toggle').click()
    await expect.poll(() => bound(page)).toHaveLength(1)
    const [browser] = await bound(page)
    await page.getByTestId('ab-chat-sheet-close').click()
    await agentSays(page, browser, 'Ready in the browser.')

    // MessageHub opens on the Agent's main conversation, not on one an App started, however recent.
    await openFromAllApps(page, 'messagehub')
    const hub = page.getByTestId('window-messagehub')
    await expect(hub.getByTestId('conversation-history')).not.toContainText('Ready in the browser.')
    await hub.getByRole('button', { name: 'Sessions', exact: true }).click()
    await hub.locator(`[data-testid="session-row"][data-session-id="${browser}"] button`).first().click()
    await expect(hub.getByTestId('conversation-history')).toContainText('Ready in the browser.')
    await hub.getByTestId('agent-panel-toggle').click()
    const entry = hub.getByTestId('agent-browser-entry')
    await expect(entry).toHaveAttribute('data-state', 'session')
    await expect(entry.getByTestId('agent-browser-entry-open')).toHaveText('Open browser')
    await entry.getByTestId('agent-browser-entry-open').click()
    const app = page.getByTestId('agent-browser')
    await expect(app).toHaveAttribute('data-agent', AGENT)
    // Opened again, the App finds the same conversation: nothing is started or bound.
    await page.getByTestId('ab-mode-chat').click()
    await expect(page.getByTestId('ab-chat-panel').getByTestId('ab-conversation')).toHaveAttribute('data-session', browser)
    expect(await bound(page)).toEqual([browser])

    const raise = async (id: string) => {
      const bar = page.getByTestId(`window-drag-${id}`)
      await bar.dispatchEvent('pointerdown', { bubbles: true, button: 0, pointerId: 1, pointerType: 'mouse' })
      await bar.dispatchEvent('pointerup', { bubbles: true, button: 0, pointerId: 1, pointerType: 'mouse' })
      await bar.dispatchEvent('click', { bubbles: true })
    }
    // Another conversation: nothing while the Agent does not operate the browser.
    await raise('messagehub')
    await hub.locator('div.absolute.inset-0.z-40[style]').click({ position: { x: 8, y: 8 } })
    const sessions = hub.getByRole('button', { name: 'Sessions', exact: true })
    if (!(await hub.getByTestId('session-sidebar').isVisible())) await sessions.click()
    await hub.locator(`[data-testid="session-row"][data-session-id="${DOCS}"] button`).first().click()
    await expect(hub.locator('[data-testid="session-row"]:has(button[aria-current="true"])')).toHaveAttribute('data-session-id', DOCS)
    await agentSays(page, DOCS, 'Looking it up.')
    await expect(hub.getByTestId('conversation-history')).toContainText('Looking it up.')
    await hub.getByTestId('agent-panel-toggle').click()
    await expect(hub.getByText('Start a new conversation')).toBeVisible()
    await page.waitForTimeout(500)
    await expect(entry).toHaveCount(0)
    // The Agent operates its browser: the way to it is offered.
    await agentSide(page, 'agentAcquire')
    await agentSide(page, 'agentOpen', 'https://api.example.org')
    await agentSays(page, DOCS, 'Found the page.')
    await expect(entry).toHaveAttribute('data-state', 'working')
    await expect(entry).toContainText('CodeAssistant is using the browser')
    await entry.getByTestId('agent-browser-entry-open').click()
    await expect(app).toHaveAttribute('data-agent', AGENT)
    await expect(page.getByTestId('ab-screen-readonly')).toBeVisible()
    await page.getByTestId('ab-mode-page').click()
    await expect(page.getByTestId('ab-address-input')).toHaveValue('api.example.org')
  })
})

test.describe('on a phone', () => {
  test.use({ viewport: { width: 375, height: 812 }, hasTouch: true, isMobile: true })

  async function openOnPhone(page: Page) {
    await page.goto('/?scenario=normal')
    await expect(page.getByTestId('mobile-taskbar')).toBeAttached()
    await openFromAllApps(page, 'agent-browser')
    await page.getByTestId(`ab-agent-${AGENT}`).tap()
    await expect(page.getByTestId('ab-screen')).toHaveAttribute('data-status', 'open')
    const address = page.getByTestId('ab-address-input')
    await address.tap()
    await address.fill('example.com')
    await address.press('Enter')
    await expect(page.getByTestId('ab-screen-canvas')).toHaveAttribute('data-seq', /\d+/)
    await expect(address).toHaveValue('example.com')
  }

  test('the conversation comes up from the bottom over the page and goes away again; in conversation mode it is the stage', async ({ page }) => {
    await seedMarks(page, [DOCS])
    await openOnPhone(page)
    await agentSays(page, DOCS, 'Ready when you are.')
    const sheet = page.getByTestId('ab-chat-sheet')
    await expect(sheet).toHaveCount(0)
    await page.getByTestId('ab-chat-toggle').tap()
    await expect(sheet.getByTestId('conversation-history')).toContainText('Ready when you are.')
    const stage = await page.getByTestId('ab-stage').boundingBox()
    if (!stage) throw new Error('no stage')
    await expect.poll(async () => {
      const box = await sheet.boundingBox()
      return box ? Math.round(box.y + box.height) : 0
    }).toBe(Math.round(stage.y + stage.height))
    const box = await sheet.boundingBox()
    expect(box).toMatchObject({ x: 0, width: 375 })
    expect(box?.y ?? 0).toBeGreaterThan(stage.y + 100)
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(375)
    // The page above it still takes touches.
    const count = await touches(page)
    const field = await elementAt(page, 'input')
    expect(field.y).toBeLessThan(box?.y ?? 0)
    await page.touchscreen.tap(field.x, field.y)
    await expect.poll(() => touches(page)).toBe(count + 2)

    // The phone's Back puts it away; so does its own button.
    await page.getByLabel('Status bar').getByRole('button', { name: 'Back' }).tap()
    await expect(sheet).toHaveCount(0)
    await page.getByTestId('ab-chat-toggle').tap()
    await expect(sheet).toBeVisible()
    await page.getByTestId('ab-chat-sheet-close').tap()
    await expect(sheet).toHaveCount(0)
    await expect(page.getByTestId('ab-screen-canvas')).toBeVisible()

    await page.getByTestId('ab-mode-chat').tap()
    const panel = page.getByTestId('ab-chat-panel')
    await expect(panel.getByTestId('ab-conversation')).toHaveAttribute('data-session', DOCS)
    expect(await panel.boundingBox()).toEqual(await page.getByTestId('ab-stage').boundingBox())
    await expect(page.getByTestId('ab-chat-toggle')).toHaveCount(0)
    await composer(page).fill('Go ahead')
    await page.getByTestId('message-composer').getByRole('button', { name: 'Send' }).tap()
    await expect.poll(async () => (await sent(page, DOCS)).at(-1)).toBe('Go ahead')
  })

  test('pressing a text field of the page brings up the keyboard input; pressing elsewhere puts it away', async ({ page }) => {
    await openOnPhone(page)
    const input = page.getByTestId('ab-screen-input')
    const keyboard = page.getByTestId('ab-keyboard')
    const field = await elementAt(page, 'input')
    const elsewhere = await elementAt(page, 'signIn')

    await page.touchscreen.tap(field.x, field.y)
    await expect(input).toBeFocused()
    await expect(input).toHaveAttribute('inputmode', 'text')
    await expect(keyboard).toHaveAttribute('aria-pressed', 'true')
    await page.keyboard.insertText('手机')
    await expect.poll(async () => (await mockPage(page))?.text).toBe('手机')

    await page.touchscreen.tap(elsewhere.x, elsewhere.y)
    await expect(input).not.toBeFocused()
    await expect(keyboard).toHaveAttribute('aria-pressed', 'false')

    // A field the page focuses by itself raises no keyboard; the button still does.
    await page.waitForTimeout(3100)
    await agentSide(page, 'pageFocus', true)
    await expect.poll(async () => (await mockPage(page))?.focused).toBe(true)
    await page.waitForTimeout(200)
    await expect(input).not.toBeFocused()
    await keyboard.tap()
    await expect(input).toBeFocused()
    await agentSide(page, 'pageFocus', false)
    await expect(input).not.toBeFocused()
  })
})

test.describe('on an iPhone', () => {
  test.use({
    viewport: { width: 375, height: 812 }, hasTouch: true, isMobile: true,
    userAgent: 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1',
  })

  test('the focus is taken inside the press, without a keyboard until the page says a text field was pressed', async ({ page }) => {
    await page.goto('/?scenario=normal')
    await expect(page.getByTestId('mobile-taskbar')).toBeAttached()
    await openFromAllApps(page, 'agent-browser')
    await page.getByTestId(`ab-agent-${AGENT}`).tap()
    const address = page.getByTestId('ab-address-input')
    await address.tap()
    await address.fill('example.com')
    await address.press('Enter')
    await expect(page.getByTestId('ab-screen-canvas')).toHaveAttribute('data-seq', /\d+/)
    const input = page.getByTestId('ab-screen-input')
    const keyboard = page.getByTestId('ab-keyboard')
    const field = await elementAt(page, 'input')
    const elsewhere = await elementAt(page, 'signIn')

    // Not a text field: the input holds the focus with no keyboard (`inputmode="none"`).
    await page.touchscreen.tap(elsewhere.x, elsewhere.y)
    await expect(input).toBeFocused()
    await expect(input).toHaveAttribute('inputmode', 'none')
    await expect(keyboard).toHaveAttribute('aria-pressed', 'false')

    await page.touchscreen.tap(field.x, field.y)
    await expect(input).toHaveAttribute('inputmode', 'text')
    await expect(input).toBeFocused()
    await expect(keyboard).toHaveAttribute('aria-pressed', 'true')
    await page.keyboard.insertText('abc')
    await expect.poll(async () => (await mockPage(page))?.text).toBe('abc')

    await page.touchscreen.tap(elsewhere.x, elsewhere.y)
    await expect(keyboard).toHaveAttribute('aria-pressed', 'false')
    // The button raises the keyboard inside its own press.
    await keyboard.tap()
    await expect(input).toBeFocused()
    await expect(input).toHaveAttribute('inputmode', 'text')
    await expect(keyboard).toHaveAttribute('aria-pressed', 'true')
  })
})
