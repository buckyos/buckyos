import { expect, expectAllCommitted, openWorkspace, setCanvasMode, test } from './fixtures'

test('a comment-only account can annotate a read-only table in edit mode', async ({ page, api }) => {
  const ws = await api.sample('tok-alice', 'Comment-only table')
  const granted = await api.rpc('tok-alice', 'acl.set', {
    workspace_id: ws.workspace_id,
    changes: [{ subject: 'bob', resource: '/', allow: ['read', 'comment'] }],
  })
  expect(granted.ok).toBe(true)
  const before = await api.cell('tok-alice', ws.workspace_id, 'tasks', 'task-41', 'owner')
  await openWorkspace(page, 'tok-bob', ws.workspace_id)
  await setCanvasMode(page, 'edit')
  const cell = page.getByTestId('aiws-table-cell-all-tasks').getByTestId('aiws-cell-task-41-owner')
  await expect(cell.getByRole('button').first()).toBeDisabled()
  await cell.hover()
  await cell.getByRole('button', { name: '评论 负责人 task-41', exact: true }).click()
  await page.getByLabel('评论内容').fill('Feedback without changing the source')
  await page.getByTestId('aiws-annotation-save').click()
  await expectAllCommitted(page)
  const annotations = await api.rpc('tok-alice', 'doc.list_annotations', { workspace_id: ws.workspace_id, target_ids: ['tasks'] })
  const note = annotations.annotations.find((item: { content: { payload: { body?: string } } }) => item.content.payload.body === 'Feedback without changing the source')
  expect(note.content.payload.author).toBe('bob')
  const rejected = await api.commit('tok-bob', ws, [{
    op: 'table.set_values', source_id: 'tasks',
    values: [{ record_id: 'task-41', field_id: 'owner', value: 'must not persist', expect: { rev: before.rev } }],
  }])
  expect(rejected).toMatchObject({ status: 'rejected', code: 'PERMISSION_DENIED' })
  expect(await api.cell('tok-alice', ws.workspace_id, 'tasks', 'task-41', 'owner')).toEqual(before)
})
