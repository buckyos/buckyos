import { expect, test, type Page } from '@playwright/test'

/**
 * The goals of one of the viewer's own Agents (opendan-goal-session-improve-todo.md §4.7): the list behind the
 * count in the Agent panel, a goal as a conversation (why it woke, what it did, what it concluded, what it asked
 * and sent), saying something to it, and asking for a new one.
 */

declare global {
  interface Window {
    __mockGoals: typeof import('../../../src/components/conversation/mock/goals').mockGoals
  }
}

const CODER = 'did:buckyos:agent:codeassistant'
const WORK = 'session-coder-work'
const WEEKLY = 'goal-weekly-report'

async function openHub(page: Page, sessionId = WORK) {
  await page.goto(`/messagehub?entityId=${encodeURIComponent(CODER)}&sessionId=${sessionId}`)
  await expect(page.getByRole('button', { name: 'Sessions', exact: true })).toBeVisible()
}

async function openList(page: Page) {
  const toggle = page.getByTestId('agent-panel-toggle')
  if (!await toggle.isVisible()) await page.getByRole('button', { name: 'Entity details: CodeAssistant' }).click()
  else if (await toggle.getAttribute('aria-pressed') === 'false') await toggle.click()
  await page.getByTestId('summary-goals').click()
  await expect(page.getByTestId('goals-live')).toBeVisible()
}

const row = (page: Page, goalId: string) => page.locator(`[data-testid="goal-row"][data-goal="${goalId}"]`)
const view = (page: Page) => page.getByTestId('goal-conversation')
const activation = (page: Page, id: string) => view(page).locator(`[data-testid="goal-activation"][data-activation="${id}"]`)
const calls = (page: Page, method: string) => page.evaluate(name => window.__mockGoals.calls.filter(call => call.method === name).map(call => call.params), method)

async function openGoal(page: Page, goalId: string) {
  await row(page, goalId).click()
  await expect(view(page)).toHaveAttribute('data-goal', goalId)
}

async function say(page: Page, text: string) {
  const box = view(page).getByRole('textbox', { name: 'Say something to this goal' })
  await box.fill(text)
  await box.press('Enter')
}

async function scrollUp(page: Page, dy: number) {
  await view(page).getByTestId('conversation-history').hover()
  await page.mouse.wheel(0, -dy)
  await page.waitForTimeout(300)
}

async function bringIntoView(page: Page, id: string) {
  await expect(view(page).getByTestId('goal-activation').first()).toBeVisible()
  for (let index = 0; index < 12 && !await activation(page, id).count(); index++) await scrollUp(page, 400)
  await activation(page, id).getByTestId('goal-process-toggle').scrollIntoViewIfNeeded()
}

test('the count in the panel opens the list: active first, dormant next, the ended ones folded away', async ({ page }) => {
  await page.setViewportSize({ width: 1500, height: 900 })
  await openHub(page)
  // Nothing of the goals is read before the list is opened.
  await page.getByTestId('agent-panel-toggle').click()
  await expect(page.getByTestId('summary-goals')).toHaveAccessibleName('Goals: 4 active')
  expect(await calls(page, 'goals.list')).toHaveLength(0)
  await page.getByTestId('summary-goals').click()

  await expect(page.getByTestId('goals-live').getByTestId('goal-row-title')).toHaveText(['Weekly project report', 'Personal reminders', 'Electricity price watch', 'Self improvement', 'Keep my sites reachable'])
  const weekly = row(page, WEEKLY)
  await expect(weekly.getByTestId('goal-status')).toHaveText('Active')
  await expect(weekly.getByTestId('goal-row-last')).toHaveAttribute('data-result', 'needs_info')
  await expect(weekly.getByTestId('goal-row-last')).toContainText('Needs you · 1 hour ago')
  await expect(weekly.getByTestId('goal-row-summary')).toContainText('I do not know which day you read it')
  await expect(weekly.getByTestId('goal-row-usage')).toHaveText('128.5k of 1.2M tokens this period')
  await expect(weekly.getByTestId('goal-row-questions')).toHaveText('Waiting for your answer: 2')
  // Without a budget there is nothing to measure against, and a goal held by its budget says until when.
  await expect(row(page, 'goal-reminders').getByTestId('goal-row-usage')).toHaveText('0 tokens this period')
  await expect(row(page, 'goal-reminders').getByTestId('goal-row-questions')).toHaveCount(0)
  await expect(row(page, 'goal-prices').getByTestId('goal-row-hold')).toContainText('It resumes at')
  await expect(row(page, 'goal-self-improve').getByTestId('goal-builtin')).toBeVisible()
  await expect(row(page, 'goal-sites').getByTestId('goal-status')).toHaveText('Dormant')
  await expect(row(page, 'goal-sites').getByTestId('goal-row-last')).toHaveAttribute('data-result', 'failed')

  // The ended ones are only looked at once they are unfolded.
  await expect(page.getByTestId('goals-ended')).toHaveCount(0)
  expect((await calls(page, 'goals.state')).map(params => params.goal_id)).not.toContain('goal-photos')
  await page.getByTestId('goals-ended-toggle').click()
  await expect(page.getByTestId('goals-ended').getByTestId('goal-row-title')).toHaveText(['Move the photos to the new disk', 'Track the parcel'])
  await expect(row(page, 'goal-photos').getByTestId('goal-row-summary')).toContainText('the checksums match')

  await page.getByTestId('goals-back').click()
  await expect(page.getByTestId('goal-list')).toHaveCount(0)
  await expect(page.getByTestId('agent-overview')).toBeVisible()
})

test('a built-in goal the owner switched off says so and comes after the active ones', async ({ page }) => {
  await page.setViewportSize({ width: 1500, height: 900 })
  await openHub(page)
  await page.evaluate(() => { window.__mockGoals.get('goal-self-improve')!.schedule.switched_off = true })
  await openList(page)
  await expect(row(page, 'goal-self-improve').getByTestId('goal-status')).toHaveText('Off')
  await expect(page.getByTestId('goals-live').getByTestId('goal-row-title')).toHaveText(['Weekly project report', 'Personal reminders', 'Electricity price watch', 'Keep my sites reachable', 'Self improvement'])
  await openGoal(page, 'goal-self-improve')
  await expect(page.getByTestId('goal-banner')).toHaveAttribute('data-note', 'off')
})

test('each activation shows why it woke, what it did (folded) and what it concluded', async ({ page }) => {
  await page.setViewportSize({ width: 1500, height: 900 })
  await openHub(page)
  await openList(page)
  await openGoal(page, WEEKLY)
  const last = activation(page, '1.6')
  await expect(last.getByTestId('goal-reason')).toContainText('On schedule')
  await expect(last.getByTestId('goal-reason')).toContainText('2 reviews wait for more than three days')
  await expect(last.getByTestId('goal-result')).toHaveText('Needs you')
  await expect(last.getByTestId('goal-conclusion')).toContainText('The report is ready')
  await expect(last.getByTestId('goal-process-toggle')).toContainText('2m10s · 33.6k tokens')

  // The process is the Turn's work log, read when it is unfolded: the calls, not the header the runtime wrote.
  await expect(view(page).getByTestId('goal-process')).toHaveCount(0)
  await last.getByTestId('goal-process-toggle').click()
  const process = last.getByTestId('goal-process')
  await expect(process.locator('[data-testid="worklog-row"][data-kind="tool"]')).toHaveCount(2)
  await expect(process).toContainText('List what changed')
  await expect(process).not.toContainText('goal_activation')
  await expect(process.locator('[data-kind="user"]')).toHaveCount(0)
  await last.getByTestId('goal-process-toggle').click()
  await expect(process).toHaveCount(0)

  // Its work opens its own log and goes back to the goal.
  await last.getByTestId('work-card').click()
  await expect(page.getByTestId('turn-worklog')).toHaveAttribute('data-session-id', 'work-goal-builds')
  await expect(view(page)).toBeHidden()
  await page.getByTestId('agent-panel-worklog').getByRole('button', { name: 'Close', exact: true }).click()
  await expect(view(page)).toBeVisible()

  // A work session that ended is named; the report is Markdown, folded when it is long.
  await bringIntoView(page, '1.3')
  const report = activation(page, '1.3')
  await expect(report.getByTestId('goal-reason')).toHaveText('“Collect the release notes of week 47” ended')
  await expect(report.getByRole('table')).toBeVisible()
  await expect(report.getByTestId('goal-report-more')).toHaveAttribute('aria-expanded', 'false')
  await report.getByTestId('goal-report-more').click()
  await expect(report.getByTestId('goal-report-more')).toHaveAttribute('aria-expanded', 'true')
  await bringIntoView(page, '1.2')
  await expect(activation(page, '1.2').getByTestId('goal-reason')).toContainText('3 scheduled times together')

  // What was said from another device is read from the opening of its Turn.
  await expect(view(page).getByTestId('goal-own-message').filter({ hasText: 'Please keep it shorter & skip the docs area.' })).toHaveCount(1)
})

test('a short history starts with the goal being set up, and the panel over it tells what it looks after', async ({ page }) => {
  await page.setViewportSize({ width: 1500, height: 900 })
  await openHub(page)
  await openList(page)
  await openGoal(page, WEEKLY)
  const panel = view(page).getByTestId('session-panel')
  await expect(panel.getByTestId('session-panel-fields')).toContainText('128.5k of 1.2M')
  await expect(panel.getByTestId('goal-details')).toHaveCount(0)
  await panel.getByTestId('session-panel-toggle').click()
  await expect(panel.getByTestId('goal-details')).toContainText('the day to send it is still open')
  await expect(panel.getByTestId('goal-details')).toContainText('Compare the build times with last week')
  await page.getByTestId('goal-back').click()
  await openGoal(page, 'goal-reminders')
  await expect(activation(page, '1.1').getByTestId('goal-reason')).toHaveText('The goal was set up')
})

test('its questions and the notifications it sent are messages, and a question is answered in place', async ({ page }) => {
  await page.setViewportSize({ width: 1500, height: 900 })
  await openHub(page)
  await openList(page)
  await openGoal(page, WEEKLY)
  const card = view(page).locator('[data-notify-id="nt-agent-goal"]')
  await expect(card).toHaveCount(1)
  // A question the notification list does not have shows its options and is answered in the box.
  const plain = view(page).getByTestId('goal-question')
  await expect(plain).toHaveCount(1)
  await expect(plain).toContainText('No, only the count')
  await expect(plain).toContainText('Answer in the box below.')
  await card.getByTestId('notify-text').fill('Friday')
  await card.getByTestId('notify-submit').click()
  await expect(card).toHaveAttribute('data-status', 'answered')
  await expect(card.getByTestId('notify-answer')).toHaveText('Answer: Friday')

  await page.getByTestId('goal-back').click()
  await openGoal(page, 'goal-reminders')
  await expect(view(page).locator('[data-notify-id="nt-agent-reminder"]')).toContainText('dentist appointment')
})

test('what is said to a goal is posted, waits visibly, and is answered by an activation', async ({ page }) => {
  await page.setViewportSize({ width: 1500, height: 900 })
  await openHub(page)
  await page.evaluate(() => { window.__mockGoals.wakeMs = 1_500; window.__mockGoals.workMs = 60_000 })
  await openList(page)
  await openGoal(page, WEEKLY)
  await say(page, 'Send it on Friday morning.')
  const own = view(page).getByTestId('goal-own-message').filter({ hasText: 'Send it on Friday morning.' })
  await expect(own).toHaveAttribute('data-pending', 'true')
  await expect(own.getByTestId('goal-post-note')).toHaveText('Waiting to be read when it wakes')
  expect(await calls(page, 'goals.post')).toEqual([{ goal_id: WEEKLY, text: 'Send it on Friday morning.' }])

  // The Activation it opens runs: no more waiting note, its process can be followed.
  const running = activation(page, '1.7')
  await expect(running).toHaveAttribute('data-running', 'true', { timeout: 15_000 })
  await expect(own).not.toHaveAttribute('data-pending')
  await expect(running.getByTestId('goal-reason')).toHaveText('You wrote to it')
  await expect(running.getByTestId('goal-working')).toBeVisible()
  await expect(own).toHaveCount(1)

  await page.evaluate(goalId => window.__mockGoals.finish(goalId, { decision: 'acted', summary: 'It goes out on Fridays at 8:00.' }), WEEKLY)
  await expect(running.getByTestId('goal-conclusion')).toContainText('It goes out on Fridays at 8:00.', { timeout: 15_000 })
  await expect(running).not.toHaveAttribute('data-running')
  await expect(page.getByTestId('goal-title').locator('xpath=..')).toContainText('woke now')
})

test('a message that was not read yet is still there after a reload; one that cannot be sent stays in the box', async ({ page }) => {
  await page.setViewportSize({ width: 1500, height: 900 })
  await openHub(page)
  await page.evaluate(() => { window.__mockGoals.wakeMs = 600_000 })
  await openList(page)
  await openGoal(page, WEEKLY)
  await say(page, 'Are you there?')
  await expect(view(page).getByTestId('goal-own-message').filter({ hasText: 'Are you there?' })).toHaveAttribute('data-pending', 'true')
  await page.reload()
  await page.getByTestId('summary-goals').click()
  await openGoal(page, WEEKLY)
  await expect(view(page).getByTestId('goal-own-message').filter({ hasText: 'Are you there?' })).toHaveAttribute('data-pending', 'true')

  await page.evaluate(goalId => { window.__mockGoals.get(goalId)!.definition.status = 'archived' }, WEEKLY)
  await say(page, 'Too late')
  await expect(view(page).getByRole('alert')).toContainText('goal_conflict')
  await expect(view(page).getByRole('textbox', { name: 'Say something to this goal' })).toHaveValue('Too late')
})

test('a goal held by its budget says when it resumes, and so does what was said to it', async ({ page }) => {
  await page.setViewportSize({ width: 1500, height: 900 })
  await openHub(page)
  await openList(page)
  await openGoal(page, 'goal-prices')
  const banner = page.getByTestId('goal-banner')
  await expect(banner).toHaveAttribute('data-note', 'period_budget')
  await expect(banner).toContainText('The budget of this period is used up. It resumes at')
  await expect(activation(page, '1.2').getByTestId('goal-result')).toHaveText('Ran out of budget')
  await say(page, 'What about tomorrow?')
  const own = view(page).getByTestId('goal-own-message').filter({ hasText: 'What about tomorrow?' })
  await expect(own.getByTestId('goal-post-note')).toContainText('Waiting: the budget is used up, it resumes at')

  await page.evaluate(() => window.__mockGoals.hold('goal-prices', null))
  await page.evaluate(() => document.dispatchEvent(new Event('visibilitychange')))
  await expect(banner).toHaveCount(0)
  await expect(own).not.toHaveAttribute('data-pending')
  await expect(activation(page, '1.3')).toHaveAttribute('data-running', 'true')

  await page.evaluate(() => window.__mockGoals.hold('goal-prices', 'agent_budget'))
  await page.evaluate(() => document.dispatchEvent(new Event('visibilitychange')))
  await expect(banner).toHaveText("The agent's background budget is used up. It resumes with the next period.")
})

test('a dormant goal keeps what is said to it; one that ended has no input box', async ({ page }) => {
  await page.setViewportSize({ width: 1500, height: 900 })
  await openHub(page)
  await openList(page)
  await openGoal(page, 'goal-sites')
  await expect(page.getByTestId('goal-banner')).toHaveAttribute('data-note', 'dormant')
  await expect(activation(page, '1.1').getByTestId('goal-conclusion')).toContainText('the model provider refused the request')
  await say(page, 'Look again when you wake')
  await expect(view(page).getByTestId('goal-post-note')).toHaveText('Waiting: the goal is dormant')
  await page.getByTestId('goal-back').click()

  await page.getByTestId('goals-ended-toggle').click()
  for (const [id, text] of [['goal-photos', 'This goal was achieved.'], ['goal-parcel', 'This goal is archived.']]) {
    await openGoal(page, id)
    await expect(view(page).getByTestId('composer-readonly')).toHaveText(text)
    await expect(view(page).getByRole('textbox')).toHaveCount(0)
    await page.getByTestId('goal-back').click()
  }
})

test('nothing is read while the page is hidden, or of what is covered', async ({ page }) => {
  await page.setViewportSize({ width: 1500, height: 900 })
  await openHub(page)
  await openList(page)
  await openGoal(page, WEEKLY)
  await page.evaluate(goalId => window.__mockGoals.open(goalId, [{ kind: 'manual', key: 'm1', note: 'activated by self' }]), WEEKLY)
  await page.evaluate(() => document.dispatchEvent(new Event('visibilitychange')))
  await expect(activation(page, '1.7')).toHaveAttribute('data-running', 'true')
  await expect(activation(page, '1.7').getByTestId('goal-reason')).toHaveText('Started by hand')
  const total = () => page.evaluate(() => window.__mockGoals.calls.length)
  const lists = (await calls(page, 'goals.list')).length

  const hide = (hidden: boolean) => page.evaluate(value => {
    Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => value ? 'hidden' : 'visible' })
    document.dispatchEvent(new Event('visibilitychange'))
  }, hidden)
  await hide(true)
  await page.waitForTimeout(500)
  const before = await total()
  await page.waitForTimeout(6_500)
  expect(await total()).toBe(before)
  await hide(false)
  await expect.poll(total).toBeGreaterThan(before)
  // The list lies under the goal.
  expect((await calls(page, 'goals.list')).length).toBe(lists)
  // Under a work log the goal rests too.
  await activation(page, '1.6').getByTestId('work-card').click()
  await expect(page.getByTestId('turn-worklog')).toBeVisible()
  await page.waitForTimeout(500)
  const covered = (await calls(page, 'goals.state')).length
  await page.waitForTimeout(6_000)
  expect((await calls(page, 'goals.state')).length).toBe(covered)
})

test('older activations are loaded while scrolling back', async ({ page }) => {
  await page.setViewportSize({ width: 1500, height: 900 })
  await openHub(page)
  await page.evaluate(() => {
    const goal = window.__mockGoals.get('goal-self-improve')!
    const first = goal.activations[0]
    goal.activations = Array.from({ length: 70 }, (_, index) => ({ ...first, seq: index + 1, started_at_ms: Date.now() - (70 - index) * 3_600_000, ended_at_ms: Date.now() - (70 - index) * 3_600_000 + 30_000, reasons: [{ kind: 'scheduled', key: `s${index}` }], summary: `Look ${index + 1}` }))
    goal.state.last_activation = { ...goal.activations[69], at_ms: goal.activations[69].ended_at_ms }
  })
  await openList(page)
  await openGoal(page, 'goal-self-improve')
  await expect(activation(page, '1.70')).toBeVisible()
  expect((await calls(page, 'goals.activations')).map(params => (params.query as { limit: number }).limit)).toEqual([30])
  for (let index = 0; index < 40 && !await activation(page, '1.1').count(); index++) await scrollUp(page, 1_500)
  await expect(activation(page, '1.1')).toBeVisible()
  expect((await calls(page, 'goals.activations')).map(params => (params.query as { limit: number }).limit)).toContain(90)
})

test('"New goal" opens the default conversation with the request begun, and keeps a draft', async ({ page }) => {
  await page.setViewportSize({ width: 1500, height: 900 })
  await openHub(page)
  await openList(page)
  await page.getByTestId('goal-new').click()
  await expect(page.getByTestId('goal-list')).toHaveCount(0)
  const box = page.getByRole('textbox', { name: 'Message', exact: true })
  await expect(box).toHaveValue('Set up a goal and look after this for me: ')
  await expect(page.locator('[aria-label="Session details"]')).not.toHaveText('Work Sessions')

  // The default conversation is the open one now: what is typed there stays, and the request is begun under it.
  await box.fill('Something I was writing')
  await page.waitForTimeout(500)
  await openList(page)
  await page.getByTestId('goal-new').click()
  await expect(box).toHaveValue('Something I was writing\nSet up a goal and look after this for me: ')
  await openList(page)
  await page.getByTestId('goal-new').click()
  await expect(box).toHaveValue('Something I was writing\nSet up a goal and look after this for me: ')
  expect(await calls(page, 'goals.post')).toHaveLength(0)
})

test('an Agent that does not answer leaves the list unavailable until it does', async ({ page }) => {
  await page.setViewportSize({ width: 1500, height: 900 })
  await openHub(page)
  await page.evaluate(() => { window.__mockGoals.denied = true })
  await page.getByTestId('agent-panel-toggle').click()
  await page.getByTestId('summary-goals').click()
  await expect(page.getByTestId('goals-unavailable')).toBeVisible()
  await page.evaluate(() => { window.__mockGoals.denied = false })
  await page.getByTestId('goals-unavailable').getByRole('button', { name: 'Retry' }).click()
  await expect(page.getByTestId('goals-live')).toBeVisible()
})

test.describe('on a phone', () => {
  test.use({ viewport: { width: 390, height: 844 }, hasTouch: true })

  test('the list and a goal fit the screen, with targets a finger hits, and back goes one step', async ({ page }) => {
    await openHub(page)
    await openList(page)
    const fits = (testId: string) => page.getByTestId(testId).first().evaluate(element => element.scrollWidth <= element.clientWidth && element.getBoundingClientRect().right <= window.innerWidth)
    const noSideScroll = () => page.evaluate(() => document.scrollingElement!.scrollWidth <= window.innerWidth)
    const tappable = async (testId: string) => {
      for (const element of await page.getByTestId(testId).all()) {
        const box = await element.boundingBox()
        if (box) expect(Math.min(box.width, box.height), testId).toBeGreaterThanOrEqual(43.5)
      }
    }
    expect(Math.round((await page.getByTestId('goal-list').boundingBox())!.width)).toBe(390)
    expect(await fits('goal-list-body')).toBe(true)
    expect(await noSideScroll()).toBe(true)
    for (const id of ['goal-row', 'goal-new', 'goals-back', 'goals-ended-toggle']) await tappable(id)

    await openGoal(page, WEEKLY)
    const box = (await view(page).boundingBox())!
    expect([Math.round(box.x), Math.round(box.width)]).toEqual([0, 390])
    const history = view(page).getByTestId('conversation-history')
    await expect(view(page).locator('[data-notify-id="nt-agent-goal"]')).toBeVisible()
    expect(await history.evaluate(element => element.scrollWidth <= element.clientWidth)).toBe(true)
    for (const id of ['goal-back', 'goal-process-toggle']) await tappable(id)
    // The long report has a table and the process has output: both stay inside.
    await bringIntoView(page, '1.3')
    await activation(page, '1.3').getByTestId('goal-report-more').tap()
    await activation(page, '1.3').getByTestId('goal-process-toggle').tap()
    await expect(activation(page, '1.3').getByTestId('goal-process').getByTestId('worklog-row').first()).toBeVisible()
    for (const id of ['goal-process', 'goal-conclusion']) expect(await activation(page, '1.3').getByTestId(id).evaluate(element => element.scrollWidth <= element.clientWidth && element.getBoundingClientRect().right <= window.innerWidth)).toBe(true)
    expect(await history.evaluate(element => element.scrollWidth <= element.clientWidth)).toBe(true)
    expect(await noSideScroll()).toBe(true)

    // A swipe to the right goes one step back: the goal, its list, the Agent panel, the conversation.
    const swipe = () => page.getByTestId('mobile-details').evaluate(element => {
      const rect = element.getBoundingClientRect()
      const touch = (x: number) => new Touch({ identifier: 1, target: element, clientX: x, clientY: rect.top + 20 })
      element.dispatchEvent(new TouchEvent('touchstart', { bubbles: true, touches: [touch(rect.left + 30)], changedTouches: [touch(rect.left + 30)] }))
      element.dispatchEvent(new TouchEvent('touchend', { bubbles: true, touches: [], changedTouches: [touch(rect.left + 230)] }))
    })
    await swipe()
    await expect(view(page)).toHaveCount(0)
    await expect(page.getByTestId('goal-list')).toBeVisible()
    await swipe()
    await expect(page.getByTestId('goal-list')).toHaveCount(0)
    await expect(page.getByTestId('agent-overview')).toBeVisible()
    await swipe()
    await expect(page.getByTestId('agent-panel')).toHaveCount(0)
    await expect(page.getByTestId('conversation-history')).toBeVisible()
  })

  test('a goal is spoken to and a new one is asked for from the phone', async ({ page }) => {
    await openHub(page)
    await page.evaluate(() => { window.__mockGoals.wakeMs = 300; window.__mockGoals.workMs = 300 })
    await openList(page)
    await openGoal(page, WEEKLY)
    await say(page, 'Friday it is')
    await expect(activation(page, '1.7').getByTestId('goal-conclusion')).toContainText('Noted: Friday it is', { timeout: 15_000 })
    await expect(activation(page, '1.7').getByTestId('goal-process-toggle')).toBeInViewport()
    // Unfolding the last activation keeps its head in view.
    await activation(page, '1.7').getByTestId('goal-process-toggle').tap()
    await expect(activation(page, '1.7').getByTestId('goal-process').getByTestId('worklog-row').first()).toBeVisible()
    await page.waitForTimeout(600)
    await expect(activation(page, '1.7').getByTestId('goal-process-toggle')).toBeInViewport()
    await page.getByTestId('goal-back').click()
    await page.getByTestId('goal-new').tap()
    await expect(page.getByTestId('mobile-details')).toHaveCount(0)
    await expect(page.getByRole('textbox', { name: 'Message', exact: true })).toHaveValue('Set up a goal and look after this for me: ')
  })
})
