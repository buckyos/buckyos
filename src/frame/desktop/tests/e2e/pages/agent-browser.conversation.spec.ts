import { expect, test, type Page } from '@playwright/test'
import { openFromAllApps } from '../fixtures/desktop'

/**
 * The conversation side of the Agent browser (APP7, APP8, DG1–DG4) on the mock Agent Service and the MessageHub mock:
 * a thing is a MessageHub session with the Agent plus its group of tabs; the default thing takes the owner's default
 * conversation the first time it is used (`browser.bind_default`); handing the page over wakes the Agent with a
 * message. The Agent's side is played through `window.__agentBrowserMock` and `window.__messageHubMock`.
 */

const AGENT = 'codeassistant'
const CODER = 'did:buckyos:agent:codeassistant'
const SELF = 'did:buckyos:user:self'
const OWN = { viewerDid: SELF, ownerDid: SELF, mode: 'self' as const }
/** "API Documentation": an ordinary conversation with the Agent in the MessageHub mock. */
const DOCS = 'session-coder-2'
/** "Docs Cleanup": the Agent waits there for the answer to a question. */
const ASKING = 'session-coder-ask'
const MARKS = 'buckyos.agent-browser.mock.marks'

type MockElement = 'input' | 'select' | 'alert' | 'signIn'
interface Message { [field: string]: unknown; type: string; kind?: string; phase?: string }
interface BrowserMock {
  log: Array<{ agentId: string; groupId: string; dropped: boolean; message: Message }>
  rpc: Array<{ agentId: string; method: string; params: Record<string, unknown> }>
  rects(width: number): Record<MockElement, { x: number; y: number; width: number; height: number }>
  page(agentId: string, groupId: string): { text: string; url: string; focused: boolean } | null
  agentAcquire(agentId: string, groupId: string): boolean
  agentOpen(agentId: string, groupId: string, url: string): boolean
  agentRelease(agentId: string, groupId: string, state?: 'done' | 'stuck', note?: string): void
  agentRequestHandover(agentId: string, groupId: string, reason: 'password' | 'captcha' | 'other', note?: string): void
  agentAsk(agentId: string, groupId: string, waiting: boolean): void
  pageFocus(agentId: string, groupId: string, focused: boolean): void
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
const hellos = (page: Page, group: string) =>
  page.evaluate((g) => (window as unknown as MockWindow).__agentBrowserMock.log.filter((entry) => entry.message.type === 'hello' && entry.groupId === g).length, group)
const touches = (page: Page) =>
  page.evaluate(() => (window as unknown as MockWindow).__agentBrowserMock.log.filter((entry) => entry.message.type === 'input' && entry.message.kind === 'touch').length)
const mockPage = (page: Page, group = 'default') =>
  page.evaluate(([a, g]) => (window as unknown as MockWindow).__agentBrowserMock.page(a, g), [AGENT, group] as const)
/** The Agent's side on a group of its browser. */
const agentSide = (page: Page, method: 'agentAcquire' | 'agentOpen' | 'agentRelease' | 'agentRequestHandover' | 'agentAsk' | 'pageFocus', group: string, ...args: unknown[]) =>
  page.evaluate(([m, agent, g, list]) => ((window as unknown as MockWindow).__agentBrowserMock[m] as (...values: unknown[]) => unknown)(agent, g, ...list), [method, AGENT, group, args] as const)

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

  test('the default thing takes the owner’s default conversation the first time it is opened; it shows the history and sends', async ({ page }) => {
    const errors: string[] = []
    page.on('console', (message) => { if (message.type() === 'error') errors.push(message.text()) })
    await openBrowser(page)
    await agentSays(page, DOCS, 'The API docs are ready for review.')

    // Closed by default, and nothing is bound for browsing alone.
    await expect(page.getByTestId('ab-chat-sheet')).toHaveCount(0)
    expect(await rpcCalls(page, 'browser.bind_default')).toEqual([])
    const toggle = page.getByTestId('ab-chat-toggle')
    await expect(toggle).toHaveAttribute('aria-expanded', 'false')
    await toggle.click()
    const sheet = page.getByTestId('ab-chat-sheet')
    await expect(sheet).toHaveAccessibleName('Conversation with CodeAssistant')
    await expect(sheet.getByTestId('ab-conversation')).toHaveAttribute('data-session', DOCS)
    expect(await rpcCalls(page, 'browser.bind_default')).toEqual([{ session_id: DOCS }])
    await expect(sheet.getByTestId('conversation-history')).toContainText('The API docs are ready for review.')
    await expect(toggle).toHaveAttribute('aria-expanded', 'true')
    // The page stays on the stage beside it, in the size it had.
    await expect(page.getByTestId('ab-screen-canvas')).toBeVisible()
    expect(await page.evaluate(() => (window as unknown as MockWindow).__agentBrowserMock.log.filter((entry) => entry.message.type === 'viewport').length)).toBe(0)

    // The bound conversation is the default thing, not a thing of its own.
    await page.getByTestId('ab-thing-button').click()
    await expect(page.getByTestId('ab-thing-list').getByRole('button')).toHaveCount(1)
    await page.getByTestId('ab-things-close').click()

    await composer(page).fill('Find the changelog page')
    await composer(page).press('Enter')
    await expect(sheet.getByTestId('conversation-history')).toContainText('Find the changelog page')
    await expect.poll(async () => (await sent(page, DOCS)).at(-1)).toBe('Find the changelog page')
    await agentSays(page, DOCS, 'On it.')
    await expect(sheet.getByTestId('conversation-history')).toContainText('On it.')

    await page.getByTestId('ab-chat-sheet-close').click()
    await expect(sheet).toHaveCount(0)
    // A message that arrives while it is closed shows on the way in.
    await agentSays(page, DOCS, 'The changelog is at /changes.')
    await expect(page.getByTestId('ab-chat-unread')).toBeVisible()

    // Conversation mode: the same conversation is the stage; nothing is bound again.
    await page.getByTestId('ab-mode-chat').click()
    const panel = page.getByTestId('ab-chat-panel')
    await expect(panel.getByTestId('ab-conversation')).toHaveAttribute('data-session', DOCS)
    await expect(panel.getByTestId('conversation-history')).toContainText('The changelog is at /changes.')
    await expect(toggle).toHaveCount(0)
    expect(await rpcCalls(page, 'browser.bind_default')).toHaveLength(1)

    // The bound conversation is archived: the next use binds another one.
    await page.evaluate(([context, session]) => (window as unknown as MockWindow).__messageHubMock.manage(context, session, 'archive'), [OWN, DOCS] as const)
    await expect.poll(async () => (await rpcCalls(page, 'browser.bind_default')).length).toBe(2)
    const rebound = (await rpcCalls(page, 'browser.bind_default'))[1].session_id as string
    expect(rebound).not.toBe(DOCS)
    await expect(panel.getByTestId('ab-conversation')).toHaveAttribute('data-session', rebound)
    expect(errors).toEqual([])
  })

  test('a new task is a session with the Agent that carries the source mark; switching things switches the screen and the conversation', async ({ page }) => {
    await openBrowser(page)
    const app = page.getByTestId('agent-browser')
    await page.getByTestId('ab-thing-button').click()
    const things = page.getByTestId('ab-thing-list').getByRole('button')
    // The Agent's ordinary conversations are not things.
    await expect(things).toHaveCount(1)
    await expect(page.getByTestId(`ab-thing-${DOCS}`)).toHaveCount(0)
    const before = (await sessionsOfAgent(page)).map((session) => session.id)

    await page.getByTestId('ab-new-thing').click()
    await expect(page.getByTestId('ab-things')).toHaveCount(0)
    await expect(app).not.toHaveAttribute('data-group', 'default')
    const group = (await app.getAttribute('data-group')) ?? ''
    await expect(page.getByTestId('ab-thing-title')).toHaveText(/^Browser task \d\d-\d\d \d\d:\d\d$/)
    // The screen follows: a connection for the new group, which has no tab yet.
    await expect(page.getByTestId('ab-screen')).toHaveAttribute('data-status', 'open')
    await expect(page.getByTestId('ab-screen-empty')).toBeVisible()
    expect(await hellos(page, group)).toBe(1)
    // It is a new MessageHub session with the Agent, marked as started here.
    const created = (await sessionsOfAgent(page)).filter((session) => !before.includes(session.id))
    expect(created).toEqual([{ id: group, title: expect.stringMatching(/^Browser task /) }])
    expect(await page.evaluate((key) => JSON.parse(window.localStorage.getItem(key) ?? '[]') as string[], MARKS)).toEqual([`${SELF}\n${group}`])

    await browse(page, 'docs.example.com')
    await expect.poll(async () => (await mockPage(page, group))?.url).toBe('https://docs.example.com')
    // Its conversation is its own session.
    await page.getByTestId('ab-chat-toggle').click()
    const sheet = page.getByTestId('ab-chat-sheet')
    await expect(sheet.getByTestId('ab-conversation')).toHaveAttribute('data-session', group)
    await expect(sheet.getByTestId('conversation-history')).toContainText('Tell CodeAssistant what you want done.')
    await composer(page).fill('Compare the two plans')
    await composer(page).press('Enter')
    await expect.poll(() => sent(page, group)).toEqual(['Compare the two plans'])
    expect(await rpcCalls(page, 'browser.bind_default')).toEqual([])

    // Back to the default thing: its tab, and its own conversation, which is not the new task's
    // (that one is the owner's latest conversation with the Agent now, and keeps its own tabs).
    await page.getByTestId('ab-thing-button').click()
    await expect(things).toHaveCount(2)
    await expect(page.getByTestId(`ab-thing-${group}`)).toContainText('1 tab')
    await page.getByTestId('ab-thing-default').click()
    await expect(app).toHaveAttribute('data-group', 'default')
    await expect(page.getByTestId('ab-address-input')).toHaveValue('example.com')
    await expect.poll(async () => (await rpcCalls(page, 'browser.bind_default')).length).toBe(1)
    const bound = (await rpcCalls(page, 'browser.bind_default'))[0].session_id as string
    expect(bound).not.toBe(group)
    await expect(sheet.getByTestId('ab-conversation')).toHaveAttribute('data-session', bound)
    await expect.poll(async () => (await mockPage(page, group))?.url).toBe('https://docs.example.com')

    // A conversation in which the Agent uses the browser by itself is a thing as well.
    await agentSide(page, 'agentAcquire', DOCS)
    await agentSide(page, 'agentOpen', DOCS, 'https://api.example.org')
    await page.getByTestId('ab-thing-button').click()
    await expect(things).toHaveCount(3)
    await expect(page.getByTestId(`ab-thing-${DOCS}`)).toContainText('API Documentation')
    await page.getByTestId(`ab-thing-${DOCS}`).click()
    await expect(app).toHaveAttribute('data-group', DOCS)
    await expect(page.getByTestId('ab-screen-readonly')).toBeVisible()
    await expect(sheet.getByTestId('ab-conversation')).toHaveAttribute('data-session', DOCS)
  })

  test('handing the page to the Agent tells it in the conversation where to go on and what to do', async ({ page }) => {
    await openBrowser(page)
    await agentSays(page, DOCS, 'What can I do for you?')
    const before = (await sent(page, DOCS)).length

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
    await expect.poll(async () => (await sent(page, DOCS)).slice(before)).toEqual(['Please continue from the current page.\nPage: example.com\nURL: https://example.com\n\nBook two tickets for Friday'])
    // The default thing had no conversation: one was bound for the message, after the page changed hands.
    const order = await page.evaluate(() => (window as unknown as MockWindow).__agentBrowserMock.rpc.map((call) => call.method).filter((method) => method === 'browser.hand_to_agent' || method === 'browser.bind_default'))
    expect(order).toEqual(['browser.hand_to_agent', 'browser.bind_default'])
    expect(await rpcCalls(page, 'browser.bind_default')).toEqual([{ session_id: DOCS }])

    // The owner sees the message in the conversation.
    await page.getByTestId('ab-chat-toggle').click()
    await expect(page.getByTestId('ab-chat-sheet').getByTestId('conversation-history')).toContainText('Book two tickets for Friday')
    await page.getByTestId('ab-chat-sheet-close').click()

    // Nothing added: the Agent is still told which page to go on from.
    await page.getByTestId('ab-hand-to-agent').click()
    await ask.getByTestId('ab-handover-confirm').click()
    await expect.poll(async () => (await sent(page, DOCS)).at(-1)).toBe('Please continue from the current page.\nPage: example.com\nURL: https://example.com')
    expect(await rpcCalls(page, 'browser.bind_default')).toHaveLength(1)
  })

  test('a password: the Agent waits for the user to take over; handing back sends “Done, please continue.”', async ({ page }) => {
    await openBrowser(page)
    await agentSays(page, DOCS, 'I am logging in to the bank.')
    const before = (await sent(page, DOCS)).length
    const status = page.getByTestId('ab-status')

    await agentSide(page, 'agentAcquire', 'default')
    await agentSide(page, 'agentRequestHandover', 'default', 'password', 'the bank login')
    await expect(status).toHaveAttribute('data-agent-state', 'waiting_takeover')
    await expect(status.getByTestId('ab-status-operator')).toHaveText('CodeAssistant asks you to take over')
    await expect(status.getByTestId('ab-status-agent')).toHaveText('It needs you to enter a password · the bank login')
    // The page is the user's until it is handed back.
    await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-operable', 'true')
    expect(await agentSide(page, 'agentAcquire', 'default')).toBe(false)

    await status.getByTestId('ab-hand-back').click()
    const ask = page.getByTestId('ab-hand-back-sheet')
    await expect(ask).toHaveAccessibleName('Hand back to CodeAssistant')
    await expect(ask.getByTestId('ab-handover-note')).toHaveValue('Done, please continue.')
    expect(await rpcCalls(page, 'browser.hand_back')).toEqual([])
    await ask.getByTestId('ab-handover-confirm').click()
    await expect(status).toHaveAttribute('data-agent-state', 'idle')
    expect(await rpcCalls(page, 'browser.hand_back')).toEqual([{ group_id: 'default' }])
    await expect.poll(async () => (await sent(page, DOCS)).slice(before)).toEqual(['Done, please continue.'])
    expect(await rpcCalls(page, 'browser.bind_default')).toEqual([{ session_id: DOCS }])

    // The message can be changed before it is sent.
    await agentSide(page, 'agentAcquire', 'default')
    await agentSide(page, 'agentRequestHandover', 'default', 'captcha')
    await expect(status.getByTestId('ab-status-agent')).toHaveText('It needs you to pass a verification')
    await status.getByTestId('ab-hand-back').click()
    await ask.getByTestId('ab-handover-note').fill('Solved it, the code was 4711')
    await ask.getByTestId('ab-handover-confirm').click()
    await expect.poll(async () => (await sent(page, DOCS)).slice(before)).toEqual(['Done, please continue.', 'Solved it, the code was 4711'])

    // After taking over on one's own the default differs; an emptied message hands back without waking the Agent.
    await agentSide(page, 'agentAcquire', 'default')
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
    expect((await sent(page, DOCS)).slice(before)).toHaveLength(2)
  })

  test('the handover messages follow the language', async ({ page }) => {
    await page.addInitScript(() => window.localStorage.setItem('buckyos.prototype.locale.v1', 'zh-CN'))
    await openBrowser(page)
    await agentSays(page, DOCS, '我去登录银行。')
    const before = (await sent(page, DOCS)).length
    const status = page.getByTestId('ab-status')

    await page.getByTestId('ab-hand-to-agent').click()
    const hand = page.getByTestId('ab-hand-to-agent-sheet')
    await expect(hand).toHaveAccessibleName('交给 CodeAssistant')
    await hand.getByTestId('ab-handover-note').fill('订两张周五的票')
    await hand.getByTestId('ab-handover-confirm').click()
    await expect.poll(async () => (await sent(page, DOCS)).slice(before)).toEqual(['从当前页面接着做。\n页面：example.com\n网址：https://example.com\n\n订两张周五的票'])

    await agentSide(page, 'agentAcquire', 'default')
    await agentSide(page, 'agentRequestHandover', 'default', 'password', '网银登录')
    await expect(status.getByTestId('ab-status-operator')).toHaveText('CodeAssistant 请你接手')
    await expect(status.getByTestId('ab-status-agent')).toHaveText('需要你输入密码 · 网银登录')
    await status.getByTestId('ab-hand-back').click()
    const back = page.getByTestId('ab-hand-back-sheet')
    await expect(back.getByTestId('ab-handover-note')).toHaveValue('填好了，继续')
    await back.getByTestId('ab-handover-confirm').click()
    await expect.poll(async () => (await sent(page, DOCS)).at(-1)).toBe('填好了，继续')

    await agentSide(page, 'agentAsk', 'default', true)
    await expect(status.getByTestId('ab-status-agent')).toHaveText('CodeAssistant 在等你回答 · 到对话里回答')
    await status.getByTestId('ab-answer').click()
    await expect(page.getByTestId('ab-chat-sheet')).toHaveAccessibleName('和 CodeAssistant 的对话')
    // A thing started in this language is named in it.
    await page.getByTestId('ab-thing-button').click()
    await page.getByTestId('ab-new-thing').click()
    await expect(page.getByTestId('ab-thing-title')).toHaveText(/^浏览器里的事 \d\d-\d\d \d\d:\d\d$/)
  })

  test('a link opens the App on an Agent and a conversation; a question of the Agent is answered in the conversation', async ({ page }) => {
    await page.goto(`/?scenario=normal&agent_browser=${AGENT}&group=${ASKING}`)
    const app = page.getByTestId('agent-browser')
    await expect(app).toHaveAttribute('data-agent', AGENT)
    await expect(app).toHaveAttribute('data-group', ASKING)
    await expect(app).toHaveAttribute('data-mode', 'page')
    await expect(page.getByTestId('ab-agent-picker')).toHaveCount(0)
    await expect(page.getByTestId('ab-thing-title')).toHaveText('Docs Cleanup')
    await expect(page.getByTestId('ab-screen')).toHaveAttribute('data-status', 'open')
    expect(await hellos(page, ASKING)).toBe(1)
    // The link is consumed: a reload does not open the App again.
    expect(new URL(page.url()).search).toBe('?scenario=normal')

    // The service reports the open question of this conversation (DG4): the strip sends the user to the conversation.
    await agentSide(page, 'agentAsk', ASKING, true)
    const status = page.getByTestId('ab-status')
    await expect(status).toHaveAttribute('data-agent-state', 'waiting_answer')
    await expect(status.getByTestId('ab-status-agent')).toHaveText('CodeAssistant is waiting for your answer · answer in the conversation')
    await status.getByTestId('ab-answer').click()
    const sheet = page.getByTestId('ab-chat-sheet')
    await expect(sheet.getByTestId('ab-conversation')).toHaveAttribute('data-session', ASKING)
    // `ask_user` is answered in the composer, as in MessageHub.
    const answer = sheet.getByTestId('composer-answer')
    await expect(answer).toHaveAttribute('data-kind', 'choice')
    await expect(answer.getByTestId('answer-question')).toHaveText('Which docs should I tidy?')
    await answer.getByTestId('answer-option').filter({ hasText: 'docs/ only' }).click()
    await expect(sheet.getByTestId('composer-answer')).toHaveCount(0)
    await expect.poll(async () => (await sent(page, ASKING)).at(-1)).toBe('docs/ only')
    await agentSide(page, 'agentAsk', ASKING, false)
    await expect(status).toHaveAttribute('data-agent-state', 'idle')
    await expect(status.getByTestId('ab-hand-to-agent')).toBeVisible()
    // A named thing's conversation is its own session: nothing is bound to the default group.
    expect(await rpcCalls(page, 'browser.bind_default')).toEqual([])
  })

  test('a link with the Agent’s DID opens conversation mode on the conversation it names', async ({ page }) => {
    await page.goto(`/?scenario=normal&agent_browser=${encodeURIComponent(CODER)}&group=${DOCS}&mode=chat`)
    const app = page.getByTestId('agent-browser')
    await expect(app).toHaveAttribute('data-agent', AGENT)
    await expect(app).toHaveAttribute('data-group', DOCS)
    await expect(app).toHaveAttribute('data-mode', 'chat')
    await expect(page.getByTestId('ab-thing-title')).toHaveText('API Documentation')
    await expect(page.getByTestId('ab-chat-panel').getByTestId('ab-conversation')).toHaveAttribute('data-session', DOCS)
    await expect(page.getByTestId('ab-chat-toggle')).toHaveCount(0)
    expect(new URL(page.url()).search).toBe('?scenario=normal')

    // A conversation that is not there any more: its tabs can still be looked at, and the panel says why nothing can be said.
    await page.goto(`/?scenario=normal&agent_browser=${AGENT}&group=deleted-session&mode=chat`)
    await expect(app).toHaveAttribute('data-group', 'deleted-session')
    await expect(page.getByTestId('ab-conversation-gone')).toContainText('The conversation of this task was archived or deleted.')
    expect(await rpcCalls(page, 'browser.bind_default')).toEqual([])
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
    await agentSide(page, 'agentAcquire', 'default')
    await page.getByTestId('ab-take-over').click()
    await page.getByTestId('ab-hand-back').click()
    await expect(page.getByTestId('ab-status')).toHaveAttribute('data-user-hold', 'false')
    expect(await rpcCalls(page, 'browser.bind_default')).toEqual([])

    await page.getByTestId('ab-chat-toggle').click()
    const unavailable = page.getByTestId('ab-conversation-unavailable')
    await expect(unavailable).toContainText('Your conversations cannot be reached right now.')
    await page.evaluate((owner) => (window as unknown as { __messageHubMock: { revokedOwners: Set<string> } }).__messageHubMock.revokedOwners.delete(owner), SELF)
    await unavailable.getByTestId('ab-conversation-unavailable-retry').click()
    await expect(page.getByTestId('ab-chat-sheet').getByTestId('ab-conversation')).toBeVisible()
    expect(await rpcCalls(page, 'browser.bind_default')).toHaveLength(1)
    // With the conversations back, handing over asks for the message again.
    await page.getByTestId('ab-chat-sheet-close').click()
    await page.getByTestId('ab-hand-to-agent').click()
    await expect(page.getByTestId('ab-hand-to-agent-sheet')).toBeVisible()
  })

  test('MessageHub’s Agent panel says when the Agent uses the browser for the conversation, and opens the App there', async ({ page }) => {
    await page.goto('/?scenario=normal')
    await enterAgent(page)
    // The Agent works in its browser for the conversation "API Documentation".
    await agentSays(page, DOCS, 'Looking it up.')
    await agentSide(page, 'agentAcquire', DOCS)
    await agentSide(page, 'agentOpen', DOCS, 'https://api.example.org')

    await openFromAllApps(page, 'messagehub')
    const hub = page.getByTestId('window-messagehub')
    await expect(hub.getByTestId('conversation-history')).toContainText('Looking it up.')
    await hub.getByTestId('agent-panel-toggle').click()
    const entry = hub.getByTestId('agent-browser-entry')
    await expect(entry).toHaveAttribute('data-state', 'working')
    await expect(entry).toHaveAttribute('data-group', DOCS)
    await expect(entry).toContainText('CodeAssistant is using the browser')
    await entry.getByTestId('agent-browser-entry-open').click()

    const app = page.getByTestId('agent-browser')
    await expect(app).toHaveAttribute('data-agent', AGENT)
    await expect(app).toHaveAttribute('data-group', DOCS)
    await expect(page.getByTestId('ab-thing-title')).toHaveText('API Documentation')
    await expect(page.getByTestId('ab-address-input')).toHaveValue('api.example.org')
    await expect(page.getByTestId('ab-screen-readonly')).toBeVisible()

    // Bound to the default group, the conversation's tabs are the default thing's: the link opens the default thing.
    await agentSide(page, 'agentRelease', DOCS)
    await page.getByTestId('ab-thing-button').click()
    await page.getByTestId('ab-thing-default').click()
    await page.getByTestId('ab-chat-toggle').click()
    await expect.poll(() => rpcCalls(page, 'browser.bind_default')).toEqual([{ session_id: DOCS }])
    await expect(page.getByTestId('ab-address-input')).toHaveValue('api.example.org')
    await page.getByTestId('ab-thing-button').click()
    await expect(page.getByTestId('ab-thing-list').getByRole('button')).toHaveCount(1)
    await page.getByTestId('ab-things-close').click()

    const raise = async (id: string) => {
      const bar = page.getByTestId(`window-drag-${id}`)
      await bar.dispatchEvent('pointerdown', { bubbles: true, button: 0, pointerId: 1, pointerType: 'mouse' })
      await bar.dispatchEvent('pointerup', { bubbles: true, button: 0, pointerId: 1, pointerType: 'mouse' })
      await bar.dispatchEvent('click', { bubbles: true })
    }
    await raise('messagehub')
    await agentSays(page, DOCS, 'Found it.')
    await expect(entry).toHaveAttribute('data-group', 'default')
    await expect(entry).toHaveAttribute('data-state', 'tabs')
    await expect(entry).toContainText('This conversation has pages open in the browser')
    await entry.getByTestId('agent-browser-entry-open').click()
    await expect(app).toHaveAttribute('data-group', 'default')
    await expect(page.getByTestId('ab-thing-title')).toHaveText('General browsing')
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
    await agentSide(page, 'pageFocus', 'default', true)
    await expect.poll(async () => (await mockPage(page))?.focused).toBe(true)
    await page.waitForTimeout(200)
    await expect(input).not.toBeFocused()
    await keyboard.tap()
    await expect(input).toBeFocused()
    await agentSide(page, 'pageFocus', 'default', false)
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
