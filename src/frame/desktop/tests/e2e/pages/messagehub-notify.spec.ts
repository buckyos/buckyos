import { expect, test, type Page } from '@playwright/test'

/**
 * System Notify in MessageHub (mock): the System notifications entity with
 * its three category sessions, notification cards handled in place, the
 * `?notify=` link, copies in an Agent conversation, and the Task Center
 * leaving notification tasks to MessageHub.
 */

declare global {
  interface Window {
    __messageHubMock: import('../../../src/app/messagehub/mock/store').MessageHubMockStore
    __messageHubMockNotify: import('../../../src/app/messagehub/mock/notify').MockNotifyBackend
  }
}

const SELF = 'did:buckyos:user:self'
const OWN = { viewerDid: SELF, ownerDid: SELF, mode: 'self' as const }
const SENDER = 'did:web:zone.buckyos.mock'
const CODER = 'did:buckyos:agent:codeassistant'
const AGENT_SESSION = 'session-coder-notify'
const SHOTS = 'test-results/messagehub-notify'

const card = (page: Page, id: string) => page.locator(`[data-testid="notify-card"][data-notify-id="${id}"]`)
const cardIds = (page: Page) => page.getByTestId('notify-card').evaluateAll(cards => cards.map(item => item.getAttribute('data-notify-id')))

async function openSession(page: Page, sessionId: string, entityId = SENDER) {
  await page.goto(`/messagehub?entityId=${encodeURIComponent(entityId)}&sessionId=${encodeURIComponent(sessionId)}`)
  await expect(page.getByTestId('conversation-history')).toBeVisible()
}

async function sessionRows(page: Page) {
  const toggle = page.getByRole('button', { name: 'Sessions', exact: true })
  if (!(await page.getByTestId('session-sidebar').isVisible())) await toggle.click()
  return page.getByTestId('session-row').evaluateAll(rows => rows.map(row => row.getAttribute('data-session-id')))
}

test('the System notifications entity holds the three category sessions in order, with filters and no composer', async ({ page }) => {
  await page.goto(`/messagehub?entityId=${encodeURIComponent(SENDER)}`)
  await expect(page.getByTestId('notify-filter')).toBeVisible()
  // It opens on the most important category; the sessions keep the category order whatever their activity.
  expect(await sessionRows(page)).toEqual(['sysnotify:todo', 'sysnotify:agent', 'sysnotify:app'])
  await expect(page.locator('[data-session-id="sysnotify:todo"] button[aria-current="true"]')).toBeVisible()
  await expect(page.locator('[data-session-id="sysnotify:agent"]')).toContainText('Agent')
  await expect(page.locator('[data-session-id="sysnotify:app"]')).toContainText('App notifications')
  // The unread counts are the pending hints: 1 + 1 + 2.
  const unread = await page.evaluate(context => window.__messageHubMock.sessions(context).filter(session => session.entityId === 'did:web:zone.buckyos.mock').map(session => [session.id, session.unreadCount]), OWN)
  expect(unread).toEqual([['sysnotify:todo', 1], ['sysnotify:agent', 1], ['sysnotify:app', 2]])
  expect(await page.evaluate(context => window.__messageHubMock.entities(context).find(entity => entity.systemNotify)?.unreadCount, OWN)).toBe(4)
  await expect(page.getByRole('button', { name: /^System notifications/ }).first()).toBeVisible()
  // Not a conversation: no composer, no read-only footer, no new session.
  await expect(page.getByTestId('message-composer')).toHaveCount(0)
  await expect(page.getByTestId('composer-readonly')).toHaveCount(0)

  await page.locator('[data-session-id="sysnotify:agent"] button').first().click()
  await expect(card(page, 'nt-agent-work')).toHaveAttribute('data-status', 'open')
  expect(await cardIds(page)).toEqual(['nt-agent-timeout', 'nt-agent-tests', 'nt-agent-goal', 'nt-agent-reminder', 'nt-agent-work'])
  const work = card(page, 'nt-agent-work')
  await expect(work.getByTestId('notify-source')).toHaveText('CodeAssistant · Work: Deploy the docs preview')
  await expect(work.getByTestId('notify-level')).toHaveText('Timely')
  await expect(work.getByTestId('notify-status')).toHaveText('Awaiting your answer')
  await expect(card(page, 'nt-agent-goal').getByTestId('notify-source')).toHaveText('CodeAssistant · Goal: Weekly project report')
  await expect(card(page, 'nt-agent-reminder').getByTestId('notify-status')).toHaveText('New')
  await expect(card(page, 'nt-agent-timeout').getByTestId('notify-result')).toContainText('Withdrawn by the sender · no answer in time')
  await expect(card(page, 'nt-agent-tests').getByTestId('notify-result')).toContainText('Answer: Full suite')
  await expect(card(page, 'nt-agent-tests').getByTestId('notify-result-meta')).toContainText('by CodeAssistant for You')

  const filter = page.getByTestId('notify-filter')
  await filter.getByRole('button', { name: 'Pending' }).click()
  await expect.poll(() => cardIds(page)).toEqual(['nt-agent-goal', 'nt-agent-reminder', 'nt-agent-work'])
  await filter.getByRole('button', { name: 'Handled' }).click()
  await expect.poll(() => cardIds(page)).toEqual(['nt-agent-timeout', 'nt-agent-tests'])
  await expect(filter.getByRole('button', { name: 'Handled' })).toHaveAttribute('aria-pressed', 'true')
  // A card answered while "Pending" is shown leaves the pending list.
  await filter.getByRole('button', { name: 'Pending' }).click()
  await expect.poll(() => cardIds(page)).toHaveLength(3)
  await card(page, 'nt-agent-work').getByTestId('notify-option').filter({ hasText: 'main' }).click()
  await expect.poll(() => cardIds(page)).toEqual(['nt-agent-goal', 'nt-agent-reminder'])
  await filter.getByRole('button', { name: 'All' }).click()
  await expect(card(page, 'nt-agent-work')).toHaveAttribute('data-status', 'answered')

  await page.locator('[data-session-id="sysnotify:todo"] button').first().click()
  await filter.getByRole('button', { name: 'Handled' }).click()
  await expect.poll(() => cardIds(page)).toEqual(['nt-todo-contract'])
  await filter.getByRole('button', { name: 'Pending' }).click()
  await expect.poll(() => cardIds(page)).toEqual(['nt-todo-budget'])
})

test('a card answers by option and by text, dismisses an input, and acknowledges or dismisses a notice', async ({ page }) => {
  await openSession(page, 'sysnotify:agent')
  const work = card(page, 'nt-agent-work')
  await expect(work).toHaveAttribute('data-status', 'open')
  await work.getByTestId('notify-option').filter({ hasText: 'release-2.4' }).click()
  await expect(work).toHaveAttribute('data-status', 'answered')
  await expect(work.getByTestId('notify-answer')).toHaveText('Answer: release-2.4')
  await expect(work.getByTestId('notify-result-meta')).toContainText('by You')
  await expect(work.getByTestId('notify-controls')).toHaveCount(0)
  // The task result is exactly `NotifyAnswer::to_task_result`.
  const result = await page.evaluate(() => window.__messageHubMockNotify.view('nt-agent-work').result)
  expect(result).toMatchObject({ action: 'answer', values: { option: 'release' }, submitted_by: 'self' })

  const goal = card(page, 'nt-agent-goal')
  await expect(goal.getByTestId('notify-option')).toHaveCount(0)
  await expect(goal.getByTestId('notify-submit')).toBeDisabled()
  await goal.getByTestId('notify-text').fill('Monday morning, before standup')
  await goal.getByTestId('notify-submit').click()
  await expect(goal).toHaveAttribute('data-status', 'answered')
  await expect(goal.getByTestId('notify-answer')).toHaveText('Answer: Monday morning, before standup')
  expect(await page.evaluate(() => window.__messageHubMockNotify.view('nt-agent-goal').result?.values)).toEqual({ text: 'Monday morning, before standup' })

  // An input can be dismissed: the task closes with action `dismiss`.
  await openSession(page, 'sysnotify:todo')
  const todo = card(page, 'nt-todo-budget')
  await todo.getByTestId('notify-dismiss').click()
  await expect(todo).toHaveAttribute('data-status', 'dismissed')
  await expect(todo.getByTestId('notify-result')).toContainText('Dismissed')
  expect(await page.evaluate(() => window.__messageHubMockNotify.view('nt-todo-budget').result?.action)).toBe('dismiss')

  // A notice: "Got it" marks it read, "Dismiss" closes it.
  await openSession(page, 'sysnotify:app')
  const backup = card(page, 'nt-app-backup')
  await expect(backup.getByTestId('notify-link')).toHaveAttribute('href', 'https://www.buckyos.ai/')
  // A failed write keeps the card as it was, says so, and can be tried again.
  await page.evaluate(() => window.__messageHubMockNotify.configure({ failNext: 'offline' }))
  await backup.getByTestId('notify-ack').click()
  await expect(backup.getByTestId('notify-error')).toHaveText('Could not submit. Try again.')
  await expect(backup).toHaveAttribute('data-status', 'open')
  await backup.getByTestId('notify-ack').click()
  await expect(backup).toHaveAttribute('data-status', 'read')
  await expect(backup.getByTestId('notify-status')).toHaveText('Read')
  await expect(backup.getByTestId('notify-ack')).toHaveCount(0)
  await backup.getByTestId('notify-dismiss').click()
  await expect(backup).toHaveAttribute('data-status', 'dismissed')
  await expect(backup.getByTestId('notify-result')).toContainText('Dismissed')
  await expect(card(page, 'nt-app-import').getByTestId('notify-result')).toContainText('Expired without an answer')
})

test('two cards of one notification submitted at once: one answer wins, the other card shows the final result', async ({ context }) => {
  // Two tabs of one browser stand for two devices: they share the scripted msg-center / TaskMgr.
  const home = await context.newPage()
  await openSession(home, 'sysnotify:agent')
  const copy = await context.newPage()
  await openSession(copy, AGENT_SESSION, CODER)
  const homeCard = card(home, 'nt-agent-work'), copyCard = card(copy, 'nt-agent-work')
  await expect(homeCard).toHaveAttribute('data-home', 'true')
  await expect(copyCard).toHaveAttribute('data-home', 'false')
  await expect(homeCard).toHaveAttribute('data-status', 'open')
  await expect(copyCard).toHaveAttribute('data-status', 'open')
  // Both read the task before either commits; the home card commits first.
  await home.evaluate(() => window.__messageHubMockNotify.configure({ commitDelayMs: 300 }))
  await copy.evaluate(() => window.__messageHubMockNotify.configure({ commitDelayMs: 1_200 }))
  await Promise.all([
    homeCard.getByTestId('notify-option').filter({ hasText: 'main' }).click(),
    copyCard.getByTestId('notify-option').filter({ hasText: 'release-2.4' }).click(),
  ])
  for (const item of [homeCard, copyCard]) {
    await expect(item).toHaveAttribute('data-status', 'answered')
    await expect(item.getByTestId('notify-answer')).toHaveText('Answer: main')
  }
  await expect(copyCard.getByTestId('notify-conflict')).toBeVisible()
  await expect(homeCard.getByTestId('notify-conflict')).toHaveCount(0)
  expect(await copy.evaluate(() => window.__messageHubMockNotify.view('nt-agent-work').result?.values)).toEqual({ option: 'main' })

  // Answered on one device, the other card follows (the task's change is the hint to read again).
  await expect(card(home, 'nt-agent-reminder')).toHaveAttribute('data-status', 'open')
  await copy.locator('[data-testid="notify-card"][data-notify-id="nt-agent-reminder"]').getByTestId('notify-ack').click()
  await expect(card(home, 'nt-agent-reminder')).toHaveAttribute('data-status', 'read')
})

test('an Agent conversation with notification copies keeps its composer typeable', async ({ page }) => {
  await openSession(page, AGENT_SESSION, CODER)
  await expect(card(page, 'nt-agent-work')).toBeVisible()
  await expect(card(page, 'nt-agent-reminder')).toBeVisible()
  const composer = page.getByTestId('message-composer')
  await expect(composer).toBeVisible()
  await expect(page.getByTestId('composer-answer')).toHaveCount(0)
  const input = composer.locator('textarea')
  await expect(input).toBeEditable()
  // Answering the card leaves the composer alone, and the conversation still takes messages.
  await card(page, 'nt-agent-work').getByTestId('notify-option').filter({ hasText: 'main' }).click()
  await expect(card(page, 'nt-agent-work')).toHaveAttribute('data-status', 'answered')
  await expect(page.getByTestId('composer-answer')).toHaveCount(0)
  await input.fill('Thanks, go ahead.')
  await input.press('Enter')
  await expect(page.getByTestId('conversation-history')).toContainText('Thanks, go ahead.')
  await expect(input).toBeEditable()
})

test('?notify= opens the card on the desktop and on a phone; an unknown notification says so', async ({ page }) => {
  await page.goto('/messagehub?notify=nt-agent-goal')
  await expect(card(page, 'nt-agent-goal')).toBeVisible()
  expect(await sessionRows(page)).toContain('sysnotify:agent')
  await expect(page.locator('[data-session-id="sysnotify:agent"] button[aria-current="true"]')).toBeVisible()
  // The card is handled in place: the message details do not open.
  await expect(page.getByTestId('message-details-pane')).toHaveCount(0)
  await expect(page.getByTestId('notify-link-failed')).toHaveCount(0)

  await page.goto('/messagehub?notify=nt-does-not-exist')
  await expect(page.getByTestId('notify-link-failed')).toBeVisible()
  await expect(page.getByRole('button', { name: 'Sessions', exact: true })).toBeVisible()

  // A phone (a Telegram link): straight into the conversation, card ready to answer.
  await page.setViewportSize({ width: 390, height: 844 })
  await page.goto('/messagehub?notify=nt-app-sync')
  const sync = card(page, 'nt-app-sync')
  await expect(sync).toBeVisible()
  await expect(page.getByTestId('notify-filter')).toBeVisible()
  await expect(page.getByTestId('message-composer')).toHaveCount(0)
  await sync.getByTestId('notify-option').filter({ hasText: 'Keep both' }).click()
  await expect(sync).toHaveAttribute('data-status', 'answered')
  await page.screenshot({ path: `${SHOTS}/phone-light.png` })
})

test('Task Center: no approve / reject card for a notification task; list and detail link to its MessageHub card', async ({ page }) => {
  await page.goto('/taskcenter')
  const notifications = page.locator('section', { has: page.getByRole('heading', { name: /System Notifications/ }) })
  await expect(notifications).toContainText('Remove 3.2 GB of temporary files?')
  await expect(notifications).not.toContainText('Which branch should I deploy the docs preview from?')

  await page.goto('/taskcenter?taskid=task-nt-agent-work')
  const hint = page.getByTestId('task-notify')
  await expect(hint).toBeVisible()
  await expect(hint.getByTestId('task-notify-link')).toHaveAttribute('href', '/messagehub?notify=nt-agent-work')
  await expect(page.getByTestId('task-notify')).toHaveCount(1)
  await page.goto('/taskcenter?taskid=task-input-cleanup')
  await expect(page.getByRole('heading', { name: 'Confirm the temp file cleanup' })).toBeVisible()
  await expect(page.getByTestId('task-notify')).toHaveCount(0)

  await page.getByRole('button', { name: /Back to Tasks/ }).click()
  const link = page.getByTestId('task-notify-link')
  await expect(link).toHaveCount(1)
  await expect(link).toHaveAttribute('href', '/messagehub?notify=nt-agent-work')
  await link.click()
  await expect(card(page, 'nt-agent-work')).toBeVisible()
  await expect(page).toHaveURL(/\/messagehub\?notify=nt-agent-work$/)
})

for (const theme of ['light', 'dark'] as const) {
  test(`notification cards in the ${theme} theme`, async ({ page }) => {
    await page.addInitScript(value => window.localStorage.setItem('buckyos.prototype.theme.v1', value), theme)
    await page.setViewportSize({ width: 1400, height: 900 })
    await openSession(page, 'sysnotify:agent')
    await expect(card(page, 'nt-agent-work')).toHaveAttribute('data-status', 'open')
    await expect(card(page, 'nt-agent-tests')).toHaveAttribute('data-status', 'answered')
    await page.getByTestId('notify-filter').getByRole('button', { name: 'Pending' }).hover()
    await page.screenshot({ path: `${SHOTS}/agent-${theme}.png` })
    await openSession(page, AGENT_SESSION, CODER)
    await expect(card(page, 'nt-agent-work')).toBeVisible()
    await page.screenshot({ path: `${SHOTS}/copy-${theme}.png` })
    await openSession(page, 'sysnotify:app')
    await expect(card(page, 'nt-app-sync')).toHaveAttribute('data-status', 'open')
    await page.screenshot({ path: `${SHOTS}/app-${theme}.png` })
    await page.goto('/taskcenter?taskid=task-nt-agent-work')
    await expect(page.getByTestId('task-notify')).toBeVisible()
    await page.screenshot({ path: `${SHOTS}/taskcenter-${theme}.png` })
  })
}

test('Chinese labels: 系统通知, the category sessions, filters and card actions', async ({ page }) => {
  await page.addInitScript(() => window.localStorage.setItem('buckyos.prototype.locale.v1', 'zh-CN'))
  await page.setViewportSize({ width: 1400, height: 900 })
  await openSession(page, 'sysnotify:app')
  await expect(page.getByRole('button', { name: /^系统通知/ }).first()).toBeVisible()
  const filter = page.getByTestId('notify-filter')
  await expect(filter.getByRole('button')).toHaveText(['全部', '待处理', '已处理'])
  const backup = card(page, 'nt-app-backup')
  await expect(backup.getByTestId('notify-ack')).toHaveText('知道了')
  await expect(backup.getByTestId('notify-dismiss')).toHaveText('忽略')
  await expect(backup.getByTestId('notify-status')).toHaveText('新通知')
  await expect(card(page, 'nt-app-sync').getByTestId('notify-status')).toHaveText('待回答')
  await expect(card(page, 'nt-app-import').getByTestId('notify-result')).toContainText('已过期，未回答')
  await page.getByRole('button', { name: '会话列表', exact: true }).click()
  await expect(page.getByTestId('session-row').locator('button[aria-current]')).toBeVisible()
  expect(await page.getByTestId('session-row').evaluateAll(rows => rows.map(row => row.querySelector('button span span')?.textContent))).toEqual(['待办', 'Agent', '应用通知'])
  await page.screenshot({ path: `${SHOTS}/app-zh-light.png` })
})
