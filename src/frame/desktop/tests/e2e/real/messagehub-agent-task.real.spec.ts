import { expect, test } from '@playwright/test'

/**
 * Agent replies carrying `agent_task` in a real zone, after
 * `test/test_opendan/test_agent_chat_task.ts` produced them. Anchors are the
 * message ids that script prints:
 *
 *   AGENT_TASK_TREE_MSG    a placeholder replaced by its edit, whose Turn started a work session
 *   AGENT_TASK_FAST_MSG    a fast reply (no placeholder)
 *   AGENT_TASK_CANCEL_MSG  a placeholder closed by a cancel
 */
const baseURL = process.env.MESSAGEHUB_REAL_BASE_URL || 'https://test.buckyos.io'
const zoneHost = new URL(baseURL).hostname
const zoneIp = process.env.MESSAGEHUB_REAL_ZONE_IP
const agentDid = process.env.BUCKYOS_TEST_AGENT_DID || `did:web:jarvis.${zoneHost}`
const treeMsg = process.env.AGENT_TASK_TREE_MSG || ''
const fastMsg = process.env.AGENT_TASK_FAST_MSG || ''
const cancelMsg = process.env.AGENT_TASK_CANCEL_MSG || ''

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
