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
    __messageHubMockWorkCards: typeof import('../../../src/app/messagehub/mock/worksessions').mockWorkCards
    __messageHubWorkCards: import('../../../src/app/messagehub/conversation/worksessions/watch').WorkCardsWatch
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

  // Only tasks that are still open are subscribed, and nothing below the first level was read. The
  // running Turn has sub-tasks but no work session (yet): its tree is followed for the work cards.
  const state = await page.evaluate(() => ({ subscriptions: window.__messageHubMockTasks.subscriptions(), events: window.__messageHubMockTasks.reads.listEvents }))
  expect(state).toEqual({ subscriptions: [`task:${RUNNING}`, `tree:${RUNNING}`], events: 0 })
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
  expect(await page.evaluate(() => window.__messageHubMockTasks.subscriptions())).toEqual([`task:${RUNNING}`, `tree:${RUNNING}`])
})

test('click and keyboard reveal the toolbar, whose Details action opens the live task', async ({ page }) => {
  await openTaskSession(page)
  const details = page.getByTestId('message-details-pane')

  await bubble(page, 'msg-ct-10').getByText('Got it, working on it').click()
  await expect(details).toHaveCount(0)
  await expect(bubble(page, 'msg-ct-10').getByTestId('message-hover-bar')).toHaveCSS('opacity', '1')
  await bubble(page, 'msg-ct-10').getByTestId('message-hover-bar').getByTestId('message-details').click()
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
  await expect.poll(() => page.evaluate(() => window.__messageHubMockTasks.subscriptions())).toEqual([`task:${RUNNING}`, `tree:${RUNNING}`])

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
  await expect(details).toHaveCount(0)
  await expect(bubble(page, 'msg-ct-4').getByTestId('message-hover-bar')).toHaveCSS('opacity', '1')
  await bubble(page, 'msg-ct-4').getByTestId('message-hover-bar').getByTestId('message-details').focus()
  await page.keyboard.press('Enter')
  await expect(details).toHaveAttribute('data-message-id', 'msg-ct-4')
  await details.getByTestId('detail-tab-message').click()
  await expect(details.getByTestId('detail-task-summary')).toHaveAttribute('data-state', 'succeeded')

  // A task the viewer may not read says so instead of showing a state.
  await details.getByRole('button', { name: 'Close', exact: true }).click()
  await bubble(page, 'msg-ct-8').getByText('another workspace').click()
  await expect(details).toHaveCount(0)
  await expect(bubble(page, 'msg-ct-8').getByTestId('message-hover-bar')).toHaveCSS('opacity', '1')
  await bubble(page, 'msg-ct-8').getByTestId('message-hover-bar').getByTestId('message-details').click()
  await expect(details.getByTestId('detail-task')).toHaveAttribute('data-status', 'denied')
  await expect(details.getByTestId('detail-task-unavailable')).toContainText('do not have access')
  await expect(details.getByTestId('detail-task-id')).toHaveText('task-turn-foreign')

  // A message without a task has details too, without a task section.
  await details.getByRole('button', { name: 'Close', exact: true }).click()
  await bubble(page, 'msg-ct-9').getByText('release checklist').click()
  await expect(details).toHaveCount(0)
  await expect(bubble(page, 'msg-ct-9').getByTestId('message-hover-bar')).toHaveCSS('opacity', '1')
  await bubble(page, 'msg-ct-9').getByTestId('message-hover-bar').getByTestId('message-details').click()
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

/* ── An Agent Turn that asks the user (`agent_ask`) ── */

const ASK_SESSION = 'session-coder-ask'
const DONE_SESSION = 'session-coder-qa'
const ANCHOR = 'msg-qa-2'

async function openSession(page: Page, session: string, firstBubble: string) {
  await page.goto(`/messagehub?entityId=${encodeURIComponent(CODER)}&sessionId=${session}`)
  await expect(page.getByTestId('conversation-history')).toBeVisible()
  await expect(bubble(page, firstBubble)).toBeVisible()
}

/** The newest raw (unfolded) message of a mock session. */
const lastSent = (page: Page, session: string) => page.evaluate(({ context, session }) => {
  const timeline = window.__messageHubMock.reader(context, session).timeline ?? []
  const message = timeline[timeline.length - 1]
  return { from: message.from, content: message.content.content, relates_to: message.relates_to ?? null, machine: message.content.machine ?? null, mentions: message.mentions ?? null }
}, { context: OWN, session })

/** The Agent asks: a `thread` reply to the anchor carrying `agent_ask`. */
const ask = (page: Page, id: string, text: string, input: { kind: 'text' | 'choice'; options?: Array<{ id: string; label: string }>; allow_text?: boolean }, anchor: string | null = ANCHOR, session = ASK_SESSION) => page.evaluate(({ session, id, text, input, anchor }) => window.__messageHubMock.simulateIncoming(session, {
  ui_message_id: id,
  content: { format: 'text/plain', content: text },
  ...(anchor ? { relates_to: { rel: 'thread', target: anchor } } : {}),
  agent_ask: { ...(anchor ? { anchor } : {}), call_id: `call-${id}`, turn: 3, input: { kind: input.kind, options: input.options ?? [], allow_text: input.allow_text ?? false } },
}), { session, id, text, input, anchor })

test('a choice question replaces the input with its options; a pick answers in the thread of the anchor and ends the answer state', async ({ page }) => {
  await openSession(page, ASK_SESSION, ANCHOR)
  const composer = page.getByTestId('message-composer')
  const answer = composer.getByTestId('composer-answer')
  await expect(answer).toHaveAttribute('data-kind', 'choice')
  await expect(answer).toContainText('Answer CodeAssistant\'s question')
  // The question without the option list its body repeats for other clients.
  await expect(answer.getByTestId('answer-question')).toHaveText('Which docs should I tidy?')
  await expect(answer.getByTestId('answer-option')).toHaveText(['docs/ only', 'The whole repository'])
  await expect(answer.getByTestId('answer-other')).toHaveCount(0)
  await expect(composer.locator('textarea')).toHaveCount(0)
  await expect(composer.getByRole('button', { name: 'Cancel' })).toHaveCount(0)

  // The questions and answers live under the anchor, open while the Turn waits.
  expect(await anchors(page)).toEqual(['msg-qa-1', ANCHOR])
  const qa = bubble(page, ANCHOR).getByTestId('agent-qa')
  await expect(qa).toHaveAttribute('data-expanded', 'true')
  await expect(qa.getByTestId('agent-qa-item')).toHaveCount(3)
  expect(await qa.getByTestId('agent-qa-item').evaluateAll(items => items.map(item => item.getAttribute('data-kind')))).toEqual(['question', 'answer', 'question'])
  await expect(qa.getByTestId('agent-qa-item').nth(2)).toContainText('Which docs should I tidy?')
  await expect(bubble(page, ANCHOR).getByTestId('message-task')).toHaveAttribute('data-state', 'waitingInput')
  await expect(bubble(page, ANCHOR).getByTestId('task-line-1')).toHaveText('Waiting for your answer')
  // Nothing but the answer may be sent: no reply action meanwhile.
  await bubble(page, 'msg-qa-1').hover()
  await expect(bubble(page, 'msg-qa-1').getByTestId('message-reply')).toHaveCount(0)

  await answer.getByTestId('answer-option').filter({ hasText: 'docs/ only' }).click()
  await expect(composer.getByTestId('composer-answer')).toHaveCount(0)
  await expect(composer.locator('textarea')).toBeVisible()
  expect(await lastSent(page, ASK_SESSION)).toEqual({ from: SELF, content: 'docs/ only', relates_to: { rel: 'thread', target: ANCHOR }, machine: { intent: 'agent_ask.answer', data: { call_id: 'call-docs-2', option: 'a' } }, mentions: null })
  await expect(qa.getByTestId('agent-qa-item')).toHaveCount(4)
  await expect(qa.getByTestId('agent-qa-item').nth(2).locator('[data-picked]')).toHaveText('docs/ only')
  expect(await anchors(page)).toEqual(['msg-qa-1', ANCHOR])
  await bubble(page, 'msg-qa-1').hover()
  await expect(bubble(page, 'msg-qa-1').getByTestId('message-reply')).toBeVisible()

  // The Turn replies: its Q&A folds away by default.
  await page.evaluate(({ session, anchor, task }) => {
    window.__messageHubMockTasks.update(task, { phase: 'Terminal', outcome: 'Succeeded', waitReason: undefined, message: 'Docs tidied' })
    return window.__messageHubMock.simulateIncoming(session, { content: { format: 'text/plain', content: 'docs/ is tidied up for new contributors.' }, relates_to: { rel: 'edit', target: anchor } })
  }, { session: ASK_SESSION, anchor: ANCHOR, task: 'task-turn-docs' })
  await expect(bubble(page, ANCHOR)).toContainText('docs/ is tidied up')
  await expect(qa).toHaveAttribute('data-expanded', 'false')
  await expect(qa.getByTestId('agent-qa-toggle')).toHaveText('4 Q&A message(s)')
  await expect(qa.getByTestId('agent-qa-item')).toHaveCount(0)
})

test('a text question locks the input to answering it, without a way to cancel; the answer goes to the anchor', async ({ page }) => {
  await openSession(page, ASK_SESSION, ANCHOR)
  await ask(page, 'msg-qa-6', 'Should I keep the current folder layout?', { kind: 'text' })
  const composer = page.getByTestId('message-composer')
  const answer = composer.getByTestId('composer-answer')
  await expect(answer).toHaveAttribute('data-kind', 'text')
  await expect(answer).toContainText('Answer CodeAssistant\'s question')
  await expect(answer).toContainText('Should I keep the current folder layout?')
  await expect(composer.getByTestId('composer-relation')).toHaveCount(0)
  await expect(composer.getByRole('button', { name: 'Cancel' })).toHaveCount(0)
  const input = composer.getByRole('textbox', { name: 'Type your answer…' })
  await input.fill('Yes, keep it.')
  // Escape does not leave the answer state.
  await input.press('Escape')
  await expect(answer).toBeVisible()
  await page.screenshot({ path: 'test-results/messagehub-agent-ask-text.png' })
  await input.press('Enter')
  await expect(composer.getByTestId('composer-answer')).toHaveCount(0)
  expect(await lastSent(page, ASK_SESSION)).toEqual({ from: SELF, content: 'Yes, keep it.', relates_to: { rel: 'thread', target: ANCHOR }, machine: null, mentions: null })
  await expect(bubble(page, ANCHOR).getByTestId('agent-qa-item')).toHaveCount(5)
  await expect(bubble(page, ANCHOR).getByTestId('agent-qa-item').last()).toHaveText(/Yes, keep it\./)
  expect(await anchors(page)).toEqual(['msg-qa-1', ANCHOR])
  await expect(composer.getByRole('textbox', { name: 'Message...' })).toBeVisible()

  // Without an anchor (a channel that cannot edit) the question is a row and the answer replies to it.
  await ask(page, 'msg-qa-bare', 'Which branch?', { kind: 'text' }, null)
  await expect(composer.getByTestId('composer-answer')).toHaveAttribute('data-kind', 'text')
  await composer.getByRole('textbox', { name: 'Type your answer…' }).fill('main')
  await composer.getByRole('textbox', { name: 'Type your answer…' }).press('Enter')
  await expect(composer.getByTestId('composer-answer')).toHaveCount(0)
  expect((await lastSent(page, ASK_SESSION)).relates_to).toEqual({ rel: 'thread', target: 'msg-qa-bare' })
  await expect.poll(() => anchors(page)).toHaveLength(4)
  const rows = await anchors(page)
  expect(rows.slice(0, 3)).toEqual(['msg-qa-1', ANCHOR, 'msg-qa-bare'])
  await expect(page.getByTestId('conversation-history').getByTestId('reply-quote').last()).toContainText('Which branch?')
})

test('"Other…" turns a choice that accepts free text into the locked input; /stop still goes out as a command', async ({ page }) => {
  await openSession(page, ASK_SESSION, ANCHOR)
  const composer = page.getByTestId('message-composer')
  await ask(page, 'msg-qa-7', 'Where should the index page go?\n1. README.md\n2. docs/index.md', { kind: 'choice', options: [{ id: '1', label: 'README.md' }, { id: '2', label: 'docs/index.md' }], allow_text: true })
  await expect(composer.getByTestId('answer-question')).toHaveText('Where should the index page go?')
  await composer.getByTestId('answer-other').click()
  await expect(composer.getByTestId('composer-answer')).toHaveAttribute('data-kind', 'text')
  await expect(composer.getByRole('textbox', { name: 'Type your answer…' })).toBeFocused()
  await composer.getByTestId('answer-back-to-options').click()
  await expect(composer.getByTestId('answer-option')).toHaveCount(2)
  await composer.getByTestId('answer-other').click()
  await composer.getByRole('textbox', { name: 'Type your answer…' }).fill('Both, README.md links to docs/index.md')
  await composer.getByRole('textbox', { name: 'Type your answer…' }).press('Enter')
  await expect(composer.getByTestId('composer-answer')).toHaveCount(0)
  expect(await lastSent(page, ASK_SESSION)).toEqual({ from: SELF, content: 'Both, README.md links to docs/index.md', relates_to: { rel: 'thread', target: ANCHOR }, machine: null, mentions: null })

  // The stop button sends `/stop` as the command it is, not as an answer.
  await ask(page, 'msg-qa-8', 'Delete the outdated pages?', { kind: 'choice', options: [{ id: 'y', label: 'Yes' }, { id: 'n', label: 'No' }] })
  const before = (await anchors(page)).length
  await composer.getByTestId('answer-stop').click()
  await expect(composer.getByTestId('composer-answer')).toHaveCount(0)
  expect(await lastSent(page, ASK_SESSION)).toEqual({ from: SELF, content: '/stop', relates_to: null, machine: null, mentions: null })
  await expect.poll(() => anchors(page)).toHaveLength(before + 1)
  await expect(page.getByTestId('conversation-history').getByTestId('message-bubble').last()).toContainText('/stop')

  // Typed in a text answer, too.
  await ask(page, 'msg-qa-9', 'Anything else?', { kind: 'text' })
  await composer.getByRole('textbox', { name: 'Type your answer…' }).fill('/stop')
  await composer.getByRole('textbox', { name: 'Type your answer…' }).press('Enter')
  await expect(composer.getByTestId('composer-answer')).toHaveCount(0)
  expect(await lastSent(page, ASK_SESSION)).toEqual({ from: SELF, content: '/stop', relates_to: null, machine: null, mentions: null })
})

test('Q&A under a finished Turn starts collapsed and under a waiting one open; a hand-set state is remembered; other replies keep their rows', async ({ page }) => {
  await openSession(page, DONE_SESSION, 'msg-qd-2')
  expect(await anchors(page)).toEqual(['msg-qd-1', 'msg-qd-2', 'msg-qd-8'])
  const done = bubble(page, 'msg-qd-2')
  await expect(done).toContainText('Release 2.4')
  await expect(done.getByTestId('edited-marker')).toBeVisible()
  const qa = done.getByTestId('agent-qa')
  await expect(qa).toHaveAttribute('data-expanded', 'false')
  await expect(qa.getByTestId('agent-qa-toggle')).toHaveText('4 Q&A message(s)')
  // An ordinary reply to the Agent's reply is a row of its own that quotes it.
  await expect(bubble(page, 'msg-qd-8').getByTestId('reply-quote')).toContainText('Release 2.4')
  await expect(page.getByTestId('message-composer').getByTestId('composer-answer')).toHaveCount(0)

  await qa.getByTestId('agent-qa-toggle').click()
  await expect(qa.getByTestId('agent-qa-item')).toHaveCount(4)
  await expect(qa.getByTestId('agent-qa-item').first().locator('[data-picked]')).toHaveText('End users')
  await expect(qa.getByTestId('agent-qa-item').first()).not.toContainText('- a. Developers')
  await page.reload()
  await expect(bubble(page, 'msg-qd-2').getByTestId('agent-qa')).toHaveAttribute('data-expanded', 'true')

  // A reply to a quick Agent reply keeps its row too.
  await openTaskSession(page)
  await bubble(page, 'msg-ct-4').hover()
  await bubble(page, 'msg-ct-4').getByTestId('message-reply').click()
  await page.getByTestId('message-composer').locator('textarea').fill('Thanks!')
  await page.getByTestId('message-composer').locator('textarea').press('Enter')
  await expect.poll(async () => (await anchors(page)).length).toBe(11)
  await expect(page.getByTestId('conversation-history').getByTestId('reply-quote').last()).toContainText('09:41 UTC')
  await expect(bubble(page, 'msg-ct-4').getByTestId('agent-qa')).toHaveCount(0)

  // Closed by hand while waiting: it stays closed, also after the reply arrived.
  await openSession(page, ASK_SESSION, ANCHOR)
  const waiting = bubble(page, ANCHOR).getByTestId('agent-qa')
  await expect(waiting).toHaveAttribute('data-expanded', 'true')
  await waiting.getByTestId('agent-qa-toggle').click()
  await page.reload()
  await expect(bubble(page, ANCHOR).getByTestId('agent-qa')).toHaveAttribute('data-expanded', 'false')
  await page.getByTestId('message-composer').getByTestId('answer-option').first().click()
  await expect(bubble(page, ANCHOR).getByTestId('agent-qa-toggle')).toHaveText('4 Q&A message(s)')
  await expect(bubble(page, ANCHOR).getByTestId('agent-qa')).toHaveAttribute('data-expanded', 'false')
})

for (const theme of ['light', 'dark'] as const) {
  test(`answer mode and the Q&A list read well in the ${theme} theme`, async ({ page }) => {
    await page.addInitScript(value => { try { localStorage.setItem('buckyos.prototype.theme.v1', value) } catch { /* ignored */ } }, theme)
    await openSession(page, ASK_SESSION, ANCHOR)
    await expect(page.getByTestId('message-composer').getByTestId('composer-answer')).toHaveAttribute('data-kind', 'choice')
    await page.getByTestId('message-composer').getByTestId('answer-option').first().hover()
    await page.screenshot({ path: `test-results/messagehub-agent-ask-choice-${theme}.png` })
    await ask(page, 'msg-qa-6', 'Should I keep the current folder layout?', { kind: 'text' })
    await expect(page.getByTestId('message-composer').getByTestId('composer-answer')).toHaveAttribute('data-kind', 'text')
    await page.screenshot({ path: `test-results/messagehub-agent-ask-text-${theme}.png` })
    await openSession(page, DONE_SESSION, 'msg-qd-2')
    await bubble(page, 'msg-qd-2').getByTestId('agent-qa-toggle').click()
    await expect(bubble(page, 'msg-qd-2').getByTestId('agent-qa-item')).toHaveCount(4)
    await page.screenshot({ path: `test-results/messagehub-agent-ask-qa-${theme}.png` })
  })
}

const WORK_SESSION = 'session-coder-work'
const WORK = { simple: 'task-turn-work-builds', plan: 'task-turn-work-docs', done: 'task-turn-work-notes', analyze: 'task-turn-work-deploy' }
const SHOTS = '/home/lzc/project/messagehub_work_ui_dv_runs/mock'
const workReads = (page: Page, taskId: string) => page.evaluate(id => window.__messageHubMockWorkCards.reads[id] ?? 0, taskId)
const workCard = (page: Page, bubbleId: string, sessionId: string) => bubble(page, bubbleId).locator(`[data-session-id="${sessionId}"]`).first()

test('a Turn that created work sessions shows their two-line cards instead of the task area; analysis still shows the task area', async ({ page }) => {
  await openSession(page, WORK_SESSION, 'msg-cw-8')
  const simple = bubble(page, 'msg-cw-6')
  await expect(simple.getByTestId('work-cards')).toBeVisible()
  await expect(simple.getByTestId('message-task')).toHaveCount(0)
  const builds = workCard(page, 'msg-cw-6', 'work-builds')
  await expect(builds).toHaveAttribute('data-state', 'running')
  await expect(builds.getByTestId('work-card-title')).toHaveText('Run both slow builds')
  await expect(builds.getByTestId('work-card-time')).toHaveText(/^4m\d\ds$/)
  await expect(builds.getByTestId('work-card-tokens')).toHaveText('in 86.4k · out 9.1k')
  await expect(builds.getByTestId('work-card-line')).toHaveText('Watching the two cargo builds')
  await expect(builds.getByTestId('work-card-model')).toHaveText('gpt-6.1-sol')
  // The time runs to the second while a card runs.
  const first = await builds.getByTestId('work-card-time').textContent()
  await expect(builds.getByTestId('work-card-time')).not.toHaveText(first ?? '', { timeout: 3_000 })

  // Intent analysis (no sub-task yet): the task area, and no card read at all.
  const analyze = bubble(page, 'msg-cw-8')
  await expect(analyze.getByTestId('message-task')).toHaveAttribute('data-state', 'running')
  await expect(analyze.getByTestId('work-cards')).toHaveCount(0)
  expect(await workReads(page, WORK.analyze)).toBe(0)

  // The analysis created a work session: the bubble turns into its card.
  await page.evaluate(({ id, ui }) => {
    const cards = window.__messageHubMockWorkCards
    cards.set(id, { agent_did: 'did:buckyos:agent:codeassistant', session_id: ui, task_id: id, complete: false, sessions: [{ session_id: 'work-deploy', shape: 'simple', title: 'Clean up the deploy scripts', created_at_ms: Date.now() - 5_000, finished_at_ms: null, state: 'queued', line: '', usage: { input_tokens: 0, output_tokens: 0, rounds: 0, models: [] }, model: 'llm.chat', subtree_usage: null, todos: null }] })
    window.__messageHubMockTasks.add({ taskId: `${id}.work-deploy`, parentId: id, name: 'Clean up the deploy scripts', phase: 'Accepted', input: { agent_did: 'did:buckyos:agent:codeassistant', session_id: 'work-deploy', turn: 1 } }, true)
    window.__messageHubMockTasks.update(id, { phase: 'Terminal', outcome: 'Succeeded', message: 'created a work session' })
  }, { id: WORK.analyze, ui: 'ui-session-work' })
  const deploy = workCard(page, 'msg-cw-8', 'work-deploy')
  await expect(deploy).toHaveAttribute('data-state', 'queued')
  await expect(deploy.getByTestId('work-card-line')).toHaveText('Queued')
  await expect(deploy.getByTestId('work-card-tokens')).toHaveCount(0)
  await expect(deploy.getByTestId('work-card-model')).toHaveCount(0)
  await expect(analyze.getByTestId('message-task')).toHaveCount(0)
})

test('a plan card sums its Todos; the TodoList folds, remembers a hand-set state, lists unbound Todos as one line and long lists as "N more"', async ({ page }) => {
  await openSession(page, WORK_SESSION, 'msg-cw-8')
  const plan = workCard(page, 'msg-cw-4', 'plan-docs')
  await expect(plan).toHaveAttribute('data-shape', 'plan')
  await expect(plan.getByTestId('work-card-tokens')).toHaveText('in 86k · out 9.1k')
  await expect(plan.getByTestId('work-card-tokens')).toHaveAttribute('title', /Plan itself: 25,000 input · 1,800 output tokens · 4 round\(s\)\nTodos in total: 61,000 input · 7,300 output tokens · 14 round\(s\)/)
  await expect(plan.getByTestId('work-card-line')).toHaveText('Waiting for 1 sub-work session(s)')
  const list = bubble(page, 'msg-cw-4').getByTestId('todo-list')
  await expect(list).toHaveAttribute('data-expanded', 'true')
  await expect(list.getByTestId('todo-list-toggle')).toHaveText('TodoList · 1/4')
  await expect(list.getByTestId('todo-card')).toHaveCount(2)
  await expect(list.getByTestId('todo-card').nth(0).getByTestId('work-card-title')).toHaveText('T01 Survey the current state')
  await expect(list.getByTestId('todo-card').nth(0)).toHaveAttribute('data-state', 'succeeded')
  await expect(list.getByTestId('todo-card').nth(1).getByTestId('work-card-line')).toHaveText('Editing doc/llm_context/LLM Context Design.md')
  const rows = list.getByTestId('todo-row')
  await expect(rows).toHaveCount(3)
  await expect(rows.nth(0)).toHaveText(/T03 Add the tests/)
  await expect(rows.nth(1)).toHaveAttribute('title', 'Waits for T03\nruns after T03')
  await expect(rows.nth(2).locator('.line-through')).toHaveText('T05 Translate the doc')
  for (const index of [0, 1, 2]) expect((await rows.nth(index).boundingBox())!.height).toBeLessThan(30)

  await list.getByTestId('todo-list-toggle').click()
  await expect(list.getByTestId('todo-items')).toHaveCount(0)
  await page.reload()
  await expect(bubble(page, 'msg-cw-4').getByTestId('todo-list')).toHaveAttribute('data-expanded', 'false')

  // A finished plan starts folded; above 12 Todos only the last three that ended are listed.
  const done = bubble(page, 'msg-cw-2')
  await expect(workCard(page, 'msg-cw-2', 'plan-notes')).toHaveAttribute('data-state', 'succeeded')
  await expect(workCard(page, 'msg-cw-2', 'plan-notes').getByTestId('work-card-model')).toHaveCount(0)
  const doneList = done.getByTestId('todo-list')
  await expect(doneList).toHaveAttribute('data-expanded', 'false')
  await expect(doneList.getByTestId('todo-list-toggle')).toHaveText('TodoList · 14/14')
  await doneList.getByTestId('todo-list-toggle').click()
  await expect(doneList.getByTestId('todo-card')).toHaveCount(3)
  await expect(doneList.getByTestId('todo-card').first().getByTestId('work-card-title')).toHaveText('T12 Check section 12')
  await doneList.getByTestId('todo-more').click()
  await expect(doneList.getByTestId('todo-card')).toHaveCount(14)
  await expect(doneList.getByTestId('todo-card').nth(2).getByTestId('work-card-title')).toHaveText('T03 Check section 3 #2')
})

test('clicking a card opens that session\'s continuous work log with its work content; back returns to the Turn', async ({ page }) => {
  await openSession(page, WORK_SESSION, 'msg-cw-8')
  await workCard(page, 'msg-cw-6', 'work-builds').click()
  const details = page.getByTestId('message-details-pane')
  await expect(details.getByTestId('detail-tab-worklog')).toHaveAttribute('aria-selected', 'true')
  const log = details.getByTestId('turn-worklog')
  await expect(log).toHaveAttribute('data-scope', 'session')
  await expect(log).toHaveAttribute('data-session-id', 'work-builds')
  await expect(log.getByRole('heading', { name: 'Run both slow builds' })).toBeVisible()
  await expect(log).toContainText('Whole session')
  await expect(log.getByText('Turn started', { exact: true })).toHaveCount(2)
  await expect(log.getByText('Turn ended', { exact: true })).toHaveCount(1)
  await expect(log.getByText('Starting both builds in the background.')).toBeAttached()
  await expect(log.getByText('Both builds finished without errors.')).toBeVisible()
  const info = log.getByTestId('work-info')
  await expect(info.getByTestId('work-info-objective')).toContainText('Run both slow builds.')
  await expect(info.getByTestId('work-info-runtime')).toHaveText('container')
  await expect(info.getByTestId('work-info-workspace')).toHaveText('ws-buckyos · read_write')
  await expect(info.getByTestId('work-info-tokens')).toHaveText('gpt-6.1-sol · in 86.4k · out 9.1k')
  await expect(info.getByTestId('work-info-report')).toHaveCount(0)
  await page.screenshot({ path: `${SHOTS}/worklog-session-desktop.png` })
  await info.getByTestId('work-info-toggle').click()
  await expect(info.getByTestId('work-info-objective')).toHaveCount(0)
  await details.getByTestId('worklog-back').click()
  await expect(log).toHaveAttribute('data-scope', 'turn')
  await expect(log).toHaveAttribute('data-turn', '6')

  // A Todo card of another bubble opens its own session; a finished one shows its report.
  await details.getByRole('button', { name: 'Close', exact: true }).click()
  await bubble(page, 'msg-cw-4').locator('[data-session-id="work-docs-t01"]').click()
  await expect(log).toHaveAttribute('data-session-id', 'work-docs-t01')
  await expect(log.getByRole('heading', { name: 'T01 Survey the current state' })).toBeVisible()
  await expect(log.getByTestId('work-info-report')).toContainText('Found 3 places that disagree with the source')
  await expect(log.getByTestId('work-info-finished')).toBeVisible()
})

test('finished cards stop all reads; a refused read keeps the task area', async ({ page }) => {
  await openSession(page, WORK_SESSION, 'msg-cw-8')
  await expect(workCard(page, 'msg-cw-2', 'plan-notes')).toBeVisible()
  await expect(workCard(page, 'msg-cw-6', 'work-builds')).toBeVisible()
  expect(await page.evaluate(id => window.__messageHubWorkCards.isLive({ agentDid: 'did:buckyos:agent:codeassistant', sessionId: 'ui-session-work', taskId: id }), WORK.done)).toBe(false)

  // The running card finishes: one more read on the tree hint, then nothing.
  await page.evaluate(id => {
    window.__messageHubMockWorkCards.update(id, 'work-builds', { state: 'succeeded', finished_at_ms: Date.now(), line: 'Both builds passed' })
    window.__messageHubMockWorkCards.complete(id)
    window.__messageHubMockTasks.update(`${id}.work-builds`, { phase: 'Terminal', outcome: 'Succeeded', message: 'Both builds passed' })
  }, WORK.simple)
  const builds = workCard(page, 'msg-cw-6', 'work-builds')
  await expect(builds).toHaveAttribute('data-state', 'succeeded')
  await expect(builds.getByTestId('work-card-line')).toHaveText('Both builds passed')
  const time = await builds.getByTestId('work-card-time').textContent()
  const reads = { simple: await workReads(page, WORK.simple), done: await workReads(page, WORK.done) }
  expect(reads.done).toBe(1)
  await page.evaluate(id => window.__messageHubMockTasks.update(`${id}.work-builds`, { message: 'late hint' }), WORK.simple)
  await page.waitForTimeout(3_000)
  expect({ simple: await workReads(page, WORK.simple), done: await workReads(page, WORK.done) }).toEqual(reads)
  await expect(builds.getByTestId('work-card-time')).toHaveText(time ?? '')

  // An observer (or an Agent without `session.work_cards`): the old task area.
  await page.addInitScript(() => { try { localStorage.setItem('buckyos.messagehub.mock.workCardsDenied', '1') } catch { /* ignored */ } })
  await openSession(page, WORK_SESSION, 'msg-cw-8')
  await expect(bubble(page, 'msg-cw-6').getByTestId('message-task')).toBeVisible()
  await expect(bubble(page, 'msg-cw-6').getByTestId('work-cards')).toHaveCount(0)
  await expect(bubble(page, 'msg-cw-4').getByTestId('message-task')).toBeVisible()
})

test('a narrow card keeps only the output tokens and the time', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 })
  await openSession(page, WORK_SESSION, 'msg-cw-8')
  const builds = workCard(page, 'msg-cw-6', 'work-builds')
  await expect(builds).toBeVisible()
  expect((await bubble(page, 'msg-cw-6').getByTestId('work-cards').boundingBox())!.width).toBeLessThan(360)
  await expect(builds.locator('.mh-work-in')).toBeHidden()
  await expect(builds.getByText('out 9.1k')).toBeVisible()
  await expect(builds.getByTestId('work-card-time')).toBeVisible()
  await expect(builds.getByTestId('work-card-model')).toBeHidden()
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
})

for (const theme of ['light', 'dark'] as const) {
  test(`work session cards read well in the ${theme} theme on desktop and phone`, async ({ page }) => {
    await page.addInitScript(value => { try { localStorage.setItem('buckyos.prototype.theme.v1', value) } catch { /* ignored */ } }, theme)
    await openSession(page, WORK_SESSION, 'msg-cw-8')
    await expect(workCard(page, 'msg-cw-4', 'plan-docs')).toBeVisible()
    await workCard(page, 'msg-cw-6', 'work-builds').hover()
    await bubble(page, 'msg-cw-4').scrollIntoViewIfNeeded()
    await page.screenshot({ path: `${SHOTS}/cards-desktop-${theme}.png` })
    await workCard(page, 'msg-cw-6', 'work-builds').click()
    await expect(page.getByTestId('work-info').getByTestId('work-info-tokens')).toBeVisible()
    await expect(page.getByTestId('turn-worklog').getByText('Both builds finished without errors.')).toBeVisible()
    await page.screenshot({ path: `${SHOTS}/worklog-desktop-${theme}.png` })
    await page.setViewportSize({ width: 390, height: 844 })
    await openSession(page, WORK_SESSION, 'msg-cw-8')
    await bubble(page, 'msg-cw-4').scrollIntoViewIfNeeded()
    await page.screenshot({ path: `${SHOTS}/cards-phone-${theme}.png` })
  })
}
