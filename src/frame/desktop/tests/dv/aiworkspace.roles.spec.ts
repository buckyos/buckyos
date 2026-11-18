import { createHash, randomBytes } from 'node:crypto'
import { readFileSync } from 'node:fs'
import type { BrowserContext, Page } from '@playwright/test'
import type { ReadOk } from '../../src/app/aiworkspace/api/session'
import type { AnnotationPayload, CommitEvent, EntityEnvelope, ExportResult, KeyedContent, Operation, Result, WorkspaceInfo } from '../../src/app/aiworkspace/api/types'
import { setCanvasMode } from '../aiworkspace/fixtures'
import { zipEntries, zipRead } from '../../../aiworkspace/import-table/xlsx'
import { createSample, expect, expectAllCommitted, login, openWorkspace, screenshot, test as base, type Actor } from './aiworkspace.fixtures'

type Workspace = Pick<WorkspaceInfo, 'workspace_id' | 'epoch'>
type AnnotationRead = ReadOk<KeyedContent<AnnotationPayload>>
type Participants = { commenter: Actor; editor: Actor; outsider: Actor }

function hash(user: string, password: string, nonce?: number) {
  const stored = createHash('sha256').update(password + user + '.buckyos').digest('base64')
  return nonce === undefined ? stored : createHash('sha256').update(stored + nonce).digest('base64')
}

const test = base.extend<{ participants: Participants }>({
  participants: async ({ owner, browser, baseURL }, use) => {
    const passwordPath = process.env.BUCKYOS_TEST_ADMIN_PASSWORD_FILE
    const password = passwordPath ? readFileSync(passwordPath, 'utf8').trimEnd() : process.env.BUCKYOS_TEST_ADMIN_PASSWORD!
    const sudo = async () => {
      const nonce = Date.now()
      const pair = await owner.client.service<{ session_token: string }>('verify-hub', 'sudo_by_password', {
        username: owner.userId, password: hash(owner.userId, password, nonce),
        target: { kind: 'system', service_id: 'control-panel' }, aud: 'system-config', login_nonce: nonce,
      })
      return pair.session_token
    }
    const users: string[] = []
    const contexts: BrowserContext[] = []
    const actors = {} as Participants
    try {
      for (const role of ['commenter', 'editor', 'outsider'] as const) {
        const userId = 'dv741' + role + randomBytes(5).toString('hex')
        const value = 'Dv-' + randomBytes(18).toString('hex') + '!'
        const created = await owner.client.service<{ created: boolean }>('control-panel', 'user.create', {
          user_id: userId, show_name: 'Workspace DV ' + role, password_hash: hash(userId, value),
        }, await sudo())
        if (created.created) users.push(userId)
        expect(created.created).toBe(true)
        const context = await browser.newContext({ baseURL, ignoreHTTPSErrors: false, viewport: { width: 1600, height: 1000 } })
        contexts.push(context)
        await context.addInitScript(() => localStorage.setItem('buckyos.prototype.locale.v1', 'zh-CN'))
        actors[role] = await login(await context.newPage(), userId, value)
      }
      await use(actors)
    } finally {
      for (const context of contexts) await context.close()
      for (const user_id of users) {
        expect(await owner.client.service('control-panel', 'user.delete', { user_id }, await sudo())).toMatchObject({ ok: true, user_id })
        expect(await owner.client.service('control-panel', 'user.get', { user_id }, await sudo())).toMatchObject({ state: 'deleted' })
      }
    }
  },
})

async function grant(owner: Actor, ws: Workspace, actor: Actor, capabilities: string[]) {
  await owner.client.ok('acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: actor.userId, resource: '/', allow: capabilities }] })
}

async function read<T = EntityEnvelope>(actor: Actor, ws: Workspace, entity_id: string) {
  return actor.client.ok<T>('doc.read', { workspace_id: ws.workspace_id, entity_id })
}

test('real accounts cannot read another private workspace through data, annotations, exports, history or references', async ({ owner, participants }, info) => {
  const { editor, outsider, commenter } = participants
  const ws = await createSample(owner)
  await grant(owner, ws, editor, ['read', 'update', 'comment'])
  await grant(owner, ws, commenter, ['read', 'comment'])
  await openWorkspace(editor, ws)
  await openWorkspace(commenter, ws)
  expect((await editor.client.info(ws)).workspace_id).toBe(ws.workspace_id)
  expect((await outsider.client.list()).workspaces.some(item => item.workspace_id === ws.workspace_id)).toBe(false)
  const requests = [
    ['ws.get_info', {}],
    ['doc.read', { entity_id: 'notes' }],
    ['doc.outline', {}],
    ['doc.list', { at: '/data' }],
    ['doc.list_annotations', { target_ids: ['notes'] }],
    ['doc.get_changes', { epoch: ws.epoch, after_seq: 0 }],
    ['doc.export', { mode: 'share' }],
    ['replica.bootstrap', {}],
    ['doc.resolve', { reference: { entity_id: 'notes' } }],
    ['doc.query', { sql: 'SELECT * FROM tasks' }],
  ] as const
  for (const [method, params] of requests) {
    const result = await outsider.client.call<Result<unknown>>(method, { workspace_id: ws.workspace_id, ...params })
    expect(result, method).toMatchObject({ ok: false, error: { code: 'NOT_FOUND' } })
  }
  for (const targets of [[], [{ entity_id: 'notes' }]]) {
    const batch = await outsider.client.call('doc.read', { workspace_id: ws.workspace_id, targets })
    expect.soft(batch, 'private batch must hide workspace metadata').toMatchObject({ ok: false, error: { code: 'NOT_FOUND' } })
    expect.soft(batch).not.toHaveProperty('head_seq')
  }
  await owner.client.ok('acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: editor.userId, resource: '/' }] })
  expect(await editor.client.call('doc.read', { workspace_id: ws.workspace_id, entity_id: 'notes' })).toMatchObject({ ok: false, error: { code: 'NOT_FOUND' } })
  await editor.page.reload()
  await expect(editor.page.getByTestId('aiws-workspace')).toHaveCount(0)
  await screenshot(commenter.page, info, 'commenter-can-read-private-workspace')
})

test('real accounts keep private subtrees out of batch reads, indexes, SQL, annotations and change history', async ({ owner, participants }) => {
  const { editor, outsider, commenter } = participants
  const ws = await createSample(owner)
  for (const actor of [editor, outsider, commenter]) await grant(owner, ws, actor, ['read', 'comment', 'export'])
  await owner.client.ok('acl.set', {
    workspace_id: ws.workspace_id,
    changes: [
      { subject: outsider.userId, resource: '/data/项目说明', deny: ['read'] },
      { subject: editor.userId, resource: '/data/任务', deny: ['read'] },
    ],
  })
  expect(await outsider.client.call('doc.read', { workspace_id: ws.workspace_id, entity_id: 'notes' })).toMatchObject({ ok: false, error: { code: 'PERMISSION_DENIED' } })
  expect(await editor.client.call('doc.read', { workspace_id: ws.workspace_id, entity_id: 'tasks' })).toMatchObject({ ok: false, error: { code: 'PERMISSION_DENIED' } })
  expect((await read(editor, ws, 'notes')).entity_id).toBe('notes')
  expect((await read(commenter, ws, 'notes')).entity_id).toBe('notes')
  const batch = await outsider.client.ok<{ results: Result<EntityEnvelope>[] }>('doc.read', {
    workspace_id: ws.workspace_id, targets: [{ entity_id: 'notes' }, { entity_id: 'tasks' }],
  })
  expect(batch.results[0]).toMatchObject({ ok: false, error: { code: 'PERMISSION_DENIED' } })
  expect(batch.results[1]).toMatchObject({ ok: true, entity_id: 'tasks' })
  const outline = await outsider.client.ok<{ entities: EntityEnvelope[] }>('doc.outline', { workspace_id: ws.workspace_id })
  expect(JSON.stringify(outline)).not.toContain('"entity_id":"notes"')
  expect(await outsider.client.call('doc.resolve', { workspace_id: ws.workspace_id, path: '/data/项目说明' })).toMatchObject({ ok: false, error: { code: 'NOT_FOUND' } })
  expect(await editor.client.call('doc.query', { workspace_id: ws.workspace_id, sql: 'SELECT * FROM tasks' })).toMatchObject({ ok: false, error: { code: 'NOT_FOUND' } })
  expect(await owner.client.commit(ws, [{
    op: 'richtext.insert_blocks', entity_id: 'notes', position: { end: true },
    blocks: [{ type: 'paragraph', attrs: { block_id: 'private-synthetic-block' }, content: [{ type: 'text', text: 'private-synthetic-source' }] }],
  }, {
    op: 'entity.create', entity_id: 'context-check', type_id: 'buckyos.wish', parent_id: 'data', order_key: 'zy',
    payload: { title: 'Context permissions', prompt: 'Synthetic input check', executor: 'xllm', output_mode: 'overwrite' },
  }])).toMatchObject({ status: 'accepted' })
  await owner.client.ok('acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: outsider.userId, resource: 'context-check', allow: ['update'] }] })
  const context = await outsider.client.commit(ws, [{
    op: 'entity.set_keys', entity_id: 'context-check',
    keys: [{ key: 'inputs', value: [{ entity_id: 'notes', version: { mode: 'follow' } }], expect: { rev: 0 } }],
  }])
  expect(context).toMatchObject({ status: 'rejected', code: 'REFERENCE_BROKEN' })
  const note = await owner.client.commit(ws, [{
    op: 'entity.create', entity_id: 'private-note', type_id: 'buckyos.annotation', parent_id: 'data', order_key: 'zz',
    scope: 'personal', payload: { target: { entity_id: 'notes' }, kind: 'note', body: 'private-synthetic-comment' },
  }])
  expect(note.status).toBe('accepted')
  const annotations = await outsider.client.ok<{ annotations: EntityEnvelope[] }>('doc.list_annotations', { workspace_id: ws.workspace_id, target_ids: ['notes'] })
  expect(JSON.stringify(annotations)).not.toContain('private-synthetic-comment')
  const history = await outsider.client.ok<{ changes: CommitEvent[] }>('doc.get_changes', { workspace_id: ws.workspace_id, epoch: ws.epoch, after_seq: 0 })
  expect(JSON.stringify(history)).not.toContain('private-synthetic-comment')
  const exported = await outsider.client.ok<ExportResult>('doc.export', { workspace_id: ws.workspace_id, mode: 'share' })
  expect(exported.manifest.excluded_entities).toBeGreaterThan(0)
  const download = await outsider.page.evaluate(async (item) => {
    const url = new URL('/bucket/v1/' + item.bucket_id + '/' + item.key.split('/').map(encodeURIComponent).join('/'), location.origin)
    url.searchParams.set('version', String(item.version))
    url.searchParams.set('token', item.token)
    const response = await fetch(url)
    return { status: response.status, bytes: Array.from(new Uint8Array(await response.arrayBuffer())) }
  }, exported.download)
  expect(download.status).toBe(200)
  const bytes = new Uint8Array(download.bytes)
  for (const entry of zipEntries(bytes).values()) {
    const text = new TextDecoder().decode(await zipRead(bytes, entry))
    expect(text, entry.name).not.toContain('private-synthetic-comment')
    expect(text, entry.name).not.toContain('private-synthetic-source')
  }
  const published = await owner.client.ok('ws.publish', { workspace_id: ws.workspace_id, entity_id: 'tasks', name: 'synthetic-public' })
  expect(published).toMatchObject({ entity_id: 'tasks' })
  const anonymous = await outsider.page.evaluate(async (workspaceId) => {
    const responses = []
    for (const name of ['synthetic-public', 'notes']) {
      const response = await fetch('/kapi/aiworkspace/pub/' + workspaceId + '/' + name, { credentials: 'omit', headers: { Accept: 'application/json' } })
      responses.push({ status: response.status, body: await response.text() })
    }
    return responses
  }, ws.workspace_id)
  expect(anonymous[0].status).toBe(200)
  expect(anonymous[0].body).not.toContain('private-synthetic-source')
  expect(anonymous[0].body).not.toContain('private-synthetic-comment')
  expect(anonymous[1].status).toBe(404)
})

test('real comment-only accounts annotate without editing source content and history preserves the actor', async ({ owner, participants }, info) => {
  const { editor, commenter } = participants
  const ws = await createSample(owner)
  await grant(owner, ws, editor, ['read', 'update', 'comment'])
  await grant(owner, ws, commenter, ['read', 'comment'])
  await openWorkspace(commenter, ws)
  const before = await owner.client.cell(ws, 'task-41', 'title')
  const head = (await owner.client.info(ws)).head_seq
  const denied: Operation[] = [{
    op: 'table.set_values', source_id: 'tasks',
    values: [{ record_id: 'task-41', field_id: 'title', value: 'must-not-persist', expect: { rev: before.rev } }],
  }]
  expect(await commenter.client.commit(ws, denied)).toMatchObject({ status: 'rejected', code: 'PERMISSION_DENIED' })
  const notes = await read(owner, ws, 'notes')
  expect(await commenter.client.commit(ws, [{ op: 'entity.delete', entity_id: 'notes', expect: { rev: notes.life_rev } }])).toMatchObject({ status: 'rejected', code: 'PERMISSION_DENIED' })
  expect(await owner.client.cell(ws, 'task-41', 'title')).toEqual(before)
  expect((await owner.client.info(ws)).head_seq).toBe(head)
  const created = { entity_id: 'comment-only-feedback' }
  expect(await commenter.client.commit(ws, [{
    op: 'entity.create', entity_id: created.entity_id, type_id: 'buckyos.annotation', parent_id: 'data', order_key: 'zz',
    payload: { target: { entity_id: 'tasks' }, kind: 'note', body: 'synthetic-comment-only-feedback' },
  }])).toMatchObject({ status: 'accepted' })
  const stored = await read<AnnotationRead>(owner, ws, created.entity_id)
  expect(stored.content.payload.author).toBe(commenter.userId)
  const changeName = await commenter.client.commit(ws, [{
    op: 'entity.set_keys', entity_id: created.entity_id,
    keys: [{ key: 'author', value: editor.userId, expect: { rev: stored.content.key_revs.author ?? 0 } }],
  }])
  expect(changeName.status).toBe('rejected')
  const changed = await commenter.client.commit(ws, [{
    op: 'entity.set_keys', entity_id: created.entity_id,
    keys: [{ key: 'body', value: 'synthetic-feedback-edited', expect: { rev: stored.content.key_revs.body ?? 0 } }],
  }])
  expect(changed.status).toBe('accepted')
  const history = await owner.client.ok<{ changes: CommitEvent[] }>('doc.get_changes', { workspace_id: ws.workspace_id, epoch: ws.epoch, after_seq: head })
  expect(history.changes).toHaveLength(2)
  for (const event of history.changes) {
    expect(event.author).toBe(commenter.userId)
    expect(event.accepted_at).toBeTruthy()
    expect(event.ops?.some(op => op.entity_id === created.entity_id)).toBe(true)
  }
  const audit = await owner.client.ok<{ items: unknown[] }>('acl.audit', { workspace_id: ws.workspace_id })
  expect(JSON.stringify(audit)).toContain(commenter.userId)
  await info.attach('comment-api-verification', { body: Buffer.from(JSON.stringify({
    sourceUnchanged: await owner.client.cell(ws, 'task-41', 'title'),
    immutableAuthor: (await read<AnnotationRead>(owner, ws, created.entity_id)).content.payload.author,
    attributedCommits: history.changes.map(event => ({ author: event.author, accepted_at: event.accepted_at, seq: event.seq })),
  })), contentType: 'application/json' })
  console.log('Comment-only API verified: source writes refused, own feedback editable, author immutable, both commits attributed')
  await setCanvasMode(commenter.page, 'edit')
  await commenter.page.getByTestId('aiws-table-cell-all-tasks').getByTestId('aiws-cell-task-41-owner').hover()
  await commenter.page.getByLabel('评论 负责人 task-41', { exact: true }).click()
  await commenter.page.getByLabel('评论内容').fill('synthetic-comment-only-ui-feedback')
  await commenter.page.getByTestId('aiws-annotation-save').click()
  await expectAllCommitted(commenter.page)
  await screenshot(commenter.page, info, 'comment-only-account-feedback')
})

test('two real editor accounts retain independent edits and both inputs at a same-field conflict', async ({ owner, participants }, info) => {
  const { editor: first, outsider: second } = participants
  const ws = await createSample(owner)
  for (const actor of [first, second]) await grant(owner, ws, actor, ['read', 'update', 'comment', 'append'])
  await openWorkspace(first, ws)
  await openWorkspace(second, ws)
  for (const actor of [first, second]) {
    await actor.page.getByTestId('aiws-richtext-notes').locator('[data-block-id="n-end"]').click()
    await actor.page.keyboard.press('End')
  }
  await Promise.all([first.page.keyboard.insertText('【first-editor】'), second.page.keyboard.insertText('【second-editor】')])
  for (const actor of [first, second]) {
    await expectAllCommitted(actor.page)
    await expect(actor.page.getByTestId('aiws-richtext-notes')).toContainText('【first-editor】')
    await expect(actor.page.getByTestId('aiws-richtext-notes')).toContainText('【second-editor】')
  }
  const edit = async (page: Page, record: string, field: string, label: string, value: string) => {
    await page.getByTestId('aiws-table-cell-all-tasks').getByTestId(`aiws-cell-${record}-${field}`).getByRole('button').first().click()
    const input = page.getByLabel(`${label} ${record}`, { exact: true })
    await input.fill(value)
    return input
  }
  const ownerInput = await edit(first.page, 'task-42', 'owner', '负责人', 'first-value')
  const budgetInput = await edit(second.page, 'task-42', 'budget', '预算', '1888')
  await Promise.all([ownerInput.press('Enter'), budgetInput.press('Enter')])
  await expectAllCommitted(first.page)
  await expectAllCommitted(second.page)
  expect((await first.client.cell(ws, 'task-42', 'owner')).value).toBe('first-value')
  expect((await second.client.cell(ws, 'task-42', 'budget')).value).toBe('1888.00')
  const firstTitle = await edit(first.page, 'task-41', 'title', '任务', 'first-title')
  const secondTitle = await edit(second.page, 'task-41', 'title', '任务', 'second-title')
  await firstTitle.press('Enter')
  await expectAllCommitted(first.page)
  await secondTitle.press('Enter')
  const cell = second.page.getByTestId('aiws-table-cell-all-tasks').getByTestId('aiws-cell-task-41-title')
  await expect(cell).toHaveAttribute('data-state', 'needs_attention')
  await expect(cell.getByTestId('aiws-conflict-theirs')).toHaveText('first-title')
  await expect(cell.getByTestId('aiws-conflict-mine')).toHaveText('second-title')
  expect((await first.client.cell(ws, 'task-41', 'title')).value).toBe('first-title')
  await screenshot(second.page, info, 'two-real-accounts-conflict')
})
