import { randomUUID } from 'node:crypto'
import type { Page } from '@playwright/test'
import type { Result, WorkspaceInfo, WorkspaceSummary } from '../../src/app/aiworkspace/api/types'
import { createSample, expect, expectAllCommitted, openWorkspace, screenshot, test } from './aiworkspace.fixtures'

function table(page: Page) { return page.getByTestId('aiws-table-cell-all-tasks') }

async function edit(page: Page, record: string, field: string, label: string, value: string) {
  await table(page).getByTestId(`aiws-cell-${record}-${field}`).getByRole('button').first().click()
  const input = page.getByLabel(`${label} ${record}`, { exact: true })
  await input.fill(value)
  return input
}

test('real login, desktop launch, table and Chinese rich text persist after reload', async ({ owner }, info) => {
  const ws = await createSample(owner, true)
  const page = owner.page
  const title = `VM 中文任务 ${randomUUID().slice(0, 8)}`
  await (await edit(page, 'task-41', 'title', '任务', title)).press('Enter')
  await expectAllCommitted(page)
  await expect.poll(async () => (await owner.client.cell(ws, 'task-41', 'title')).value).toBe(title)

  const paragraph = page.getByTestId('aiws-richtext-notes').locator('[data-block-id="n-end"]')
  await paragraph.click()
  await page.keyboard.press('End')
  const cdp = await page.context().newCDPSession(page)
  await cdp.send('Input.imeSetComposition', { text: 'zhongwen', selectionStart: 8, selectionEnd: 8 })
  await cdp.send('Input.insertText', { text: '中文输入持久化' })
  await cdp.detach()
  await expect(paragraph).toContainText('中文输入持久化')
  await expectAllCommitted(page)
  await expect.poll(async () => JSON.stringify(await owner.client.ast(ws))).toContain('中文输入持久化')
  expect(JSON.stringify(await owner.client.ast(ws))).not.toContain('zhongwen')
  await screenshot(page, info, 'desktop-edited')

  await openWorkspace(owner, ws)
  await page.reload()
  await expect(table(page).getByTestId('aiws-cell-task-41-title')).toContainText(title)
  await expect(page.getByTestId('aiws-richtext-notes')).toContainText('中文输入持久化')
  const image = page.getByTestId('aiws-asset-image')
  await expect(image).toBeVisible()
  await expect.poll(() => image.evaluate((element: HTMLImageElement) => element.naturalWidth)).toBe(16)
  await expectAllCommitted(page)
  await screenshot(page, info, 'reloaded-document')
})

test('private workspace, UI reader grant, refused write and revocation use real identities', async ({ owner, peer }, info) => {
  const ws = await createSample(owner)
  const privateInfo = await peer.client.call<Result<WorkspaceInfo>>('ws.get_info', { workspace_id: ws.workspace_id })
  expect(privateInfo).toMatchObject({ ok: false, error: { code: 'NOT_FOUND' } })
  expect((await peer.client.list()).workspaces.some((item) => item.workspace_id === ws.workspace_id)).toBe(false)

  await owner.page.getByTestId('aiws-top-sources').click()
  await owner.page.getByTestId('aiws-left-permissions').click()
  await owner.page.getByTestId('aiws-grant-subject').fill(peer.userId)
  await owner.page.getByTestId('aiws-grant-preset-select').selectOption('reader')
  await owner.page.getByTestId('aiws-grant-submit').click()
  await expect(owner.page.getByTestId('aiws-permissions-message')).toContainText(`已授予 ${peer.userId}`)
  await openWorkspace(peer, ws)
  await expect(table(peer.page)).toBeVisible()
  await expect(peer.page.getByTestId('aiws-fields-cell-all-tasks')).toHaveCount(0)
  const before = await owner.client.cell(ws, 'task-41', 'title')
  const refused = await peer.client.commit(ws, [{
    op: 'table.set_values', source_id: 'tasks',
    values: [{ record_id: 'task-41', field_id: 'title', value: 'must not persist', expect: { rev: before.rev } }],
  }])
  expect(refused).toMatchObject({ status: 'rejected', code: 'PERMISSION_DENIED' })
  expect(await owner.client.cell(ws, 'task-41', 'title')).toEqual(before)
  await screenshot(peer.page, info, 'reader-workspace')

  await owner.page.locator(`[data-testid="aiws-grant"][data-subject="${peer.userId}"][data-scope=""]`).getByTestId('aiws-revoke').click()
  await expect(owner.page.getByTestId('aiws-permissions-message')).toContainText(`已撤回 ${peer.userId}`)
  expect(await peer.client.call('ws.get_info', { workspace_id: ws.workspace_id })).toMatchObject({ ok: false, error: { code: 'NOT_FOUND' } })
  expect((await peer.client.list()).workspaces.some((item) => item.workspace_id === ws.workspace_id)).toBe(false)
  await peer.page.reload()
  await expect(peer.page.getByText(/无法打开链接中的工作区/)).toBeVisible()
  await expect(peer.page.getByTestId('aiws-workspace')).toHaveCount(0)
})

test('two real users converge, preserve independent edits and resolve a same-cell conflict', async ({ owner, peer }, info) => {
  const ws = await createSample(owner)
  await owner.client.ok('ws.grant', { workspace_id: ws.workspace_id, subject: peer.userId, capabilities: ['read', 'update', 'append', 'comment'] })
  await openWorkspace(peer, ws)
  const a = owner.page
  const b = peer.page
  const aText = a.getByTestId('aiws-richtext-notes').locator('[data-block-id="n-end"]')
  const bText = b.getByTestId('aiws-richtext-notes').locator('[data-block-id="n-title"]')
  await aText.click()
  await a.keyboard.press('End')
  await bText.click()
  await b.keyboard.press('End')
  await Promise.all([a.keyboard.insertText('【甲的补充】'), b.keyboard.insertText('【乙的补充】')])
  for (const page of [a, b]) {
    await expect(page.getByTestId('aiws-richtext-notes')).toContainText('【甲的补充】')
    await expect(page.getByTestId('aiws-richtext-notes')).toContainText('【乙的补充】')
    await expectAllCommitted(page)
  }
  expect(await owner.client.ast(ws)).toEqual(await peer.client.ast(ws))

  const aOwner = await edit(a, 'task-42', 'owner', '负责人', '甲')
  const bBudget = await edit(b, 'task-42', 'budget', '预算', '1888')
  await Promise.all([aOwner.press('Enter'), bBudget.press('Enter')])
  await expect(table(a).getByTestId('aiws-cell-task-42-budget')).toContainText('1888.00')
  await expect(table(b).getByTestId('aiws-cell-task-42-owner')).toContainText('甲')
  expect((await owner.client.cell(ws, 'task-42', 'owner')).value).toBe('甲')
  expect((await peer.client.cell(ws, 'task-42', 'budget')).value).toBe('1888.00')

  const aTitle = await edit(a, 'task-41', 'title', '任务', '甲的标题')
  const bTitle = await edit(b, 'task-41', 'title', '任务', '乙的标题')
  await aTitle.press('Enter')
  await expectAllCommitted(a)
  await expect(b.getByTestId('aiws-cell-frame-cell-open-tasks').getByTestId('aiws-cell-task-41-title')).toContainText('甲的标题')
  await expect(bTitle).toHaveValue('乙的标题')
  await bTitle.press('Enter')
  const conflicted = table(b).getByTestId('aiws-cell-task-41-title')
  await expect(conflicted).toHaveAttribute('data-state', 'needs_attention')
  await expect(conflicted.getByTestId('aiws-conflict-theirs')).toHaveText('甲的标题')
  await expect(conflicted.getByTestId('aiws-conflict-mine')).toHaveText('乙的标题')
  expect((await owner.client.cell(ws, 'task-41', 'title')).value).toBe('甲的标题')
  await screenshot(b, info, 'same-cell-conflict')
  await conflicted.getByTestId('aiws-conflict-keep-mine').click()
  await expectAllCommitted(b)
  await expect(table(a).getByTestId('aiws-cell-task-41-title')).toContainText('乙的标题')
  expect((await owner.client.cell(ws, 'task-41', 'title')).value).toBe('乙的标题')
  await a.reload()
  await b.reload()
  for (const page of [a, b]) {
    await expect(table(page).getByTestId('aiws-cell-task-41-title')).toContainText('乙的标题')
    await expect(page.getByTestId('aiws-richtext-notes')).toContainText('【甲的补充】')
    await expect(page.getByTestId('aiws-richtext-notes')).toContainText('【乙的补充】')
  }
})

test('real service rejects missing or invalid sessions through Gateway and replays a commit exactly once', async ({ owner }) => {
  const ws = await owner.client.ok<WorkspaceSummary>('ws.create', { title: owner.client.prefix })
  owner.client.owned.add(ws.workspace_id)
  await expect(owner.client.call('ws.list', {}, null)).rejects.toThrow(/session token required/)
  await expect(owner.client.call('ws.list', {}, 'invalid.jwt.token')).rejects.toThrow(/invalid.*token/i)
  const initial = await owner.client.info(ws)
  const key = randomUUID()
  const operations = [{ op: 'entity.create', entity_id: 'idempotent-note', type_id: 'buckyos.annotation', parent_id: 'data', order_key: 'a', payload: { kind: 'note', body: 'VM 幂等验证' } }]
  const first = await owner.client.commit(ws, operations, key)
  const second = await owner.client.commit(ws, operations, key)
  expect(first.status).toBe('accepted')
  expect(second).toMatchObject({ status: 'accepted', replayed: true })
  if (first.status !== 'accepted' || second.status !== 'accepted') throw new Error('commit was not accepted')
  expect(second.commit_id).toBe(first.commit_id)
  expect(second.seq).toBe(first.seq)
  expect((await owner.client.info(ws)).head_seq).toBe(initial.head_seq + 1)
})
