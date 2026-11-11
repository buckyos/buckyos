import { expect, test, type Page } from '@playwright/test'
import { createNamedSession } from '../fixtures/messagehub'

/**
 * The Agent panel of a conversation with one of the viewer's own Agents
 * (agent-ui-upgrade-todo.md §4.8): its entrances on the desktop and the
 * touch layout, the Agent's summary and preferences, the work tree and its
 * controls, the Turn controls on a placeholder and the composer they block,
 * starting a new conversation, the prompt version and recent deliveries.
 */

declare global {
  interface Window {
    __messageHubMockAgent: typeof import('../../../src/app/messagehub/mock/agentpanel').mockAgentPanel
    __messageHubMockTasks: import('../../../src/app/messagehub/mock/tasks').MockTaskSource
  }
}

const CODER = 'did:buckyos:agent:codeassistant'
const ALICE = 'did:buckyos:person:alice'
const WORK = 'session-coder-work'
const BUILDS_TURN = 'task-turn-work-builds'

async function openHub(page: Page, entityId: string, sessionId?: string) {
  await page.goto(`/messagehub?entityId=${encodeURIComponent(entityId)}${sessionId ? `&sessionId=${sessionId}` : ''}`)
  await expect(page.getByRole('button', { name: 'Sessions', exact: true })).toBeVisible()
}

const calls = (page: Page, method: string) => page.evaluate(name => window.__messageHubMockAgent.calls.filter(call => call.method === name).map(call => call.params), method)
const bubble = (page: Page, id: string) => page.getByTestId('conversation-history').locator(`[data-message-id="${id}"]`)
const confirm = (page: Page, name: string) => page.getByRole('dialog').getByRole('button', { name, exact: true }).click()

async function swipe(page: Page, selector: string, dx: number) {
  await page.locator(selector).evaluate((element, distance) => {
    const box = element.getBoundingClientRect()
    const y = box.top + box.height / 2
    const from = distance < 0 ? box.right - 30 : box.left + 30
    const touch = (x: number) => new Touch({ identifier: 1, target: element, clientX: x, clientY: y })
    element.dispatchEvent(new TouchEvent('touchstart', { bubbles: true, touches: [touch(from)], changedTouches: [touch(from)] }))
    element.dispatchEvent(new TouchEvent('touchmove', { bubbles: true, touches: [touch(from + distance / 2)], changedTouches: [touch(from + distance / 2)] }))
    element.dispatchEvent(new TouchEvent('touchend', { bubbles: true, touches: [], changedTouches: [touch(from + distance)] }))
  }, dx)
}

test('desktop: the Info button opens and closes the panel, the choice is remembered, and it never opens by itself', async ({ page }) => {
  await page.setViewportSize({ width: 1500, height: 900 })
  await openHub(page, CODER, WORK)
  const toggle = page.getByTestId('agent-panel-toggle')
  await expect(toggle).toHaveAttribute('aria-pressed', 'false')
  await expect(page.getByTestId('agent-panel')).toHaveCount(0)
  // Nothing of the Agent is read while the panel is closed.
  expect(await page.evaluate(() => window.__messageHubMockAgent.calls.length)).toBe(0)

  await toggle.click()
  const panel = page.getByTestId('agent-panel')
  await expect(panel).toBeVisible()
  await expect(toggle).toHaveAttribute('aria-pressed', 'true')
  await expect(panel.getByTestId('agent-did')).toHaveText(CODER)
  await expect(panel.getByTestId('agent-conversation-state')).toHaveAttribute('data-state', 'working')
  // Inline beside the conversation, not a drawer over it.
  const box = await panel.boundingBox()
  const history = await page.getByTestId('conversation-history').boundingBox()
  expect(history!.x + history!.width).toBeLessThanOrEqual(box!.x + 1)

  // Left open: it comes back after a reload and in another conversation with the Agent.
  await page.reload()
  await expect(page.getByTestId('agent-panel')).toBeVisible()
  await page.getByTestId('agent-panel-toggle').click()
  await expect(page.getByTestId('agent-panel')).toHaveCount(0)
  await page.reload()
  await expect(page.getByRole('button', { name: 'Sessions', exact: true })).toBeVisible()
  await expect(page.getByTestId('agent-panel')).toHaveCount(0)

  // The picture, the name and the subtitle all lead to the panel; the session's own details are folded inside it.
  await page.getByRole('button', { name: 'Entity details: CodeAssistant' }).click()
  await expect(page.getByTestId('agent-panel')).toBeVisible()
  await page.getByTestId('agent-panel').getByRole('button', { name: 'Close', exact: true }).click()
  await page.getByRole('button', { name: 'Session details', exact: true }).click()
  await expect(page.getByTestId('agent-panel')).toBeVisible()
  await page.getByTestId('agent-panel-session').locator('summary').first().click()
  await expect(page.getByTestId('agent-panel-session').getByTestId('session-details')).toContainText('Members')
  await page.getByTestId('agent-panel-entity').locator('summary').click()
  await expect(page.getByTestId('agent-panel-entity').getByText('Allow manual sessions with this entity')).toBeVisible()
})

test('a conversation with someone who is not the viewer\'s Agent keeps its details and has no Info button', async ({ page }) => {
  await page.setViewportSize({ width: 1500, height: 900 })
  await openHub(page, ALICE)
  await expect(page.getByTestId('agent-panel-toggle')).toHaveCount(0)
  await page.getByRole('button', { name: /^Entity details:/ }).click()
  await expect(page.getByTestId('entity-details')).toBeVisible()
  await expect(page.getByTestId('agent-panel')).toHaveCount(0)
  // The two entity actions that did nothing now mute and pin every session of the entity.
  await page.getByTestId('entity-mute').click()
  await expect(page.getByTestId('entity-mute')).toHaveAttribute('aria-pressed', 'true')
  await expect(page.getByRole('button', { name: /^Entity details:/ }).getByTestId('session-muted')).toBeVisible()
  await page.getByTestId('entity-pin').click()
  await expect(page.getByTestId('entity-pin')).toHaveAttribute('aria-pressed', 'true')
})

test('a narrow desktop window shows the panel as a drawer that Escape closes', async ({ page }) => {
  await page.setViewportSize({ width: 860, height: 800 })
  await openHub(page, CODER, WORK)
  await page.getByTestId('agent-panel-toggle').click()
  const panel = page.getByTestId('agent-panel')
  await expect(panel).toBeVisible()
  const box = await panel.boundingBox()
  const history = await page.getByTestId('conversation-history').boundingBox()
  // Over the conversation: the history keeps its width under it.
  expect(history!.x + history!.width).toBeGreaterThan(box!.x)
  await panel.getByRole('button', { name: 'Close', exact: true }).focus()
  await page.keyboard.press('Escape')
  await expect(panel).toHaveCount(0)
})

test.describe('touch layout', () => {
  test.use({ viewport: { width: 390, height: 844 }, hasTouch: true })

  test('the title opens the panel full screen; a swipe right returns, a swipe left over the history opens it', async ({ page }) => {
    await openHub(page, CODER, WORK)
    await expect(page.getByTestId('agent-panel-toggle')).toBeHidden()
    await page.getByRole('button', { name: 'Entity details: CodeAssistant' }).click()
    const panel = page.getByTestId('agent-panel')
    await expect(panel).toBeVisible()
    const box = await panel.boundingBox()
    expect(Math.round(box!.width)).toBe(390)
    expect(await panel.getByTestId('agent-panel-body').evaluate(element => element.scrollWidth <= element.clientWidth)).toBe(true)

    await swipe(page, '[data-testid="mobile-details"]', 160)
    await expect(panel).toHaveCount(0)
    await expect(page.getByTestId('conversation-history')).toBeVisible()

    // A vertical drag scrolls the list and opens nothing.
    await page.getByTestId('conversation-history').evaluate(element => {
      const box = element.getBoundingClientRect()
      const touch = (y: number) => new Touch({ identifier: 1, target: element, clientX: box.left + 100, clientY: y })
      element.dispatchEvent(new TouchEvent('touchstart', { bubbles: true, touches: [touch(box.top + 300)], changedTouches: [touch(box.top + 300)] }))
      element.dispatchEvent(new TouchEvent('touchend', { bubbles: true, touches: [], changedTouches: [touch(box.top + 60)] }))
    })
    await expect(panel).toHaveCount(0)

    await swipe(page, '[data-testid="conversation-history"]', -160)
    await expect(panel).toBeVisible()
  })

  test('a conversation that is not with an own Agent ignores the swipe', async ({ page }) => {
    await openHub(page, ALICE)
    await swipe(page, '[data-testid="conversation-history"]', -160)
    await expect(page.getByTestId('agent-panel')).toHaveCount(0)
    await expect(page.getByTestId('mobile-details')).toHaveCount(0)
  })
})

test('the upper part shows the Agent\'s summary; do-not-disturb mutes every session and new ones inherit it', async ({ page }) => {
  await page.setViewportSize({ width: 1800, height: 900 })
  await openHub(page, CODER, WORK)
  await page.getByTestId('agent-panel-toggle').click()
  const overview = page.getByTestId('agent-overview')
  await expect(overview.getByTestId('summary-working')).toContainText('3')
  await expect(overview.getByTestId('summary-waiting')).toContainText('1')
  await expect(overview.getByTestId('summary-cognitions')).toContainText('37')
  await expect(overview.getByTestId('summary-perceptions')).toContainText('128')
  await expect(overview.getByTestId('summary-perceptions')).toContainText('6 to consolidate')
  await expect(overview.getByTestId('summary-consolidation')).toContainText('5h')
  await expect(overview.getByTestId('summary-interrupted')).toHaveCount(0)
  expect(await calls(page, 'home.summary')).toHaveLength(1)

  const unread = () => page.getByTestId('entity-list-header').getByText(/^\d+$/).first().textContent()
  const before = Number(await unread())
  await overview.getByTestId('agent-mute').click()
  await expect(overview.getByTestId('agent-mute')).toHaveAttribute('aria-pressed', 'true')
  // The muted Agent no longer counts towards the unread total.
  await expect.poll(async () => Number(await unread())).toBeLessThan(before)
  const muted = page.getByRole('button', { name: 'Entity details: CodeAssistant' }).getByTestId('session-muted')
  await expect(muted).toBeVisible()
  await createNamedSession(page, 'Quiet from the start')
  await expect(page.getByRole('button', { name: 'Session details', exact: true })).toContainText('Quiet from the start')
  await expect(muted).toBeVisible()
  await expect(page.getByTestId('agent-mute')).toHaveAttribute('aria-pressed', 'true')
})

test('the work tree lists the unfinished work with the plan\'s Todos open and what is under a Todo folded; nodes pause, resume and stop', async ({ page }) => {
  await page.setViewportSize({ width: 1500, height: 900 })
  await openHub(page, CODER, WORK)
  await page.getByTestId('agent-panel-toggle').click()
  const tree = page.getByTestId('work-tree')
  await expect(tree.getByTestId('work-card')).toHaveCount(2)
  await expect(tree.locator('[data-session-id="plan-notes"]')).toHaveCount(0)
  await expect(tree.getByTestId('todo-list')).toHaveAttribute('data-expanded', 'true')
  const t02 = tree.locator('[data-session-id="work-docs-t02"]')
  await expect(t02).toBeVisible()
  // What the Todo's session started is folded until asked for.
  await expect(tree.locator('[data-session-id="work-docs-t02-survey"]')).toHaveCount(0)
  await tree.getByTestId('work-branch-toggle').click()
  await expect(tree.locator('[data-session-id="work-docs-t02-survey"]')).toHaveAttribute('data-state', 'running')
  expect(await calls(page, 'session.work_tree')).toContainEqual({ sid: 'ui-session-work', unfinished_only: true })

  // Pausing the plan pauses what is under it; each says why it is paused.
  const plan = tree.locator('[data-session-id="plan-docs"]')
  await plan.getByTestId('work-pause').click()
  await expect(plan).toHaveAttribute('data-paused', 'user')
  await expect(plan.getByTestId('work-card-line')).toHaveText('Paused by you')
  await expect(t02).toHaveAttribute('data-paused', 'parent')
  await expect(t02.getByTestId('work-card-line')).toHaveText('Paused with its parent')
  expect(await calls(page, 'session.pause')).toEqual([{ sid: 'plan-docs' }])
  await plan.getByTestId('work-resume').click()
  await expect(plan).toHaveAttribute('data-state', 'running')
  await expect(t02).toHaveAttribute('data-state', 'running')

  // Stopping asks first; the stopped node leaves the tree and keeps a line below it.
  const builds = tree.locator('[data-session-id="work-builds"]')
  await builds.getByTestId('work-stop').click()
  await expect(page.getByRole('dialog')).toContainText('cannot be resumed')
  await page.getByRole('dialog').getByRole('button', { name: 'Cancel', exact: true }).click()
  await expect(builds).toHaveAttribute('data-state', 'running')
  await builds.getByTestId('work-stop').click()
  await confirm(page, 'Stop')
  await expect(builds).toHaveCount(0)
  await expect(page.getByTestId('work-stopped')).toContainText('Stopped: Run both slow builds')
  expect(await calls(page, 'session.stop')).toEqual([{ sid: 'work-builds', scope: 'session' }])

  // A card opens that session's work log; back returns to the panel as it was.
  await plan.click()
  await expect(page.getByTestId('agent-panel-worklog')).toBeVisible()
  await expect(page.getByTestId('agent-panel-worklog').getByTestId('worklog-row').first()).toBeVisible()
  await page.getByTestId('worklog-back').click()
  await expect(page.getByTestId('agent-panel-worklog')).toHaveCount(0)
  await expect(tree.locator('[data-session-id="work-docs-t02-survey"]')).toBeVisible()
})

test('after a restart the interrupted work is resumed together, and the interrupted message blocks the composer until resumed or stopped', async ({ page }) => {
  await page.setViewportSize({ width: 1500, height: 900 })
  await openHub(page, CODER, WORK)
  await expect(page.getByTestId('composer-blocked')).toHaveAttribute('data-mode', 'running')
  await page.evaluate(session => window.__messageHubMockAgent.interrupt(session), WORK)
  await expect(page.getByTestId('composer-blocked')).toHaveAttribute('data-mode', 'interrupted')
  await expect(page.getByTestId('composer-blocked')).toContainText('restart interrupted')
  const placeholder = bubble(page, 'msg-cw-8')
  await expect(placeholder.getByTestId('message-task')).toHaveAttribute('data-state', 'interrupted')
  await expect(placeholder.getByTestId('turn-interrupted-note')).toBeVisible()
  await expect(placeholder.getByTestId('turn-resume')).toBeVisible()
  await expect(placeholder.getByTestId('turn-retract')).toHaveCount(0)

  await page.getByTestId('agent-panel-toggle').click()
  await expect(page.getByTestId('agent-conversation-state')).toHaveAttribute('data-state', 'interrupted')
  await expect(page.getByTestId('summary-interrupted')).toBeVisible()
  const banner = page.getByTestId('sub-sessions-interrupted')
  await expect(banner).toContainText('4 background task(s) were interrupted')
  await expect(page.locator('[data-testid="work-tree"] [data-session-id="plan-docs"]').getByTestId('work-card-line')).toHaveText('Interrupted by a restart')
  await banner.getByTestId('sub-sessions-resume-all').click()
  await expect(banner).toHaveCount(0)
  // Only the topmost interrupted nodes are resumed; the rest follows them.
  expect(await calls(page, 'session.resume')).toEqual([{ sid: 'plan-docs' }, { sid: 'work-builds' }])
  await expect(page.locator('[data-testid="work-tree"] [data-session-id="work-docs-t02"]')).toHaveAttribute('data-state', 'running')

  // The conversation itself is still interrupted: resumed from its placeholder.
  await placeholder.getByTestId('turn-resume').click()
  await expect(page.getByTestId('composer-blocked')).toHaveAttribute('data-mode', 'running')
  expect(await calls(page, 'session.resume')).toContainEqual({ sid: 'ui-session-work' })
  await expect(page.getByTestId('agent-conversation-state')).toHaveAttribute('data-state', 'working')
})

test('a message being handled blocks sending; stopping it keeps the input, lists the work it started, and frees the composer', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 })
  await openHub(page, CODER, WORK)
  const send = page.getByRole('button', { name: 'Send', exact: true })
  const input = page.getByPlaceholder('Message...')
  await input.fill('One more thing')
  await expect(send).toBeDisabled()
  await input.press('Enter')
  await expect(input).toHaveValue('One more thing')
  const blocked = page.getByTestId('composer-blocked')
  await expect(blocked).toContainText('handling your message')
  const placeholder = bubble(page, 'msg-cw-8')
  await expect(placeholder.getByTestId('task-line-1')).toHaveText('Working · Running shell')
  await expect(placeholder.getByTestId('turn-stop')).toBeVisible()
  await expect(placeholder.getByTestId('turn-retract')).toBeVisible()
  // No pause for the message in front.
  await expect(placeholder.getByText('Pause')).toHaveCount(0)

  await blocked.getByTestId('turn-stop').click()
  await expect(blocked).toHaveCount(0)
  expect(await calls(page, 'session.stop')).toEqual([{ sid: 'ui-session-work', scope: 'turn' }])
  await expect(placeholder.getByTestId('message-task')).toHaveAttribute('data-state', 'canceled')
  await expect(placeholder.getByTestId('turn-controls')).toHaveCount(0)
  // The old input is still there; the next message goes out.
  await expect(bubble(page, 'msg-cw-7')).toContainText('Clean up the deploy scripts.')
  await expect(send).toBeEnabled()
  await send.click()
  await expect(page.locator('.mh-row', { hasText: 'One more thing' })).toBeVisible()
})

test('stopping a message that started work says the work keeps running and stops each on request', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 })
  await openHub(page, CODER, WORK)
  // The Turn of the builds is handled again (running) with its work session under it.
  await page.evaluate(task => {
    window.__messageHubMockTasks.update(task, { phase: 'Running', outcome: undefined, message: 'working (analyze_intent)' })
    window.__messageHubMockAgent.conversation('session-coder-work').openTask = { turn: 6, task_id: task }
  }, BUILDS_TURN)
  const turn = bubble(page, 'msg-cw-6')
  await expect(turn.getByTestId('work-cards')).toBeVisible()
  await turn.getByTestId('turn-stop').click()
  await expect(page.getByRole('dialog')).toContainText('1 background task(s) this message started will keep running')
  await confirm(page, 'Stop')
  await expect(turn.getByTestId('turn-works-running')).toContainText('1 background task(s) this message started are still running')
  const card = turn.locator('[data-session-id="work-builds"]')
  await expect(card).toHaveAttribute('data-state', 'running')
  await card.getByTestId('work-stop').click()
  await confirm(page, 'Stop')
  await expect(card).toHaveAttribute('data-state', 'stopped')
  await expect(turn.getByTestId('turn-works-running')).toHaveCount(0)
  expect(await calls(page, 'session.stop')).toEqual([{ sid: 'ui-session-work', scope: 'turn' }, { sid: 'work-builds', scope: 'session' }])
})

test('stop and retract says what it cannot undo, takes the input out of the conversation and ends the Turn', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 })
  await openHub(page, CODER, WORK)
  const placeholder = bubble(page, 'msg-cw-8')
  await placeholder.getByTestId('turn-retract').click()
  await expect(page.getByRole('dialog')).toContainText('What the agent already did is not undone')
  await confirm(page, 'Stop and retract')
  await expect(page.getByTestId('message-redacted')).toHaveCount(1)
  await expect(page.getByTestId('conversation-history')).not.toContainText('Clean up the deploy scripts.')
  await expect(page.getByTestId('composer-blocked')).toHaveCount(0)
  await expect(placeholder.getByTestId('turn-controls')).toHaveCount(0)
  // The retraction is the redact alone: the Agent stops the Turn on receiving it.
  expect(await calls(page, 'session.stop')).toEqual([])
  // A Turn that already ended offers nothing to retract.
  await expect(bubble(page, 'msg-cw-6').getByTestId('turn-retract')).toHaveCount(0)
})

test('a Turn that only waits for its work leaves the composer free', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 })
  await openHub(page, CODER, 'session-coder-task')
  await expect(bubble(page, 'msg-ct-10').getByTestId('message-task')).toHaveAttribute('data-state', 'waitingChild')
  await expect(page.getByTestId('composer-blocked')).toHaveCount(0)
  await expect(bubble(page, 'msg-ct-10').getByTestId('turn-controls')).toHaveCount(0)
  await page.getByPlaceholder('Message...').fill('Still here?')
  await expect(page.getByRole('button', { name: 'Send', exact: true })).toBeEnabled()
})

test('the answer state\'s Stop closes the asking Turn through the Agent service instead of sending /stop', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 })
  await openHub(page, CODER, 'session-coder-ask')
  await expect(page.getByTestId('composer-blocked')).toHaveCount(0)
  await page.getByTestId('answer-stop').click()
  await expect.poll(() => calls(page, 'session.stop')).toEqual([{ sid: 'ui-session-1', scope: 'turn' }])
  await expect(page.locator('.mh-row', { hasText: '/stop' })).toHaveCount(0)
})

test('"Start a new conversation" is refused while work runs, archives the Agent side otherwise, and leaves a separator that survives a reload', async ({ page }) => {
  await page.setViewportSize({ width: 1500, height: 900 })
  await openHub(page, CODER, WORK)
  await page.getByTestId('agent-panel-toggle').click()
  await expect(page.getByTestId('agent-rotate')).toBeDisabled()
  await expect(page.getByTestId('agent-rotate-note')).toContainText('4 background task(s) are still running')
  await expect(page.getByTestId('agent-generation')).toContainText('#2')

  await openHub(page, CODER, 'session-coder-qa')
  await expect(page.getByTestId('agent-generation')).toContainText('#1')
  await page.getByTestId('agent-rotate').click()
  await expect(page.getByRole('dialog')).toContainText('no message is deleted')
  await confirm(page, 'Start a new conversation')
  await expect(page.getByTestId('agent-conversation-state')).toHaveAttribute('data-state', 'fresh')
  await expect(page.getByTestId('agent-generation')).toContainText('#2')
  await expect(page.getByTestId('agent-rotate')).toBeDisabled()
  expect(await calls(page, 'ui.rotate')).toEqual([{ sid: 'ui-session-coder-qa' }])
  const separator = page.locator('.mh-row', { hasText: 'New conversation started here' })
  await expect(separator).toBeVisible()
  // The messages are all still there, and the menu's "Clear history" is a different action.
  await expect(page.locator('.mh-row', { hasText: 'Draft the release notes' }).first()).toBeVisible()

  await page.getByTestId('agent-panel-toggle').click()
  await page.reload()
  await expect(page.getByRole('button', { name: 'Sessions', exact: true })).toBeVisible()
  await expect(page.locator('.mh-row', { hasText: 'New conversation started here' })).toBeVisible()
})

test('the prompt version sits in the advanced fold and names what changed', async ({ page }) => {
  await page.setViewportSize({ width: 1500, height: 900 })
  await openHub(page, CODER, WORK)
  await page.getByTestId('agent-panel-toggle').click()
  const advanced = page.getByTestId('agent-panel-advanced')
  await expect(advanced.getByTestId('agent-prompt')).toBeHidden()
  await advanced.locator('summary').click()
  await expect(advanced.getByTestId('agent-prompt')).toHaveAttribute('data-current', 'false')
  await expect(advanced.getByTestId('agent-prompt')).toContainText('start a new conversation, or after the next history compaction')
  await expect(advanced.getByTestId('agent-prompt-changed')).toContainText('role.md')

  await openHub(page, CODER, 'session-coder-1')
  await page.getByTestId('agent-panel-advanced').locator('summary').click()
  await expect(page.getByTestId('agent-prompt')).toHaveAttribute('data-current', 'true')
})

test('recent deliveries load only when opened, by conversation or all, page on scroll, and can be forwarded', async ({ page }) => {
  await page.setViewportSize({ width: 1500, height: 900 })
  await openHub(page, CODER, WORK)
  await page.getByTestId('agent-panel-toggle').click()
  await expect(page.getByTestId('agent-conversation-state')).toBeVisible()
  expect(await calls(page, 'deliveries.recent')).toEqual([])
  const section = page.getByTestId('agent-deliveries')
  await section.locator('summary').click()
  const rows = section.getByTestId('delivery-row')
  await expect(rows).toHaveCount(2)
  await expect(rows.first()).toContainText('llm-context-survey.md')
  await expect(rows.first()).toContainText('work report')
  expect(await calls(page, 'deliveries.recent')).toEqual([{ limit: 20, route_key: 'mock-route:session-coder-work' }])

  await section.getByTestId('deliveries-tab-all').click()
  await expect(rows).toHaveCount(20)
  await section.getByTestId('deliveries-list').evaluate(element => { element.scrollTop = element.scrollHeight })
  await expect(rows).toHaveCount(28)
  const pages = await calls(page, 'deliveries.recent')
  expect(pages[1]).toEqual({ limit: 20 })
  expect(pages[2]).toMatchObject({ limit: 20 })
  expect(typeof pages[2].before).toBe('number')

  await rows.first().getByTestId('delivery-menu').click()
  await expect(page.getByTestId('delivery-download')).toBeEnabled()
  // Not carried by a loaded message of this conversation: nothing to locate or collect through.
  await expect(page.getByTestId('delivery-locate')).toBeDisabled()
  await expect(page.getByTestId('delivery-collect')).toBeDisabled()
  await page.getByTestId('delivery-forward').click()
  await expect(page.getByTestId('forward-dialog')).toBeVisible()
  await expect(page.getByTestId('forward-preview')).toContainText('llm-context-survey.md')
})

test('an Agent that does not answer leaves its parts unavailable and the rest of the panel usable', async ({ page }) => {
  await page.setViewportSize({ width: 1500, height: 900 })
  await page.addInitScript(() => localStorage.setItem('buckyos.messagehub.mock.agentDenied', '1'))
  await openHub(page, CODER, WORK)
  await page.getByTestId('agent-panel-toggle').click()
  await expect(page.getByTestId('agent-summary-unavailable')).toBeVisible()
  await expect(page.getByTestId('agent-conversation-unavailable')).toBeVisible()
  await expect(page.getByTestId('agent-new-conversation')).toHaveCount(0)
  await page.getByTestId('agent-pin').click()
  await expect(page.getByTestId('agent-pin')).toHaveAttribute('aria-pressed', 'true')
  await page.getByTestId('agent-panel-session').locator('summary').first().click()
  await expect(page.getByTestId('agent-panel-session').getByTestId('session-details')).toBeVisible()
})

test('the desktop opens Settings on the page a `?settings=` link names and ignores unknown pages', async ({ page }) => {
  await page.goto('/?scenario=normal&settings=agent-runtimes')
  await expect(page.getByTestId('window-settings').getByTestId('settings-agent-runtimes')).toBeVisible()
  await expect(page).not.toHaveURL(/settings=/)
  await page.goto('/?scenario=normal&settings=no-such-page')
  await expect(page.getByTestId('desktop-app-settings')).toBeVisible()
  await expect(page).not.toHaveURL(/settings=/)
  await expect(page.getByTestId('window-settings')).toHaveCount(0)
})
