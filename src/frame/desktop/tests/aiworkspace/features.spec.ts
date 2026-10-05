import type { Page } from '@playwright/test'
import { expect, expectAllCommitted, openApp, openWorkspace, test } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'

function notesBlock(page: Page, blockId: string) {
  return page.getByTestId('aiws-richtext-notes').locator(`[data-block-id="${blockId}"]`)
}

async function typeAtEnd(page: Page, blockId: string, text: string) {
  await notesBlock(page, blockId).click()
  await page.keyboard.press('End')
  await page.keyboard.type(text)
}

test('write locks: policy toggle, acquire, holder display, break, LOCK_LOST keeps my edits, re-acquire', async ({ browser, api }) => {
  const ws = await api.sample(ALICE, `lock ${Date.now()}`)
  await api.rpc(ALICE, 'ws.grant', { workspace_id: ws.workspace_id, subject: 'bob', capabilities: ['read', 'update', 'append', 'comment'] })
  const aliceContext = await browser.newContext({ viewport: { width: 1600, height: 1000 } })
  const bobContext = await browser.newContext({ viewport: { width: 1600, height: 1000 } })
  const alice = await aliceContext.newPage()
  const bob = await bobContext.newPage()
  await openWorkspace(alice, ALICE, ws.workspace_id)
  await openWorkspace(bob, BOB, ws.workspace_id)
  const editable = (page: Page) => page.getByTestId('aiws-richtext-notes').getAttribute('contenteditable')

  // the manager turns the policy on for the rich text
  await alice.locator('[data-testid="aiws-outline-item"][data-entity-id="notes"] > button').click()
  await alice.getByTestId('aiws-toggle-policy').click()
  await expect(alice.getByTestId('aiws-lock-notes')).toBeVisible()
  await expect(bob.getByTestId('aiws-lock-notes')).toBeVisible()
  // nobody holds it: read-only for both; a manager UI only for alice
  expect(await editable(alice)).toBe('false')
  expect(await editable(bob)).toBe('false')
  await expect(bob.getByTestId('aiws-lock-admin')).toHaveCount(0)

  // bob acquires and edits
  await bob.getByTestId('aiws-lock-notes').getByTestId('aiws-lock-acquire').click()
  await expect(bob.getByTestId('aiws-lock-notes')).toHaveAttribute('data-held', 'true')
  expect(await editable(bob)).toBe('true')
  await typeAtEnd(bob, 'n-end', '乙持锁写入。')
  await expectAllCommitted(bob)
  expect(JSON.stringify(await api.ast(ALICE, ws.workspace_id, 'notes'))).toContain('乙持锁写入。')

  // alice sees who holds it, stays read-only, and cannot take it
  await expect(alice.getByTestId('aiws-lock-holder')).toContainText('由 bob 编辑中', { timeout: 20_000 })
  expect(await editable(alice)).toBe('false')
  await alice.getByTestId('aiws-lock-notes').getByRole('button', { name: '再试一次' }).click()
  await expect(alice.getByTestId('aiws-notice').filter({ hasText: '写锁由 bob 持有' })).toBeVisible()
  await expect(alice.getByTestId('aiws-richtext-notes')).toContainText('乙持锁写入。')

  // the manager breaks the lock; bob's next edit is refused with LOCK_LOST and is kept, not dropped
  await alice.locator('[data-testid="aiws-outline-item"][data-entity-id="notes"] > button').click()
  await alice.locator('[data-testid="aiws-outline-item"][data-entity-id="notes"] > button').click()
  await alice.getByTestId('aiws-break-lock').click()
  await expect(alice.getByTestId('aiws-notice').filter({ hasText: '已强制解除' })).toBeVisible()
  await bob.getByTestId('aiws-richtext-notes').locator('[data-block-id="n-end"]').click()
  await bob.keyboard.press('End')
  await bob.keyboard.type('失锁后的输入')
  await expect(bob.getByTestId('aiws-lock-lost')).toBeVisible()
  await expect(bob.getByTestId('aiws-lock-notes')).toHaveAttribute('data-held', 'false')
  expect(await editable(bob)).toBe('false')
  await expect(bob.getByTestId('aiws-save-summary')).toHaveAttribute('data-attention', '1')
  await expect(bob.getByTestId('aiws-edit-entry')).toContainText('修改保留在本窗口')
  await expect(bob.getByTestId('aiws-richtext-notes')).toContainText('失锁后的输入')
  expect(JSON.stringify(await api.ast(ALICE, ws.workspace_id, 'notes'))).not.toContain('失锁后的输入')

  // bob takes the lock again: the kept edits are committed
  await bob.getByTestId('aiws-lock-notes').getByTestId('aiws-lock-acquire').click()
  await expect(bob.getByTestId('aiws-lock-notes')).toHaveAttribute('data-held', 'true')
  await expectAllCommitted(bob)
  await expect.poll(async () => JSON.stringify(await api.ast(ALICE, ws.workspace_id, 'notes'))).toContain('失锁后的输入')

  // releasing makes it available again; a table source can be locked the same way
  await bob.getByTestId('aiws-lock-release').click()
  await expect(bob.getByTestId('aiws-lock-notes')).toHaveAttribute('data-held', 'false')
  expect((await api.rpc(ALICE, 'lock.list', { workspace_id: ws.workspace_id })).locks).toEqual([])
  await alice.locator('[data-testid="aiws-outline-item"][data-entity-id="tasks"] > button').click()
  await alice.getByTestId('aiws-toggle-policy').click()
  const all = alice.getByTestId('aiws-cell-frame-cell-all-tasks')
  await expect(all.getByTestId('aiws-lock-tasks')).toBeVisible()
  await expect(all.getByTestId('aiws-cell-task-41-title')).toHaveAttribute('data-editable', 'false')
  await all.getByTestId('aiws-lock-acquire').click()
  await expect(all.getByTestId('aiws-cell-task-41-title')).toHaveAttribute('data-editable', 'true')
  await aliceContext.close()
  await bobContext.close()
})

test('a refused rich text update is kept as a recoverable draft and the editor returns to the confirmed content', async ({ browser, api }) => {
  const ws = await api.sample(ALICE, `draft ${Date.now()}`)
  await api.rpc(ALICE, 'ws.grant', { workspace_id: ws.workspace_id, subject: 'bob', capabilities: ['read', 'update'] })
  const context = await browser.newContext({ viewport: { width: 1600, height: 1000 } })
  const bob = await context.newPage()
  await openWorkspace(bob, BOB, ws.workspace_id)
  await typeAtEnd(bob, 'n-end', '被接受的一句。')
  await expectAllCommitted(bob)
  // the permission is withdrawn while bob keeps typing
  await api.rpc(ALICE, 'ws.grant', { workspace_id: ws.workspace_id, subject: 'bob', capabilities: ['read'] })
  await bob.keyboard.type('不会被接受的一句。')
  await expect(bob.getByTestId('aiws-notice').filter({ hasText: 'PERMISSION_DENIED' })).toContainText('已保留为草稿')
  await expect(bob.getByTestId('aiws-save-summary')).toHaveAttribute('data-attention', '1')
  // the working document was rebuilt from the confirmed one
  await expect(notesBlock(bob, 'n-end')).toHaveText('以上为未完成任务的实时视图。被接受的一句。')
  // the refused text is recoverable
  const drafts = bob.getByTestId('aiws-drafts')
  await drafts.getByRole('button').first().click()
  await expect(drafts).toContainText('不会被接受的一句。')
  await expect(drafts).toContainText('PERMISSION_DENIED')
  expect(JSON.stringify(await api.ast(ALICE, ws.workspace_id, 'notes'))).not.toContain('不会被接受的一句')
  await context.close()
})

test('unknown commit results are resolved through doc.get_submission and a same-key resend, exactly once', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `unknown ${Date.now()}`)
  await openWorkspace(page, ALICE, ws.workspace_id)
  const all = page.getByTestId('aiws-table-cell-all-tasks')
  const seen: { method: string; key?: string }[] = []
  let mode: 'lose-response' | 'lose-request' | 'pass' = 'lose-response'
  await page.route('**/kapi/aiworkspace', async (route) => {
    const body = route.request().postDataJSON() as { method: string; params: { idempotency_key?: string } }
    if (body.method === 'doc.commit' || body.method === 'doc.get_submission') seen.push({ method: body.method, key: body.params.idempotency_key })
    if (body.method === 'doc.commit' && mode === 'lose-response') {
      mode = 'pass'
      await route.fetch() // the backend accepts it …
      await route.abort('connectionreset') // … but the answer never arrives
      return
    }
    if (body.method === 'doc.commit' && mode === 'lose-request') {
      mode = 'pass'
      await route.abort('connectionreset') // never reaches the backend
      return
    }
    await route.continue()
  })

  // 1) accepted but the response is lost → get_submission says accepted → no second commit
  const before = await api.headSeq(ALICE, ws.workspace_id)
  await all.getByTestId('aiws-cell-task-40-owner').getByRole('button').first().click()
  await page.getByLabel('负责人 task-40', { exact: true }).fill('响应丢失')
  await page.getByLabel('负责人 task-40', { exact: true }).press('Enter')
  await expect(all.getByTestId('aiws-cell-task-40-owner')).toContainText('响应丢失')
  await expectAllCommitted(page)
  expect((await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-40', 'owner')).value).toBe('响应丢失')
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(before + 1)
  expect(seen.map((item) => item.method)).toEqual(['doc.commit', 'doc.get_submission'])
  expect(seen[1].key).toBe(seen[0].key)
  // the commit is on the undo stack although its response never arrived
  await expect(page.getByTestId('aiws-undo')).toHaveText('撤销 1')

  // 2) the request is lost → get_submission says not_found → the same key is sent again
  seen.length = 0
  mode = 'lose-request'
  await all.getByTestId('aiws-cell-task-41-owner').getByRole('button').first().click()
  await page.getByLabel('负责人 task-41', { exact: true }).fill('请求丢失')
  await page.getByLabel('负责人 task-41', { exact: true }).press('Enter')
  await expectAllCommitted(page)
  expect((await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-41', 'owner')).value).toBe('请求丢失')
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(before + 2)
  expect(seen.map((item) => item.method)).toEqual(['doc.commit', 'doc.get_submission', 'doc.commit'])
  expect(new Set(seen.map((item) => item.key)).size).toBe(1)
})

test('annotations, field and option management, migration pre-check, broken view diagnostics', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `schema ${Date.now()}`)
  await openWorkspace(page, ALICE, ws.workspace_id)
  const all = page.getByTestId('aiws-cell-frame-cell-all-tasks')
  const open = page.getByTestId('aiws-cell-frame-cell-open-tasks')

  // annotation on a table cell
  await all.getByTestId('aiws-cell-task-41-owner').hover()
  await all.getByLabel('批注 负责人 task-41', { exact: true }).click()
  await page.getByLabel('批注内容').fill('请确认负责人')
  await page.getByTestId('aiws-annotation-save').click()
  await expect(page.getByTestId('aiws-annotation').filter({ hasText: '请确认负责人' })).toContainText('锚点有效')
  await expect(all.getByTestId('aiws-cell-task-41-owner').getByTestId('aiws-cell-annotation')).toBeVisible()
  // annotation on a rich text block
  await notesBlock(page, 'n-intro').click()
  await page.getByTestId('aiws-annotate-block').click()
  await page.getByLabel('批注内容').fill('这一段需要更新')
  await page.getByTestId('aiws-annotation-save').click()
  await expect(page.getByTestId('aiws-annotation').filter({ hasText: '这一段需要更新' })).toContainText('块 n-intro')
  await expect(page.getByTestId('aiws-annotation')).toHaveCount(3)

  // rename a field: same field_id, both views and the annotation keep pointing at it (V08)
  await page.getByTestId('aiws-fields-cell-all-tasks').click()
  const manager = all.getByTestId('aiws-field-manager')
  await manager.getByTestId('aiws-field-row-owner').getByRole('button', { name: '改名' }).click()
  await page.getByLabel('字段名 owner').fill('责任人')
  await page.getByLabel('字段名 owner').press('Enter')
  await expect(all.locator('.aiws-th').filter({ hasText: '责任人' })).toBeVisible()
  await expect(all.getByTestId('aiws-cell-task-41-owner').getByTestId('aiws-cell-annotation')).toBeVisible()
  await expect(page.getByTestId('aiws-annotation').filter({ hasText: '请确认负责人' })).toContainText('锚点有效')

  // add a field (also added to this view's explicit column list), then an option
  await page.getByLabel('新字段名').fill('标签')
  await page.getByLabel('新字段类型').selectOption('select')
  await page.getByTestId('aiws-add-field').click()
  await expect(all.locator('.aiws-th').filter({ hasText: '标签' })).toBeVisible()
  const source = async () => (await api.rpc(ALICE, 'doc.read', { workspace_id: ws.workspace_id, entity_id: 'tasks' })).content.fields as { field_id: string; name: string; type: string; options?: { option_id: string; label: string }[] }[]
  const tag = (await source()).find((field) => field.name === '标签')
  expect(tag?.type).toBe('select')
  await page.getByLabel(`新选项 ${tag?.field_id}`).fill('紧急')
  await page.getByLabel(`新选项 ${tag?.field_id}`).press('Enter')
  await expect.poll(async () => (await source()).find((field) => field.name === '标签')?.options?.map((option) => option.label)).toEqual(['紧急'])
  // rename an option of an existing field: values keep their option_id, the label changes everywhere
  await page.getByLabel('选项新名 option-open').fill('处理中')
  await manager.getByTestId('aiws-field-row-status').locator('.aiws-chip', { hasText: '进行中' }).getByRole('button', { name: '改', exact: true }).click()
  await expect(open.getByTestId('aiws-cell-task-41-status')).toContainText('处理中')
  // deleting an option that is in use is refused and reported
  await manager.getByTestId('aiws-field-row-status').locator('.aiws-chip', { hasText: '完成' }).getByRole('button', { name: '删', exact: true }).click()
  await expect(page.getByTestId('aiws-edit-entry').filter({ hasText: '删除选项' })).toContainText('未被接受')
  expect((await source()).find((field) => field.field_id === 'status')?.options).toHaveLength(2)

  // migration pre-check: text → date on "owner" cannot convert, nothing changes
  await manager.getByTestId('aiws-migrate-owner').click()
  await page.getByLabel('目标类型').selectOption('date')
  await page.getByTestId('aiws-migration-precheck').click()
  await expect(page.getByTestId('aiws-migration-status')).toContainText('预检未通过')
  await expect(page.getByTestId('aiws-migration-report')).toContainText('无法转换 4')
  expect((await source()).find((field) => field.field_id === 'owner')?.type).toBe('text')
  // number → decimal on a new field converts
  await page.getByLabel('新字段名').fill('工时')
  await page.getByLabel('新字段类型').selectOption('number')
  await page.getByTestId('aiws-add-field').click()
  await expect.poll(async () => (await source()).some((field) => field.name === '工时')).toBe(true)
  const hours = (await source()).find((field) => field.name === '工时')?.field_id as string
  await all.getByTestId(`aiws-cell-task-40-${hours}`).getByRole('button').first().click()
  await page.getByLabel('工时 task-40', { exact: true }).fill('7.5')
  await page.getByLabel('工时 task-40', { exact: true }).press('Enter')
  await expect.poll(async () => (await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-40', hours)).value).toBe(7.5)
  await manager.getByTestId(`aiws-migrate-${hours}`).click()
  await page.getByTestId('aiws-migration-precheck').click()
  await expect(page.getByTestId('aiws-migration-status')).toContainText('预检通过')
  await expect(page.getByTestId('aiws-migration-report')).toContainText('可转换 1')
  await page.getByTestId('aiws-migration-run').click()
  await expect.poll(async () => (await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-40', hours)).value).toBe('7.50')
  await expect(all.getByTestId(`aiws-cell-task-40-${hours}`)).toContainText('7.50')

  // deleting a field a saved filter uses breaks that view visibly (never silently shows more rows)
  await manager.getByTestId('aiws-field-row-status').getByRole('button', { name: '删除', exact: true }).click()
  await expect(open.getByTestId('aiws-table-error-cell-open-tasks')).toContainText('VIEW_BROKEN')
  await expect(open.getByTestId('aiws-diagnostics-cell-open-tasks')).toContainText('已删除的字段')
  await expect(open.getByTestId('aiws-row')).toHaveCount(0)
  // undo restores the field and the view works again without having been rewritten
  await page.getByTestId('aiws-undo').click()
  await expect(open.getByTestId('aiws-row')).toHaveCount(4)
  await expect(open.getByTestId('aiws-table-error-cell-open-tasks')).toHaveCount(0)

  // deleting an annotated record: the annotation stays and reports its target as deleted
  await all.getByTestId('aiws-row').filter({ has: page.getByTestId('aiws-cell-task-41-title') }).getByRole('button', { name: '删除' }).click()
  await expect(all.getByTestId('aiws-row')).toHaveCount(4)
  await expect(page.getByTestId('aiws-annotation').filter({ hasText: '请确认负责人' })).toContainText('目标已删除')
})

test('session filter and sort stay local until "save view"; structure edits: create, rename, reorder, move, delete', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `view ${Date.now()}`)
  await openWorkspace(page, ALICE, ws.workspace_id)
  const all = page.getByTestId('aiws-cell-frame-cell-all-tasks')
  const open = page.getByTestId('aiws-cell-frame-cell-open-tasks')
  const cellPayload = async (id: string) => (await api.rpc(ALICE, 'doc.read', { workspace_id: ws.workspace_id, entity_id: id })).content.payload
  const head = await api.headSeq(ALICE, ws.workspace_id)

  await all.getByRole('button', { name: '筛选 / 排序' }).click()
  await all.getByLabel('筛选字段').selectOption('owner')
  await all.getByLabel('筛选值').fill('林')
  await all.getByLabel('排序字段').selectOption('budget')
  await all.getByLabel('排序方向').selectOption('desc')
  await all.getByTestId('aiws-filter-apply').click()
  await expect(all.getByTestId('aiws-row')).toHaveCount(2)
  await expect(all.getByTestId('aiws-row').first()).toContainText('完成第一期架构验证')
  // session state: no commit, the other view and the stored view are untouched
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head)
  await expect(open.getByTestId('aiws-row')).toHaveCount(4)
  expect((await cellPayload('cell-all-tasks')).filter).toBeUndefined()

  await all.getByTestId('aiws-view-save').click()
  await expectAllCommitted(page)
  const saved = await cellPayload('cell-all-tasks')
  expect(saved.filter).toEqual({ op: 'cmp', field_id: 'owner', operator: 'eq', value: '林' })
  expect(saved.sorts).toEqual([{ field_id: 'budget', direction: 'desc' }])
  await expect(all.getByTestId('aiws-row')).toHaveCount(2)
  // saving a view does not change the source
  expect((await api.rpc(ALICE, 'doc.read', { workspace_id: ws.workspace_id, entity_id: 'tasks' })).content_rev).toBe(2)

  // ---- structure
  const order = async () => (await api.rpc(ALICE, 'doc.list_children', { workspace_id: ws.workspace_id, entity_id: 'page-main' })).children
    .filter((child: { type_id: string; kind?: string }) => child.type_id === 'buckyos.cell' || child.kind === 'group').map((child: { entity_id: string }) => child.entity_id) as string[]
  const frames = () => page.locator('[data-testid="aiws-page"] > .aiws-flow > [data-cell-id]').evaluateAll((nodes) => nodes.map((node) => node.getAttribute('data-cell-id')))
  expect(await order()).toEqual(['cell-notes', 'cell-all-tasks', 'cell-open-tasks', 'cell-info', 'cell-diagram'])
  // reorder with the buttons (tree.place, order key from the WASM core)
  await page.getByTestId('aiws-down-cell-notes').click()
  await expect.poll(order).toEqual(['cell-all-tasks', 'cell-notes', 'cell-open-tasks', 'cell-info', 'cell-diagram'])
  await expect.poll(frames).toEqual(['cell-all-tasks', 'cell-notes', 'cell-open-tasks', 'cell-info', 'cell-diagram'])
  await page.getByTestId('aiws-up-cell-diagram').click()
  await expect.poll(order).toEqual(['cell-all-tasks', 'cell-notes', 'cell-open-tasks', 'cell-diagram', 'cell-info'])

  // create a group and a rich text in the page
  await page.getByTestId('aiws-add-open-page-main').click()
  await page.getByLabel('标题', { exact: true }).fill('附录')
  await page.getByTestId('aiws-new-group').click()
  await expect(page.locator('[data-testid^="aiws-group-"]')).toHaveCount(1)
  const groupId = await page.locator('[data-testid^="aiws-group-"]').getAttribute('data-cell-id') as string
  await page.getByTestId('aiws-add-open-page-main').click()
  await page.getByLabel('标题', { exact: true }).fill('会议纪要')
  await page.getByTestId('aiws-new-richtext').click()
  const newCell = page.locator('[data-testid^="aiws-cell-frame-"]').filter({ hasText: '会议纪要' })
  await expect(newCell).toBeVisible()
  await newCell.locator('.aiws-prose').click()
  await page.keyboard.type('今天的结论')
  await expectAllCommitted(page)
  const newCellId = await newCell.getAttribute('data-cell-id') as string
  const newTextId = await newCell.getAttribute('data-cell-source') as string
  expect(JSON.stringify(await api.ast(ALICE, ws.workspace_id, newTextId))).toContain('今天的结论')

  // move the new cell into the group through the outline (tree.move), rename its source, then delete the group with its subtree
  await page.locator(`[data-testid="aiws-outline-item"][data-entity-id="${newCellId}"] > button`).click()
  await page.getByLabel('移动到').selectOption(groupId)
  await expect(page.locator(`[data-testid="aiws-group-${groupId}"] [data-cell-id="${newCellId}"]`)).toBeVisible()
  await expect.poll(async () => (await api.rpc(ALICE, 'doc.read', { workspace_id: ws.workspace_id, entity_id: newCellId })).parent_id).toBe(groupId)
  await page.locator(`[data-testid="aiws-outline-item"][data-entity-id="${newTextId}"] > button`).click()
  await page.getByLabel('查找名').fill('纪要正文')
  await page.getByTestId('aiws-rename').click()
  await expect.poll(async () => (await api.rpc(ALICE, 'doc.resolve', { workspace_id: ws.workspace_id, path: '/项目工作区/纪要正文' })).reference?.entity_id).toBe(newTextId)
  await page.locator(`[data-testid="aiws-outline-item"][data-entity-id="${groupId}"] > button`).click()
  await page.getByTestId('aiws-delete-entity').click()
  await expect(page.locator('[data-testid^="aiws-group-"]')).toHaveCount(0)
  await expect(page.locator(`[data-cell-id="${newCellId}"]`)).toHaveCount(0)
  // a source that a cell still shows cannot be deleted: the refusal is reported, nothing is lost
  await page.locator('[data-testid="aiws-outline-item"][data-entity-id="tasks"] > button').click()
  await page.getByTestId('aiws-delete-entity').click()
  await expect(page.getByTestId('aiws-edit-entry').filter({ hasText: '删除 任务' })).toContainText('REFERENCE_BROKEN')
  await expect(all.getByTestId('aiws-row')).toHaveCount(2)
})

test('packages: export, import as new, fork, and a restore that replaces history stops the open window (EPOCH_MISMATCH)', async ({ browser, api }) => {
  test.setTimeout(150_000)
  const title = `包 ${Date.now()}`
  const ws = await api.sample(ALICE, title)
  const context = await browser.newContext({ viewport: { width: 1600, height: 1000 }, acceptDownloads: true })
  const list = await context.newPage()
  await openApp(list, ALICE)
  const card = list.locator(`[data-testid="aiws-workspace-card"][data-workspace-id="${ws.workspace_id}"]`)

  // share export → import as a new workspace
  const [share] = await Promise.all([list.waitForEvent('download'), card.getByTestId('aiws-export').click()])
  const sharePath = await share.path()
  await expect(card.getByTestId('aiws-export-manifest')).toContainText('分享')
  const importForm = list.getByTestId('aiws-import')
  await importForm.getByTestId('aiws-import-file').setInputFiles({ name: 'share.zip', mimeType: 'application/zip', buffer: (await import('node:fs')).readFileSync(sharePath) })
  // the semantics have no default: nothing can be imported before choosing
  await expect(importForm.getByTestId('aiws-import-submit')).toBeDisabled()
  await importForm.getByLabel('作为新的工作区').check()
  await importForm.getByTestId('aiws-import-submit').click()
  await expect(list.getByTestId('aiws-list-message')).toContainText('已导入为新的工作区')
  const imported = (await list.getByTestId('aiws-list-message').innerText()).match(/ws_[a-z0-9]+/)?.[0] as string
  expect(imported).not.toBe(ws.workspace_id)
  expect((await api.rpc(ALICE, 'doc.outline', { workspace_id: imported })).entities).toHaveLength(13)
  expect((await api.rpc(ALICE, 'doc.checkpoint', { workspace_id: imported })).content_root).toBe((await api.rpc(ALICE, 'doc.checkpoint', { workspace_id: ws.workspace_id })).content_root)

  // fork
  await card.getByTestId('aiws-fork').click()
  await expect(list.getByTestId('aiws-list-message')).toContainText('已 Fork 为新的工作区')

  // personal backup, then an edit in a second window, then restore + replace: the open window must stop
  await card.getByLabel('导出方式').selectOption('personal_backup')
  const [backup] = await Promise.all([list.waitForEvent('download'), card.getByTestId('aiws-export').click()])
  const backupBytes = (await import('node:fs')).readFileSync(await backup.path())
  const editor = await context.newPage()
  await openWorkspace(editor, ALICE, ws.workspace_id)
  const all = editor.getByTestId('aiws-table-cell-all-tasks')
  await all.getByTestId('aiws-cell-task-40-owner').getByRole('button').first().click()
  await editor.getByLabel('负责人 task-40', { exact: true }).fill('备份之后')
  await editor.getByLabel('负责人 task-40', { exact: true }).press('Enter')
  await expectAllCommitted(editor)

  await importForm.getByTestId('aiws-import-file').setInputFiles({ name: 'backup.zip', mimeType: 'application/zip', buffer: backupBytes })
  await importForm.getByLabel('恢复为包内的同一工作区').check()
  await importForm.getByLabel('若该工作区已存在则替换').check()
  await importForm.getByTestId('aiws-import-submit').click()
  await expect(list.getByTestId('aiws-list-message')).toContainText('已恢复工作区')
  const info = await api.rpc(ALICE, 'ws.get_info', { workspace_id: ws.workspace_id })
  expect(info.epoch).not.toBe(ws.epoch)
  expect((await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-40', 'owner')).value).toBe('林')

  // the window on the old history: told, stopped, and its next edit is neither sent nor dropped
  await all.getByTestId('aiws-cell-task-41-owner').getByRole('button').first().click()
  await editor.getByLabel('负责人 task-41', { exact: true }).fill('旧历史上的修改')
  await editor.getByLabel('负责人 task-41', { exact: true }).press('Enter')
  await expect(editor.getByTestId('aiws-stopped')).toContainText('历史已被恢复操作替换', { timeout: 40_000 })
  await expect(editor.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'stopped')
  const kept = all.getByTestId('aiws-cell-task-41-owner')
  await expect(kept).toHaveAttribute('data-state', 'needs_attention')
  await expect(kept.getByTestId('aiws-conflict-mine')).toHaveText('旧历史上的修改')
  expect((await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-41', 'owner')).value).toBe('王')
  await context.close()
})

test('pasted blocks always get new block ids; joining keeps the first block id', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `paste ${Date.now()}`)
  await openWorkspace(page, ALICE, ws.workspace_id)
  await notesBlock(page, 'n-end').click()
  await page.keyboard.press('End')
  // paste HTML that carries block ids already used in this document
  await page.getByTestId('aiws-richtext-notes').evaluate((element) => {
    const data = new DataTransfer()
    data.setData('text/html', '<p data-block-id="n-title">粘贴的第一段</p><p data-block-id="n-title">粘贴的第二段</p><ul data-block-id="n-list"><li data-block-id="n-li1"><p data-block-id="n-li1p">粘贴的列表项</p></li></ul>')
    data.setData('text/plain', '粘贴的第一段\n粘贴的第二段\n粘贴的列表项')
    element.dispatchEvent(new ClipboardEvent('paste', { clipboardData: data, bubbles: true, cancelable: true }))
  })
  await expect(page.getByTestId('aiws-richtext-notes')).toContainText('粘贴的列表项')
  await expectAllCommitted(page)
  const ast = await api.ast(ALICE, ws.workspace_id, 'notes')
  const ids: string[] = []
  const walk = (node: { attrs?: { block_id?: string }; content?: unknown[] }) => {
    if (node.attrs?.block_id) ids.push(node.attrs.block_id)
    for (const child of node.content ?? []) walk(child as never)
  }
  walk(ast)
  expect(new Set(ids).size).toBe(ids.length)
  for (const id of ids) expect(id).toMatch(/^[a-z0-9][a-z0-9_-]{0,63}$/)
  expect(JSON.stringify(ast)).toContain('粘贴的第二段')
  // the original blocks kept their ids
  for (const original of ['n-title', 'n-intro', 'n-list', 'n-li1', 'n-li1p', 'n-embed', 'n-end']) expect(ids).toContain(original)

  // join: Backspace at the start of the second pasted paragraph merges it into the block before it, which keeps its id
  const before = ids.length
  await page.getByTestId('aiws-richtext-notes').getByText('粘贴的第二段').click()
  await page.keyboard.press('Home')
  await page.keyboard.press('Backspace')
  await expectAllCommitted(page)
  const joined = await api.ast(ALICE, ws.workspace_id, 'notes')
  const after: string[] = []
  const collect = (node: { attrs?: { block_id?: string }; content?: unknown[] }) => {
    if (node.attrs?.block_id) after.push(node.attrs.block_id)
    for (const child of node.content ?? []) collect(child as never)
  }
  collect(joined)
  expect(after.length).toBe(before - 1)
  expect(after).toContain('n-end')
  expect(JSON.stringify(joined)).toContain('粘贴的第一段粘贴的第二段')
})

test('record properties, asset upload, object embed and object link', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `objects ${Date.now()}`)
  await openWorkspace(page, ALICE, ws.workspace_id)

  // record: each property is its own version cell
  const record = page.getByTestId('aiws-cell-frame-cell-info').getByTestId('aiws-record-project-info')
  await record.getByTestId('aiws-prop-owner').getByRole('button').first().click()
  await page.getByLabel('负责人', { exact: true }).fill('王')
  await page.getByLabel('负责人', { exact: true }).press('Enter')
  await record.getByTestId('aiws-prop-deadline').getByRole('button').first().click()
  await page.getByLabel('截止', { exact: true }).fill('2026-11-01')
  await page.getByLabel('截止', { exact: true }).press('Enter')
  await expectAllCommitted(page)
  const info = (await api.rpc(ALICE, 'doc.read', { workspace_id: ws.workspace_id, entity_id: 'project-info' })).content
  expect(info.props).toEqual({ owner: '王', deadline: '2026-11-01', budget: '1200.00' })
  // a stale write to a property conflicts and shows both values
  await record.getByTestId('aiws-prop-budget').getByRole('button').first().click()
  await page.getByLabel('预算', { exact: true }).fill('999')
  expect((await api.commit(ALICE, ws, [{ op: 'entity.set_keys', entity_id: 'project-info', keys: [{ key: 'p:budget', value: '1500.00', expect: { rev: info.key_revs['p:budget'] } }] }])).status).toBe('accepted')
  await expect(page.getByTestId('aiws-cell-frame-cell-info')).toBeVisible()
  await page.getByLabel('预算', { exact: true }).press('Enter')
  const budget = record.getByTestId('aiws-prop-budget')
  await expect(budget).toHaveAttribute('data-state', 'needs_attention')
  await expect(budget.getByTestId('aiws-conflict-theirs')).toHaveText('1500.00')
  await expect(budget.getByTestId('aiws-conflict-mine')).toHaveText('999')
  await budget.getByTestId('aiws-conflict-discard').click()
  await expect(budget).toContainText('1500.00')

  // asset: upload → verified object → asset-ref + cell in one commit; shown through the authenticated route
  const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAABAAAAAJCAIAAAC0SDtlAAAAFElEQVR42mPQiDpBEmIY1TAoNAAAk1q5oVLxFXwAAAAASUVORK5CYII=', 'base64')
  await page.getByTestId('aiws-add-open-page-main').click()
  await page.getByTestId('aiws-new-image').setInputFiles({ name: '新图.png', mimeType: 'image/png', buffer: png })
  await expect(page.getByTestId('aiws-asset-image')).toHaveCount(2)
  const outline = (await api.rpc(ALICE, 'doc.outline', { workspace_id: ws.workspace_id })).entities as { entity_id: string; type_id: string }[]
  const assets = outline.filter((entity) => entity.type_id === 'buckyos.asset-ref')
  expect(assets).toHaveLength(2)
  const uploaded = assets.find((entity) => entity.entity_id !== 'diagram') as { entity_id: string }
  const content = (await api.rpc(ALICE, 'doc.read', { workspace_id: ws.workspace_id, entity_id: uploaded.entity_id })).content
  expect(content.availability).toBe('available')
  expect(content.payload.media_type).toBe('image/png')
  expect(content.payload.size).toBe(png.length)

  // object embed (a cell, rendered read-only inside the document) and object link
  await notesBlock(page, 'n-end').click()
  await page.keyboard.press('End')
  await page.getByRole('button', { name: '对象链接…' }).click()
  await page.getByLabel('对象链接', { exact: true }).selectOption('project-info')
  await page.getByRole('button', { name: '嵌入单元…' }).click()
  await page.getByLabel('嵌入单元', { exact: true }).selectOption('cell-info')
  await expect(page.getByTestId('aiws-embed-cell-info')).toContainText('王')
  await expectAllCommitted(page)
  const ast = await api.ast(ALICE, ws.workspace_id, 'notes')
  const text = JSON.stringify(ast)
  expect(text).toContain('"type":"object_link"')
  expect(ast.content.filter((node: { type: string }) => node.type === 'object_embed').map((node: { attrs: { ref: { entity_id: string } } }) => node.attrs.ref.entity_id).sort()).toEqual(['cell-info', 'cell-open-tasks'])
  expect(text).toContain('"entity_id":"project-info"')
  // the embedded record is live: a change of the source shows up inside the document
  await record.getByTestId('aiws-prop-owner').getByRole('button').first().click()
  await page.getByLabel('负责人', { exact: true }).fill('赵')
  await page.getByLabel('负责人', { exact: true }).press('Enter')
  await expect(page.getByTestId('aiws-embed-cell-info')).toContainText('赵')
  // clicking an object link selects its target in the outline
  await page.getByTestId('aiws-richtext-notes').locator('.aiws-object-link', { hasText: '任务 42 详情' }).click()
  await expect(page.locator('[data-testid="aiws-outline-item"][data-entity-id="task-42-details"]')).toHaveAttribute('aria-selected', 'true')
})

test('an unreachable backend is shown as such, edits stay unsaved, and work resumes when it is back', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `offline ${Date.now()}`)
  await openWorkspace(page, ALICE, ws.workspace_id)
  const all = page.getByTestId('aiws-table-cell-all-tasks')
  await expect(all.getByTestId('aiws-row')).toHaveCount(5)
  let down = true
  await page.route('**/kapi/aiworkspace', async (route) => { if (down) await route.abort('connectionrefused'); else await route.continue() })
  await all.getByTestId('aiws-cell-task-40-owner').getByRole('button').first().click()
  await page.getByLabel('负责人 task-40', { exact: true }).fill('断线时的输入')
  await page.getByLabel('负责人 task-40', { exact: true }).press('Enter')
  const cell = all.getByTestId('aiws-cell-task-40-owner')
  // not accepted, not refused: unsaved, with my input still shown
  await expect(cell).toHaveAttribute('data-state', 'unsaved')
  await expect(cell.getByTestId('aiws-edit-state')).toHaveText('未保存')
  await expect(cell).toContainText('断线时的输入')
  await expect(page.getByTestId('aiws-save-summary')).toHaveAttribute('data-unsaved', '1')
  expect((await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-40', 'owner')).value).toBe('林')
  down = false
  await cell.getByRole('button', { name: '重试' }).click()
  await expectAllCommitted(page)
  expect((await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-40', 'owner')).value).toBe('断线时的输入')
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live', { timeout: 20_000 })
})

test('a new table from the UI: add record, boolean / multi-select / datetime / number editors; delete a workspace', async ({ page, api }) => {
  const title = `空白 ${Date.now()}`
  await openApp(page, ALICE)
  await page.getByLabel('工作区标题', { exact: true }).fill(title)
  await page.getByTestId('aiws-create').click()
  const card = page.getByTestId('aiws-workspace-card').filter({ hasText: title })
  const workspaceId = await card.getAttribute('data-workspace-id') as string
  await card.getByTestId('aiws-open').click()
  // an empty workspace has only the root: the page is created explicitly
  await expect(page.getByTestId('aiws-outline-item')).toHaveCount(1)
  await page.getByTestId('aiws-create-page').click()
  await page.getByTestId('aiws-add-open-page-main').click()
  await page.getByLabel('标题', { exact: true }).fill('清单')
  await page.getByTestId('aiws-new-table').click()
  const frame = page.locator('[data-testid^="aiws-cell-frame-"]').filter({ hasText: '清单' })
  await expect(frame).toBeVisible()
  const sourceId = await frame.getAttribute('data-cell-source') as string
  const cellId = await frame.getAttribute('data-cell-id') as string

  // add two records through the form (the required title is asked for)
  for (const name of ['买菜', '写周报']) {
    await page.getByTestId(`aiws-add-record-${cellId}`).click()
    await page.getByLabel('新记录 标题').fill(name)
    await page.getByTestId('aiws-add-record-submit').click()
    await expect(frame.getByTestId('aiws-row').filter({ hasText: name })).toBeVisible()
  }
  await expect(frame.getByTestId('aiws-row')).toHaveCount(2)
  const rows = async () => (await api.rpc(ALICE, 'doc.query', { workspace_id: workspaceId, source_id: sourceId })).rows as { record_id: string; values: Record<string, unknown> }[]
  const first = (await rows()).find((row) => row.values.title === '买菜')?.record_id as string

  // boolean
  await frame.getByTestId(`aiws-cell-${first}-done`).getByRole('button').first().click()
  await page.getByLabel(`完成 ${first}`, { exact: true }).selectOption('true')
  await expect(frame.getByTestId(`aiws-cell-${first}-done`)).toContainText('是')
  // more field types
  await page.getByTestId(`aiws-fields-${cellId}`).click()
  for (const [name, type] of [['标签', 'multi_select'], ['提醒', 'datetime'], ['数量', 'number']]) {
    await page.getByLabel('新字段名').fill(name)
    await page.getByLabel('新字段类型').selectOption(type)
    await page.getByTestId('aiws-add-field').click()
    await expect(frame.locator('.aiws-th').filter({ hasText: name })).toBeVisible()
  }
  const fields = (await api.rpc(ALICE, 'doc.read', { workspace_id: workspaceId, entity_id: sourceId })).content.fields as { field_id: string; name: string }[]
  const id = (name: string) => fields.find((field) => field.name === name)?.field_id as string
  for (const label of ['家', '工作']) {
    await page.getByLabel(`新选项 ${id('标签')}`).fill(label)
    await page.getByLabel(`新选项 ${id('标签')}`).press('Enter')
    await expect(frame.getByTestId('aiws-field-manager').locator('.aiws-chip', { hasText: label })).toBeVisible()
  }
  await page.getByTestId(`aiws-fields-${cellId}`).click()
  // multi_select
  await frame.getByTestId(`aiws-cell-${first}-${id('标签')}`).getByRole('button').first().click()
  const group = page.getByRole('group', { name: `标签 ${first}` })
  await group.getByLabel('工作').check()
  await group.getByLabel('家').check()
  await group.getByRole('button', { name: '确定' }).click()
  await expect(frame.getByTestId(`aiws-cell-${first}-${id('标签')}`)).toContainText('家、工作')
  // datetime: entered in local time, stored as a UTC instant
  await frame.getByTestId(`aiws-cell-${first}-${id('提醒')}`).getByRole('button').first().click()
  await page.getByLabel(`提醒 ${first}`, { exact: true }).fill('2026-10-04T16:00')
  await page.getByLabel(`提醒 ${first}`, { exact: true }).press('Enter')
  // number: text that is not a number never leaves the editor; a number is stored as a number
  await frame.getByTestId(`aiws-cell-${first}-${id('数量')}`).getByRole('button').first().click()
  await page.getByLabel(`数量 ${first}`, { exact: true }).fill('很多')
  await page.getByLabel(`数量 ${first}`, { exact: true }).press('Enter')
  await expect(frame.getByRole('alert').filter({ hasText: '不是有效的数字' })).toBeVisible()
  await page.getByLabel(`数量 ${first}`, { exact: true }).fill('3')
  await page.getByLabel(`数量 ${first}`, { exact: true }).press('Enter')
  await expectAllCommitted(page)
  const stored = (await rows()).find((row) => row.record_id === first)?.values as Record<string, unknown>
  expect(stored.done).toBe(true)
  expect([...(stored[id('标签')] as string[])].length).toBe(2)
  expect(stored[id('提醒')]).toBe(await page.evaluate(() => new Date('2026-10-04T16:00').toISOString()))
  expect(stored[id('数量')]).toBe(3)

  // delete the workspace from the list (explicit confirmation)
  await page.getByTestId('aiws-back').click()
  const again = page.locator(`[data-testid="aiws-workspace-card"][data-workspace-id="${workspaceId}"]`)
  await again.getByTestId('aiws-delete').click()
  await again.getByTestId('aiws-delete-confirm').click()
  await expect(again).toHaveCount(0)
  expect((await api.rpc(ALICE, 'ws.list', {})).workspaces.some((item: { workspace_id: string }) => item.workspace_id === workspaceId)).toBe(false)
})
