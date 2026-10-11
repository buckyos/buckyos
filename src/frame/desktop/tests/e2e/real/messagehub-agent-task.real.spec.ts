import { expect, test, type Page } from '@playwright/test'

/**
 * Agent replies carrying `agent_task` in a real zone, after
 * `test/test_opendan/test_agent_chat_task.ts` produced them. Anchors are the
 * message ids that script prints:
 *
 *   AGENT_TASK_TREE_MSG    a placeholder replaced by its edit, whose Turn started a work session
 *   AGENT_TASK_FAST_MSG    a fast reply (no placeholder)
 *   AGENT_TASK_CANCEL_MSG  a placeholder closed by a cancel
 *   AGENT_ASK_MSG          a placeholder whose Turn asked the user (`agent_ask`) before its edit
 *
 * `AGENT_ASK_LIVE=1` instead asks the Agent from the UI something it has to
 * clarify (`AGENT_ASK_PROMPT`) and answers its questions in the composer.
 */
const baseURL = process.env.MESSAGEHUB_REAL_BASE_URL || 'https://test.buckyos.io'
const zoneHost = new URL(baseURL).hostname
const zoneIp = process.env.MESSAGEHUB_REAL_ZONE_IP
const agentDid = process.env.BUCKYOS_TEST_AGENT_DID || `did:web:jarvis.${zoneHost}`
const treeMsg = process.env.AGENT_TASK_TREE_MSG || ''
const fastMsg = process.env.AGENT_TASK_FAST_MSG || ''
const cancelMsg = process.env.AGENT_TASK_CANCEL_MSG || ''
const askMsg = process.env.AGENT_ASK_MSG || ''
const askPrompt = process.env.AGENT_ASK_PROMPT || '帮我整理一下这个项目的文档。范围和放在哪里你不确定的话先问我，问清楚再开始。'
const askAnswer = process.env.AGENT_ASK_ANSWER || '只整理 docs/ 目录，放在原位置。'

test.use({
  baseURL,
  ignoreHTTPSErrors: true,
  launchOptions: zoneIp ? {
    args: [`--host-resolver-rules=MAP ${zoneHost} ${zoneIp}, MAP *.${zoneHost} ${zoneIp}`, '--no-proxy-server'],
  } : {},
})

test('the Turn task of an agent reply is shown on its anchor bubble and in message details', async ({ page }) => {
  test.skip(process.env.MESSAGEHUB_REAL_E2E !== '1' || !treeMsg, 'Requires a running development zone and the anchors of test_agent_chat_task.ts.')
  const errors: string[] = []
  page.on('pageerror', error => errors.push(error.message))
  await page.goto(`/messagehub?entityId=${encodeURIComponent(agentDid)}`)
  await page.getByLabel('Username', { exact: true }).fill(process.env.BUCKYOS_TEST_ADMIN_USER || 'devtest')
  await page.getByLabel('Password', { exact: true }).fill(process.env.BUCKYOS_TEST_ADMIN_PASSWORD || 'bucky2025')
  await page.getByRole('button', { name: 'Sign In', exact: true }).click()
  const history = page.getByTestId('conversation-history')
  await expect(history).toBeVisible({ timeout: 60_000 })
  const bubble = (id: string) => history.locator(`[data-message-id="${id}"]`)

  // One bubble at the anchor, with the edit's content and the task of the Turn.
  const tree = bubble(treeMsg)
  await expect(tree).toBeVisible({ timeout: 30_000 })
  await expect(tree.getByTestId('edited-marker')).toBeVisible()
  await expect(tree).not.toContainText('收到，开始处理')
  const task = tree.getByTestId('message-task')
  await expect(task).toHaveAttribute('data-state', 'succeeded', { timeout: 30_000 })
  await tree.screenshot({ path: 'test-results-real/agent-task-bubble.png' })
  await task.getByTestId('task-expand').click()
  await expect(task.getByTestId('task-node')).toHaveCount(1)
  await expect(task.getByTestId('task-node').first()).toHaveAttribute('data-state', 'succeeded')
  await tree.screenshot({ path: 'test-results-real/agent-task-tree.png' })

  if (fastMsg) {
    await expect(bubble(fastMsg)).toBeVisible()
    await expect(bubble(fastMsg).getByTestId('message-task')).toHaveCount(0)
  }
  if (cancelMsg) {
    await expect(bubble(cancelMsg)).toContainText('已停止')
    await expect(bubble(cancelMsg).getByTestId('message-task')).toHaveAttribute('data-state', 'canceled')
  }

  await tree.hover()
  await tree.getByTestId('message-details').first().click()
  const details = page.getByTestId('message-details-pane')
  await expect(details).toBeVisible()
  await expect(details.getByTestId('turn-worklog')).toBeVisible()
  await expect(details.getByTestId('worklog-row').first()).toBeVisible({ timeout: 30_000 })
  await page.screenshot({ path: 'test-results-real/agent-turn-worklog.png' })
  await details.getByTestId('detail-tab-message').click()
  await expect(details.getByTestId('detail-edits')).toBeVisible()
  await expect(details.getByTestId('detail-task-summary')).toHaveAttribute('data-state', 'succeeded')
  await expect(details.getByTestId('detail-task-result')).toBeVisible()
  await page.screenshot({ path: 'test-results-real/agent-task-details.png' })
  expect(errors).toEqual([])
})

async function signIn(page: Page) {
  await page.goto(`/messagehub?entityId=${encodeURIComponent(agentDid)}`)
  await page.getByLabel('Username', { exact: true }).fill(process.env.BUCKYOS_TEST_ADMIN_USER || 'devtest')
  await page.getByLabel('Password', { exact: true }).fill(process.env.BUCKYOS_TEST_ADMIN_PASSWORD || 'bucky2025')
  await page.getByRole('button', { name: 'Sign In', exact: true }).click()
  const history = page.getByTestId('conversation-history')
  await expect(history).toBeVisible({ timeout: 60_000 })
  return history
}

test('the questions a Turn asked fold under its reply, collapsed once the Turn has replied', async ({ page }) => {
  test.skip(process.env.MESSAGEHUB_REAL_E2E !== '1' || !askMsg, 'Requires a running development zone and the AGENT_ASK_MSG anchor printed by test/test_opendan/test_agent_ask.ts.')
  const errors: string[] = []
  page.on('pageerror', error => errors.push(error.message))
  const history = await signIn(page)
  const anchor = history.locator(`[data-message-id="${askMsg}"]`)
  await expect(anchor).toBeVisible({ timeout: 30_000 })
  await expect(anchor.getByTestId('edited-marker')).toBeVisible()
  const qa = anchor.getByTestId('agent-qa')
  await expect(qa).toHaveAttribute('data-expanded', 'false')
  await qa.getByTestId('agent-qa-toggle').click()
  const kinds = await qa.getByTestId('agent-qa-item').evaluateAll(items => items.map(item => item.getAttribute('data-kind')))
  expect(kinds[0]).toBe('question')
  expect(kinds).toContain('answer')
  await anchor.screenshot({ path: 'test-results-real/agent-ask-qa.png' })
  // The Turn is over: the composer is an ordinary one.
  await expect(page.getByTestId('message-composer').getByTestId('composer-answer')).toHaveCount(0)
  expect(errors).toEqual([])
})

test('a live Turn asks in the composer; answering keeps it one Turn and its reply folds the Q&A', async ({ page }) => {
  test.skip(process.env.MESSAGEHUB_REAL_E2E !== '1' || process.env.AGENT_ASK_LIVE !== '1', 'Requires a running development zone with an Agent that analyses requests (AGENT_ASK_LIVE=1).')
  test.setTimeout(15 * 60_000)
  const errors: string[] = []
  page.on('pageerror', error => errors.push(error.message))
  const history = await signIn(page)
  const composer = page.getByTestId('message-composer')
  await expect(composer.getByTestId('composer-answer')).toHaveCount(0)
  await composer.locator('textarea').fill(askPrompt)
  await composer.locator('textarea').press('Enter')

  const answerState = composer.getByTestId('composer-answer')
  await expect(answerState).toBeVisible({ timeout: 5 * 60_000 })
  await page.screenshot({ path: 'test-results-real/agent-ask-composer.png' })
  // The question hangs under the Turn's placeholder, whose task waits for the answer.
  const turn = history.locator('[data-message-id]').filter({ has: page.getByTestId('agent-qa') }).last()
  const qa = turn.getByTestId('agent-qa')
  await expect(qa).toHaveAttribute('data-expanded', 'true')
  await expect(turn.getByTestId('message-task')).toHaveAttribute('data-state', 'waitingInput', { timeout: 30_000 })

  // Answer every question the analysis asks (choice: the first option).
  for (let round = 0; round < 4; round++) {
    if (await answerState.getAttribute('data-kind') === 'choice') await answerState.getByTestId('answer-option').first().click()
    else {
      await composer.getByRole('textbox').fill(askAnswer)
      await composer.getByRole('textbox').press('Enter')
    }
    await expect(answerState).toHaveCount(0)
    const next = await answerState.waitFor({ state: 'visible', timeout: 90_000 }).then(() => true, () => false)
    if (!next) break
  }
  expect(await qa.getByTestId('agent-qa-item').evaluateAll(items => items.map(item => item.getAttribute('data-kind')))).toContain('answer')

  // The Turn replies by editing its placeholder; the Q&A folds away under the reply.
  await expect(turn.getByTestId('edited-marker')).toBeVisible({ timeout: 10 * 60_000 })
  await expect(qa).toHaveAttribute('data-expanded', 'false')
  await turn.screenshot({ path: 'test-results-real/agent-ask-reply.png' })
  expect(errors).toEqual([])
})
