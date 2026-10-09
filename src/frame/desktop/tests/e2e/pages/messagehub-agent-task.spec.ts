import { expect, test, type Page } from '@playwright/test'

/**
 * Agent replies that carry `agent_task`: the placeholder becomes the final
 * reply in the same bubble, the bubble shows the Turn's task (two lines, the
 * tree when expanded), and every message has details addressed by its anchor.
 */

declare global {
  interface Window {
    __messageHubMock: import('../../../src/app/messagehub/mock/store').MessageHubMockStore
    __messageHubMockTasks: import('../../../src/app/messagehub/mock/tasks').MockTaskSource
    __messageHubTaskWatch: import('../../../src/app/messagehub/conversation/tasks/taskWatch').TaskWatch
  }
}

const SELF = 'did:buckyos:user:self'
const OWN = { viewerDid: SELF, ownerDid: SELF, mode: 'self' as const }
const CODER = 'did:buckyos:agent:codeassistant'
const SESSION = 'session-coder-task'
const RUNNING = 'task-turn-checklist'

async function openTaskSession(page: Page, extra = '') {
  await page.goto(`/messagehub?entityId=${encodeURIComponent(CODER)}&sessionId=${SESSION}${extra}`)
  await expect(page.getByTestId('conversation-history')).toBeVisible()
  await expect(bubble(page, 'msg-ct-10')).toBeVisible()
}

const bubble = (page: Page, id: string) => page.getByTestId('conversation-history').locator(`[data-message-id="${id}"]`)
const anchors = (page: Page) => page.getByTestId('conversation-history').locator('[data-message-id]').evaluateAll(rows => rows.map(row => row.getAttribute('data-message-id')))
const unread = (page: Page) => page.evaluate(({ context, session }) => window.__messageHubMock.sessions(context).find(item => item.id === session)?.unreadCount, { context: OWN, session: SESSION })

test('a placeholder and its final edit are one bubble at the anchor; quick replies and unreadable tasks show no task area', async ({ page }) => {
  await openTaskSession(page)
  expect(await anchors(page)).toEqual(['msg-ct-1', 'msg-ct-2', 'msg-ct-3', 'msg-ct-4', 'msg-ct-5', 'msg-ct-6', 'msg-ct-7', 'msg-ct-8', 'msg-ct-9', 'msg-ct-10'])

  // The final edit replaced text, format and attachments of the placeholder.
  const report = bubble(page, 'msg-ct-2')
  await expect(report.locator('strong', { hasText: '3 failures' })).toBeVisible()
  await expect(report.getByText('design-brief.pdf')).toBeVisible()
  await expect(report.getByText('Got it, working on it')).toHaveCount(0)
  await expect(report.getByTestId('edited-marker')).toBeVisible()
  await expect(report.getByTestId('message-task')).toHaveAttribute('data-state', 'succeeded')
  await expect(report.getByTestId('task-line-2')).toContainText('2 sub-task(s) finished')

  // A quick reply (successful task without sub-tasks) and a task the viewer may not read: body only.
  await expect(bubble(page, 'msg-ct-4')).toContainText('09:41 UTC')
  await expect(bubble(page, 'msg-ct-4').getByTestId('message-task')).toHaveCount(0)
  await expect(bubble(page, 'msg-ct-8')).toContainText('another workspace')
  await expect(bubble(page, 'msg-ct-8').getByTestId('message-task')).toHaveCount(0)

  const failed = bubble(page, 'msg-ct-6')
  await expect(failed).toContainText('over quota')
  await expect(failed.getByTestId('message-task')).toHaveAttribute('data-state', 'failed')
  await expect(failed.getByTestId('task-line-1')).toContainText('Preview bucket quota exceeded')

  // The running Turn: placeholder body plus the default two lines, no task id, no percentage.
  const live = bubble(page, 'msg-ct-10')
  await expect(live).toContainText('Got it, working on it')
  await expect(live.getByTestId('message-task')).toHaveAttribute('data-state', 'waitingChild')
  await expect(live.getByTestId('task-line-1')).toHaveText('Waiting for sub-tasks · Checking the message projection')
  await expect(live.getByTestId('task-line-2')).toContainText('1 sub-task(s) running · 1 sub-task(s) waiting · Waiting for the UI test run · updated')
  await expect(live).not.toContainText(RUNNING)
  await expect(live).not.toContainText('%')
  await page.screenshot({ path: 'test-results/messagehub-agent-task-bubbles.png' })

  // Only tasks that are still open are subscribed, and nothing below the first level was read.
  const state = await page.evaluate(() => ({ subscriptions: window.__messageHubMockTasks.subscriptions(), events: window.__messageHubMockTasks.reads.listEvents }))
  expect(state).toEqual({ subscriptions: [`task:${RUNNING}`], events: 0 })
})

test('progress only refreshes the task area; the final edit replaces the body in place and is not a second unread message', async ({ page }) => {
  await openTaskSession(page)
  const live = bubble(page, 'msg-ct-10')
  const before = await unread(page)
  await page.evaluate(id => window.__messageHubMockTasks.update(id, { phase: 'Running', waitReason: undefined, message: 'Running the UI tests' }), RUNNING)
  await expect(live.getByTestId('task-line-1')).toHaveText('Working · Running the UI tests')
  await expect(live).toContainText('Got it, working on it')
  expect(await anchors(page)).toHaveLength(10)
  expect(await unread(page)).toBe(before)

  // A stop request is "stopping", never "canceled", until the runner says so.
  await page.evaluate(id => window.__messageHubMockTasks.update(id, { pendingControl: 'Cancel' }), RUNNING)
  await expect(live.getByTestId('message-task')).toHaveAttribute('data-state', 'stopping')
  await expect(live.getByTestId('message-task')).not.toContainText('Canceled')
  await page.evaluate(id => window.__messageHubMockTasks.update(id, { pendingControl: undefined }), RUNNING)

  // The event is lost: the bounded poll still picks the change up.
  await page.evaluate(id => window.__messageHubMockTasks.update(id, { message: 'Polled, not pushed' }, true), RUNNING)
  await expect(live.getByTestId('task-line-1')).toContainText('Polled, not pushed', { timeout: 15_000 })

  // The task finishes before the final edit arrives.
  await page.evaluate(id => {
    const tasks = window.__messageHubMockTasks
    for (const child of ['explore', 'tests', 'tests.shard', 'docs']) tasks.update(`${id}.${child}`, { phase: 'Terminal', outcome: 'Succeeded', waitReason: undefined })
    tasks.update(id, { phase: 'Terminal', outcome: 'Succeeded', message: 'All checks pass' })
  }, RUNNING)
  await expect(live.getByTestId('message-task')).toHaveAttribute('data-state', 'replySyncing')
  await expect(live.getByTestId('task-line-1')).toContainText('Done, syncing the reply')
  await expect.poll(() => page.evaluate(() => window.__messageHubMockTasks.subscriptions())).toEqual([])

  await page.evaluate(session => window.__messageHubMock.simulateIncoming(session, { content: { format: 'text/markdown', content: '**All 12 checks pass.**' }, relates_to: { rel: 'edit', target: 'msg-ct-10' }, agent_task: { task_id: 'task-turn-hijack' } }), SESSION)
  await expect(live.locator('strong', { hasText: 'All 12 checks pass.' })).toBeVisible()
  await expect(live.getByText('Got it, working on it')).toHaveCount(0)
  await expect(live.getByTestId('message-task')).toHaveAttribute('data-state', 'succeeded')
  expect(await anchors(page)).toHaveLength(10)
  expect(await unread(page)).toBe(before)
  // The session preview reads the effective content.
  expect(await page.evaluate(({ context, session }) => window.__messageHubMock.sessions(context).find(item => item.id === session)?.lastMessage?.text, { context: OWN, session: SESSION })).toBe('**All 12 checks pass.**')

  // A late answer about the old state cannot bring "working" back: the terminal snapshot is final.
  await page.evaluate(id => window.__messageHubTaskWatch.refresh(id), RUNNING)
  await expect(live.getByTestId('message-task')).toHaveAttribute('data-state', 'succeeded')

  // A new placeholder followed by its edit counts as one unread message.
  await page.evaluate(async session => {
    await window.__messageHubMock.simulateIncoming(session, { ui_message_id: 'msg-ct-12', content: { format: 'text/plain', content: 'Got it, working on it…' }, agent_task: { task_id: 'task-turn-clock' } })
    await window.__messageHubMock.simulateIncoming(session, { content: { format: 'text/plain', content: 'Second answer.' }, relates_to: { rel: 'edit', target: 'msg-ct-12' } })
  }, SESSION)
  await expect(bubble(page, 'msg-ct-12')).toContainText('Second answer.')
  expect(await unread(page)).toBe((before ?? 0) + 1)
})

test('expanding a bubble shows the Turn task tree level by level and releases it on collapse', async ({ page }) => {
  await openTaskSession(page)
  const live = bubble(page, 'msg-ct-10')
  await live.getByTestId('task-expand').click()
  const tree = live.getByTestId('task-tree')
  await expect(tree.getByTestId('task-node')).toHaveCount(3)
  await expect(tree.locator(`[data-task-id="${RUNNING}.tests"]`)).toContainText('Work session: run UI tests')
  await expect(tree.locator(`[data-task-id="${RUNNING}.tests"]`)).toContainText('Working · Running 42 Playwright specs')
  await expect(tree.locator(`[data-task-id="${RUNNING}.docs"]`)).toContainText('Waiting for an external dependency · Queued behind another edit · Waiting for the doc lock')
  await expect(tree.locator(`[data-task-id="${RUNNING}.explore"]`)).toHaveAttribute('data-state', 'succeeded')
  // The details did not open: the expand button is its own control.
  await expect(page.getByTestId('message-details-pane')).toHaveCount(0)
  expect(await page.evaluate(() => window.__messageHubMockTasks.subscriptions())).toEqual([`task:${RUNNING}`, `tree:${RUNNING}`])

  await tree.locator(`[data-task-id="${RUNNING}.tests"]`).getByTestId('task-node-expand').first().click()
  await expect(tree.locator(`[data-task-id="${RUNNING}.tests.shard"]`)).toContainText('exec_bash: playwright test')
  // A tree event refreshes a loaded node.
  await page.evaluate(id => window.__messageHubMockTasks.update(`${id}.tests.shard`, { message: 'messagehub-agent-task.spec.ts' }), RUNNING)
  await expect(tree.locator(`[data-task-id="${RUNNING}.tests.shard"]`)).toContainText('messagehub-agent-task.spec.ts')
  await page.screenshot({ path: 'test-results/messagehub-agent-task-tree.png' })

  await live.getByTestId('task-expand').click()
  await expect(live.getByTestId('task-tree')).toHaveCount(0)
  expect(await page.evaluate(() => window.__messageHubMockTasks.subscriptions())).toEqual([`task:${RUNNING}`])
})

test('message details open by click, by the Details action and by keyboard, and append the live task', async ({ page }) => {
  await openTaskSession(page)
  const details = page.getByTestId('message-details-pane')

  await bubble(page, 'msg-ct-10').getByText('Got it, working on it').click()
  await expect(details).toHaveAttribute('data-message-id', 'msg-ct-10')
  await details.getByTestId('detail-tab-message').click()
  await expect(details.getByTestId('detail-message-id')).toHaveText('msg-ct-10')
  await expect(details.getByTestId('detail-original-same')).toBeVisible()
  await expect(details.getByTestId('detail-task-id')).toHaveText(RUNNING)
  await expect(details.getByTestId('detail-task-summary')).toHaveAttribute('data-state', 'waitingChild')
  await expect(details.getByTestId('task-node')).toHaveCount(3)
  await expect(details.getByTestId('detail-task-events')).toContainText('TaskCreated')
  // The task section is live.
  await page.evaluate(id => window.__messageHubMockTasks.update(id, { phase: 'Running', waitReason: undefined, message: 'Almost there' }), RUNNING)
  await expect(details.getByTestId('detail-task-summary')).toContainText('Working · Almost there')
  await expect(details.getByTestId('detail-task-events')).toContainText('PhaseChanged')
  await details.getByRole('button', { name: 'Close' }).click()
  await expect(details).toHaveCount(0)
  await expect.poll(() => page.evaluate(() => window.__messageHubMockTasks.subscriptions())).toEqual([`task:${RUNNING}`])

  // The explicit action; the edited message keeps its original, its edit record and the anchor's task.
  const report = bubble(page, 'msg-ct-2')
  await report.hover()
  await report.getByTestId('message-hover-bar').getByTestId('message-details').click()
  await expect(details).toHaveAttribute('data-message-id', 'msg-ct-2')
  await details.getByTestId('detail-tab-message').click()
  await expect(details.getByTestId('detail-effective')).toHaveAttribute('data-format', 'text/markdown')
  await expect(details.getByTestId('detail-effective')).toContainText('design-brief.pdf')
  await expect(details.getByTestId('detail-original')).toContainText('Got it, working on it')
  await expect(details.getByTestId('detail-edits').locator('li')).toHaveCount(1)
  await expect(details.getByTestId('detail-delivery')).toBeVisible()
  await expect(details.getByTestId('detail-task-id')).toHaveText('task-turn-report')
  await expect(details.getByTestId('detail-task-result')).toContainText('3 failures, 2 flaky')
  await expect(details.getByTestId('detail-task-artifacts')).toContainText('build-failures.md')
  await expect(details.getByTestId('detail-message-link')).toContainText(`sessionId=${SESSION}&messageId=msg-ct-2`)
  await page.screenshot({ path: 'test-results/messagehub-agent-task-details.png' })

  // Keyboard: Enter on a focused bubble. A quick reply shows its task only here.
  await details.getByRole('button', { name: 'Close', exact: true }).click()
  await bubble(page, 'msg-ct-4').focus()
  await page.keyboard.press('Enter')
  await expect(details).toHaveAttribute('data-message-id', 'msg-ct-4')
  await details.getByTestId('detail-tab-message').click()
  await expect(details.getByTestId('detail-task-summary')).toHaveAttribute('data-state', 'succeeded')

  // A task the viewer may not read says so instead of showing a state.
  await details.getByRole('button', { name: 'Close', exact: true }).click()
  await bubble(page, 'msg-ct-8').getByText('another workspace').click()
  await expect(details.getByTestId('detail-task')).toHaveAttribute('data-status', 'denied')
  await expect(details.getByTestId('detail-task-unavailable')).toContainText('do not have access')
  await expect(details.getByTestId('detail-task-id')).toHaveText('task-turn-foreign')

  // A message without a task has details too, without a task section.
  await details.getByRole('button', { name: 'Close', exact: true }).click()
  await bubble(page, 'msg-ct-9').getByText('release checklist').click()
  await expect(details).toHaveAttribute('data-message-id', 'msg-ct-9')
  await expect(details.getByTestId('detail-effective')).toContainText('Check the MessageHub release checklist.')
  await expect(details.getByTestId('detail-task')).toHaveCount(0)

  // An attachment inside the body keeps its own click.
  await details.getByRole('button', { name: 'Close' }).click()
  await report.getByText('design-brief.pdf').click()
  await expect(details).toHaveCount(0)
})

test('a message link locates the anchor, opens its details and survives a reload', async ({ page }) => {
  await page.goto(`/messagehub?sessionId=${SESSION}&messageId=msg-ct-2`)
  const details = page.getByTestId('message-details-pane')
  await expect(details).toHaveAttribute('data-message-id', 'msg-ct-2')
  await details.getByTestId('detail-tab-message').click()
  await expect(details.getByTestId('detail-task-id')).toHaveText('task-turn-report')
  await expect(bubble(page, 'msg-ct-2')).toBeVisible()
  await page.reload()
  await details.getByTestId('detail-tab-message').click()
  await expect(details.getByTestId('detail-task-id')).toHaveText('task-turn-report')

  // A message deep in a long history that is not rendered yet.
  const target = await page.evaluate(async context => {
    const reader = window.__messageHubMock.reader(context, 'session-coder-1')
    const rows = await reader.readRange(0, 40)
    const row = rows.find(message => message.kind === 'chat' && typeof message.ui_message_id === 'string')
    return { id: row?.ui_message_id as string, total: reader.totalCount }
  }, OWN)
  expect(target.total).toBeGreaterThan(200)
  await page.goto(`/messagehub?entityId=${encodeURIComponent(CODER)}&sessionId=session-coder-1&messageId=${encodeURIComponent(target.id)}`)
  await expect(details).toHaveAttribute('data-message-id', target.id)
  await expect(details.getByTestId('detail-effective')).toBeVisible()
  await expect(page.getByTestId('conversation-history').locator(`[data-message-id="${target.id}"]`)).toBeInViewport()

  await page.goto(`/messagehub?sessionId=${SESSION}&messageId=no-such-message`)
  await expect(page.getByTestId('message-details-missing')).toBeVisible()
})

test('a dropped connection never keeps saying "working"; reconnecting catches up; a lost permission or a cleaned-up task hides the area', async ({ page }) => {
  await openTaskSession(page)
  const area = bubble(page, 'msg-ct-10').getByTestId('message-task')
  await expect(area).toHaveAttribute('data-state', 'waitingChild')
  await page.evaluate(id => { window.__messageHubMockTasks.setOffline(true); return window.__messageHubTaskWatch.refresh(id) }, RUNNING)
  await expect(area).toHaveAttribute('data-state', 'unavailable')
  await expect(area).not.toContainText('Working')
  await expect(bubble(page, 'msg-ct-10')).toContainText('Got it, working on it')

  await page.evaluate(id => {
    window.__messageHubMockTasks.setOffline(false)
    window.__messageHubMockTasks.update(id, { phase: 'Running', waitReason: undefined, message: 'Back online' }, true)
    window.dispatchEvent(new Event('online'))
  }, RUNNING)
  await expect(area.getByTestId('task-line-1')).toHaveText('Working · Back online')

  // The task is read as the logged-in viewer: without a grant there is no task area, the body stays.
  await page.evaluate(id => { window.__messageHubMockTasks.viewerDid = 'did:buckyos:person:alice'; return window.__messageHubTaskWatch.refresh(id) }, RUNNING)
  await expect(area).toHaveCount(0)
  await expect(bubble(page, 'msg-ct-10')).toContainText('Got it, working on it')
  await page.evaluate(id => { window.__messageHubMockTasks.viewerDid = 'did:buckyos:user:self'; return window.__messageHubTaskWatch.refresh(id) }, RUNNING)
  await expect(area).toHaveAttribute('data-state', 'running')

  await page.evaluate(id => window.__messageHubMockTasks.remove(id), RUNNING)
  await expect(area).toHaveCount(0)
  await expect(bubble(page, 'msg-ct-10')).toContainText('Got it, working on it')
})
