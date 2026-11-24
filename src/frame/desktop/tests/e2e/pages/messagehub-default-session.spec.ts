import { expect, test, type Page } from '@playwright/test'
import { closeSessionList, createNamedSession, openNewSession, openSessionDetails } from '../fixtures/messagehub'

/**
 * An Agent's default session (`notepads/jarvis-chat-route-todo.md` §2, D1 / D2 / Q6): its main entry, `/new`, and
 * leaving a channel. The mock Agent has a Telegram conversation in its own mailbox.
 */

declare global {
  interface Window { __messageHubMock: import('../../../src/app/messagehub/mock/store').MessageHubMockStore }
}

const SELF = 'did:buckyos:user:self'
const OWN = { viewerDid: SELF, ownerDid: SELF, mode: 'self' as const }
const CODER = 'did:buckyos:agent:codeassistant'
const ALICE = 'did:buckyos:person:alice'
const TELEGRAM = 'tg:agent-bot:4201'
const MARKS = 'buckyos.conversation.mock.sources.v1'

async function openHub(page: Page, entityId = CODER) {
  await page.goto(`/messagehub?entityId=${encodeURIComponent(entityId)}`)
  await expect(page.getByRole('button', { name: 'Sessions', exact: true })).toBeVisible()
}

const sessionList = async (page: Page) => {
  if (!await page.getByTestId('session-sidebar').isVisible()) await page.getByRole('button', { name: 'Sessions', exact: true }).click()
  return page.getByTestId('session-sidebar')
}
const row = (page: Page, id: string) => page.locator(`[data-testid="session-row"][data-session-id="${id}"]`)
const current = (page: Page) => page.evaluate(() => document.querySelector('[data-testid="session-row"] [aria-current="true"]')?.closest('[data-session-id]')?.getAttribute('data-session-id') ?? null)
const stored = (page: Page) => page.evaluate(({ context, agent }) => window.__messageHubMock.defaultSession(context, agent)?.id ?? null, { context: OWN, agent: CODER })
const latest = (page: Page) => page.evaluate(({ context, agent }) => [...window.__messageHubMock.sessions(context, agent)].sort((a, b) => b.createdAt - a.createdAt)[0].id, { context: OWN, agent: CODER })
const count = (page: Page) => page.evaluate(({ context, agent }) => window.__messageHubMock.sessions(context, agent).length, { context: OWN, agent: CODER })
const setDefault = (page: Page, id: string) => page.evaluate(async ({ context, agent, id }) => {
  const store = window.__messageHubMock
  await store.setDefaultSession(context, [...store.sessions(context, agent), ...store.channelSessions(context, agent)].find(session => session.id === id)!)
}, { context: OWN, agent: CODER, id })
const enterAgain = async (page: Page) => {
  await page.getByRole('button', { name: /^Alice Chen/ }).first().click()
  await page.getByRole('button', { name: /^CodeAssistant/ }).first().click()
}
const send = async (page: Page, text: string) => {
  await page.locator('textarea').fill(text)
  await page.locator('textarea').press('Enter')
}
const sentTexts = (page: Page, id: string) => page.evaluate(async ({ context, id }) => {
  const reader = window.__messageHubMock.reader(context, id)
  return (await reader.readRange(0, reader.totalCount)).filter(message => message.from === context.ownerDid).map(message => message.content.content)
}, { context: OWN, id })

test.use({ viewport: { width: 1440, height: 900 } })

test('never set: the main entry is the earliest chat, whatever was active last, and it stays', async ({ page }) => {
  await openHub(page)
  const list = await sessionList(page)
  await expect.poll(() => current(page)).toBe('session-coder-1')
  await expect(row(page, 'session-coder-1').getByTestId('session-badge')).toHaveText('Default')
  await expect(list.getByTestId('session-badge')).toHaveCount(1)
  await page.evaluate(({ agent, self }) => window.__messageHubMock.simulateIncoming('session-coder-2', { from: agent, to: [self], kind: 'chat', content: { content: 'Latest news' } }), { agent: CODER, self: SELF })
  await enterAgain(page)
  await sessionList(page)
  await expect.poll(() => current(page)).toBe('session-coder-1')
  // Chosen for the viewer, it is the default like one they set.
  expect(await stored(page)).toBe('session-coder-1')
  const details = await openSessionDetails(page)
  await expect(details.getByTestId('default-session')).toHaveAttribute('data-default', 'yes')
  await expect(details.getByRole('button', { name: 'Set as default', exact: true })).toHaveCount(0)
})

test('a session set as default stays the main entry while others and an App session are more recent', async ({ page }) => {
  await openHub(page)
  await sessionList(page)
  await row(page, 'session-coder-2').getByRole('button').first().click()
  const details = await openSessionDetails(page)
  await expect(details.getByTestId('default-session')).toHaveAttribute('data-default', 'no')
  await details.getByRole('button', { name: 'Set as default', exact: true }).click()
  await expect(details.getByTestId('default-session')).toHaveAttribute('data-default', 'yes')
  await expect(details.getByRole('button', { name: 'Set as default', exact: true })).toHaveCount(0)
  await expect(row(page, 'session-coder-2').getByTestId('session-badge')).toHaveText('Default')
  await details.getByRole('button', { name: 'Close', exact: true }).click()
  await createNamedSession(page, 'App chat')
  const app = await latest(page)
  await page.evaluate(({ key, mark, app }) => localStorage.setItem(key, JSON.stringify({ [mark]: app })), { key: MARKS, mark: `${SELF}\n${app}`, app: 'agent-browser' })
  await enterAgain(page)
  await sessionList(page)
  await expect.poll(() => current(page)).toBe('session-coder-2')
  for (const id of ['session-coder-qa', app]) {
    await row(page, id).getByRole('button').first().click()
    await expect.poll(() => current(page)).toBe(id)
  }
  await page.reload()
  await sessionList(page)
  await expect.poll(() => current(page)).toBe('session-coder-2')
})

test('a first session an App started is not the main entry', async ({ page }) => {
  await page.addInitScript(({ key, mark }) => localStorage.setItem(key, JSON.stringify({ [mark]: 'agent-browser' })), { key: MARKS, mark: `${SELF}\nsession-coder-1` })
  await openHub(page)
  await sessionList(page)
  await expect.poll(() => current(page)).not.toBe('session-coder-1')
  await expect.poll(() => current(page)).not.toBeNull()
})

test('New Session sets the default and the memory mode', async ({ page }) => {
  await openHub(page)
  await openNewSession(page)
  const dialog = page.getByRole('dialog', { name: 'New Session: CodeAssistant' })
  await dialog.getByLabel('Session name', { exact: true }).fill('Session D')
  await dialog.getByLabel('Set as default session', { exact: true }).check()
  await dialog.getByRole('combobox', { name: 'Memory', exact: true }).selectOption({ label: 'No writing' })
  await expect(dialog.getByText('is kept in long-term memory', { exact: false })).toBeVisible()
  await page.screenshot({ path: 'test-results/messagehub-new-session-default.png' })
  await dialog.getByRole('button', { name: 'Create', exact: true }).click()
  await expect(dialog).toHaveCount(0)
  const created = await latest(page)
  expect(await stored(page)).toBe(created)
  const details = await openSessionDetails(page)
  await expect(details.getByTestId('default-session')).toHaveAttribute('data-default', 'yes')
  await expect(details.getByText('No writing', { exact: true })).toBeVisible()
  await page.screenshot({ path: 'test-results/messagehub-default-details.png' })
  await details.getByRole('button', { name: 'Close', exact: true }).click()
  await enterAgain(page)
  await sessionList(page)
  await expect.poll(() => current(page)).toBe(created)
  // People and groups have neither.
  await closeSessionList(page)
  await page.getByRole('button', { name: /^Alice Chen/ }).first().click()
  await openNewSession(page)
  await expect(page.getByRole('dialog').getByLabel('Set as default session', { exact: true })).toHaveCount(0)
  await expect(page.getByRole('dialog').getByRole('combobox', { name: 'Memory', exact: true })).toHaveCount(0)
})

test('/new in a chat the viewer started opens a new default chat and is not sent; /clean is', async ({ page }) => {
  await openHub(page)
  await createNamedSession(page, 'Session A')
  const a = await latest(page)
  await setDefault(page, a)
  const before = await count(page)
  await send(page, '/clean')
  await expect.poll(() => sentTexts(page, a)).toEqual(['/clean'])
  expect(await count(page)).toBe(before)
  await send(page, ' /new ')
  await expect.poll(() => count(page)).toBe(before + 1)
  const e = await latest(page)
  await expect.poll(() => stored(page)).toBe(e)
  await sessionList(page)
  await expect.poll(() => current(page)).toBe(e)
  await expect(row(page, a)).toBeVisible()
  expect(await sentTexts(page, a)).toEqual(['/clean'])
  expect(await sentTexts(page, e)).toEqual([])
  await expect(page.locator('textarea')).toHaveValue('')
  // Not the bare command: an ordinary message.
  await send(page, '/new plan')
  await expect.poll(() => sentTexts(page, e)).toEqual(['/new plan'])
  expect(await count(page)).toBe(before + 1)
})

test('/new goes out as it is in a session an App started, in one that was not created here and to a person', async ({ page }) => {
  await openHub(page)
  await createNamedSession(page, 'App chat')
  const app = await latest(page)
  await page.evaluate(({ key, mark }) => localStorage.setItem(key, JSON.stringify({ [mark]: 'agent-browser' })), { key: MARKS, mark: `${SELF}\n${app}` })
  // The mark shows with the next change of the conversation.
  await page.locator('textarea').fill('x')
  await send(page, '/new')
  await expect.poll(() => sentTexts(page, app)).toEqual(['/new'])
  const before = await count(page)
  await sessionList(page)
  await row(page, 'session-coder-2').getByRole('button').first().click()
  await send(page, '/new')
  await expect.poll(() => sentTexts(page, 'session-coder-2').then(texts => texts.at(-1))).toBe('/new')
  expect(await count(page)).toBe(before)
})

test('a default on Telegram opens read-only from the main entry and is followed there', async ({ page }) => {
  await openHub(page)
  await setDefault(page, TELEGRAM)
  await enterAgain(page)
  await sessionList(page)
  await expect.poll(() => current(page)).toBe(TELEGRAM)
  await expect(row(page, TELEGRAM).getByTestId('session-badge')).toHaveText('Default')
  await expect(page.getByTestId('composer-readonly')).toContainText('another platform')
  const history = page.getByTestId('conversation-history')
  await expect(history.getByText('Remind me about the release on Friday.')).toBeVisible()
  // Seen from the viewer's side: the Agent's reply is the peer's, with its Markdown.
  await expect(history.locator('strong', { hasText: 'Friday at 9:00' })).toBeVisible()
  await expect(history.locator('.mh-row', { hasText: 'Remind me about' }).locator('.justify-end').first()).toBeVisible()
  await expect(history.locator('.mh-row', { hasText: 'I will remind you here' }).locator('.justify-start').first()).toBeVisible()
  await page.evaluate(({ agent, id, from }) => window.__messageHubMock.injectMessage(agent, id, { from, to: [agent], kind: 'chat', created_at_ms: Date.now(), ui_message_id: 'msg-tg-live', content: { content: 'One more thing from my phone.' } } as never), { agent: CODER, id: TELEGRAM, from: 'did:msgtunnel:4201.user.tg-agent-bot' })
  await expect(history.getByText('One more thing from my phone.')).toBeVisible()
  await page.screenshot({ path: 'test-results/messagehub-default-telegram.png' })
  await expect(row(page, TELEGRAM).getByRole('button', { name: /^Manage session/ })).toHaveCount(0)
  const details = await openSessionDetails(page)
  await expect(details.getByTestId('default-session')).toHaveAttribute('data-default', 'yes')
  await expect(details.getByRole('button', { name: 'Manage session', exact: true })).toBeDisabled()
  // Elsewhere the viewer can write: a forward goes to a native session of the Agent.
  expect(await page.evaluate(({ context, agent }) => window.__messageHubMock.ensureDefaultSession(context, agent, { writable: true }).then(session => session!.id), { context: OWN, agent: CODER })).toBe('session-coder-1')
})

test('leaving a Telegram default asks first: creating, /new and Set as default', async ({ page }) => {
  await openHub(page)
  await setDefault(page, TELEGRAM)
  const leave = page.getByTestId('leave-channel')
  const create = async (name: string) => {
    await openNewSession(page)
    const dialog = page.getByRole('dialog', { name: 'New Session: CodeAssistant' })
    await dialog.getByLabel('Session name', { exact: true }).fill(name)
    await dialog.getByLabel('Set as default session', { exact: true }).check()
    await dialog.getByRole('button', { name: 'Create', exact: true }).click()
    await expect(leave).toContainText('Telegram no longer receives system notifications from CodeAssistant')
  }
  // Kept: the session is created, Telegram stays the default.
  await create('Session B')
  await page.screenshot({ path: 'test-results/messagehub-leave-channel.png' })
  await leave.getByRole('button', { name: 'Keep Telegram', exact: true }).click()
  await expect(page.getByRole('dialog')).toHaveCount(0)
  const b = await latest(page)
  await closeSessionList(page)
  await sessionList(page)
  await expect.poll(() => current(page)).toBe(b)
  expect(await stored(page)).toBe(TELEGRAM)
  // The same with /new in that chat.
  await send(page, '/new')
  await leave.getByRole('button', { name: 'Keep Telegram', exact: true }).click()
  await expect.poll(() => latest(page)).not.toBe(b)
  const c = await latest(page)
  await expect.poll(() => current(page)).toBe(c)
  expect(await stored(page)).toBe(TELEGRAM)
  // "Set as default" does nothing when kept, and changes the default when confirmed.
  const details = await openSessionDetails(page)
  await details.getByRole('button', { name: 'Set as default', exact: true }).click()
  await leave.getByRole('button', { name: 'Keep Telegram', exact: true }).click()
  await expect(details.getByTestId('default-session')).toHaveAttribute('data-default', 'no')
  expect(await stored(page)).toBe(TELEGRAM)
  await details.getByRole('button', { name: 'Set as default', exact: true }).click()
  await leave.getByRole('button', { name: 'Change default', exact: true }).click()
  await expect.poll(() => stored(page)).toBe(c)
  // Away from a session of the viewer's own nothing is asked; back to Telegram neither.
  await details.getByRole('button', { name: 'Close', exact: true }).click()
  await row(page, TELEGRAM).getByRole('button').first().click()
  await (await openSessionDetails(page)).getByRole('button', { name: 'Set as default', exact: true }).click()
  await expect.poll(() => stored(page)).toBe(TELEGRAM)
  await details.getByRole('button', { name: 'Close', exact: true }).click()
  // Confirmed while creating.
  await create('Session D')
  await leave.getByRole('button', { name: 'Change default', exact: true }).click()
  await expect(page.getByRole('dialog')).toHaveCount(0)
  await expect.poll(async () => await stored(page) === await latest(page)).toBe(true)
})

test('an archived default is restored from the main entry instead of replaced', async ({ page }) => {
  await openHub(page)
  await setDefault(page, 'session-coder-2')
  await page.evaluate(context => window.__messageHubMock.manage(context, 'session-coder-2', 'archive'), OWN)
  const before = await count(page)
  await enterAgain(page)
  await sessionList(page)
  await expect.poll(() => current(page)).toBe('session-coder-2')
  expect(await page.evaluate(({ context, agent }) => window.__messageHubMock.sessions(context, agent, 'active').some(session => session.id === 'session-coder-2'), { context: OWN, agent: CODER })).toBe(true)
  expect(await count(page)).toBe(before)
  // Archived while it is open: another session shows, and the default waits for the main entry.
  await page.locator('[data-session-id="session-coder-2"]').getByRole('button', { name: /^Manage session/ }).click()
  await page.getByRole('dialog').getByRole('button', { name: 'Archive', exact: true }).click()
  await expect(page.getByRole('dialog')).toHaveCount(0)
  await expect.poll(() => current(page)).not.toBe('session-coder-2')
  await expect.poll(() => current(page)).not.toBeNull()
})

test('a deleted default is replaced by a new chat the next time the main entry is entered, again and again', async ({ page }) => {
  await openHub(page)
  await expect.poll(() => stored(page)).toBe('session-coder-1')
  const others = async () => (await page.evaluate(({ context, agent }) => window.__messageHubMock.sessions(context, agent).map(session => session.id), { context: OWN, agent: CODER })).sort()
  const deleteSession = async (id: string) => {
    await sessionList(page)
    await row(page, id).getByRole('button', { name: /^Manage session/ }).click()
    await page.getByRole('dialog').getByRole('button', { name: 'Delete permanently', exact: true }).click()
    await expect(page.getByRole('dialog')).toHaveCount(0)
  }
  const rest = (await others()).filter(id => id !== 'session-coder-1')
  await deleteSession('session-coder-1')
  // Another session shows; nothing replaces the default until it is asked for.
  await expect.poll(() => current(page)).not.toBeNull()
  expect(rest).toContain(await current(page))
  expect([await stored(page), await others()]).toEqual([null, rest])
  await expect(page.getByTestId('session-sidebar').getByTestId('session-badge')).toHaveCount(0)
  let previous = ''
  for (let round = 0; round < 2; round++) {
    await enterAgain(page)
    await expect(page.locator('textarea')).toBeVisible()
    await sessionList(page)
    const created = await latest(page)
    expect(rest).not.toContain(created)
    expect(created).not.toBe(previous)
    await expect.poll(() => current(page)).toBe(created)
    await expect(row(page, created).getByTestId('session-badge')).toHaveText('Default')
    expect([await stored(page), await others()]).toEqual([created, [...rest, created].sort()])
    await page.reload()
    await sessionList(page)
    await expect.poll(() => current(page)).toBe(created)
    previous = created
    await deleteSession(created)
    expect(await others()).toEqual(rest)
  }
})

test('deleting the default with nothing left opens a new chat at once', async ({ page }) => {
  await openHub(page)
  await setDefault(page, 'session-coder-2')
  await page.evaluate(async ({ context, agent }) => {
    const store = window.__messageHubMock
    for (const session of store.sessions(context, agent)) if (session.id !== 'session-coder-2') await store.manage(context, session.id, 'delete')
  }, { context: OWN, agent: CODER })
  await sessionList(page)
  await row(page, 'session-coder-2').getByRole('button').first().click()
  await row(page, 'session-coder-2').getByRole('button', { name: /^Manage session/ }).click()
  await page.getByRole('dialog').getByRole('button', { name: 'Delete permanently', exact: true }).click()
  await expect(page.getByRole('dialog')).toHaveCount(0)
  await expect.poll(() => count(page)).toBe(1)
  const created = await latest(page)
  await expect.poll(() => current(page)).toBe(created)
  await expect(page.locator('textarea')).toBeVisible()
  expect(await stored(page)).toBe(created)
})

test('phone: a Telegram default opens read-only and the list fits', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 780 })
  await openHub(page, ALICE)
  await setDefault(page, TELEGRAM)
  await page.getByRole('button', { name: 'Back to conversation list', exact: true }).click()
  await page.getByRole('button', { name: /^CodeAssistant/ }).first().click()
  await expect(page.getByTestId('composer-readonly')).toContainText('another platform')
  await sessionList(page)
  await expect(row(page, TELEGRAM)).toBeVisible()
  await page.screenshot({ path: 'test-results/messagehub-default-phone.png' })
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
})
