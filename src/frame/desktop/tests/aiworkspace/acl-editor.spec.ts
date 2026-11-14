/* UI 框架 PRD §8 (acceptance §15 #24–#27): the Workspace's permission editor in the data mode — logical groups, members,
 * rules by path with the path help and the object picker, "check permissions" — the by-entity view of one object with
 * refusals and what they override, and what someone without `manage` sees. The decisions themselves are asserted
 * through kRPC (`acl.query`, reads and commits as the member). Plus the phone layout: one screen per part, the rule
 * editor as a bottom sheet. */

import { devices, type Page } from '@playwright/test'
import { expect, openWorkspace, openWorkspacePage, test } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'
const SALES = '/data/原始销售数据'

/** The data mode with the Workspace's permission editor open. */
async function openEditor(page: Page, token: string, workspaceId: string) {
  await openWorkspace(page, token, workspaceId)
  await page.getByTestId('aiws-top-sources').click()
  await page.getByTestId('aiws-dm-perm-node').click()
  await expect(page.getByTestId('aiws-sources').getByTestId('aiws-permissions')).toBeVisible()
}

/** One part of the editor (a tab, or a screen of its own on a narrow panel). */
async function part(page: Page, name: 'groups' | 'rules' | 'check' | 'audit' | 'hosted') {
  const editor = page.getByTestId('aiws-permissions').first()
  for (let i = 0; i < 3 && (await editor.getAttribute('data-narrow')) === 'true' && !['home', name].includes((await editor.getAttribute('data-part')) ?? ''); i++) {
    await editor.getByTestId('aiws-perm-back').click()
  }
  await editor.getByTestId(`aiws-perm-tab-${name}`).click()
  await expect(editor).toHaveAttribute('data-part', name)
}

test('§15-24 a new logical group with a member, read-only on /data/原始销售数据: the member reads and cannot change it', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `acl24 ${Date.now()}`)
  await openEditor(page, ALICE, ws.workspace_id)
  const editor = page.getByTestId('aiws-permissions')
  await expect(editor).toHaveAttribute('data-complete', 'true')
  // the fixed groups come first, with what they mean; Guests say they are not in effect while anonymous access is off
  const groups = editor.getByTestId('aiws-perm-groups')
  await expect(groups.getByTestId('aiws-perm-group').first()).toHaveAttribute('data-subject', '@owner')
  await expect(groups.locator('[data-testid="aiws-perm-group"][data-subject="@guest"]').getByTestId('aiws-perm-guest-inactive')).toContainText('当前不生效')
  await expect(groups.getByTestId('aiws-perm-guest-enable')).toBeVisible()

  // a new group: id, title; then its page
  await editor.getByTestId('aiws-perm-group-new').click()
  await editor.getByTestId('aiws-perm-group-id').fill('Bad Id')
  await expect(editor.getByTestId('aiws-perm-group-save')).toBeDisabled()
  await editor.getByTestId('aiws-perm-group-id').fill('sales-readers')
  await editor.getByTestId('aiws-perm-group-title').fill('销售只读')
  await editor.getByTestId('aiws-perm-group-save').click()
  const detail = editor.locator('[data-testid="aiws-perm-group-detail"][data-subject="@sales-readers"]')
  await expect(detail).toContainText('销售只读')
  // a member from the candidates; a group cannot be a member of a group
  await detail.getByTestId('aiws-perm-member-choice').selectOption('__typed')
  await detail.getByTestId('aiws-perm-member-input').fill('@all')
  await expect(detail.getByTestId('aiws-perm-member-problem')).toContainText('组不能作为另一个组的成员')
  await expect(detail.getByTestId('aiws-perm-member-submit')).toBeDisabled()
  await detail.getByTestId('aiws-perm-member-choice').selectOption('bob')
  await detail.getByTestId('aiws-perm-member-submit').click()
  await expect(detail.locator('[data-testid="aiws-perm-member"][data-member="bob"]').getByTestId('aiws-perm-member-kind')).toHaveText('用户')
  // its rule: read on the sales table, the path typed and checked as it is typed
  await detail.getByTestId('aiws-perm-group-rule-add').click()
  const rule = page.getByTestId('aiws-perm-rule-editor')
  await expect(rule.getByTestId('aiws-perm-rule-subject')).toHaveText('销售只读')
  await rule.getByTestId('aiws-perm-path').fill(SALES)
  await expect(rule.getByTestId('aiws-perm-path-check')).toHaveAttribute('data-state', 'ok')
  await expect(rule.getByTestId('aiws-perm-path-check')).toContainText('原始销售数据')
  await rule.getByTestId('aiws-perm-preset-reader').click()
  await expect(rule.getByTestId('aiws-perm-cap-read')).toHaveAttribute('data-state', 'allow')
  await expect(rule.getByTestId('aiws-perm-cap-update')).toHaveAttribute('data-state', 'unset')
  await rule.getByTestId('aiws-perm-rule-save').click()
  await expect(rule).toHaveCount(0)
  const row = detail.getByTestId('aiws-perm-group-rules').locator(`[data-testid="aiws-perm-rule"][data-resource="${SALES}"]`)
  await expect(row.getByTestId('aiws-perm-rule-allow').locator('[data-cap="read"]')).toBeVisible()
  await expect(detail.getByTestId('aiws-perm-group-rules').getByTestId('aiws-perm-rule')).toHaveCount(1)
  // the group in the list: one member, one rule
  await expect(groups.locator('[data-testid="aiws-perm-group"][data-subject="@sales-readers"]').getByTestId('aiws-perm-group-counts')).toHaveText('1 个成员 · 1 条规则')

  // what the service holds: the group, bob in it, its entry by entity id
  const all = await api.rpc(ALICE, 'acl.query', { workspace_id: ws.workspace_id, by: 'all' })
  expect(all.entries.filter((e: { subject: string }) => e.subject === '@sales-readers').map((e: { resource: { entity_id: string }; allow: string[]; deny: string[] }) => [e.resource.entity_id, e.allow, e.deny])).toEqual([['sales', ['read'], []]])
  // bob reads the table, nothing else, and cannot change it
  expect((await api.rpc(BOB, 'doc.read', { workspace_id: ws.workspace_id, entity_id: 'sales' })).ok).toBe(true)
  expect((await api.rpc(BOB, 'doc.read', { workspace_id: ws.workspace_id, entity_id: 'intro' })).ok).toBe(false)
  const rev = (await api.cell(ALICE, ws.workspace_id, 'sales', 's-1', 'cost')).rev
  const edit = await api.commit(BOB, ws, [{ op: 'table.set_values', source_id: 'sales', values: [{ record_id: 's-1', field_id: 'cost', value: 1, expect: { rev } }] }])
  expect(edit.status).toBe('rejected')
  expect(edit.code).toBe('PERMISSION_DENIED')

  // "check permissions": each capability and the entry deciding it, with the order of a decision in words
  await part(page, 'check')
  const check = editor.getByTestId('aiws-perm-check')
  await expect(check.getByTestId('aiws-perm-order')).toContainText('离对象最近')
  await check.getByTestId('aiws-perm-check-who').selectOption('bob')
  await check.getByTestId('aiws-perm-check-path').fill(SALES)
  await check.getByTestId('aiws-perm-check-run').click()
  const result = check.getByTestId('aiws-perm-check-result')
  await expect(result).toHaveAttribute('data-principal', 'bob')
  await expect(result.getByTestId('aiws-perm-judgment-groups')).toContainText('销售只读')
  await expect(result.locator('[data-testid="aiws-perm-explain-cap"][data-cap="read"]')).toHaveAttribute('data-allowed', 'true')
  await expect(result.locator('[data-testid="aiws-perm-explain-cap"][data-cap="read"]').getByTestId('aiws-perm-explain-by')).toContainText('销售只读')
  await expect(result.locator('[data-testid="aiws-perm-explain-cap"][data-cap="update"]')).toHaveAttribute('data-allowed', 'false')
  await expect(result.locator('[data-testid="aiws-perm-explain-cap"][data-cap="update"]').getByTestId('aiws-perm-explain-by')).toHaveText('没有规则提到它')
  // the operation record sits beside the editor (the data mode's right column)
  const audit = page.getByTestId('aiws-sources').getByTestId('aiws-perm-audit')
  for (const action of ['acl.create_group', 'acl.members', 'acl.set']) await expect(audit.locator(`[data-testid="aiws-perm-audit-item"][data-action="${action}"]`).first()).toBeVisible()
})

test('§15-25 adding a rule: a path that names nothing is flagged at once; completion and "choose from the tree" fill it', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `acl25 ${Date.now()}`)
  const outline = await api.outline(ALICE, ws.workspace_id)
  const intro = outline.find((e: { entity_id: string }) => e.entity_id === 'intro')
  await openEditor(page, ALICE, ws.workspace_id)
  await part(page, 'rules')
  const editor = page.getByTestId('aiws-permissions')
  await editor.getByTestId('aiws-perm-rule-add').click()
  const rule = page.getByTestId('aiws-perm-rule-editor')
  // the examples are always shown
  await expect(rule.getByTestId('aiws-perm-path-field')).toContainText('/surfaces/看板/text-3')
  await rule.getByTestId('aiws-perm-subject').selectOption('bob')
  // a member spec is no subject
  await rule.getByTestId('aiws-perm-subject').selectOption('__other')
  await rule.getByTestId('aiws-perm-subject-input').fill('group:teachers')
  await expect(rule.getByTestId('aiws-perm-subject-problem')).toContainText('成员来源')
  await rule.getByTestId('aiws-perm-subject-input').fill('bob')
  await expect(rule.getByTestId('aiws-perm-subject-problem')).toHaveCount(0)
  // nothing there
  await rule.getByTestId('aiws-perm-path').fill('/data/没有这个')
  await expect(rule.getByTestId('aiws-perm-path-check')).toHaveAttribute('data-state', 'error')
  await expect(rule.getByTestId('aiws-perm-path-check')).toContainText('找不到')
  await rule.getByTestId('aiws-perm-preset-reader').click()
  await expect(rule.getByTestId('aiws-perm-rule-save')).toBeDisabled()
  // completion one level at a time
  await rule.getByTestId('aiws-perm-path').fill('/data/原')
  await rule.locator(`[data-testid="aiws-perm-path-suggestion"][data-path="${SALES}"]`).click()
  await expect(rule.getByTestId('aiws-perm-path')).toHaveValue(SALES)
  await expect(rule.getByTestId('aiws-perm-path-check')).toHaveAttribute('data-state', 'ok')
  // the object picker: the tree, then the path of what is picked
  await rule.getByTestId('aiws-perm-path-pick').click()
  const picker = page.getByTestId('aiws-perm-picker')
  await expect(picker.locator('[data-testid="aiws-perm-picker-item"][data-entity-id="root"]')).toBeVisible()
  await picker.locator('[data-testid="aiws-perm-picker-item"][data-entity-id="intro"]').click()
  await expect(picker).toHaveCount(0)
  await expect(rule.getByTestId('aiws-perm-path')).toHaveValue(`/data/${intro.name}`)
  await expect(rule.getByTestId('aiws-perm-path-check')).toContainText(intro.title ?? intro.name)
  // the search of the picker finds by name
  await rule.getByTestId('aiws-perm-path-pick').click()
  await picker.getByTestId('aiws-perm-picker-search').fill('原始销售')
  await picker.locator('[data-testid="aiws-perm-picker-item"][data-entity-id="sales"]').click()
  await expect(rule.getByTestId('aiws-perm-path')).toHaveValue(SALES)
  await rule.getByTestId('aiws-perm-preset-commenter').click()
  await rule.getByTestId('aiws-perm-rule-save').click()
  await expect(rule).toHaveCount(0)
  const row = editor.locator('[data-testid="aiws-perm-rule"][data-subject="bob"][data-entity-id="sales"]')
  await expect(row.getByTestId('aiws-perm-rule-allow').locator('[data-cap]')).toHaveCount(2)
  // a canvas: its own data folder gets the same rule unless unticked
  await editor.getByTestId('aiws-perm-rule-add').click()
  await rule.getByTestId('aiws-perm-subject').selectOption('bob')
  await rule.getByTestId('aiws-perm-path').fill('/surfaces/数据明细')
  await expect(rule.getByTestId('aiws-perm-rule-pair')).toBeChecked()
  await rule.getByTestId('aiws-perm-preset-editor').click()
  await rule.getByTestId('aiws-perm-rule-save').click()
  await expect(rule).toHaveCount(0)
  const bobs = (await api.rpc(ALICE, 'acl.query', { workspace_id: ws.workspace_id, by: 'all' })).entries.filter((e: { subject: string }) => e.subject === 'bob').map((e: { resource: { entity_id: string } }) => e.resource.entity_id).sort()
  expect(bobs).toEqual(['sales', 'sf-detail', 'sf-detail-content'])
  // filters: by path prefix, only rules with a refusal
  await editor.getByTestId('aiws-perm-filter-path').fill('/surfaces')
  await expect(editor.getByTestId('aiws-perm-rule')).toHaveCount(1)
  await editor.getByTestId('aiws-perm-filter-path').fill('')
  await editor.getByTestId('aiws-perm-filter-deny').check()
  await expect(editor.getByTestId('aiws-perm-rules-empty')).toBeVisible()
})

test('§15-26 the permissions of one table: a user added without a path; a group refused, and the allowance it overrides is marked', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `acl26 ${Date.now()}`)
  // everyone who may enter reads the workspace; bob is in a group that the table will refuse
  const set = async (method: string, params: Record<string, unknown>) => expect((await api.rpc(ALICE, method, { workspace_id: ws.workspace_id, ...params })).ok).toBe(true)
  await set('acl.set_group', { group_id: 'nosales', title: '不看销售' })
  await set('acl.members', { group_id: 'nosales', add: ['bob'] })
  await set('acl.set', { changes: [{ subject: '@all', resource: '/', allow: ['read'] }] })
  await openWorkspace(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-top-sources').click()
  await page.getByTestId('aiws-tree-sales').click()
  await page.getByTestId('aiws-dm-side-tab-permissions').click()
  const view = page.locator('[data-testid="aiws-perm-entity"][data-entity-id="sales"]')
  await expect(view.locator('[data-testid="aiws-perm-entity-subject"][data-subject="@owner"]')).toBeVisible()
  const everyone = view.locator('[data-testid="aiws-perm-entity-subject"][data-subject="@all"]')
  await expect(everyone.getByTestId('aiws-perm-entity-entry')).toHaveAttribute('data-source', 'workspace')
  // add a user: the resource is already this table, no path to type
  await view.getByTestId('aiws-perm-entity-add').click()
  const rule = page.getByTestId('aiws-perm-rule-editor')
  await expect(rule.getByTestId('aiws-perm-rule-resource')).toContainText('原始销售数据')
  await expect(rule.getByTestId('aiws-perm-path')).toHaveCount(0)
  await rule.getByTestId('aiws-perm-subject').selectOption('bob')
  await rule.getByTestId('aiws-perm-preset-reader').click()
  await rule.getByTestId('aiws-perm-cap-comment-allow').click()
  await rule.getByTestId('aiws-perm-rule-save').click()
  const bob = view.locator('[data-testid="aiws-perm-entity-subject"][data-subject="bob"]')
  await expect(bob.getByTestId('aiws-perm-entity-entry')).toHaveAttribute('data-source', 'direct')
  await expect(bob.getByTestId('aiws-perm-entity-effective').locator('[data-cap]')).toHaveCount(2)
  // refuse reading to the group on this table: its refusal overrides what Everyone is allowed from the workspace
  await view.getByTestId('aiws-perm-entity-add').click()
  await rule.getByTestId('aiws-perm-subject').selectOption('@nosales')
  await rule.getByTestId('aiws-perm-cap-read-deny').click()
  await expect(rule.getByTestId('aiws-perm-cap-read')).toHaveAttribute('data-state', 'deny')
  await rule.getByTestId('aiws-perm-rule-save').click()
  const refused = view.locator('[data-testid="aiws-perm-entity-subject"][data-subject="@nosales"]')
  await expect(refused.getByTestId('aiws-perm-entity-entry').locator('[data-state="deny"][data-cap="read"]')).toBeVisible()
  await expect(refused.locator('[data-testid="aiws-perm-override"][data-subject="@all"]')).toContainText('所有人')
  await expect(everyone.locator('[data-testid="aiws-perm-overridden-by"][data-subject="@nosales"]')).toContainText('不看销售')
  // bob's own rule is more specific than his group's refusal: he still reads (§4.6)
  const explain = await api.rpc(ALICE, 'acl.query', { workspace_id: ws.workspace_id, by: 'explain', principal: 'bob', resource: 'sales' })
  expect(explain.effective).toEqual(['read', 'comment'])
  // a refusal stays visible and editable: changing bob's rule keeps it
  await bob.getByTestId('aiws-perm-entity-edit').click()
  await rule.getByTestId('aiws-perm-cap-comment-unset').click()
  await rule.getByTestId('aiws-perm-cap-delete-deny').click()
  await rule.getByTestId('aiws-perm-rule-save').click()
  await expect(bob.getByTestId('aiws-perm-entity-entry').locator('[data-state="deny"][data-cap="delete"]')).toBeVisible()
  const bobEntry = (await api.rpc(ALICE, 'acl.query', { workspace_id: ws.workspace_id, by: 'all' })).entries.find((e: { subject: string }) => e.subject === 'bob')
  expect([bobEntry.allow, bobEntry.deny]).toEqual([['read'], ['delete']])
  // clearing both is a deletion
  await bob.getByTestId('aiws-perm-entity-edit').click()
  await rule.getByTestId('aiws-perm-cap-read-unset').click()
  await rule.getByTestId('aiws-perm-cap-delete-unset').click()
  await expect(rule.getByTestId('aiws-perm-rule-removing')).toBeVisible()
  await rule.getByTestId('aiws-perm-rule-save').click()
  await expect(bob).toHaveCount(0)
})

test('§15-27 without manage the permission node shows only one’s own permissions, and follows changes as they happen', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `acl27 ${Date.now()}`)
  expect((await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: 'bob', resource: '/', allow: ['read'] }] })).ok).toBe(true)
  await openEditor(page, BOB, ws.workspace_id)
  const editor = page.getByTestId('aiws-sources').getByTestId('aiws-permissions')
  await expect(editor).toHaveAttribute('data-complete', 'false')
  const mine = editor.getByTestId('aiws-perm-mine')
  await expect(mine).toContainText('我的权限')
  await expect(mine.getByTestId('aiws-perm-judgment-groups')).toContainText('不属于这个工作区的任何组')
  await expect(mine.locator('[data-testid="aiws-perm-explain-cap"][data-cap="read"]')).toHaveAttribute('data-allowed', 'true')
  await expect(mine.locator('[data-testid="aiws-perm-explain-cap"][data-cap="update"]')).toHaveAttribute('data-allowed', 'false')
  for (const id of ['aiws-perm-group-new', 'aiws-perm-rule-add', 'aiws-perm-tab-rules', 'aiws-hosted-save']) await expect(page.getByTestId(id)).toHaveCount(0)
  await expect(page.getByTestId('aiws-sources').getByTestId('aiws-perm-audit')).toHaveCount(0)
  // the owner gives bob more (another window): his view follows the permissions' revision, no reload
  expect((await api.rpc(ALICE, 'acl.set_group', { workspace_id: ws.workspace_id, group_id: 'editors', title: '编辑' })).ok).toBe(true)
  expect((await api.rpc(ALICE, 'acl.members', { workspace_id: ws.workspace_id, group_id: 'editors', add: ['bob'] })).ok).toBe(true)
  expect((await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: '@editors', resource: '/', allow: ['update'] }] })).ok).toBe(true)
  await expect(mine.locator('[data-testid="aiws-perm-explain-cap"][data-cap="update"]')).toHaveAttribute('data-allowed', 'true', { timeout: 30_000 })
  await expect(mine.getByTestId('aiws-perm-judgment-groups')).toContainText('编辑')
  // one object: its "permissions" tab is "my permissions" too
  await page.getByTestId('aiws-tree-sales').click()
  await page.getByTestId('aiws-dm-side-tab-permissions').click()
  const onSales = page.locator('[data-testid="aiws-perm-mine"][data-entity-id="sales"]')
  await expect(onSales.locator('[data-testid="aiws-perm-explain-cap"][data-cap="read"]').getByTestId('aiws-perm-explain-by')).toContainText('工作区级')
  await expect(page.getByTestId('aiws-perm-entity-add')).toHaveCount(0)
})

test('§8.5 share entries lead into a chosen group, with an expiry and a limit; the editor shows their state and opens the share dialog', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `acl-entries ${Date.now()}`)
  expect((await api.rpc(ALICE, 'acl.set_group', { workspace_id: ws.workspace_id, group_id: 'visitors', title: '来访者' })).ok).toBe(true)
  expect((await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: '@visitors', resource: '/', allow: ['read'] }] })).ok).toBe(true)
  await openEditor(page, ALICE, ws.workspace_id)
  const editor = page.getByTestId('aiws-permissions')
  await expect(editor.getByTestId('aiws-perm-anonymous')).toHaveAttribute('data-on', 'false')
  await expect(editor.getByTestId('aiws-perm-share-entries')).toHaveAttribute('data-count', '0')
  await editor.getByTestId('aiws-perm-open-share').click()
  await page.getByTestId('aiws-share-tab-entries').click()
  const entries = page.getByTestId('aiws-share-entries')
  await entries.getByTestId('aiws-share-entry-preset').selectOption('@visitors')
  await entries.getByTestId('aiws-share-entry-expires').selectOption('7')
  await entries.getByTestId('aiws-share-entry-max').fill('0')
  await expect(entries.getByTestId('aiws-share-entry-create')).toBeDisabled()
  await entries.getByTestId('aiws-share-entry-max').fill('5')
  const before = Math.floor(Date.now() / 1000)
  await entries.getByTestId('aiws-share-entry-create').click()
  const row = entries.getByTestId('aiws-share-entry')
  await expect(row).toHaveAttribute('data-group', 'visitors')
  await expect(row.getByTestId('aiws-share-entry-group')).toContainText('来访者')
  await expect(row.getByTestId('aiws-share-entry-limits')).toContainText('已加入 0 / 5 人')
  const listed = (await api.rpc(ALICE, 'ws.list_entries', { workspace_id: ws.workspace_id })).entries[0]
  expect([listed.group_id, listed.max_joins]).toEqual(['visitors', 5])
  expect(listed.expires_at - before).toBeGreaterThanOrEqual(7 * 86400 - 5)
  expect(listed.expires_at - before).toBeLessThanOrEqual(7 * 86400 + 60)
  // whoever joins is a member of the group, marked with the entry; closing can take them out again
  expect((await api.rpc(BOB, 'ws.join_entry', { entry_id: listed.entry_id })).group_id).toBe('visitors')
  await page.keyboard.press('Escape')
  await expect(editor.getByTestId('aiws-perm-share-entries')).toHaveAttribute('data-count', '1')
  await editor.locator('[data-testid="aiws-perm-group"][data-subject="@visitors"] button').first().click()
  await expect(editor.locator('[data-testid="aiws-perm-member"][data-member="bob"]').getByTestId('aiws-perm-member-kind')).toContainText('经分享入口')
  await editor.getByTestId('aiws-perm-open-share').click()
  await page.getByTestId('aiws-share-tab-entries').click()
  await entries.getByTestId('aiws-share-entry-close-remove').check()
  await entries.getByTestId('aiws-share-entry-close').click()
  await expect(entries.getByTestId('aiws-share-entry')).toHaveCount(0)
  await expect(entries.getByTestId('aiws-share-entry-message')).toContainText('移除了经它加入的 1 人')
  await page.keyboard.press('Escape')
  await expect(editor.locator('[data-testid="aiws-perm-member"][data-member="bob"]')).toHaveCount(0)
})

test('§8.1 a Block\'s "权限…" opens the side drawer without leaving the canvas: the data it shows, the Block beside it', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `acl-drawer ${Date.now()}`)
  await openWorkspace(page, ALICE, ws.workspace_id)
  await expect(page.getByTestId('aiws-canvas')).toBeVisible()
  await page.getByTestId('aiws-canvas-block-blk-sales').click({ button: 'right' })
  const menu = page.getByTestId('aiws-context-menu')
  const group = menu.getByTestId('aiws-menu-group-relate')
  if (await group.count()) await group.click()
  await page.getByTestId('aiws-menu-permissions').click()
  const drawer = page.getByTestId('aiws-perm-drawer')
  await expect(drawer).toHaveAttribute('data-entity-id', 'sales')
  await expect(page.getByTestId('aiws-canvas')).toBeVisible()
  await expect(drawer.locator('[data-testid="aiws-perm-entity"][data-entity-id="sales"]')).toBeVisible()
  await drawer.locator('[data-testid="aiws-perm-drawer-target"][data-entity-id="blk-sales"]').click()
  await expect(drawer.locator('[data-testid="aiws-perm-entity"][data-entity-id="blk-sales"]')).toBeVisible()
  await drawer.getByTestId('aiws-perm-drawer-close').click()
  await expect(drawer).toHaveCount(0)
})

test.describe('phone', () => {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars -- the browser comes from the project, the rest is the phone
  const { defaultBrowserType, ...pixel } = devices['Pixel 7']
  test.use(pixel)

  test('§8.5 on a phone: groups, a group and the rules are screens of their own; a rule is edited in a bottom sheet, its path picked from the tree', async ({ page, api }) => {
    const ws = await api.demo(ALICE, 'quarterly', `acl-phone ${Date.now()}`)
    expect((await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: 'bob', resource: 'sales', allow: ['read'], deny: ['delete'] }] })).ok).toBe(true)
    await openWorkspacePage(page, ALICE, ws.workspace_id)
    await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
    // the phone's main menu → 协作 → 成员与权限…: the data mode with the permission editor full screen (UI 框架 PRD §5.6, §8.1)
    await page.getByTestId('aiws-main-menu').click()
    await page.getByTestId('aiws-menu-collab').click()
    await page.getByTestId('aiws-menu-members').click()
    const editor = page.getByTestId('aiws-permissions')
    await expect(editor).toHaveAttribute('data-narrow', 'true')
    await expect(editor).toHaveAttribute('data-part', 'home')
    await part(page, 'groups')
    await editor.locator('[data-testid="aiws-perm-group"][data-subject="@all"] button').first().click()
    await expect(editor.locator('[data-testid="aiws-perm-group-detail"][data-subject="@all"]')).toBeVisible()
    await expect(editor.getByTestId('aiws-perm-groups')).toHaveCount(0)
    // a rule for Everyone: the path is chosen from the tree (the first choice on a phone), the capabilities in a sheet
    await editor.getByTestId('aiws-perm-group-rule-add').click()
    const sheet = page.getByTestId('aiws-perm-rule-editor')
    await expect(sheet).toHaveClass(/aiws-perm-sheet/)
    await expect(sheet.getByTestId('aiws-perm-cap-manage-allow')).toBeDisabled()
    await expect(sheet.getByTestId('aiws-perm-cap-run-allow')).toBeDisabled()
    const target = (await sheet.getByTestId('aiws-perm-cap-read-allow').boundingBox())!
    expect(target.height).toBeGreaterThanOrEqual(44)
    await sheet.getByTestId('aiws-perm-path-pick').click()
    await page.getByTestId('aiws-perm-picker').locator('[data-testid="aiws-perm-picker-item"][data-entity-id="intro"]').click()
    await sheet.getByTestId('aiws-perm-preset-reader').click()
    await sheet.getByTestId('aiws-perm-rule-save').click()
    await expect(sheet).toHaveCount(0)
    await expect(editor.getByTestId('aiws-perm-group-rules').locator('[data-testid="aiws-perm-rule"][data-entity-id="intro"]')).toBeVisible()
    // the rules: cards that fit the screen, refusals in their own colour
    await part(page, 'rules')
    const card = editor.locator('[data-testid="aiws-perm-rule"][data-subject="bob"]')
    await expect(card.getByTestId('aiws-perm-rule-deny').locator('[data-state="deny"][data-cap="delete"]')).toBeVisible()
    const box = (await card.boundingBox())!
    const viewport = page.viewportSize()!
    expect(box.x + box.width).toBeLessThanOrEqual(viewport.width + 0.5)
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(viewport.width + 1)
  })
})
